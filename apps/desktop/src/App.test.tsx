// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { Api, type Detail, type Page, type Task } from './lib/api';

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => true,
  invoke: async () => ({ url: 'http://127.0.0.1:43120', token: 'test' }),
}));
vi.mock('./lib/theme', () => ({ useTheme: () => ['system', vi.fn()] }));
// jsdom has no layout; render the supplied rows without viewport measurement.
vi.mock('@tanstack/react-virtual', () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 48,
    getVirtualItems: () => Array.from({ length: count }, (_, index) => ({ index, size: 48, start: index * 48 })),
    scrollToIndex: vi.fn(),
  }),
}));

const task: Task = {
  id: 'completed-scan', root: 'C:\\data', status: 'completed', revision: 7,
  started_at: 1000, finished_at: 2000, files: 201, directories: 2,
  logical_bytes: 4096, allocated_bytes: null, complete: true, issue_count: 0, issues: [],
};
const rootDirectory: Detail = {
  id: 0, parent_id: null, name: 'data', kind: 'directory', logical_bytes: 4096,
  allocated_bytes: null, modified_at: null, enumerated: true, path: task.root, ancestors: [],
};
const childDirectory: Detail = {
  ...rootDirectory, id: 1, parent_id: 0, name: 'photos', path: 'C:\\data\\photos', ancestors: [rootDirectory],
};
const file: Detail = {
  ...rootDirectory, id: 2, parent_id: 1, name: 'photo.jpg', kind: 'file',
  path: 'C:\\data\\photos\\photo.jpg', ancestors: [rootDirectory, childDirectory],
};
const rootEntries = [childDirectory, ...Array.from({ length: 200 }, (_, index): Detail => ({
  ...file, id: index + 100, parent_id: 0, ancestors: [rootDirectory],
  name: index === 199 ? 'last.txt' : `file-${index + 1}.txt`,
  path: `C:\\data\\file-${index + 1}.txt`,
}))];
const rootPage: Page = { revision: 7, total: rootEntries.length, offset: 0, entries: rootEntries.slice(0, 15) };
const childPage: Page = { revision: 7, total: 1, offset: 0, entries: [file] };

let container: HTMLDivElement;
let app: Root;

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  HTMLElement.prototype.scrollTo = vi.fn();
  vi.spyOn(Api.prototype, 'health').mockResolvedValue({ name: 'orion-server', api_version: 1, version: 'test', capabilities: ['graceful_shutdown'], instance_id: 'server' });
  vi.spyOn(Api.prototype, 'tasks').mockImplementation(async () => [{ ...task }]);
  vi.spyOn(Api.prototype, 'page').mockImplementation(async (_id, parent, offset, limit) =>
    parent === 1 ? childPage : { ...rootPage, offset, entries: rootEntries.slice(offset, offset + limit) });
  vi.spyOn(Api.prototype, 'detail').mockImplementation(async (_id, entry) =>
    entry === 1 ? childDirectory : entry === 2 ? file : rootEntries.find(item => item.id === entry) ?? rootDirectory);
  container = document.createElement('div');
  document.body.append(container);
  app = createRoot(container);
  await act(async () => { app.render(<App />); });
});

afterEach(async () => {
  await act(async () => { app.unmount(); });
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

async function click(selector: string) {
  const button = container.querySelector<HTMLElement>(selector);
  expect(button, selector).not.toBeNull();
  await act(async () => { button!.click(); });
}

function expectRootVisible() {
  expect(container.querySelector('[role="listbox"]')?.textContent).toContain('photos');
  expect(container.querySelector('.explorer-top')?.textContent).toContain('201 项');
  expect(container.querySelector('nav[aria-label="目录层级"]')?.textContent).toBe(task.root);
  expect(container.querySelector('.detail-path')?.textContent).toBe(task.root);
}

describe('completed scan navigation', () => {
  it.each(['nav[aria-label="目录层级"] button'])(
    'keeps the root results when repeatedly clicking %s', async selector => {
      expectRootVisible();
      const requests = vi.mocked(Api.prototype.page).mock.calls.length;
      for (let i = 0; i < 3; i++) {
        await click(selector);
        expectRootVisible();
      }
      await act(async () => { await vi.advanceTimersByTimeAsync(800); });
      expectRootVisible();
      expect(Api.prototype.page).toHaveBeenCalledTimes(requests);
    },
  );

  it('returns from a child directory and keeps the root visible on subsequent clicks', async () => {
    await click('[aria-label="进入 photos"]');
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('photo.jpg');
    await click('nav[aria-label="目录层级"] span:last-child button');
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('photo.jpg');
    await click('nav[aria-label="目录层级"] span:first-child button');
    expectRootVisible();
    await click('nav[aria-label="目录层级"] span:first-child button');
    expectRootVisible();
  });

  it('returns to the first root page from a later page', async () => {
    await click('[aria-label="下一页"]');
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('file-15.txt');
    await click('nav[aria-label="目录层级"] span:first-child button');
    expectRootVisible();
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 0, 15, expect.any(AbortSignal));
  });

  it('clears the selection while preserving the current directory results', async () => {
    await click('[role="option"]');
    expect(container.querySelector('.detail-path')?.textContent).toBe(childDirectory.path);
    await click('nav[aria-label="目录层级"] span:first-child button');
    expectRootVisible();
    expect(container.querySelector('[role="option"]')?.getAttribute('aria-selected')).toBe('false');
  });

  it('keeps a pending root request when the root breadcrumb is clicked again', async () => {
    await click('[aria-label="进入 photos"]');
    let resolvePage!: (page: Page) => void;
    vi.mocked(Api.prototype.page).mockImplementationOnce(() => new Promise(resolve => { resolvePage = resolve; }));
    await click('nav[aria-label="目录层级"] span:first-child button');
    await click('nav[aria-label="目录层级"] span:first-child button');
    await act(async () => { resolvePage(rootPage); });
    expectRootVisible();
  });
});

async function jumpTo(page: number) {
  const input = container.querySelector<HTMLInputElement>('[aria-label="跳转页码"]')!;
  input.value = String(page);
  await act(async () => { input.form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })); });
}

async function selectPageSize(size: number) {
  const select = container.querySelector<HTMLSelectElement>('[aria-label="每页条数"]')!;
  await act(async () => {
    select.value = String(size);
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('directory pagination', () => {
  it('requests 15 entries by default and moves forward and back without mixing pages', async () => {
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 0, 15, expect.any(AbortSignal));
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(15);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="上一页"]')!.disabled).toBe(true);
    await click('[aria-label="下一页"]');
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 15, 15, expect.any(AbortSignal));
    expect(container.querySelector('[role="listbox"]')?.textContent).not.toContain('photos');
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('file-15.txt');
    expect(container.querySelector('.pagination-info')?.textContent).toContain('16–30');
    await click('[aria-label="上一页"]');
    expectRootVisible();
  });

  it('jumps to the final partial page and disables forward navigation', async () => {
    await jumpTo(14);
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 195, 15, expect.any(AbortSignal));
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(6);
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('last.txt');
    expect(container.querySelector('.pagination-info')?.textContent).toContain('196–201');
    expect(container.querySelector<HTMLButtonElement>('[aria-label="下一页"]')!.disabled).toBe(true);
  });

  it('changes the page size from a later page and starts at the first page', async () => {
    await jumpTo(14);
    await selectPageSize(50);
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 0, 50, expect.any(AbortSignal));
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(50);
    expect(container.querySelector<HTMLInputElement>('[aria-label="跳转页码"]')!.value).toBe('1');
    expectRootVisible();
  });

  it('removes the old page and selection while the next page is loading', async () => {
    await click('[role="option"]');
    let resolvePage!: (page: Page) => void;
    vi.mocked(Api.prototype.page).mockImplementationOnce(() => new Promise(resolve => { resolvePage = resolve; }));
    await click('[aria-label="下一页"]');
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(0);
    expect(container.querySelector('[role="listbox"]')?.getAttribute('aria-busy')).toBe('true');
    expect(container.querySelector('.detail-path')?.textContent).toBe(task.root);
    await act(async () => { resolvePage({ ...rootPage, offset: 15, entries: rootEntries.slice(15, 30) }); });
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(15);
    expect(container.querySelector('[role="listbox"]')?.getAttribute('aria-busy')).toBe('false');
  });

  it('ignores a late detail response from an item on the previous page', async () => {
    let resolveDetail!: (detail: Detail) => void;
    vi.mocked(Api.prototype.detail).mockImplementationOnce(() => new Promise(resolve => { resolveDetail = resolve; }));
    await click('[role="option"]');
    await click('[aria-label="下一页"]');
    await act(async () => { resolveDetail(childDirectory); });
    expect(container.querySelector('.detail-path')?.textContent).toBe(task.root);
  });

  it('resets an out-of-range page when a refreshed directory becomes empty', async () => {
    await jumpTo(14);
    vi.mocked(Api.prototype.page).mockResolvedValue({ revision: 8, total: 0, offset: 0, entries: [] });
    vi.mocked(Api.prototype.tasks).mockResolvedValue([{ ...task, revision: 8 }]);
    await act(async () => { await vi.advanceTimersByTimeAsync(800); });
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 0, 15, expect.any(AbortSignal));
    expect(container.querySelectorAll('[role="option"]')).toHaveLength(0);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="上一页"]')!.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="下一页"]')!.disabled).toBe(true);
    expect(container.querySelector('.pagination-info')?.textContent).toContain('共 0 项');
  });

  it('does not request a page outside the available range', async () => {
    const requests = vi.mocked(Api.prototype.page).mock.calls.length;
    await jumpTo(0);
    await jumpTo(15);
    expect(Api.prototype.page).toHaveBeenCalledTimes(requests);
    expectRootVisible();
  });
});
