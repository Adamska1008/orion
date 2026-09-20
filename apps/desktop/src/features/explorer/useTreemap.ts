import { useEffect, useRef, useState } from 'react';
import { ApiError, type Api, type Task, type Treemap } from '../../lib/api';
import { errorText } from '../../lib/desktop';

interface Result { key: string; data: Treemap | null; error: string; loading: boolean }

export function useTreemap(api: Api | null, task: Task, parent: number, depth: number, enabled: boolean, online: boolean) {
  const key = `${task.id}:${parent}:${depth}`;
  const [result, setResult] = useState<Result>({ key: '', data: null, error: '', loading: false });
  const [attempt, setAttempt] = useState(0);
  const revision = useRef(task.revision);
  revision.current = task.revision;

  useEffect(() => {
    if (!api || !enabled || !online) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let lastRevision: number | null = null;
    setResult(current => ({ key, data: current.key === key ? current.data : null, error: '', loading: true }));
    async function poll() {
      try {
        if (lastRevision === null || lastRevision < revision.current) {
          const data = await api!.treemap(task.id, parent, depth, controller.signal);
          if (controller.signal.aborted) return;
          lastRevision = data.revision;
          setResult({ key, data, error: '', loading: false });
        }
      } catch (cause) {
        if (controller.signal.aborted) return;
        const message = cause instanceof ApiError && cause.code === 'route_not_found'
          ? '当前后台尚不支持空间图，请重新构建并启动新版后台。' : errorText(cause);
        setResult(current => ({ ...current, key, loading: false, error: message }));
        return; // Explicit retry avoids repeating an unsupported or failing query.
      }
      if (!controller.signal.aborted) timer = setTimeout(poll, 800);
    }
    void poll();
    // A new revision does not cancel an in-flight snapshot. Refresh sequentially,
    // so fast scanning cannot indefinitely starve a slower directory query.
    return () => { controller.abort(); clearTimeout(timer); };
  }, [api, task.id, parent, depth, key, enabled, online, attempt]);

  const current = result.key === key ? result : { data: null, error: '', loading: enabled && online };
  return { ...current, retry: () => setAttempt(value => value + 1) };
}
