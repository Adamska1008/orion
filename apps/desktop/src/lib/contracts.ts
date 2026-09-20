export interface Connection { url: string; token: string }
export type Status = 'running' | 'cancelling' | 'cancelled' | 'completed' | 'failed' | 'unknown';
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
export interface TreemapNode extends Entry {
  child_count: number; zero_count: number; expanded: boolean;
  omitted_count: number; omitted_bytes: number; children: TreemapNode[];
}
export interface Treemap { revision: number; depth: number; root: TreemapNode }
export interface Health { name: string; api_version: number; instance_id: string; version: string; capabilities: string[] }

export interface ErrorBody { code: string; message: string; task_id: string | null }

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Expected an object');
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== 'string') throw new Error('Expected a string');
  return value;
}
function number(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || !Number.isInteger(value) || value < 0) throw new Error('Expected a nonnegative integer');
  return value;
}
function boolean(value: unknown): boolean {
  if (typeof value !== 'boolean') throw new Error('Expected a boolean');
  return value;
}
function nullable<T>(value: unknown, decode: (value: unknown) => T): T | null { return value === null ? null : decode(value); }
function array<T>(value: unknown, decode: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) throw new Error('Expected an array');
  return value.map(decode);
}
function decodeIssue(value: unknown): Issue {
  const item = object(value);
  return { path: text(item.path), code: text(item.code), message: text(item.message) };
}
export function decodeTask(value: unknown): Task {
  const item = object(value);
  const status = text(item.status);
  return {
    id: text(item.id), root: text(item.root), status: ['running', 'cancelling', 'cancelled', 'completed', 'failed'].includes(status) ? status as Status : 'unknown',
    revision: number(item.revision), started_at: number(item.started_at), finished_at: nullable(item.finished_at, number),
    files: number(item.files), directories: number(item.directories), logical_bytes: number(item.logical_bytes),
    allocated_bytes: nullable(item.allocated_bytes, number), complete: boolean(item.complete), issue_count: number(item.issue_count), issues: array(item.issues, decodeIssue),
  };
}
export const decodeTasks = (value: unknown): Task[] => array(value, decodeTask);
function decodeEntry(value: unknown): Entry {
  const item = object(value);
  const kind = text(item.kind);
  if (!['directory', 'file', 'link', 'other'].includes(kind)) throw new Error('Unknown entry kind');
  return {
    id: number(item.id), parent_id: nullable(item.parent_id, number), name: text(item.name), kind: kind as Entry['kind'],
    logical_bytes: number(item.logical_bytes), allocated_bytes: nullable(item.allocated_bytes, number),
    modified_at: nullable(item.modified_at, number), enumerated: boolean(item.enumerated),
  };
}
export function decodePage(value: unknown): Page {
  const item = object(value);
  return { revision: number(item.revision), total: number(item.total), offset: number(item.offset), entries: array(item.entries, decodeEntry) };
}
export function decodeDetail(value: unknown): Detail {
  const item = object(value);
  return { ...decodeEntry(value), path: text(item.path), ancestors: array(item.ancestors, decodeEntry) };
}
export function decodeTreemap(value: unknown): Treemap {
  const item = object(value);
  const depth = number(item.depth);
  if (depth < 1 || depth > 4) throw new Error('Invalid tree depth');
  let count = 0;
  function node(value: unknown, level: number): TreemapNode {
    if (level > depth || ++count > 2048) throw new Error('Tree exceeds bounds');
    const item = object(value);
    const entry = decodeEntry(item);
    const children = array(item.children, child => node(child, level + 1));
    const result = { ...entry, child_count: number(item.child_count), zero_count: number(item.zero_count),
      expanded: boolean(item.expanded), omitted_count: number(item.omitted_count), omitted_bytes: number(item.omitted_bytes), children };
    if (children.some(child => child.parent_id !== entry.id)) throw new Error('Invalid parent');
    if (result.expanded && (entry.kind !== 'directory' || children.length + result.zero_count + result.omitted_count !== result.child_count)) throw new Error('Invalid child count');
    return result;
  }
  return { revision: number(item.revision), depth, root: node(item.root, 0) };
}
export function decodeHealth(value: unknown): Health {
  const item = object(value);
  return { name: text(item.name), api_version: number(item.api_version), instance_id: text(item.instance_id), version: text(item.version), capabilities: array(item.capabilities, text) };
}
export function decodeError(value: unknown): ErrorBody {
  const item = object(value);
  return { code: text(item.code), message: text(item.message), task_id: nullable(item.task_id ?? null, text) };
}
