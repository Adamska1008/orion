import { useRef, useState, type KeyboardEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { ArrowDown, ArrowLeft, ArrowRight, Check, ChevronRight, Copy, File, Folder, FolderOpen, Info, LoaderCircle, TriangleAlert } from 'lucide-react';
import { type Api, type Entry, type Task } from '../../lib/api';
import { desktop, errorText } from '../../lib/desktop';
import { cn, displayPath, formatBytes, formatDate } from '../../lib/utils';
import { Button } from '../../components/ui/button';
import { active } from '../scan/useScanTask';
import { useExplorer } from './useExplorer';

function EntryIcon({ entry }: { entry: Entry }) {
  return entry.kind === 'directory' ? <Folder className="entry-icon folder-icon" /> : <File className="entry-icon file-icon" />;
}

export function DirectoryExplorer({ api, task, online, onError }: { api: Api | null; task: Task; online: boolean; onError: (message: string) => void }) {
  const explorer = useExplorer(api, task, online, onError);
  const { parent, offset, pageSize, page, directory, selected, detail, loadingPage, pageCount, pageNumber, setSelected } = explorer;
  const [showIssues, setShowIssues] = useState(false);
  const [copied, setCopied] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({ count: page.entries.length, getScrollElement: () => listRef.current, estimateSize: () => 48, overscan: 8 });
  function navigate(id: number) { explorer.navigate(id); listRef.current?.scrollTo(0, 0); }
  function changePage(next: number) { explorer.changePage(next); listRef.current?.scrollTo(0, 0); }
  function changePageSize(next: number) { explorer.changePageSize(next); listRef.current?.scrollTo(0, 0); }
  function moveSelection(event: KeyboardEvent<HTMLDivElement>) {
    const entries = page.entries;
    const current = entries.findIndex(e => e.id === selected);
    let next = current;
    if (event.key === 'ArrowDown') next = Math.min(current + 1, entries.length - 1);
    else if (event.key === 'ArrowUp') next = Math.max(0, current - 1);
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = entries.length - 1;
    else if (event.key === 'Enter' && entries[current]?.kind === 'directory') { event.preventDefault(); navigate(entries[current].id); return; }
    else if ((event.key === 'Backspace' || (event.altKey && event.key === 'ArrowLeft')) && directory?.parent_id !== null && directory) {
      event.preventDefault(); navigate(directory.parent_id); return;
    } else return;
    event.preventDefault();
    if (entries[next]) { setSelected(entries[next].id); virtual.scrollToIndex(next); }
  }

  const shownDetail = detail ?? directory;
  const running = active(task);
  const crumbs = directory ? [...directory.ancestors, directory].filter(c => c.id !== 0) : [];
  return <><section className="explorer">
        <div className="explorer-top"><h2>目录与文件</h2><span>{page.total.toLocaleString()} 项 · 按大小降序</span></div>
          <div className="breadcrumb-bar"><Button variant="ghost" size="icon" aria-label="返回上级目录" onClick={() => directory?.parent_id != null && navigate(directory.parent_id)} disabled={!directory || directory.parent_id === null}><ArrowLeft /></Button><nav aria-label="目录层级"><span><button title={displayPath(task.root)} onClick={() => navigate(0)}>{displayPath(task.root)}</button></span>{crumbs.map(c => <span key={c.id}><ChevronRight size={13} /><button title={c.name} onClick={() => navigate(c.id)}>{c.name}</button></span>)}</nav>{loadingPage && <LoaderCircle className="spin subtle" size={14} />}</div>
          <div className="explorer-body">
            <div className="file-panel"><div className="table-header"><span>名称</span><span>大小 <ArrowDown size={12} /></span><span>占当前目录</span></div>
              <div ref={listRef} className="file-list" aria-busy={loadingPage} role="listbox" tabIndex={0} aria-label="目录内容，方向键选择，Enter 进入目录，退格返回" aria-activedescendant={selected === null ? undefined : `entry-${selected}`} onKeyDown={moveSelection}>
                {page.entries.length ? <div style={{ height: virtual.getTotalSize(), position: 'relative' }}>{virtual.getVirtualItems().map(row => {
                  const entry = page.entries[row.index];
                  const percentage = directory?.logical_bytes ? Math.min(100, entry.logical_bytes / directory.logical_bytes * 100) : 0;
                  return <div key={entry.id} id={`entry-${entry.id}`} role="option" aria-selected={selected === entry.id} className={cn('file-row', selected === entry.id && 'selected')} style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: row.size, transform: `translateY(${row.start}px)` }} onClick={() => { setSelected(entry.id); listRef.current?.focus(); }} onDoubleClick={() => entry.kind === 'directory' && navigate(entry.id)}>
                    <div className="entry-name"><EntryIcon entry={entry} /><span title={entry.name}>{entry.name}</span>{entry.kind === 'link' && <small>链接 · 跳过</small>}{entry.kind === 'directory' && <button className="enter-directory" aria-label={`进入 ${entry.name}`} onClick={e => { e.stopPropagation(); navigate(entry.id); }}><ChevronRight size={15} /></button>}</div><span className="entry-size">{formatBytes(entry.logical_bytes)}</span><div className="entry-share"><div className="share-track"><span style={{ width: `${percentage}%` }} /></div><span>{percentage.toFixed(1)}%</span></div>
                  </div>;
                })}</div> : <div className="list-empty">{loadingPage ? '正在读取目录…' : running ? '正在发现文件，结果会逐步出现。' : '此目录没有已统计的内容。'}</div>}
              </div>
              <nav className="pagination" aria-label="目录分页">
                <div className="pagination-info">
                  <span aria-live="polite">共 {page.total.toLocaleString()} 项{page.entries.length > 0 && ` · ${offset + 1}–${offset + page.entries.length}`}</span>
                  <label>每页<select aria-label="每页条数" value={pageSize} onChange={event => changePageSize(Number(event.target.value))} disabled={!online}><option value={15}>15</option><option value={50}>50</option><option value={100}>100</option></select>项</label>
                </div>
                <div className="pagination-controls">
                  <Button variant="outline" size="sm" aria-label="上一页" disabled={!online || loadingPage || offset === 0} onClick={() => changePage(pageNumber - 1)}><ArrowLeft />上一页</Button>
                  <form key={`${parent}:${offset}:${pageSize}`} onSubmit={event => { event.preventDefault(); changePage(Number(new FormData(event.currentTarget).get('page'))); }}>
                    <label>第<input name="page" aria-label="跳转页码" type="number" min={1} max={pageCount} step={1} defaultValue={pageNumber} disabled={!online || page.total === 0} />/ {pageCount.toLocaleString()} 页</label>
                    <Button type="submit" variant="ghost" size="sm" disabled={!online || loadingPage || page.total === 0}>跳转</Button>
                  </form>
                  <Button variant="outline" size="sm" aria-label="下一页" disabled={!online || loadingPage || offset + pageSize >= page.total} onClick={() => changePage(pageNumber + 1)}>下一页<ArrowRight /></Button>
                </div>
              </nav>
            </div>
            <aside className="detail-panel"><div className="detail-heading">{selected === null ? '当前目录' : '条目详情'}<Info size={15} /></div>{shownDetail && <><h3>{shownDetail.id === 0 ? displayPath(task.root).split(/[\\/]/).filter(Boolean).at(-1) : shownDetail.name}</h3><p className="detail-kind">{{ directory: '文件夹', file: '文件', link: '链接（未跟随）', other: '特殊文件' }[shownDetail.kind]}</p><dl><dt>逻辑大小</dt><dd className="detail-size">{formatBytes(shownDetail.logical_bytes)}</dd><dt>修改时间</dt><dd>{formatDate(shownDetail.modified_at)}</dd><dt>完整路径</dt><dd className="detail-path">{displayPath(shownDetail.path)}</dd></dl><div className="detail-buttons"><Button variant="outline" size="sm" onClick={async () => { try { await navigator.clipboard.writeText(displayPath(shownDetail.path)); setCopied(true); setTimeout(() => setCopied(false), 1800); } catch { onError('复制失败，请从详情中选择路径手动复制。'); } }}>{copied ? <Check /> : <Copy />}{copied ? '已复制' : '复制路径'}</Button>{desktop.available() && <Button variant="outline" size="sm" onClick={() => desktop.reveal(displayPath(shownDetail.path)).catch(e => onError(errorText(e)))}><FolderOpen />系统定位</Button>}</div><div className="detail-footnote"><Info size={13} /><span>数据来自本次扫描。文件可能已变化；重新扫描可更新结果。</span></div></>}</aside>
          </div>
        <footer className="explorer-footer"><span>{task.complete ? '所选范围已枚举' : '部分结果，尚未完整覆盖'} · 实际磁盘占用未知</span>{task.issue_count > 0 && <button className="issues-trigger" aria-expanded={showIssues} aria-controls="scan-issues" onClick={() => setShowIssues(v => !v)}><TriangleAlert size={13} />{task.issue_count} 项未覆盖或异常<ChevronRight size={13} /></button>}</footer>
      </section>
{showIssues && task && task.issue_count > 0 && <section id="scan-issues" className="issues-panel"><h2>未覆盖与异常 <span>展示前 {task.issues.length} 项，共 {task.issue_count} 项</span></h2>{task.issues.map((issue, i) => <div key={i}><TriangleAlert size={15} /><div><code>{displayPath(issue.path)}</code><p>{issue.message}</p></div></div>)}</section>}
  </>;
}
