import type { CSSProperties, KeyboardEvent, ReactNode, Ref } from 'react';
import { File, Folder, Layers3 } from 'lucide-react';
import type { TreemapNode } from '../../lib/api';
import { cn, formatBytes } from '../../lib/utils';
import { MAP_GAP, MAP_HEADER, MAP_PADDING, type MapRect } from './treemapLayout';
import './space-map.css';

interface Props {
  root: TreemapNode; selected: number | null; interactive: boolean;
  mapRef: Ref<HTMLDivElement>; rectangles: MapRect[]; selectedAggregate: number | null;
  onSelect: (id: number) => void; onNavigate: (id: number) => void; onShowList: (id: number) => void;
  onSelectAggregate: (parent: number) => void;
}

export function SpaceMap({ root, selected, interactive, mapRef, rectangles, selectedAggregate, onSelect, onSelectAggregate, onNavigate, onShowList }: Props) {
  function renderRect(rect: MapRect): ReactNode {
    const node = rect.node;
    const directory = node?.kind === 'directory';
    const name = node?.name ?? `其他 ${rect.count.toLocaleString()} 项`;
    const percentage = root.logical_bytes ? rect.bytes / root.logical_bytes * 100 : 0;
    const tooltip = `${name}\n${formatBytes(rect.bytes)} · 占当前目录 ${percentage.toFixed(1)}%${directory ? '\n双击或按 Enter 进入目录' : '\n单击查看详情'}`;
    const style = { left: rect.x + MAP_GAP / 2, top: rect.y + MAP_GAP / 2,
      width: Math.max(0, rect.width - MAP_GAP), height: Math.max(0, rect.height - MAP_GAP),
      '--map-tone': `var(--map-color-${rect.color})` } as CSSProperties;
    const isSelected = node ? selected === node.id : selectedAggregate === rect.parent;
    const select = () => node ? onSelect(node.id) : onSelectAggregate(rect.parent);
    const enter = () => { if (directory) onNavigate(node.id); };
    const keyboard = (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === 'Enter' && directory) { event.preventDefault(); enter(); }
    };
    const icon = node ? directory ? <Folder size={15} /> : <File size={15} /> : <Layers3 size={15} />;
    if (rect.children.length) return <div key={rect.key} role="group" aria-label={name} className={cn('map-group', isSelected && 'selected')} style={style}>
      <button type="button" className="map-group-label" data-entry-id={node!.id} aria-pressed={isSelected} aria-label={`选择 ${name}`} title={tooltip} disabled={!interactive} onClick={select} onDoubleClick={enter} onKeyDown={keyboard}>{icon}<span>{name}</span><span>{formatBytes(rect.bytes)}</span></button>
      <div className="map-group-content" style={{ inset: `${MAP_HEADER}px ${MAP_PADDING}px ${MAP_PADDING}px` }}>{rect.children.map(renderRect)}</div>
    </div>;
    return <button type="button" key={rect.key} data-entry-id={node?.id} data-aggregate-parent={node ? undefined : rect.parent} className={cn('map-tile', isSelected && 'selected', !node && 'map-other', (rect.width < 165 || rect.height < 125) && 'map-small', (rect.width < 105 || rect.height < 75) && 'map-tiny', rect.height < 51 && 'map-label-only')} style={style}
      aria-label={`${name}，${formatBytes(rect.bytes)}`} aria-pressed={isSelected} title={tooltip} disabled={!interactive} onClick={select} onDoubleClick={enter} onKeyDown={keyboard}>
      <span className="map-name">{icon}<span>{name}</span></span><span className="map-size">{formatBytes(rect.bytes)}</span>
      <span className="map-meta">{percentage.toFixed(1)}%{directory ? ` · ${node.child_count.toLocaleString()} 项` : ''}</span>
    </button>;
  }

  return <div className="space-map" ref={mapRef} aria-label="空间矩形图">
    {root.logical_bytes > 0 ? rectangles.map(renderRect) : <div className="map-empty"><Folder size={25} /><span>此目录没有可绘制的大小</span><button type="button" onClick={() => onShowList(root.id)}>在列表中查看空文件和其他条目</button></div>}
  </div>;
}
