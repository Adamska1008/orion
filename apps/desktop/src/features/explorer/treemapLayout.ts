import type { TreemapNode } from '../../lib/api';

interface Item { node: TreemapNode | null; bytes: number; count: number }
export interface MapRect extends Item {
  key: string; parent: number; x: number; y: number; width: number; height: number;
  color: number; children: MapRect[];
}
interface Bounds { x: number; y: number; width: number; height: number }

/** Balanced area-preserving splits, bounded by the server's subtree budget. */
function partition(items: Item[], bounds: Bounds): Array<Item & Bounds> {
  if (!items.length) return [];
  if (items.length === 1) return [{ ...items[0], ...bounds }];
  const total = items.reduce((sum, item) => sum + item.bytes, 0);
  let sum = 0, split = 1, distance = Infinity;
  for (let i = 1; i < items.length; i++) {
    sum += items[i - 1].bytes;
    if (Math.abs(total / 2 - sum) < distance) { split = i; distance = Math.abs(total / 2 - sum); }
  }
  const first = items.slice(0, split), second = items.slice(split);
  const fraction = first.reduce((sum, item) => sum + item.bytes, 0) / total;
  const { x, y, width, height } = bounds;
  return width >= height
    ? [...partition(first, { x, y, width: width * fraction, height }), ...partition(second, { x: x + width * fraction, y, width: width * (1 - fraction), height })]
    : [...partition(first, { x, y, width, height: height * fraction }), ...partition(second, { x, y: y + height * fraction, width, height: height * (1 - fraction) })];
}

export const MAP_GAP = 3;
export const MAP_HEADER = 30;
export const MAP_PADDING = 5;

export function layoutTreemap(root: TreemapNode, width: number, height: number, inheritedColor?: number): MapRect[] {
  if (!root.expanded || root.logical_bytes <= 0 || width <= 0 || height <= 0) return [];
  let otherBytes = root.omitted_bytes, otherCount = root.omitted_count;
  const items: Item[] = [];
  for (const node of root.children) {
    if (node.logical_bytes <= 0) continue;
    // Coalesce tiny siblings without losing area or creating unusable hit targets.
    if (node.logical_bytes / root.logical_bytes * width * height < 850) {
      otherBytes += node.logical_bytes; otherCount++;
    } else items.push({ node, bytes: node.logical_bytes, count: 1 });
  }
  if (otherBytes > 0) items.push({ node: null, bytes: otherBytes, count: otherCount });
  items.sort((a, b) => b.bytes - a.bytes || (a.node?.id ?? Infinity) - (b.node?.id ?? Infinity));
  return partition(items, { x: 0, y: 0, width, height }).map(rect => {
    const color = inheritedColor ?? (rect.node?.id ?? root.id) % 6;
    const innerWidth = rect.width - MAP_GAP - MAP_PADDING * 2;
    const innerHeight = rect.height - MAP_GAP - MAP_HEADER - MAP_PADDING;
    const children = rect.node?.expanded && rect.width >= 112 && rect.height >= 106
      ? layoutTreemap(rect.node, innerWidth, innerHeight, color) : [];
    return { ...rect, key: rect.node ? `entry-${rect.node.id}` : `other-${root.id}`, parent: root.id, color, children };
  });
}
