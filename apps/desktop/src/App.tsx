import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { isTauri, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { Activity, ArrowDown, ArrowLeft, ArrowRight, Check, ChevronRight, Copy, File, Folder, FolderOpen, HardDrive, Info, LayoutGrid, LoaderCircle, Moon, Plug, RefreshCw, ScanLine, Settings2, ShieldCheck, Square, TriangleAlert, X } from 'lucide-react';
import { Api, ApiError, validateConnection, type Connection, type Detail, type Entry, type Page, type Task } from './lib/api';
import { cn, displayPath, formatBytes, formatDate } from './lib/utils';
import { useTheme, type ThemePreference } from './lib/theme';
import { Button } from './components/ui/button';

const labels = { running: '正在扫描', cancelling: '正在取消', cancelled: '已取消 · 部分结果', completed: '扫描完成', failed: '扫描失败' };
const emptyPage: Page = { revision: 0, total: 0, offset: 0, entries: [] };
const active = (task: Task | null) => task?.status === 'running' || task?.status === 'cancelling';
const errorText = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : '操作失败，请重试。';

function EntryIcon({ entry }: { entry: Entry }) {
  return entry.kind === 'directory' ? <Folder className="entry-icon folder-icon" /> : <File className="entry-icon file-icon" />;
}

export default function App() {
  const [theme, setTheme] = useTheme();
  const [connection, setConnection] = useState<Connection | null>(null);
  const [connectionForm, setConnectionForm] = useState({ url: 'http://127.0.0.1:', token: '' });
  const [settings, setSettings] = useState(false);
  const [online, setOnline] = useState(false);
  const [connectionError, setConnectionError] = useState('');
  const [message, setMessage] = useState('');
  const [notice, setNotice] = useState('');
  const [task, setTask] = useState<Task | null>(null);
  const [root, setRoot] = useState('');
  const [parent, setParent] = useState(0);
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<Page>(emptyPage);
  const [directory, setDirectory] = useState<Detail | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadingPage, setLoadingPage] = useState(false);
  const [showIssues, setShowIssues] = useState(false);
  const [copied, setCopied] = useState(false);
  const instanceRef = useRef<string | null>(null);
  const requestRef = useRef<{ root: string; id: string } | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const api = useMemo(() => connection ? new Api(connection) : null, [connection]);
  const virtual = useVirtualizer({ count: page.entries.length, getScrollElement: () => listRef.current, estimateSize: () => 48, overscan: 8 });

  useEffect(() => {
    if (isTauri()) {
      invoke<Connection>('server_connection').then(c => setConnection(validateConnection(c))).catch(e => {
        setConnectionError(errorText(e)); setSettings(true);
      });
    } else {
      setSettings(true);
    }
  }, []);

  useEffect(() => {
    if (!api) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const health = await api!.health(controller.signal);
        if (health.name !== 'orion-server' || health.api_version !== 1) throw new Error('服务版本不兼容。');
        const tasks = await api!.tasks(controller.signal);
        if (controller.signal.aborted) return;
        if (instanceRef.current && instanceRef.current !== health.instance_id) {
          setNotice('服务已重启或切换，之前的内存结果不再可用。'); requestRef.current = null;
        }
        instanceRef.current = health.instance_id;
        setOnline(true); setConnectionError(''); setTask(tasks[0] ?? null);
      } catch (error) {
        if (!controller.signal.aborted) { setOnline(false); setConnectionError(errorText(error)); }
      } finally {
        if (!controller.signal.aborted) timer = setTimeout(poll, 800);
      }
    }
    setOnline(false); void poll();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [api]);

  useEffect(() => {
    setParent(0); setOffset(0); setSelected(null); setDirectory(null); setPage(emptyPage); setDetail(null);
    if (task) setRoot(displayPath(task.root));
  }, [task?.id]);

  useEffect(() => {
    if (!api || !task || !online) return;
    const controller = new AbortController();
    setLoadingPage(true);
    Promise.all([api.page(task.id, parent, offset, controller.signal), api.detail(task.id, parent, controller.signal)])
      .then(([next, dir]) => {
        if (controller.signal.aborted) return;
        if (next.total > 0 && offset >= next.total) { setOffset(0); return; }
        setPage(next); setDirectory(dir);
      }).catch(error => { if (!controller.signal.aborted) setMessage(errorText(error)); })
      .finally(() => { if (!controller.signal.aborted) setLoadingPage(false); });
    return () => controller.abort();
  }, [api, task?.id, task?.revision, parent, offset, online]);

  useEffect(() => {
    if (!api || !task || selected === null || !online) { setDetail(null); return; }
    const controller = new AbortController();
    api.detail(task.id, selected, controller.signal).then(setDetail).catch(error => {
      if (!controller.signal.aborted) setMessage(errorText(error));
    });
    return () => controller.abort();
  }, [api, task?.id, task?.revision, selected, online]);

  const navigate = useCallback((id: number) => {
    setParent(id); setOffset(0); setSelected(null); setPage(emptyPage); setDirectory(null);
    listRef.current?.scrollTo(0, 0);
  }, []);

  async function startScan(path = root) {
    if (!api || !online || !path.trim() || busy || active(task)) return;
    setBusy(true); setMessage(''); setNotice('');
    // Keep the same key after a timeout: retry cannot silently create a second scan.
    if (!requestRef.current || requestRef.current.root !== path.trim()) requestRef.current = { root: path.trim(), id: crypto.randomUUID() };
    try {
      const next = await api.start(path.trim(), requestRef.current.id);
      setTask(next); navigate(0); requestRef.current = null;
    } catch (error) {
      setMessage(errorText(error));
      if (error instanceof ApiError && error.code !== 'disconnected') requestRef.current = null;
    } finally { setBusy(false); }
  }

  async function chooseDirectory() {
    if (!isTauri()) { document.getElementById('root-path')?.focus(); return; }
    try {
      const chosen = await open({ directory: true, multiple: false, title: '选择要扫描的目录' });
      if (typeof chosen === 'string') { setRoot(chosen); await startScan(chosen); }
    } catch (error) { setMessage(errorText(error)); }
  }

  async function cancel() {
    if (!api || !task) return;
    setBusy(true);
    try { setTask(await api.cancel(task.id)); } catch (error) { setMessage(errorText(error)); }
    finally { setBusy(false); }
  }

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
  const elapsed = task ? Math.max(0, ((task.finished_at ?? Date.now()) - task.started_at) / 1000) : 0;
  const crumbs = directory ? [...directory.ancestors, directory] : [];
  const connect = () => {
    try { setConnection(validateConnection(connectionForm)); setSettings(false); setMessage(''); }
    catch (error) { setMessage(errorText(error)); }
  };

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark"><ScanLine size={23} strokeWidth={1.8} /></div><span>orion<span className="brand-dot">.</span></span></div>
      <div className="workspace-label">我的工作空间</div>
      <button className="nav-item active" onClick={() => { setSettings(false); if (task) navigate(0); }}><LayoutGrid size={17} />空间总览</button>
      <div className="sidebar-section"><span>当前扫描</span>{task ? <button title={displayPath(task.root)} className="scan-shortcut" onClick={() => navigate(0)}><Folder size={16} /><span>{displayPath(task.root).split(/[\\/]/).filter(Boolean).at(-1) || displayPath(task.root)}</span>{running && <span className="live-dot" />}</button> : <p>选择一个目录开始</p>}</div>
      <div className="sidebar-bottom">
        <label className="theme-picker"><Moon size={15} /><span>外观</span><select aria-label="外观主题" value={theme} onChange={event => setTheme(event.target.value as ThemePreference)}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">暗色</option></select></label>
        <div className="read-only-note"><ShieldCheck size={17} /><div>只读扫描<span>文件始终留在原处</span></div></div>
        <button className="server-button" onClick={() => setSettings(v => !v)}><span className={cn('connection-dot', online && 'connected')} /><span>{online ? '本地服务已连接' : '连接本地服务'}</span><Settings2 size={14} /></button>
        <div className="version">ORION <span>0.1 · MVP</span></div>
      </div>
    </aside>

    <main className="main-content">
      <header className="page-header"><div><div className="eyebrow">DISK EXPLORER</div><h1>空间总览</h1><p>从一个目录开始，找到磁盘空间的去向。</p></div><div className="header-actions"><span className="read-only-badge"><ShieldCheck size={13} />只读模式</span><Button onClick={chooseDirectory} disabled={!online || busy || running}><FolderOpen />选择目录</Button></div></header>

      {settings && <section className="connection-panel"><div className="connection-title"><Plug size={18} /><strong>连接本地服务</strong><Button variant="ghost" size="icon" aria-label="关闭连接设置" onClick={() => setSettings(false)}><X /></Button></div><p>桌面客户端会读取本地连接信息。服务重启后可重新连接，也可以手动填写地址与令牌。</p>{isTauri() && <Button className="mb-3" variant="outline" size="sm" onClick={() => invoke<Connection>('server_connection').then(c => { setConnection(validateConnection(c)); setSettings(false); }).catch(e => setMessage(errorText(e)))}><RefreshCw />重新读取连接</Button>}<form onSubmit={e => { e.preventDefault(); connect(); }}><label>服务地址<input aria-label="服务地址" value={connectionForm.url} onChange={e => setConnectionForm(v => ({ ...v, url: e.target.value }))} placeholder="http://127.0.0.1:43120" /></label><label>连接令牌<input aria-label="连接令牌" type="password" autoComplete="off" value={connectionForm.token} onChange={e => setConnectionForm(v => ({ ...v, token: e.target.value }))} /></label><Button type="submit">连接服务<ArrowRight /></Button></form></section>}
      {(connectionError || message) && <div className="alert" role="alert"><TriangleAlert size={16} /><span>{message || connectionError}</span>{message && <button aria-label="关闭提示" onClick={() => setMessage('')}><X size={15} /></button>}</div>}
      {notice && <div className="notice" role="status"><Info size={16} />{notice}<button aria-label="关闭通知" onClick={() => setNotice('')}><X size={15} /></button></div>}

      <form className="path-bar" onSubmit={e => { e.preventDefault(); void startScan(); }}><FolderOpen size={18} /><input id="root-path" aria-label="扫描目录路径" placeholder="输入目录路径，例如 C:\Users\你的用户名\Downloads" value={root} onChange={e => setRoot(e.target.value)} disabled={running} /><Button variant="outline" type="submit" disabled={!online || busy || running || !root.trim()}><ScanLine />{task ? '重新扫描' : '开始扫描'}</Button></form>

      <section className="stats-grid" aria-label="扫描概要">
        <div className="stat-card"><div className="stat-label">已统计大小<HardDrive size={17} /></div><div className="stat-value">{task ? formatBytes(task.logical_bytes) : '—'}</div><div className="stat-caption">文件逻辑大小<span className="tiny-dot" />实际占用暂未知</div></div>
        <div className="stat-card"><div className="stat-label">已发现文件<File size={17} /></div><div className="stat-value">{task ? task.files.toLocaleString() : '—'}<span>个</span></div><div className="stat-caption">{task ? `${task.directories.toLocaleString()} 个目录（含扫描根目录）` : '包含所选目录的各级子目录'}</div></div>
        <div className="stat-card"><div className="stat-label">扫描状态<Activity size={17} /></div><div className={cn('stat-status', running && 'is-running')}>{running ? <LoaderCircle className="spin" size={21} /> : task?.complete ? <Check size={21} /> : <span className="status-orb" />}{task ? labels[task.status] : '等待扫描'}</div><div className="stat-caption">{task ? `${elapsed.toFixed(1)} 秒 · ${task.complete ? '所选范围已枚举' : '部分结果，尚未完整覆盖'}` : '扫描过程中也可以浏览结果'}</div></div>
      </section>

      <section className="explorer">
        <div className="explorer-top"><div><h2>目录与文件</h2><span>{task ? `${page.total.toLocaleString()} 项 · 按大小降序` : '你的空间，一目了然'}</span></div><div className="explorer-actions">{running && <Button variant="destructive" size="sm" onClick={cancel} disabled={!online || busy || task?.status === 'cancelling'}><Square />{task?.status === 'cancelling' ? '取消中' : '取消扫描'}</Button>}{task && <Button variant="ghost" size="sm" onClick={() => void startScan(task.root)} disabled={!online || busy || running}><RefreshCw />重扫</Button>}</div></div>
        {task ? <>
          <div className="breadcrumb-bar"><Button variant="ghost" size="icon" aria-label="返回上级目录" onClick={() => directory?.parent_id != null && navigate(directory.parent_id)} disabled={!directory || directory.parent_id === null}><ArrowLeft /></Button><nav aria-label="目录层级">{crumbs.map((c, i) => <span key={c.id}>{i > 0 && <ChevronRight size={13} />}<button title={c.id === 0 ? displayPath(task.root) : c.name} onClick={() => navigate(c.id)}>{c.id === 0 ? displayPath(task.root) : c.name}</button></span>)}</nav>{loadingPage && <LoaderCircle className="spin subtle" size={14} />}</div>
          <div className="explorer-body">
            <div className="file-panel"><div className="table-header"><span>名称</span><span>大小 <ArrowDown size={12} /></span><span>占当前目录</span></div>
              <div ref={listRef} className="file-list" role="listbox" tabIndex={0} aria-label="目录内容，方向键选择，Enter 进入目录，退格返回" aria-activedescendant={selected === null ? undefined : `entry-${selected}`} onKeyDown={moveSelection}>
                {page.entries.length ? <div style={{ height: virtual.getTotalSize(), position: 'relative' }}>{virtual.getVirtualItems().map(row => {
                  const entry = page.entries[row.index];
                  const percentage = directory?.logical_bytes ? Math.min(100, entry.logical_bytes / directory.logical_bytes * 100) : 0;
                  return <div key={entry.id} id={`entry-${entry.id}`} role="option" aria-selected={selected === entry.id} className={cn('file-row', selected === entry.id && 'selected')} style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: row.size, transform: `translateY(${row.start}px)` }} onClick={() => { setSelected(entry.id); listRef.current?.focus(); }} onDoubleClick={() => entry.kind === 'directory' && navigate(entry.id)}>
                    <div className="entry-name"><EntryIcon entry={entry} /><span title={entry.name}>{entry.name}</span>{entry.kind === 'link' && <small>链接 · 跳过</small>}{entry.kind === 'directory' && <button className="enter-directory" aria-label={`进入 ${entry.name}`} onClick={e => { e.stopPropagation(); navigate(entry.id); }}><ChevronRight size={15} /></button>}</div><span className="entry-size">{formatBytes(entry.logical_bytes)}</span><div className="entry-share"><div className="share-track"><span style={{ width: `${percentage}%` }} /></div><span>{percentage.toFixed(1)}%</span></div>
                  </div>;
                })}</div> : <div className="list-empty">{loadingPage ? '正在读取目录…' : running ? '正在发现文件，结果会逐步出现。' : '此目录没有已统计的内容。'}</div>}
              </div>
              <div className="pagination"><span>{page.total ? `${offset + 1}–${Math.min(offset + 200, page.total)} / ${page.total.toLocaleString()}` : '0 项'}{running && ' · 扫描中，排序会更新'}</span><div><Button variant="ghost" size="icon" aria-label="上一页" disabled={offset === 0} onClick={() => { setOffset(Math.max(0, offset - 200)); setSelected(null); listRef.current?.scrollTo(0, 0); }}><ArrowLeft /></Button><Button variant="ghost" size="icon" aria-label="下一页" disabled={offset + 200 >= page.total} onClick={() => { setOffset(offset + 200); setSelected(null); listRef.current?.scrollTo(0, 0); }}><ArrowRight /></Button></div></div>
            </div>
            <aside className="detail-panel"><div className="detail-heading">{selected === null ? '当前目录' : '条目详情'}<Info size={15} /></div>{shownDetail && <><div className="detail-icon"><EntryIcon entry={shownDetail} /></div><h3>{shownDetail.id === 0 ? displayPath(task.root).split(/[\\/]/).filter(Boolean).at(-1) : shownDetail.name}</h3><p className="detail-kind">{{ directory: '文件夹', file: '文件', link: '链接（未跟随）', other: '特殊文件' }[shownDetail.kind]}</p><dl><dt>逻辑大小</dt><dd className="detail-size">{formatBytes(shownDetail.logical_bytes)}</dd><dt>实际磁盘占用</dt><dd>未知</dd><dt>修改时间</dt><dd>{formatDate(shownDetail.modified_at)}</dd><dt>完整路径</dt><dd className="detail-path">{displayPath(shownDetail.path)}</dd></dl><div className="detail-buttons"><Button variant="outline" size="sm" onClick={async () => { try { await navigator.clipboard.writeText(displayPath(shownDetail.path)); setCopied(true); setTimeout(() => setCopied(false), 1800); } catch { setMessage('复制失败，请从详情中选择路径手动复制。'); } }}>{copied ? <Check /> : <Copy />}{copied ? '已复制' : '复制路径'}</Button>{isTauri() && <Button variant="outline" size="sm" onClick={() => revealItemInDir(displayPath(shownDetail.path)).catch(e => setMessage(errorText(e)))}><FolderOpen />系统定位</Button>}</div><div className="detail-footnote"><Info size={13} /><span>数据来自本次扫描。文件可能已变化；重新扫描可更新结果。</span></div></>}</aside>
          </div>
        </> : <div className="empty-state"><div className="empty-illustration"><div className="orbit one" /><div className="orbit two" /><FolderOpen size={43} strokeWidth={1.3} /></div><h3>先看看，空间都去哪了</h3><p>选择一个常用目录。按大小逐层浏览，<br />让藏在深处的大文件浮出水面。</p><Button variant="outline" onClick={chooseDirectory} disabled={!online}><FolderOpen />选择扫描目录<ArrowRight /></Button><div className="empty-hint"><ShieldCheck size={13} />仅查看文件信息，不修改或删除文件</div></div>}
        <footer className="explorer-footer"><span><span className={cn('connection-dot', online && 'connected')} />{online ? '本地计算' : '服务未连接'}<span className="footer-divider">/</span>{task ? '结果保存在当前服务内存中' : '等待选择目录'}</span>{task && task.issue_count > 0 ? <button className="issues-trigger" onClick={() => setShowIssues(v => !v)}><TriangleAlert size={13} />{task.issue_count} 项未覆盖或异常<ChevronRight size={13} /></button> : <span><ShieldCheck size={13} />只读</span>}</footer>
      </section>
      {showIssues && task && <section className="issues-panel"><h2>未覆盖与异常 <span>展示前 {task.issues.length} 项，共 {task.issue_count} 项</span></h2>{task.issues.map((issue, i) => <div key={i}><TriangleAlert size={15} /><div><code>{displayPath(issue.path)}</code><p>{issue.message}</p></div></div>)}</section>}
    </main>
  </div>;
}
