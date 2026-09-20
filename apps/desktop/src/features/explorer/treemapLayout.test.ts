import { describe, expect, it } from 'vitest';
import type { TreemapNode } from '../../lib/api';
import { layoutTreemap } from './treemapLayout';

function entry(id: number, bytes: number, children: TreemapNode[] = []): TreemapNode {
  return { id, parent_id: id ? 0 : null, name: `entry-${id}`, kind: children.length || !id ? 'directory' : 'file', logical_bytes: bytes,
    allocated_bytes: null, modified_at: null, enumerated: true, child_count: children.length, zero_count: 0,
    expanded: children.length > 0 || !id, omitted_count: 0, omitted_bytes: 0, children };
}

describe('space map geometry', () => {
  it.each([[900, 500], [360, 420], [200, 110]])('preserves area and prevents overlapping siblings at %i × %i', (width, height) => {
    const root = entry(0, 100, [entry(1, 60), entry(2, 25), entry(3, 15)]);
    const rectangles = layoutTreemap(root, width, height);
    expect(rectangles.reduce((sum, rect) => sum + rect.width * rect.height, 0)).toBeCloseTo(width * height);
    for (const [i, rect] of rectangles.entries()) {
      expect(rect.width * rect.height / (width * height)).toBeCloseTo(rect.bytes / root.logical_bytes);
      expect(rect.x).toBeGreaterThanOrEqual(0); expect(rect.y).toBeGreaterThanOrEqual(0);
      expect(rect.x + rect.width).toBeLessThanOrEqual(width + 0.001);
      expect(rect.y + rect.height).toBeLessThanOrEqual(height + 0.001);
      for (const other of rectangles.slice(i + 1)) {
        const overlap = Math.max(0, Math.min(rect.x + rect.width, other.x + other.width) - Math.max(rect.x, other.x))
          * Math.max(0, Math.min(rect.y + rect.height, other.y + other.height) - Math.max(rect.y, other.y));
        expect(overlap).toBeCloseTo(0);
      }
    }
  });

  it('combines server omissions and small visible items without dropping bytes or counts', () => {
    const root = { ...entry(0, 10000, [entry(1, 9000), ...Array.from({ length: 40 }, (_, i) => entry(i + 2, 1))]), omitted_count: 960, omitted_bytes: 960, child_count: 1001 };
    const rectangles = layoutTreemap(root, 600, 400);
    const other = rectangles.find(rect => !rect.node)!;
    expect(other.bytes).toBe(1000); expect(other.count).toBe(1000); expect(other.parent).toBe(0);
    expect(rectangles.reduce((sum, rect) => sum + rect.bytes, 0)).toBe(10000);
  });

  it('keeps nested descendants inside their frame and handles empty or zero-size layouts', () => {
    const root = entry(0, 100, [entry(1, 100, [entry(2, 75), entry(3, 25)])]);
    const group = layoutTreemap(root, 600, 400)[0];
    expect(group.children).toHaveLength(2);
    expect(group.children.every(child => child.x + child.width < group.width && child.y + child.height < group.height)).toBe(true);
    expect(group.children.every(child => child.color === group.color)).toBe(true);
    expect(layoutTreemap(entry(0, 0), 600, 400)).toEqual([]);
    expect(layoutTreemap(root, 0, 400)).toEqual([]);
    expect(layoutTreemap(root, 600, 0)).toEqual([]);
  });
});
