import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RecallService, type ScopedService } from "./service/client";
import { appError, type AppError } from "./service/contracts";
import { useJobs } from "./service/hooks";
import {
  scopeLabel,
  type BuildInfo,
  type Destination,
  type Workspace,
} from "./service/types";
import { Icon, type IconName } from "./components/Icon";
import { ErrorNotice, Loading } from "./components/common";
import { MemoriesPage } from "./pages/Memories";
import { SourcesPage } from "./pages/Sources";
import { ConnectionsPage } from "./pages/Connections";
import { ActivityPage, SettingsPage } from "./pages/Activity";
import { SearchPage } from "./pages/Search";
import { BackgroundPage } from "./pages/Background";

const destinations: { id: Destination; label: string; icon: IconName }[] = [
  { id: "background", label: "首页", icon: "home" },
  { id: "memories", label: "记忆", icon: "book" },
  { id: "sources", label: "原始资料", icon: "sources" },
];
const extraDestinations: { id: Destination; label: string; icon: IconName }[] = [
  { id: "connections", label: "连接", icon: "link" },
  { id: "settings", label: "设置", icon: "settings" },
];
export interface Route {
  page: Destination;
  item: string;
  query: string;
  focus: string;
}
function routeFromHash(): Route {
  const [name, parameters = ""] = window.location.hash.slice(1).split("?");
  const page = [
    "background",
    "memories",
    "sources",
    "connections",
    "activity",
    "settings",
    "search",
  ].includes(name)
    ? (name as Destination)
    : "background";
  const search = new URLSearchParams(parameters);
  return {
    page,
    item: search.get("item") || "",
    query: search.get("q") || "",
    focus: search.get("at") || "",
  };
}
export function navigate(page: Destination, item = "", query = "", focus = "") {
  const params = new URLSearchParams();
  if (item) params.set("item", item);
  if (query) params.set("q", query);
  if (focus) params.set("at", focus);
  window.location.hash = page + (params.size ? "?" + params : "");
}
export function App({ service }: { service: RecallService | null }) {
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [buildInfo, setBuildInfo] = useState<BuildInfo | null>(
    service?.demo || !service ? __BUILD_INFO__ : null,
  );
  const [scope, setScope] = useState("personal");
  const [importOnOpen, setImportOnOpen] = useState(false);
  const [busy, setBusy] = useState(!!service);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    if (!service) return;
    let current = true;
    service
      .info()
      .then((info) => {
        if (current) setBuildInfo(info);
      })
      .catch(() => {
        if (current) setBuildInfo(null);
      });
    service
      .restore()
      .then((result) => {
        if (current && result) {
          setWorkspace(result.vault);
          setScope(result.scope);
        }
      })
      .catch((reason) => {
        if (current) setError(appError(reason));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
    };
  }, [service]);
  const open = async (create: boolean, importAfter = false) => {
    if (!service || busy) return;
    setBusy(true);
    setError(null);
    try {
      const selected = await (create
        ? service.createWorkspace()
        : service.chooseWorkspace());
      if (selected) {
        setImportOnOpen(importAfter);
        if (importAfter) navigate("background");
        setWorkspace(selected);
        setScope(
          selected.scopes.includes("personal")
            ? "personal"
            : selected.scopes[0] || "personal",
        );
      }
    } catch (reason) {
      setError(appError(reason));
    } finally {
      setBusy(false);
    }
  };
  if (workspace && service)
    return (
      <WorkspaceApp
        key={workspace.session_id + scope}
        service={service.scoped(workspace, scope)}
        demo={service.demo}
        chooseWorkspace={() => void open(false)}
        setScope={setScope}
        workspaceError={error}
        buildInfo={buildInfo}
        importOnOpen={importOnOpen}
        consumeFirstImport={() => setImportOnOpen(false)}
      />
    );
  return (
    <main className="welcome">
      <div className="welcome-brand">
        <span className="brand-mark">
          <Icon name="memory" size={22} />
        </span>
        RecallCard
      </div>
      <div className="welcome-content">
        <span className="eyebrow">你的本地上下文</span>
        <h1>
          让每次开始，
          <br />
          都接得上。
        </h1>
        <p>
          把对话和记忆留在自己手里。
          <br />
          随时找到原话，让新的 AI 从已有背景出发。
        </p>
        {error && <ErrorNotice error={error} />}
        {busy ? (
          <Loading label="正在打开上次的空间" />
        ) : service ? (
          <div className="welcome-actions">
            <button
              className="button primary"
              onClick={() => void open(true, true)}
            >
              导入聊天记录 <Icon name="arrow" size={16} />
            </button>
            <button className="button quiet" onClick={() => void open(false)}>
              打开已有资料
            </button>
          </div>
        ) : (
          <div className="connection-required">
            <Icon name="shield" />
            <div>
              <strong>请在 RecallCard 桌面应用中打开</strong>
              <p>
                此浏览器页面没有本地资料访问权限，也不会用示例数据代替你的记忆。
              </p>
            </div>
          </div>
        )}
        <div className="welcome-note">
          <Icon name="shield" size={15} />
          直接选择官方 ZIP 或 JSON · 不用先配置模型
        </div>
      </div>
      <div className="welcome-footer">
        RecallCard {buildInfo?.version || "版本信息暂不可读取"}
      </div>
    </main>
  );
}
function WorkspaceApp({
  service: unstableService,
  demo,
  chooseWorkspace,
  setScope,
  workspaceError,
  buildInfo,
  importOnOpen,
  consumeFirstImport,
}: {
  service: ScopedService;
  demo: boolean;
  chooseWorkspace: () => void;
  setScope: (scope: string) => void;
  workspaceError: AppError | null;
  buildInfo: BuildInfo | null;
  importOnOpen: boolean;
  consumeFirstImport: () => void;
}) {
  const service = useMemo(
    () => unstableService,
    [unstableService.workspace.session_id, unstableService.scope],
  );
  const [route, setRoute] = useState(routeFromHash);
  const [query, setQuery] = useState(
    route.page === "search" ? route.query : "",
  );
  const [refresh, setRefresh] = useState(0);
  const [importing, setImporting] = useState(false);
  const importLock = useRef(false);
  const [importError, setImportError] = useState<AppError | null>(null);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const changed = useCallback(() => setRefresh((value) => value + 1), []);
  const jobs = useJobs(service, changed);
  useEffect(() => {
    const listener = () => {
      const next = routeFromHash();
      setRoute(next);
      if (next.page === "search") setQuery(next.query);
    };
    window.addEventListener("hashchange", listener);
    return () => window.removeEventListener("hashchange", listener);
  }, []);
  useEffect(() => {
    service.remember().catch((reason) => setSaveError(appError(reason)));
  }, [service]);
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        document.querySelector<HTMLInputElement>("#global-search")?.focus();
      }
    };
    window.addEventListener("keydown", keyboard);
    return () => window.removeEventListener("keydown", keyboard);
  }, []);
  const importSources = async () => {
    if (importLock.current) return;
    importLock.current = true;
    setImporting(true);
    setImportError(null);
    try {
      const job = await service.importSources(crypto.randomUUID());
      if (job) {
        navigate("background");
        jobs.refresh();
        changed();
      }
    } catch (reason) {
      setImportError(appError(reason));
    } finally {
      importLock.current = false;
      setImporting(false);
    }
  };
  const firstImportStarted = useRef(false);
  useEffect(() => {
    if (importOnOpen && !firstImportStarted.current) {
      firstImportStarted.current = true;
      consumeFirstImport();
      void importSources();
    }
  }, [importOnOpen]);
  const pageLabel =
    route.page === "settings"
      ? "设置"
      : route.page === "search"
        ? "搜索"
        : route.page === "activity" ? "处理记录"
        : [...destinations, ...extraDestinations].find(
            (item) => item.id === route.page,
          )!.label;
  return (
    <div
      className={`app-shell ${demo ? "is-demo" : ""}`}
      data-build-version={buildInfo?.version}
      data-build-commit={buildInfo?.commit}
      data-build-dirty={buildInfo?.dirty}
    >
      <aside className="sidebar">
        <button
          className="brand"
          onClick={() => navigate("background")}
          aria-label="RecallCard 首页"
        >
          <span className="brand-mark">
            <Icon name="memory" size={21} />
          </span>
          <span>RecallCard</span>
        </button>
        <nav aria-label="主导航">
          {destinations.map((item) => (
            <button
              key={item.id}
              aria-label={item.label}
              title={item.label}
              className={`nav-item ${route.page === item.id ? "active" : ""}`}
              aria-current={route.page === item.id ? "page" : undefined}
              onClick={() => navigate(item.id)}
            >
              <Icon name={item.icon} />
              <span>{item.label}</span>

            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <nav aria-label="连接与设置">
            {extraDestinations.map((item) => (
              <button
                className={`nav-item ${route.page === item.id ? "active" : ""}`}
                key={item.id}
                aria-label={item.label}
                aria-current={route.page === item.id ? "page" : undefined}
                title={item.label}
                onClick={() => navigate(item.id)}
              >
                <Icon name={item.icon} />
                <span>{item.label}</span>
              </button>
            ))}
          </nav>
          <div className="workspace-switch">
            <span className="workspace-avatar"><Icon name="folder" size={17} /></span>
            <select
              aria-label="资料范围"
              value={service.scope}
              onChange={(event) => {
                navigate("background");
                setScope(event.target.value);
              }}
            >
              {[...new Set([service.scope, ...service.workspace.scopes])].map((scope) => (
                <option key={scope} value={scope}>{scopeLabel(scope)}</option>
              ))}
            </select>
          </div>
          <div className="local-status">
            <span className="status-dot" />
            <span>资料保存在本机</span>
            <span className="app-version">
              {buildInfo?.version || "版本未知"}
            </span>
          </div>
        </div>
      </aside>
      <div className="workspace-main">
        {demo && (
          <div className="demo-banner">
            <Icon name="book" size={14} />
            界面验证 · 以下均为合成示例，非真实资料，操作不会写入本机空间
          </div>
        )}
        <header className="topbar">
          <div className="breadcrumb">
            <span>{scopeLabel(service.scope)}</span>
            <Icon name="chevron" size={13} />
            <strong>{pageLabel}</strong>
          </div>
          <form
            className="global-search"
            role="search"
            onSubmit={(event) => {
              event.preventDefault();
              if (query.trim()) navigate("search", "", query.trim());
            }}
          >
            <Icon name="search" size={16} />
            <input
              id="global-search"
              aria-label="搜索全部原话和记忆"
              placeholder="搜索原话和记忆"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <kbd>⌘ K</kbd>
          </form>
          <button
            className="button"
            onClick={() => void importSources()}
            disabled={importing}
          >
            <Icon name="plus" size={16} />
            {importing ? "正在选择…" : "添加记录"}
          </button>
        </header>
        {(workspaceError || saveError) && (
          <div className="workspace-error">
            <ErrorNotice error={(workspaceError || saveError)!} />
          </div>
        )}
        <main className="page-content" id="main-content">
          {route.page === "background" && (
            <BackgroundPage
              service={service}
              refresh={refresh}
              changed={changed}
              jobs={jobs.jobs}
              jobsError={jobs.error}
              importError={importError}
              importing={importing}
              importSources={importSources}
            />
          )}

          {route.page === "memories" && (
            <MemoriesPage
              service={service}
              selected={route.item}
              initialFilter={route.focus}
              refresh={refresh}
              changed={changed}
            />
          )}
          {route.page === "sources" && (
            <SourcesPage
              service={service}
              selected={route.item}
              focusRef={route.focus}
              refresh={refresh}
              jobs={jobs.jobs}
              jobsError={jobs.error}
              importError={importError}
              importing={importing}
              importSources={importSources}
              reloadJobs={jobs.refresh}
            />
          )}
          {route.page === "connections" && (
            <ConnectionsPage service={service} />
          )}
          {route.page === "activity" && (
            <ActivityPage
              service={service}
              jobs={jobs.jobs}
              error={jobs.error}
              refresh={jobs.refresh}
              importSources={importSources}
            />
          )}
          {route.page === "settings" && (
            <SettingsPage
              service={service}
              chooseWorkspace={chooseWorkspace}
              buildInfo={buildInfo}
            />
          )}
          {route.page === "search" && (
            <SearchPage service={service} query={route.query} />
          )}
        </main>
      </div>
    </div>
  );
}
