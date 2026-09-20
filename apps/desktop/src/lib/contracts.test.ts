import { expect, it } from 'vitest';
import fixture from '../../../../contracts/api-v1.json';
import { decodeDetail, decodeError, decodeHealth, decodePage, decodeTask, decodeTasks, decodeTreemap } from './contracts';

it('decodes the exact wire fixtures also checked against Rust serialization', () => {
  expect(decodeHealth(fixture.health)).toEqual(fixture.health);
  expect(decodeTasks(fixture.tasks)).toEqual(fixture.tasks);
  expect(decodePage(fixture.page)).toEqual(fixture.page);
  expect(decodeDetail(fixture.detail)).toEqual(fixture.detail);
  expect(decodeTreemap(fixture.treemap)).toEqual(fixture.treemap);
  expect(fixture.errors.map(decodeError)).toEqual(fixture.errors);
});

it('rejects malformed or unbounded subtree responses', () => {
  expect(() => decodeTreemap({ ...fixture.treemap, depth: 5 })).toThrow();
  expect(() => decodeTreemap({ ...fixture.treemap, root: { ...fixture.treemap.root, child_count: 42 } })).toThrow();
  const child = { ...fixture.treemap.root.children[0], parent_id: 999 };
  expect(() => decodeTreemap({ ...fixture.treemap, root: { ...fixture.treemap.root, children: [child] } })).toThrow();
});

it('distinguishes null allocation and incomplete results, rejecting missing or malformed fields', () => {
  const task = fixture.tasks[0];
  expect(decodeTask(task).allocated_bytes).toBeNull();
  expect(decodeTask(task).complete).toBe(false);
  expect(() => decodeTask({ ...task, allocated_bytes: undefined })).toThrow();
  expect(() => decodeTask({ ...task, revision: '7' })).toThrow();
  expect(() => decodePage({ ...fixture.page, entries: {} })).toThrow();
});

it('tolerates additive fields and explicitly marks an unknown task status', () => {
  expect(decodeTask({ ...fixture.tasks[0], status: 'paused', future_field: 1 }).status).toBe('unknown');
  expect(decodeHealth({ ...fixture.health, future_field: true })).toEqual(fixture.health);
});
