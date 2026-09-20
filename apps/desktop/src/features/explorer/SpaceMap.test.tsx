// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Api, ApiError, type Detail, type Task, type Treemap, type TreemapNode } from '../../lib/api';
import { DirectoryExplorer } from './DirectoryExplorer';

vi.mock('../../lib/desktop', () => ({ desktop: { available: () => false }, errorText: (error: unknown) => error instanceof Error ? error.message : String(error) }));
vi.mock('@tanstack/react-virtual', () => ({ useVirtualizer: ({ count }: { count: number }) => ({ getTotalSize: () => count * 48, getVirtualItems: () => Array.from({ length: count }, (_, index) => ({ index, size: 48, start: index * 48 })), scrollToIndex: vi.fn() }) }));

const task: Task = { id: 'tree-scan', root: 'C:\\data', status: 'completed', revision: 7, started_at: 1, finished_at: 2, files: 32, directories: 3, logical_bytes: 100, allocated_bytes: null, complete: true, issue_count: 0, issues: [] };
const entry = (id: number, parent: number | null, name: string, bytes: number, children: TreemapNode[] | null = null): TreemapNode => ({ id, parent_id: parent, name, logical_bytes: bytes, kind: children ? 'directory' : 'file', children: children ?? [], child_count: children?.length ?? 0, zero_count: 0, expanded: children !== null, omitted_count: 0, omitted_bytes: 0, allocated_bytes: null, modified_at: null, enumerated: true });
const tree = entry(0, null, 'data', 100, [entry(1, 0, 'projects', 70, [entry(2, 1, 'build', 50, [entry(3, 2, 'large.bin', 50)]), entry(4, 1, 'archive.zip', 20)]), ...Array.from({ length: 30 }, (_, i) => entry(i + 100, 0, `small-${i}.txt`, 1))]);
const nodes = new Map<number, TreemapNode>();
function index(node: TreemapNode) { nodes.set(node.id, node); node.children.forEach(index); }
index(tree);
function response(parent = 0, depth = 2): Treemap {
  function trim(node: TreemapNode, remaining: number): TreemapNode { return { ...node, expanded: remaining > 0 && node.kind === 'directory', children: remaining > 0 ? node.children.map(child => trim(child, remaining - 1)) : [] }; }
  return { revision: 7, depth, root: trim(nodes.get(parent)!, depth) };
}
function detail(id: number): Detail {
  const node = nodes.get(id)!; const ancestors: TreemapNode[] = [];
  let parent = node.parent_id;
  while (parent !== null) { const ancestor = nodes.get(parent)!; ancestors.unshift(ancestor); parent = ancestor.parent_id; }
  return { ...node, ancestors, path: id === 0 ? task.root : task.root + '\\' + [...ancestors.slice(1), node].map(item => item.name).join('\\') };
}
let container: HTMLDivElement, app: Root;
let mapWidth: number, resizeMap: () => void;
const api = new Api({ url: 'http://127.0.0.1:43123', token: 'test' });
const onError = vi.fn();
async function render(nextTask = task) { await act(async () => { app.render(<DirectoryExplorer api={api} task={nextTask} online onError={onError} />); }); }
async function click(selector: string, event = 'click') {
  const element = container.querySelector<HTMLElement>(selector); expect(element, selector).not.toBeNull();
  await act(async () => { element!.dispatchEvent(new MouseEvent(event, { bubbles: true })); });
}
async function depth(value: number) {
  const select = container.querySelector<HTMLSelectElement>('[aria-label="显示层级"]')!;
  await act(async () => { select.value = String(value); select.dispatchEvent(new Event('change', { bubbles: true })); });
}
beforeEach(async () => {
  vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  mapWidth = 960;
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resizeMap = callback; } observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => mapWidth);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(540);
  HTMLElement.prototype.scrollTo = vi.fn();
  vi.spyOn(Api.prototype, 'page').mockImplementation(async (_task, parent, offset, limit) => ({ revision: 7, total: nodes.get(parent)!.child_count, offset, entries: nodes.get(parent)!.children.slice(offset, offset + limit) }));
  vi.spyOn(Api.prototype, 'detail').mockImplementation(async (_task, id) => detail(id));
  vi.spyOn(Api.prototype, 'treemap').mockImplementation(async (_task, parent, depth) => response(parent, depth));
  container = document.createElement('div'); document.body.append(container); app = createRoot(container); await render();
});
afterEach(async () => { await act(async () => app.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

it('loads the whole current directory independently of the 15-row list page', async () => {
  expect(container.querySelectorAll('[role="option"]')).toHaveLength(15);
  await click('.explorer-views button:last-child');
  expect(Api.prototype.treemap).toHaveBeenLastCalledWith(task.id, 0, 2, expect.any(AbortSignal));
  expect(container.querySelector('[data-entry-id="129"]')).not.toBeNull();
  expect(container.querySelector('.pagination')).toBeNull();
  await click('.explorer-views button:first-child');
  expect(container.querySelectorAll('[role="option"]')).toHaveLength(15);
});

it('selects a deep file, drills into a nested directory, and keeps depth relative to the new location', async () => {
  await click('.explorer-views button:last-child'); await depth(3);
  await click('[data-entry-id="3"]');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data\\projects\\build\\large.bin');
  expect(container.querySelector('.space-map')).not.toBeNull();
  expect(container.querySelector('[data-entry-id="3"]')?.getAttribute('aria-pressed')).toBe('true');
  await click('[data-entry-id="2"]', 'dblclick');
  expect(Api.prototype.treemap).toHaveBeenLastCalledWith(task.id, 2, 3, expect.any(AbortSignal));
  expect(container.querySelector('nav[aria-label="目录层级"]')?.textContent).toContain('projectsbuild');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data\\projects\\build');
  await click('[aria-label="返回上级目录"]');
  expect(Api.prototype.treemap).toHaveBeenLastCalledWith(task.id, 1, 3, expect.any(AbortSignal));
});

it('ignores a late snapshot from the previous depth', async () => {
  let resolve!: (value: Treemap) => void;
  vi.mocked(Api.prototype.treemap).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await click('.explorer-views button:last-child'); await depth(3);
  await act(async () => resolve(response(0, 2)));
  expect(container.querySelector('[data-entry-id="3"]')).not.toBeNull();
  expect(container.querySelector<HTMLSelectElement>('[aria-label="显示层级"]')!.value).toBe('3');
});

it('finishes an in-flight query during scanning and then refreshes the newer revision', async () => {
  let resolve!: (value: Treemap) => void;
  vi.mocked(Api.prototype.treemap).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await click('.explorer-views button:last-child');
  const signal = vi.mocked(Api.prototype.treemap).mock.calls[0][3]!;
  await render({ ...task, revision: 8, status: 'running' });
  expect(signal.aborted).toBe(false);
  await act(async () => resolve(response()));
  expect(container.querySelector('[data-entry-id="129"]')).not.toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  expect(Api.prototype.treemap).toHaveBeenCalledTimes(2);
});

it('clears old data and rejects late replies when a new scan replaces the task', async () => {
  let resolve!: (value: Treemap) => void;
  vi.mocked(Api.prototype.treemap).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await click('.explorer-views button:last-child');
  const empty = entry(0, null, 'empty', 0, []);
  vi.mocked(Api.prototype.treemap).mockResolvedValue({ revision: 1, depth: 2, root: empty });
  await render({ ...task, id: 'new-task', revision: 1 });
  await act(async () => resolve(response()));
  expect(container.querySelector('.space-map')?.textContent).toContain('没有可绘制的大小');
  expect(container.querySelector('[data-entry-id="129"]')).toBeNull();
});

it('keeps a repeatedly clicked or double-clicked file selected in the map with its properties', async () => {
  await click('.explorer-views button:last-child');
  await click('[data-entry-id="4"]');
  await click('[data-entry-id="4"]');
  await click('[data-entry-id="4"]', 'dblclick');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data\\projects\\archive.zip');
  expect(container.querySelector('.detail-kind')?.textContent).toBe('文件');
  expect(container.querySelector('[data-entry-id="4"]')?.getAttribute('aria-pressed')).toBe('true');
  expect(container.querySelector('.space-map')).not.toBeNull();
  expect(container.querySelector('.pagination')).toBeNull();
});

it('selects an aggregate tile and opens the directory list only with the explicit detail action', async () => {
  const aggregate = { ...response(), root: { ...tree, children: [], omitted_count: 31, omitted_bytes: 100 } };
  vi.mocked(Api.prototype.treemap).mockResolvedValue(aggregate);
  await click('.explorer-views button:last-child'); await click('.map-other');
  await click('.map-other', 'dblclick');
  expect(container.querySelector('.space-map')).not.toBeNull();
  expect(container.querySelector('.pagination')).toBeNull();
  expect(container.querySelector('.map-other')?.getAttribute('aria-pressed')).toBe('true');
  expect(container.querySelector('.detail-heading')?.textContent).toBe('汇总详情');
  expect(container.querySelector('.detail-panel h3')?.textContent).toBe('其他 31 项');
  expect(container.querySelector('.detail-size')?.textContent).toBe('100 B');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data');
  expect(container.querySelector('.detail-panel')?.textContent).not.toContain('修改时间');
  await click('.detail-buttons button');
  expect(container.querySelectorAll('[role="option"]')).toHaveLength(15);
  expect(container.querySelector('.pagination')?.textContent).toContain('共 31 项');
});

it('shows a nested aggregate in its own directory and switches back to file properties on selection', async () => {
  const nested = { ...tree.children[0], children: [tree.children[0].children[1]], omitted_count: 3, omitted_bytes: 50, child_count: 4 };
  vi.mocked(Api.prototype.treemap).mockResolvedValue({ ...response(), root: { ...tree, children: [nested, ...tree.children.slice(1)] } });
  await click('.explorer-views button:last-child'); await click('[data-aggregate-parent="1"]');
  expect(container.querySelector('.detail-panel h3')?.textContent).toBe('其他 3 项');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data\\projects');
  expect(container.querySelector('.detail-panel')?.textContent).toContain('50.0%');
  expect(container.querySelector('[data-entry-id="1"]')?.getAttribute('aria-pressed')).toBe('false');
  await click('[data-entry-id="4"]');
  expect(container.querySelector('.map-other')?.getAttribute('aria-pressed')).toBe('false');
  expect(container.querySelector('.detail-path')?.textContent).toBe('C:\\data\\projects\\archive.zip');
  await click('[data-aggregate-parent="1"]'); await click('.detail-buttons button');
  expect(Api.prototype.page).toHaveBeenLastCalledWith(task.id, 1, 0, 15, expect.any(AbortSignal));
});

it('uses the displayed aggregate totals for small tiles and recalculates them on resize', async () => {
  const snapshot = { ...response(), root: { ...tree, logical_bytes: 110, child_count: 33, omitted_count: 2, omitted_bytes: 10 } };
  vi.mocked(Api.prototype.treemap).mockResolvedValue(snapshot);
  await click('.explorer-views button:last-child'); await click('[data-aggregate-parent="0"]');
  expect(container.querySelector('.detail-panel h3')?.textContent).toBe('其他 2 项');
  expect(container.querySelector('.detail-size')?.textContent).toBe('10 B');
  await act(async () => { mapWidth = 100; resizeMap(); });
  expect(container.querySelector('.detail-panel h3')?.textContent).toBe('其他 32 项');
  expect(container.querySelector('.detail-size')?.textContent).toBe('40 B');
  expect(container.querySelector('.map-other')?.getAttribute('aria-pressed')).toBe('true');
});

it('refreshes selected aggregate totals with scanning and clears them for a new task', async () => {
  vi.mocked(Api.prototype.treemap).mockResolvedValue({ ...response(), root: { ...tree, children: [], omitted_count: 31, omitted_bytes: 100 } });
  await click('.explorer-views button:last-child'); await click('.map-other');
  vi.mocked(Api.prototype.treemap).mockResolvedValue({ ...response(), revision: 8, root: { ...tree, logical_bytes: 120, child_count: 35, children: [], omitted_count: 35, omitted_bytes: 120 } });
  await render({ ...task, revision: 8, status: 'running' });
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  expect(container.querySelector('.detail-panel h3')?.textContent).toBe('其他 35 项');
  expect(container.querySelector('.detail-size')?.textContent).toBe('120 B');
  await render({ ...task, id: 'another-task' });
  expect(container.querySelector('.detail-heading')?.textContent).toBe('当前目录');
  expect(container.querySelector('.map-other')?.getAttribute('aria-pressed')).toBe('false');
});

it('clears aggregate selection when changing depth, view, or directory', async () => {
  vi.mocked(Api.prototype.treemap).mockResolvedValue({ ...response(), root: { ...tree, children: [], omitted_count: 31, omitted_bytes: 100 } });
  await click('.explorer-views button:last-child'); await click('.map-other');
  await depth(3);
  expect(container.querySelector('.detail-heading')?.textContent).toBe('当前目录');
  await click('.map-other');
  await click('.explorer-views button:first-child'); await click('.explorer-views button:last-child');
  expect(container.querySelector('.detail-heading')?.textContent).toBe('当前目录');
  await click('.map-other'); await click('nav[aria-label="目录层级"] button');
  expect(container.querySelector('.detail-heading')?.textContent).toBe('当前目录');
  expect(container.querySelector('.map-other')?.getAttribute('aria-pressed')).toBe('false');
});

it('explains an old backend and retries without hiding the list view', async () => {
  vi.mocked(Api.prototype.treemap).mockRejectedValueOnce(new ApiError('route_not_found', 'unknown route'));
  await click('.explorer-views button:last-child');
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('新版后台');
  await click('.map-error button');
  expect(container.querySelector('[data-entry-id="129"]')).not.toBeNull();
  expect(container.querySelector('[role="alert"]')).toBeNull();
});
