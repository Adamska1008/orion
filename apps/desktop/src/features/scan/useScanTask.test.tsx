// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Api, ApiError, type Task } from '../../lib/api';
import { useScanTask } from './useScanTask';

const oldTask: Task = { id: 'old', root: 'C:\\data', status: 'completed', revision: 7, started_at: 1, finished_at: 2, files: 201, directories: 1, logical_bytes: 10, allocated_bytes: null, complete: true, issue_count: 0, issues: [] };
const newTask: Task = { ...oldTask, id: 'new', status: 'running', revision: 1, finished_at: null, files: 987 };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
let root: Root;
let container: HTMLDivElement;
let state: ReturnType<typeof useScanTask>;
let api: Api;
function Harness({ client }: { client: Api }) { state = useScanTask(client); return null; }
function client(port: number) {
  const result = new Api({ url: `http://127.0.0.1:${port}`, token: 'test' });
  vi.spyOn(result, 'health').mockResolvedValue({ name: 'orion-server', api_version: 1, version: 'test', capabilities: ['graceful_shutdown'], instance_id: String(port) });
  vi.spyOn(result, 'tasks').mockResolvedValue([oldTask]);
  return result;
}
beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div');
  root = createRoot(container);
  api = client(43120);
  await act(async () => { root.render(<Harness client={api} />); });
});
afterEach(async () => {
  await act(async () => { root.unmount(); });
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers();
});

it('rejects a poll started before a new scan, then accepts another client’s fresh task', async () => {
  const poll = deferred<Task[]>();
  vi.mocked(api.tasks).mockImplementationOnce(() => poll.promise);
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  vi.spyOn(api, 'start').mockResolvedValue(newTask);
  await act(async () => { await state.start(oldTask.root); });
  await act(async () => { poll.resolve([oldTask]); });
  expect(state.task?.id).toBe('new');
  vi.mocked(api.tasks).mockResolvedValue([{ ...oldTask, id: 'external' }]);
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  expect(state.task?.id).toBe('external');
});

it('rejects a poll started during a mutation even when it arrives after the mutation', async () => {
  const start = deferred<Task>();
  const poll = deferred<Task[]>();
  vi.spyOn(api, 'start').mockImplementation(() => start.promise);
  vi.mocked(api.tasks).mockImplementationOnce(() => poll.promise);
  let starting!: Promise<boolean>;
  await act(async () => { starting = state.start(oldTask.root); });
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  await act(async () => { start.resolve(newTask); await starting; });
  await act(async () => { poll.resolve([oldTask]); });
  expect(state.task?.id).toBe('new');
});

it('does not replace cancellation with a lower revision of the same task', async () => {
  vi.mocked(api.tasks).mockResolvedValue([{ ...newTask, revision: 8 }]);
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  vi.spyOn(api, 'cancel').mockResolvedValue({ ...newTask, status: 'cancelling', revision: 9 });
  await act(async () => { await state.cancel(); });
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  expect(state.task?.status).toBe('cancelling');
  expect(state.task?.revision).toBe(9);
});

it('ignores a mutation response from the previous connection', async () => {
  const start = deferred<Task>();
  vi.spyOn(api, 'start').mockImplementation(() => start.promise);
  let starting!: Promise<boolean>;
  await act(async () => { starting = state.start(oldTask.root); });
  const next = client(43121);
  vi.mocked(next.tasks).mockResolvedValue([{ ...oldTask, id: 'other-server' }]);
  await act(async () => { root.render(<Harness client={next} />); });
  await act(async () => { start.resolve(newTask); await starting; });
  expect(state.task?.id).toBe('other-server');
  expect(state.busy).toBe(false);
});

it('keeps the previous result visible while reconnecting to the same instance', async () => {
  const next = client(43120);
  const poll = deferred<Task[]>();
  vi.mocked(next.tasks).mockImplementationOnce(() => poll.promise);
  await act(async () => { root.render(<Harness client={next} />); });
  expect(state.online).toBe(false);
  expect(state.task?.id).toBe(oldTask.id);
  await act(async () => { poll.resolve([oldTask]); });
  expect(state.online).toBe(true);
  expect(state.task?.id).toBe(oldTask.id);
});

it.each([43120, 43121])('keeps request identity only when reconnecting to the same instance (%s)', async port => {
  const first = vi.spyOn(api, 'start').mockRejectedValue(new ApiError('disconnected', 'timeout'));
  await act(async () => { await state.start(oldTask.root); });
  const next = client(port);
  const retry = vi.spyOn(next, 'start').mockResolvedValue(newTask);
  await act(async () => { root.render(<Harness client={next} />); });
  await act(async () => { await state.start(oldTask.root); });
  expect(retry.mock.calls[0][1] === first.mock.calls[0][1]).toBe(port === 43120);
});

it.each(['disconnected', 'invalid_response'])('preserves the request ID after %s and prevents simultaneous mutations', async code => {
  const start = vi.spyOn(api, 'start').mockRejectedValueOnce(new ApiError(code, 'unconfirmed response')).mockResolvedValue(newTask);
  await act(async () => { await state.start(oldTask.root); });
  await act(async () => { await Promise.all([state.start(oldTask.root), state.start(oldTask.root)]); });
  expect(start).toHaveBeenCalledTimes(2);
  expect(start.mock.calls[0][1]).toBe(start.mock.calls[1][1]);
  expect(state.task?.id).toBe('new');
});
