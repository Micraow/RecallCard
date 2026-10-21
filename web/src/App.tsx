import { useEffect, useState } from 'react';
import { api, type StateResponse } from './api';
import { SearchPage } from './pages/Search';
import { MemoriesPage } from './pages/Memories';
import { SettingsPage } from './pages/Settings';

type Page = 'search' | 'memories' | 'settings';

const PAGES: { id: Page; label: string; glyph: string }[] = [
  { id: 'search', label: '搜索', glyph: '寻' },
  { id: 'memories', label: '记忆', glyph: '忆' },
  { id: 'settings', label: '设置', glyph: '设' },
];

function initialPage(): Page {
  const hash = location.hash.replace(/^#\/?/, '');
  return hash === 'memories' || hash === 'settings' ? hash : 'search';
}

export function App() {
  const [state, setState] = useState<StateResponse | null>(null);
  const [page, setPage] = useState<Page>(initialPage);
  const [error, setError] = useState('');

  // 令牌只用于换取 Cookie，之后从地址栏抹掉，避免留在历史记录里。
  useEffect(() => {
    if (location.search) history.replaceState({}, '', location.pathname + location.hash);
    refresh();
  }, []);

  async function refresh() {
    try {
      setState(await api.state());
      setError('');
    } catch (cause) {
      setError(message(cause));
    }
  }

  async function changeScope(scope: string) {
    try {
      await api.setScope(scope);
      await refresh();
    } catch (cause) {
      setError(message(cause));
    }
  }

  if (!state) {
    return (
      <div className="app">
        <aside className="sidebar">
          <div className="brand">
            <div className="brand-mark">记</div>
            <div>
              <div className="brand-name">RecallCard</div>
              <div className="brand-sub">个人资料库</div>
            </div>
          </div>
        </aside>
        <main className="main">
          <header className="topbar">
            <span className="topbar-title">读取</span>
          </header>
          <div className="content">
            {error ? (
              <div className="panel" style={{ maxWidth: 620, marginTop: 28 }}>
                <h2>读不到资料库</h2>
                <p className="hint" style={{ marginBottom: 12 }}>
                  {error}
                </p>
                <p className="hint" style={{ marginBottom: 16 }}>
                  如果终端里的服务已经退出，重新运行 <code>recallcard ui</code>，再用新打印的地址打开。
                </p>
                <button className="primary" onClick={() => void refresh()}>
                  重试
                </button>
              </div>
            ) : (
              <span className="spin" />
            )}
          </div>
        </main>
      </div>
    );
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">记</div>
          <div>
            <div className="brand-name">RecallCard</div>
            <div className="brand-sub">个人资料库</div>
          </div>
        </div>
        <nav className="nav">
          <div className="nav-label">工作区</div>
          {PAGES.map(item => (
            <button
              key={item.id}
              onClick={() => setPage(item.id)}
              aria-current={page === item.id ? 'page' : undefined}
            >
              <span className="glyph">{item.glyph}</span>
              <span className="label">{item.label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">
          <strong>{state.vault.display_name}</strong>
          <span>
            {state.vault.event_count.toLocaleString('zh-CN')} 条原始记录 ·{' '}
            {state.vault.memory_count.toLocaleString('zh-CN')} 条记忆
          </span>
          <span>资料只存在本机</span>
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <span className="crumb">{PAGES.find(item => item.id === page)?.label}</span>
          <span className="crumb-sub">{state.vault.display_name}</span>
          <div className="spacer" />
          {state.vault.scopes.length > 1 && (
            <select
              className="scope-select"
              value={state.scope}
              onChange={event => changeScope(event.target.value)}
              aria-label="授权范围"
            >
              {state.vault.scopes.map(scope => (
                <option key={scope} value={scope}>
                  {scope}
                </option>
              ))}
            </select>
          )}
        </header>
        {error && (
          <div className="notice error banner" role="alert">
            <span>{error}</span>
            <button className="quiet" onClick={() => setError('')} aria-label="关闭">
              ✕
            </button>
          </div>
        )}
        {page === 'search' && (
          <SearchPage scope={state.scope} indexConfigured={state.index.configured} onError={setError} />
        )}
        {page === 'memories' && (
          <MemoriesPage scope={state.scope} onError={setError} onChanged={refresh} />
        )}
        {page === 'settings' && (
          <SettingsPage state={state} onError={setError} onReload={refresh} />
        )}
      </main>
    </div>
  );
}

export function message(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
