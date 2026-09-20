import { useEffect, useMemo, useState } from 'react';
import type { TreemapNode } from '../../lib/api';
import { layoutTreemap } from './treemapLayout';

export function useTreemapLayout(root: TreemapNode | null) {
  const [element, ref] = useState<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    if (!element) return;
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  const rectangles = useMemo(() => root ? layoutTreemap(root, size.width, size.height) : [], [root, size.width, size.height]);
  return { ref, rectangles };
}
