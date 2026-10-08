import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { RecallService, type ScopedService } from './service/client';
import { appError, isRunning, type AppError } from './service/contracts';
import { useJobs } from './service/hooks';
import { scopeLabel, type Destination, type Workspace } from './service/types';
import { Icon, type IconName } from './components/Icon';
import { Badge, ErrorNotice, Loading } from './components/common';
import { MemoriesPage } from './pages/Memories';
import { SourcesPage } from './pages/Sources';
import { ConnectionsPage } from './pages/Connections';
import { ActivityPage, SettingsPage } from './pages/Activity';
import { SearchPage } from './pages/Search';

const destinations: { id: Destination; label: string; icon: IconName }[] = [
  { id: 'memories', label: '记忆', icon: 'memory' }, { id: 'sources', label: '来源', icon: 'sources' },
  { id: 'connections', label: '连接', icon: 'link' }, { id: 'activity', label: '活动', icon: 'activity' },
];
export interface Route { page: Destination; item: string; query: string }
function routeFromHash(): Route {
  const [name, parameters = ''] = window.location.hash.slice(1).split('?');
  const page = ['memories', 'sources', 'connections', 'activity', 'settings', 'search'].includes(name) ? name as Destination : 'memories';
  const search = new URLSearchParams(parameters);
  return { page, item: search.get('item') || '', query: search.get('q') || '' };
}
export function navigate(page: Destination, item = '', query = '') {
  const params = new URLSearchParams(); if (item) params.set('item', item); if (query) params.set('q', query);
  window.location.hash = page + (params.size ? '?' + params : '');
}
export function App({ service }: { service: RecallService | null }) {
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [scope, setScope] = useState('personal');
  const [busy, setBusy] = useState(!!service);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    if (!service) return;
    let current = true;
    service.restore().then(result => { if (current && result) { setWorkspace(result.vault); setScope(result.scope); } }).catch(reason => { if (current) setError(appError(reason)); }).finally(() => { if (current) setBusy(false); });
    return () => { current = false; };
  }, [service]);
  const open = async (create: boolean) => {
    if (!service || busy) return;
    setBusy(true); setError(null);
    try {
      const selected = await (create ? service.createWorkspace() : service.chooseWorkspace());
      if (selected) { setWorkspace(selected); setScope(selected.scopes.includes('personal') ? 'personal' : selected.scopes[0] || 'personal'); }
    } catch (reason) { setError(appError(reason)); } finally { setBusy(false); }
  };
  if (workspace && service) return <WorkspaceApp key={workspace.session_id + scope} service={service.scoped(workspace, scope)} demo={service.demo} chooseWorkspace={() => void open(false)} setScope={setScope} workspaceError={error} />;
  return <main className="welcome"><div className="welcome-brand"><span className="brand-mark"><Icon name="memory" size={22} /></span>RecallCard</div><div className="welcome-content"><span className="eyebrow">你的本地上下文</span><h1>让每次开始，<br />都接得上。</h1><p>把对话和记忆留在自己手里。<br />随时找到原话，让新的 AI 从已有背景出发。</p>{error && <ErrorNotice error={error} />}{busy ? <Loading label="正在打开上次的空间" /> : service ? <div className="welcome-actions"><button className="button primary" onClick={() => void open(true)}>开始使用 <Icon name="arrow" size={16} /></button><button className="button quiet" onClick={() => void open(false)}>打开已有空间</button></div> : <div className="connection-required"><Icon name="shield" /><div><strong>请在 RecallCard 桌面应用中打开</strong><p>此浏览器页面没有本地资料访问权限，也不会用示例数据代替你的记忆。</p></div></div>}<div className="welcome-note"><Icon name="shield" size={15} />文件保存在本机 · 可先离线导入</div></div><div className="welcome-footer">RecallCard {__APP_VERSION__}</div></main>;
}
function WorkspaceApp({ service: unstableService, demo, chooseWorkspace, setScope, workspaceError }: { service: ScopedService; demo: boolean; chooseWorkspace: () => void; setScope: (scope: string) => void; workspaceError: AppError | null }) {
  const service = useMemo(() => unstableService, [unstableService.workspace.session_id, unstableService.scope]);
  const [route, setRoute] = useState(routeFromHash);
  const [query, setQuery] = useState(route.query);
  const [refresh, setRefresh] = useState(0);
  const [importing, setImporting] = useState(false);
  const importLock = useRef(false);
  const [importError, setImportError] = useState<AppError | null>(null);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const changed = useCallback(() => setRefresh(value => value + 1), []);
  const jobs = useJobs(service, changed);
  useEffect(() => { const listener = () => { const next = routeFromHash(); setRoute(next); if (next.page === 'search') setQuery(next.query); }; window.addEventListener('hashchange', listener); return () => window.removeEventListener('hashchange', listener); }, []);
  useEffect(() => { service.remember().catch(reason => setSaveError(appError(reason))); }, [service]);
  useEffect(() => { const keyboard = (event: KeyboardEvent) => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); document.querySelector<HTMLInputElement>('#global-search')?.focus(); } }; window.addEventListener('keydown', keyboard); return () => window.removeEventListener('keydown', keyboard); }, []);
  const importSources = async () => {
    if (importLock.current) return;
    importLock.current = true;
    setImporting(true); setImportError(null);
    navigate('sources');
    try { const job = await service.importSources(crypto.randomUUID()); if (job) { jobs.refresh(); changed(); } } catch (reason) { setImportError(appError(reason)); } finally { importLock.current = false; setImporting(false); }
  };
  const runningCount = jobs.jobs.filter(isRunning).length;
  const attentionCount = jobs.jobs.filter(job => job.state === 'failed' || job.state === 'needs_input').length;
  const pageLabel = route.page === 'settings' ? '设置' : route.page === 'search' ? '搜索' : destinations.find(item => item.id === route.page)!.label;
  return <div className={`app-shell ${demo ? 'is-demo' : ''}`}>
    <aside className="sidebar">
      <button className="brand" onClick={() => navigate('memories')} aria-label="RecallCard 记忆"><span className="brand-mark"><Icon name="memory" size={21} /></span><span>RecallCard</span></button>
      <div className="workspace-switch"><span className="workspace-avatar"><Icon name="folder" size={17} /></span><select aria-label="资料范围" value={service.scope} onChange={event => { navigate('memories'); setScope(event.target.value); }}>{[...new Set([service.scope, ...service.workspace.scopes])].map(scope => <option key={scope} value={scope}>{scopeLabel(scope)}</option>)}</select></div>
      <nav aria-label="主导航">{destinations.map(item => <button key={item.id} className={`nav-item ${route.page === item.id ? 'active' : ''}`} aria-current={route.page === item.id ? 'page' : undefined} onClick={() => navigate(item.id)}><Icon name={item.icon} /><span>{item.label}</span>{item.id === 'activity' && (attentionCount > 0 ? <span className="nav-count warning">{attentionCount}</span> : runningCount > 0 ? <span className="nav-count">{runningCount}</span> : null)}</button>)}</nav>
      <div className="sidebar-context"><span className="nav-caption">当前空间</span><div className="scope-explainer"><Icon name="shield" size={15} /><span>仅此范围可被读取</span></div><p>个人背景留在个人空间。<br />项目资料按范围单独使用。</p></div>
      <div className="sidebar-bottom"><button className={`nav-item ${route.page === 'settings' ? 'active' : ''}`} onClick={() => navigate('settings')} aria-current={route.page === 'settings' ? 'page' : undefined}><Icon name="settings" /><span>设置</span></button><div className="local-status"><span className="status-dot" /><span>本地空间已打开</span><span className="app-version">{__APP_VERSION__}</span></div></div>
    </aside>
    <div className="workspace-main">
      {demo && <div className="demo-banner"><Icon name="book" size={14} />界面验证 · 以下均为合成示例，非真实资料，操作不会写入本机空间</div>}
      <header className="topbar"><div className="breadcrumb"><span>{scopeLabel(service.scope)}</span><Icon name="chevron" size={13} /><strong>{pageLabel}</strong></div><form className="global-search" role="search" onSubmit={event => { event.preventDefault(); if (query.trim()) navigate('search', '', query.trim()); }}><Icon name="search" size={16} /><input id="global-search" aria-label="搜索全部原话和记忆" placeholder="搜索原话和记忆" value={query} onChange={event => setQuery(event.target.value)} /><kbd>⌘ K</kbd></form><button className="button primary" onClick={() => void importSources()} disabled={importing}><Icon name="plus" size={16} />{importing ? '正在选择…' : '添加来源'}</button></header>
      {(workspaceError || saveError) && <div className="workspace-error"><ErrorNotice error={(workspaceError || saveError)!} /></div>}
      <main className="page-content" id="main-content">
        {route.page === 'memories' && <MemoriesPage service={service} selected={route.item} refresh={refresh} changed={changed} />}
        {route.page === 'sources' && <SourcesPage service={service} selected={route.item} refresh={refresh} jobs={jobs.jobs} jobsError={jobs.error} importError={importError} importing={importing} importSources={importSources} reloadJobs={jobs.refresh} />}
        {route.page === 'connections' && <ConnectionsPage service={service} />}
        {route.page === 'activity' && <ActivityPage service={service} jobs={jobs.jobs} error={jobs.error} refresh={jobs.refresh} />}
        {route.page === 'settings' && <SettingsPage service={service} chooseWorkspace={chooseWorkspace} />}
        {route.page === 'search' && <SearchPage service={service} query={route.query} />}
      </main>
    </div>
  </div>;
}
