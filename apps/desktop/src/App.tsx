import { useEffect, useState } from 'react';
import { ArrowRight, Check, FolderOpen, Info, LoaderCircle, Moon, Plug, RefreshCw, ScanLine, Settings2, ShieldCheck, Square, TriangleAlert, X } from 'lucide-react';
import { desktop, errorText } from './lib/desktop';
import { cn, displayPath, formatBytes } from './lib/utils';
import { useTheme, type ThemePreference } from './lib/theme';
import { Button } from './components/ui/button';
import { useServerSession } from './features/session/useServerSession';
import { active, useScanTask } from './features/scan/useScanTask';
import { DirectoryExplorer } from './features/explorer/DirectoryExplorer';

const labels = { running: '正在扫描', cancelling: '正在取消', cancelled: '已取消 · 部分结果', completed: '扫描完成', failed: '扫描失败', unknown: '未知任务状态' };

export default function App() {
  const [theme, setTheme] = useTheme();
  const session = useServerSession();
  const { api, connecting, connectDesktop } = session;
  const scan = useScanTask(api);
  const { task, online, busy, message, setMessage, notice, setNotice, cancel } = scan;
  const connectionError = session.error || scan.connectionError;
  const [settings, setSettings] = useState(!desktop.available());
  const [connectionForm, setConnectionForm] = useState({ url: 'http://127.0.0.1:', token: '' });
  const [root, setRoot] = useState('');
  useEffect(() => { if (task) setRoot(displayPath(task.root)); }, [task?.id]);
  useEffect(() => { if (session.error) setSettings(true); else if (api) setSettings(false); }, [session.error, api]);
  async function startScan(path = root) { await scan.start(path); }
  async function chooseDirectory() {
    if (!desktop.available()) return;
    try {
      const chosen = await desktop.chooseDirectory();
      if (typeof chosen === 'string') { setRoot(chosen); await startScan(chosen); }
    } catch (error) { setMessage(errorText(error)); }
  }
  const running = active(task);
  const elapsed = task ? Math.max(0, ((task.finished_at ?? Date.now()) - task.started_at) / 1000) : 0;
  const connect = () => {
    try { session.connect(connectionForm); setSettings(false); setMessage(''); }
    catch (error) { setMessage(errorText(error)); }
  };

  return <div className="app-shell">
    <header className="app-header">
      <div className="app-title"><div className="brand"><ScanLine size={22} strokeWidth={1.8} /><span>orion<span className="brand-dot">.</span></span></div><h1>磁盘空间</h1></div>
      <div className="header-actions">
        <label className="theme-picker"><Moon size={15} /><select aria-label="外观主题" value={theme} onChange={event => setTheme(event.target.value as ThemePreference)}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">暗色</option></select></label>
        <button className="server-button" aria-expanded={settings} aria-controls="connection-settings" onClick={() => setSettings(v => !v)}><span className={cn('connection-dot', online && 'connected')} /><span>{connecting ? '正在连接后台…' : online ? '服务已连接' : '连接本地服务'}</span><Settings2 size={14} /></button>
      </div>
    </header>

    <main className="main-content">
      {settings && <section id="connection-settings" className="connection-panel">
        <div className="connection-title"><Plug size={18} /><strong>连接本地服务</strong><Button variant="ghost" size="icon" aria-label="关闭连接设置" onClick={() => setSettings(false)}><X /></Button></div>
        {desktop.available() ? <>
          <p>后台会随 Orion 自动启动。关闭窗口后仍会在系统托盘中运行，右键托盘图标选择“退出 Orion”可关闭后台。</p>
          <Button className="mb-3" variant="outline" size="sm" disabled={connecting} onClick={() => void connectDesktop()}><RefreshCw />{connecting ? '正在连接后台…' : '重新连接后台'}</Button>
        </> : <>
          <p>从正在运行的 Orion 连接文件中填写地址与令牌。</p>
          <form onSubmit={e => { e.preventDefault(); connect(); }}><label>服务地址<input aria-label="服务地址" value={connectionForm.url} onChange={e => setConnectionForm(v => ({ ...v, url: e.target.value }))} placeholder="http://127.0.0.1:43120" /></label><label>连接令牌<input aria-label="连接令牌" type="password" autoComplete="off" value={connectionForm.token} onChange={e => setConnectionForm(v => ({ ...v, token: e.target.value }))} /></label><Button type="submit">连接服务<ArrowRight /></Button></form>
        </>}
      </section>}
      {(connectionError || message) && <div className="alert" role="alert"><TriangleAlert size={16} /><span>{message || connectionError}</span>{message && <button aria-label="关闭提示" onClick={() => setMessage('')}><X size={15} /></button>}</div>}
      {notice && <div className="notice" role="status"><Info size={16} />{notice}<button aria-label="关闭通知" onClick={() => setNotice('')}><X size={15} /></button></div>}

      <form className="path-bar" onSubmit={e => { e.preventDefault(); void startScan(); }}>
        <Button variant="outline" type="button" onClick={chooseDirectory} disabled={!desktop.available() || !online || busy || running || task?.status === 'unknown'} aria-describedby={!desktop.available() ? 'directory-picker-help' : undefined}><FolderOpen />选择目录</Button>
        <input id="root-path" aria-label="扫描目录路径" placeholder={desktop.available() ? '选择目录，或输入完整路径' : '粘贴要扫描的目录完整路径'} value={root} onChange={e => setRoot(e.target.value)} disabled={running} />
        <Button type="submit" disabled={!online || busy || running || task?.status === 'unknown' || !root.trim()}>{busy && !running ? <LoaderCircle className="spin" /> : <ScanLine />}{task && root.trim() === displayPath(task.root) ? '重新扫描' : '开始扫描'}</Button>
      </form>
      <div className="scan-note"><span><ShieldCheck size={13} />只读扫描，不修改或删除文件。</span>{!desktop.available() && <span id="directory-picker-help">浏览器预览需粘贴路径；桌面版可直接选择目录。</span>}</div>

      {task && <section className="scan-summary" aria-label="扫描概要">
        <div className="scan-totals"><span>逻辑大小 <strong>{formatBytes(task.logical_bytes)}</strong></span><span>{task.files.toLocaleString()} 个文件</span><span title="含扫描根目录">{task.directories.toLocaleString()} 个目录</span></div>
        <div className="scan-progress"><span className={cn('scan-status', running && 'is-running', !running && !task.complete && 'is-incomplete')}>{running ? <LoaderCircle className="spin" size={15} /> : task.complete ? <Check size={15} /> : <TriangleAlert size={15} />}{labels[task.status]}</span><span>{elapsed.toFixed(1)} 秒</span>{running && <Button variant="destructive" size="sm" onClick={cancel} disabled={!online || busy || task.status === 'cancelling'}><Square />{task.status === 'cancelling' ? '取消中' : '取消扫描'}</Button>}</div>
      </section>}

      {task ? <DirectoryExplorer api={api} task={task} online={online} onError={setMessage} /> : <div className="empty-state"><FolderOpen size={24} strokeWidth={1.5} /><div><h2>扫描一个目录开始</h2><p>{desktop.available() ? '选择目录或输入路径，扫描后按大小逐层浏览。' : '在上方输入目录路径，扫描后按大小逐层浏览。'}</p></div></div>}

    </main>
  </div>;
}
