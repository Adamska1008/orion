import { afterEach, describe, expect, it, vi } from 'vitest';
import { Api, validateConnection } from './api';
import { displayPath, formatBytes } from './utils';

afterEach(() => vi.unstubAllGlobals());
describe('local API client', () => {
  it('rejects remote endpoints before sending credentials', () => {
    for (const url of ['https://example.com', 'http://127.0.0.1.evil.test', 'http://user@127.0.0.1', 'http://127.0.0.1/?token=x']) {
      expect(() => validateConnection({ url, token: 'secret' })).toThrow();
    }
  });
  it('preserves request identity on retries and structured conflicts', async () => {
    const fetcher = vi.fn().mockResolvedValue({ ok: false, json: async () => ({ code: 'scan_in_progress', message: '已有任务', task_id: 'other' }) });
    vi.stubGlobal('fetch', fetcher);
    const api = new Api({ url: 'http://127.0.0.1:1234', token: 'test' });
    await expect(api.start('C:\\资料', 'request-id')).rejects.toMatchObject({ code: 'scan_in_progress', taskId: 'other' });
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ root: 'C:\\资料', request_id: 'request-id' });
    expect(fetcher.mock.calls[0][1].headers.Authorization).toBe('Bearer test');
  });
  it('sends the selected page size and offset to the server', async () => {
    const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ revision: 1, total: 201, offset: 150, entries: [] }) });
    vi.stubGlobal('fetch', fetcher);
    const api = new Api({ url: 'http://127.0.0.1:1234', token: 'test' });
    await api.page('scan', 7, 150, 50);
    expect(fetcher.mock.calls[0][0]).toBe('http://127.0.0.1:1234/api/v1/scans/scan/entries?parent=7&offset=150&limit=50');
  });
  it('keeps unknown allocation distinct from zero and renders Windows paths', () => {
    expect(formatBytes(null)).toBe('未知'); expect(formatBytes(0)).toBe('0 B'); expect(formatBytes(1024)).toBe('1 KiB');
    expect(displayPath('\\\\?\\C:\\资料')).toBe('C:\\资料');
    expect(displayPath('\\\\?\\UNC\\server\\folder')).toBe('\\\\server\\folder');
  });

  it('rejects malformed success bodies and tolerates non-JSON HTTP errors', async () => {
    const fetcher = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => ({ id: 'missing-fields' }) })
      .mockResolvedValueOnce({ ok: false, status: 503, json: async () => { throw new Error('not JSON'); } });
    vi.stubGlobal('fetch', fetcher);
    const api = new Api({ url: 'http://127.0.0.1:1234', token: 'test' });
    await expect(api.start('C:\\data', 'request')).rejects.toMatchObject({ code: 'invalid_response' });
    await expect(api.tasks()).rejects.toMatchObject({ code: 'request_failed', message: '请求失败（503）' });
  });
});
