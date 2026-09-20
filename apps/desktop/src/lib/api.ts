export interface Connection { url: string; token: string }
export type Status = 'running' | 'cancelling' | 'cancelled' | 'completed' | 'failed';
export interface Issue { path: string; code: string; message: string }
export interface Task {
  id: string; root: string; status: Status; revision: number; started_at: number;
  finished_at: number | null; files: number; directories: number; logical_bytes: number;
  allocated_bytes: number | null; complete: boolean; issue_count: number; issues: Issue[];
}
export interface Entry {
  id: number; parent_id: number | null; name: string; kind: 'directory' | 'file' | 'link' | 'other';
  logical_bytes: number; allocated_bytes: number | null; modified_at: number | null; enumerated: boolean;
}
export interface Detail extends Entry { path: string; ancestors: Entry[] }
export interface Page { revision: number; total: number; offset: number; entries: Entry[] }
export interface Health { name: string; api_version: number; instance_id: string }

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

  async request<T>(path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
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
      const error = await response.json().catch(() => ({ code: 'request_failed', message: `请求失败（${response.status}）` }));
      throw new ApiError(error.code, error.message, error.task_id);
    }
    return response.json() as Promise<T>;
  }

  health(signal?: AbortSignal) { return this.request<Health>('/health', undefined, signal); }
  tasks(signal?: AbortSignal) { return this.request<Task[]>('/tasks', undefined, signal); }
  start(root: string, requestId: string) { return this.request<Task>('/scans', { root, request_id: requestId }); }
  cancel(id: string) { return this.request<Task>(`/tasks/${id}/cancel`, {}); }
  page(id: string, parent: number, offset: number, signal?: AbortSignal) {
    return this.request<Page>(`/scans/${id}/entries?parent=${parent}&offset=${offset}&limit=200`, undefined, signal);
  }
  detail(id: string, entry: number, signal?: AbortSignal) { return this.request<Detail>(`/scans/${id}/entries/${entry}`, undefined, signal); }
}
