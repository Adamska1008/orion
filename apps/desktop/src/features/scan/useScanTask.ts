import { useEffect, useRef, useState } from 'react';
import { Api, ApiError, type Task } from '../../lib/api';
import { errorText } from '../../lib/desktop';

export const active = (task: Task | null) => task?.status === 'running' || task?.status === 'cancelling';

interface Session {
  api: Api;
  controller: AbortController;
  generation: number;
  mutating: boolean;
  instance: string | null;
}

export function useScanTask(api: Api | null) {
  const [task, setTask] = useState<Task | null>(null);
  const [online, setOnline] = useState(false);
  const [busy, setBusy] = useState(false);
  const [connectionError, setConnectionError] = useState('');
  const [message, setMessage] = useState('');
  const [notice, setNotice] = useState('');
  const session = useRef<Session | null>(null);
  const previousInstance = useRef<string | null>(null);
  const pendingRequest = useRef<{ root: string; id: string; instance: string | null } | null>(null);

  function accept(next: Task | null) {
    setTask(current => current && next && current.id === next.id && current.revision > next.revision ? current : next);
  }

  useEffect(() => {
    setOnline(false); setBusy(false); setMessage(''); setConnectionError('');
    if (!api) return;
    const current: Session = { api, controller: new AbortController(), generation: 0, mutating: false, instance: null };
    session.current = current;
    let timer: ReturnType<typeof setTimeout>;
    const alive = () => session.current === current && !current.controller.signal.aborted;
    async function poll() {
      const generation = current.generation;
      const valid = () => alive() && generation === current.generation && !current.mutating;
      try {
        const health = await current.api.health(current.controller.signal);
        if (health.name !== 'orion-server' || health.api_version !== 1) throw new Error('服务版本不兼容。');
        const tasks = await current.api.tasks(current.controller.signal);
        if (!valid()) return;
        if (previousInstance.current && previousInstance.current !== health.instance_id) {
          setNotice('服务已重启或切换，之前的内存结果不再可用。');
          pendingRequest.current = null;
        }
        current.instance = health.instance_id;
        previousInstance.current = health.instance_id;
        setOnline(true); setConnectionError(''); accept(tasks[0] ?? null);
      } catch (cause) {
        if (valid()) { setOnline(false); setConnectionError(errorText(cause)); }
      } finally {
        if (alive()) timer = setTimeout(poll, 800);
      }
    }
    void poll();
    return () => { current.controller.abort(); clearTimeout(timer); };
  }, [api]);

  async function mutate(operation: (current: Session) => Promise<Task>) {
    const current = session.current;
    if (!api || !online || !current || current.api !== api || current.controller.signal.aborted || current.mutating) return false;
    current.mutating = true;
    current.generation++; // Reject polls started before this mutation.
    setBusy(true); setMessage('');
    const valid = () => session.current === current && !current.controller.signal.aborted;
    try {
      const next = await operation(current);
      if (!valid()) return false;
      accept(next);
      return true;
    } catch (cause) {
      if (valid()) setMessage(errorText(cause));
      return false;
    } finally {
      current.generation++; // Also reject polls started while the mutation was pending.
      current.mutating = false;
      if (valid()) setBusy(false);
    }
  }

  async function start(root: string) {
    root = root.trim();
    if (!root || active(task) || task?.status === 'unknown') return false;
    setNotice('');
    return mutate(async current => {
      if (!pendingRequest.current || pendingRequest.current.root !== root || pendingRequest.current.instance !== current.instance) {
        pendingRequest.current = { root, id: crypto.randomUUID(), instance: current.instance };
      }
      const request = pendingRequest.current;
      try {
        const next = await current.api.start(root, request.id, current.controller.signal);
        if (pendingRequest.current === request) pendingRequest.current = null;
        return next;
      } catch (cause) {
        if (pendingRequest.current === request && cause instanceof ApiError && !['disconnected', 'invalid_response'].includes(cause.code)) pendingRequest.current = null;
        throw cause;
      }
    });
  }

  async function cancel() {
    if (!task) return false;
    return mutate(current => current.api.cancel(task.id, current.controller.signal));
  }

  return { task, online, busy, connectionError, message, setMessage, notice, setNotice, start, cancel };
}
