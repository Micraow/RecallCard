import { useState } from "react";
import { ModelRuntimePanel } from "../components/ModelRuntimePanel";
import type { ScopedService } from "../service/client";
import type { AppError, JobStatus } from "../service/contracts";
import { ErrorNotice, Badge, EmptyState, Loading } from "../components/common";
import { Icon } from "../components/Icon";
import { JobRow } from "../components/Jobs";
import { scopeLabel, type BuildInfo } from "../service/types";
import { navigate } from "../App";
import { useResource } from "../service/hooks";
import { runtimeLabel } from "../service/runtime";

export function ActivityPage({
  service,
  jobs,
  error,
  refresh,
  importSources,
}: {
  service: ScopedService;
  jobs: JobStatus[];
  error: AppError | null;
  refresh: () => void;
  importSources: () => Promise<void>;
}) {
  const [runtimeRevision, setRuntimeRevision] = useState(0);
  return (
    <div className="standard-page">
      <div className="page-heading">
        <div>
          <h1>活动</h1>
          <p>后台工作、处理结果与需要你决定的事。</p>
        </div>
        <button className="button" onClick={() => { refresh(); setRuntimeRevision(value => value + 1); }}>
          <Icon name="refresh" size={15} />
          刷新
        </button>
      </div>
      <ModelRuntimePanel service={service} refreshToken={runtimeRevision} />
      <section className="activity-section">
        <div className="section-heading">
          <h2>导入记录</h2>
          <span>{jobs.length ? `${jobs.length} 项` : ""}</span>
        </div>
        {error ? (
          <ErrorNotice error={error} retry={refresh} />
        ) : jobs.length ? (
          <div className="activity-list">
            {[...jobs]
              .sort((a, b) => b.created_at.localeCompare(a.created_at))
              .map((job) => (
                <JobRow
                  key={job.job_id}
                  job={job}
                  service={service}
                  refresh={refresh}
                  importSources={importSources}
                />
              ))}
          </div>
        ) : (
          <EmptyState icon="activity" title="这里会留下处理记录">
            <p>
              导入之后，在这里查看保存了什么、重复了多少，
              <br />
              以及需要恢复的任务。
            </p>
          </EmptyState>
        )}
      </section>
      <p className="coverage-note">
        任务进度来自本地服务。暂停保留已经保存的内容；总量未知时不显示百分比。
      </p>
    </div>
  );
}
export function SettingsPage({
  service,
  chooseWorkspace,
  buildInfo,
}: {
  service: ScopedService;
  chooseWorkspace: () => void;
  buildInfo: BuildInfo | null;
}) {
  return (
    <div className="standard-page settings-page">
      <div className="page-heading">
        <div>
          <h1>设置</h1>
          <p>本地资料、访问范围与运行方式。</p>
        </div>
      </div>
      <section className="settings-section">
        <h2>本地空间</h2>
        <div className="setting-row">
          <div>
            <h3>当前范围</h3>
            <p>{scopeLabel(service.scope)}</p>
          </div>
          <Badge>范围隔离</Badge>
        </div>
        <div className="setting-row">
          <div>
            <h3>资料保存位置</h3>
            <p className="path-value">{service.workspace.root}</p>
            <span className="muted">
              Event 与 Memory 文件是正本；索引和阅读视图可以重新生成。
            </span>
          </div>
          <button className="button" onClick={chooseWorkspace}>
            打开其他空间
          </button>
        </div>
      </section>
      <section className="settings-section"><ModelRuntimePanel service={service} settings /></section>
      <section className="settings-section">
        <h2>连接与数据边界</h2>
        <div className="setting-row">
          <div>
            <h3>AI 的访问范围</h3>
            <p>客户端只读当前明确授权的范围。网页中的最后发送始终由你确认。</p>
          </div>
          <button className="button" onClick={() => navigate("connections")}>
            管理连接
          </button>
        </div>
        <div className="setting-row">
          <div>
            <h3>同步不等于共享</h3>
            <p>个人对话不会因为选择项目范围或配置 Git 而自动公开。</p>
          </div>
          <Icon name="shield" size={18} />
        </div>
      </section>
      <footer className="settings-footer">
        <span className="brand-mark small">
          <Icon name="memory" size={16} />
        </span>
        RecallCard <span>{buildInfo?.version || "版本信息暂不可读取"}</span>
        <span>
          {buildInfo
            ? `${buildInfo.commit.slice(0, 12)}${buildInfo.dirty ? " · 有未提交更改" : ""}`
            : "未收到本机版本信息"}
        </span>
      </footer>
    </div>
  );
}
