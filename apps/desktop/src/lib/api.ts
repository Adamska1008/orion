import { type Connection, type Task, decodeTask, decodeTasks, decodePage, decodeDetail, decodeHealth, decodeError, decodeTreemap } from './contracts';
export type { Connection, Status, Issue, Task, Entry, Detail, Page, Health, Treemap, TreemapNode } from './contracts';

export class ApiError extends Error {
  constructor(public code: string, message: string, public taskId?: string) { super(message); }
}

export function validateConnection(connection: Connection): Connection {
  const url = new URL(connection.url);
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw new Error('请输入本机服务地址，例如 http://127.0.0.1:43120。');
  }
  if (!connection.token.trim()) throw new Error('请填写连接令牌。');
  return { url: url.origin, token: connection.token.trim() };
}

export class Api {
  private connection: Connection;
  constructor(connection: Connection) { this.connection = validateConnection(connection); }

  private async request<T>(path: string, decode: (value: unknown) => T, body?: unknown, signal?: AbortSignal): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`${this.connection.url}/api/v1${path}`, {
        method: body === undefined ? 'GET' : 'POST',
        headers: { Authorization: `Bearer ${this.connection.token}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(5000)]) : AbortSignal.timeout(5000),
      });
    } catch (error) {
      if (signal?.aborted) throw error;
      throw new ApiError('disconnected', '无法连接本地服务。请确认 Server 已启动，或重新填写连接信息。');
    }
    if (!response.ok) {
      const error = await response.json().then(decodeError).catch(() => ({ code: 'request_failed', message: `请求失败（${response.status}）`, task_id: null }));
      throw new ApiError(error.code, error.message, error.task_id ?? undefined);
    }
    try { return decode(await response.json()); }
    catch { throw new ApiError('invalid_response', '后台返回的数据格式不兼容，请检查 Server 版本。'); }
  }

  health(signal?: AbortSignal) { return this.request('/health', decodeHealth, undefined, signal); }
  tasks(signal?: AbortSignal) { return this.request('/tasks', decodeTasks, undefined, signal); }
  start(root: string, requestId: string, signal?: AbortSignal) { return this.request<Task>('/scans', decodeTask, { root, request_id: requestId }, signal); }
  cancel(id: string, signal?: AbortSignal) { return this.request<Task>(`/tasks/${id}/cancel`, decodeTask, {}, signal); }
  page(id: string, parent: number, offset: number, limit: number, signal?: AbortSignal) {
    return this.request(`/scans/${id}/entries?parent=${parent}&offset=${offset}&limit=${limit}`, decodePage, undefined, signal);
  }
  detail(id: string, entry: number, signal?: AbortSignal) { return this.request(`/scans/${id}/entries/${entry}`, decodeDetail, undefined, signal); }
  treemap(id: string, parent: number, depth: number, signal?: AbortSignal) {
    return this.request(`/scans/${id}/treemap?parent=${parent}&depth=${depth}`, decodeTreemap, undefined, signal);
  }
}
