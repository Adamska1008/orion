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
const rootPage: Page = { revision: 7, total: 201, offset: 0, entries: [childDirectory] };
const childPage: Page = { revision: 7, total: 1, offset: 0, entries: [file] };

let container: HTMLDivElement;
let app: Root;

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  HTMLElement.prototype.scrollTo = vi.fn();
  vi.spyOn(Api.prototype, 'health').mockResolvedValue({ name: 'orion-server', api_version: 1, instance_id: 'server' });
  vi.spyOn(Api.prototype, 'tasks').mockImplementation(async () => [{ ...task }]);
  vi.spyOn(Api.prototype, 'page').mockImplementation(async (_id, parent, offset) =>
    parent === 1 ? childPage : offset === 200 ? { ...rootPage, offset, entries: [{ ...file, parent_id: 0, name: 'last.txt' }] } : rootPage);
  vi.spyOn(Api.prototype, 'detail').mockImplementation(async (_id, entry) =>
    entry === 1 ? childDirectory : entry === 2 ? file : rootDirectory);
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
  it.each(['.nav-item', '.scan-shortcut', 'nav[aria-label="目录层级"] button'])(
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
    await click('.nav-item');
    expectRootVisible();
    await click('.nav-item');
    expectRootVisible();
  });

  it('returns to the first root page from a later page', async () => {
    await click('[aria-label="下一页"]');
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain('last.txt');
    await click('.nav-item');
    expectRootVisible();
    expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 0, 0, expect.any(AbortSignal));
  });

  it('clears the selection while preserving the current directory results', async () => {
    await click('[role="option"]');
    expect(container.querySelector('.detail-path')?.textContent).toBe(childDirectory.path);
    await click('.nav-item');
    expectRootVisible();
    expect(container.querySelector('[role="option"]')?.getAttribute('aria-selected')).toBe('false');
  });

  it('keeps a pending root request when overview is clicked again', async () => {
    await click('[aria-label="进入 photos"]');
    let resolvePage!: (page: Page) => void;
    vi.mocked(Api.prototype.page).mockImplementationOnce(() => new Promise(resolve => { resolvePage = resolve; }));
    await click('.nav-item');
    await click('.nav-item');
    await act(async () => { resolvePage(rootPage); });
    expectRootVisible();
  });
});
