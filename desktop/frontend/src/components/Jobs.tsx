import { useState } from "react";
import type { ScopedService } from "../service/client";
import {
  appError,
  isRunning,
  importRecovery,
  coverageNotes,
  type AppError,
  type JobStatus,
} from "../service/contracts";
import { dateLabel } from "../service/types";
import { Badge, ErrorNotice, Loading } from "./common";
import { Icon } from "./Icon";
import { useResource } from "../service/hooks";
const states = {
  queued: "等待处理",
  running: "正在导入",
  paused: "已暂停",
  needs_input: "需要处理",
  failed: "导入未完成",
  completed: "导入完成",
  cancelled: "已取消",
};
const phases = {
  preflight: "检查来源",
  parsing: "读取会话",
  committing: "保存消息",
  finished: "已结束",
};
const bytes = (value: number) =>
  value < 1024 * 1024
    ? `${Math.round(value / 1024)} KB`
    : `${(value / 1024 / 1024).toFixed(1)} MB`;
export function JobRow({
  job,
  service,
  refresh,
  importSources,
  compact = false,
}: {
  job: JobStatus;
  service: ScopedService;
  refresh: () => void;
  importSources: () => Promise<void>;
  compact?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [expanded, setExpanded] = useState(false);
  const progress = job.progress;
  const recovery = importRecovery(job);
  const action = async (resume: boolean) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await (resume
        ? service.resumeJob(job.job_id)
        : service.pauseJob(job.job_id));
      refresh();
    } catch (reason) {
      setError(appError(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div
      className={`job-row ${compact ? "compact-job" : ""}`}
      data-job-state={job.state}
    >
      <div className={`job-symbol ${job.state}`}>
        <Icon
          name={
            job.state === "completed"
              ? "check"
              : job.state === "failed" || job.state === "needs_input"
                ? "alert"
                : "download"
          }
          size={18}
        />
      </div>
      <div className="job-body">
        <div className="job-title">
          <strong>{job.error?.file_name || "导出文件导入"}</strong>
          <Badge
            tone={
              job.state === "completed"
                ? "success"
                : job.error
                  ? "warning"
                  : isRunning(job)
                    ? "accent"
                    : "muted"
            }
          >
            {states[job.state]}
          </Badge>
        </div>
        <p className="job-summary">
          {job.state === "completed"
            ? `新增 ${progress.events_added.toLocaleString()} 条消息 · 跳过 ${progress.events_duplicates.toLocaleString()} 条重复`
            : `${phases[job.phase]} · 已保存 ${progress.events_added.toLocaleString()} 条消息${progress.events_duplicates ? ` · ${progress.events_duplicates.toLocaleString()} 条重复` : ""}`}
        </p>
        {isRunning(job) && (
          <>
            {progress.events_total !== null &&
            progress.events_total > 0 &&
            job.phase === "committing" ? (
              <progress
                aria-label="已处理消息"
                max={progress.events_total}
                value={progress.events_processed}
              />
            ) : (
              <div
                className="indeterminate-track"
                role="status"
                aria-label="总量尚未知"
              >
                <span />
              </div>
            )}
            <p className="job-progress-text">
              {progress.events_total !== null
                ? `${progress.events_processed.toLocaleString()} / ${progress.events_total.toLocaleString()} 条已处理`
                : `${bytes(progress.source_bytes_read)} 已读取 · 正在确定总量`}
            </p>
          </>
        )}
        {job.error && <ErrorNotice error={job.error} />}
        {error && <ErrorNotice error={error} />}
        {!compact && (
          <button
            className="text-button job-details-toggle"
            onClick={() => setExpanded(!expanded)}
            aria-expanded={expanded}
          >
            {expanded ? "收起明细" : "处理明细"}
            <Icon name="chevron" size={12} />
          </button>
        )}
        {expanded && (
          <dl className="job-details">
            <div>
              <dt>文件</dt>
              <dd>
                {progress.files_processed} / {progress.files_total}
              </dd>
            </div>
            <div>
              <dt>已识别会话</dt>
              <dd>{progress.conversations.toLocaleString()}</dd>
            </div>
            <div>
              <dt>暂存消息</dt>
              <dd>{progress.events_staged.toLocaleString()}</dd>
            </div>
            <div>
              <dt>已展开数据</dt>
              <dd>{bytes(progress.expanded_bytes_read)}</dd>
            </div>
            <div>
              <dt>恢复依据</dt>
              <dd>
                {job.phase === "committing" || job.state === "completed"
                  ? "继续已核验快照"
                  : "重新核验来源"}
              </dd>
            </div>
            <div>
              <dt>更新于</dt>
              <dd>{dateLabel(job.updated_at, true)}</dd>
            </div>
          </dl>
        )}
        {expanded && job.state === "completed" && (
          <CoverageDetails service={service} jobId={job.job_id} />
        )}
      </div>
      <div className="job-actions">
        {isRunning(job) ? (
          <button
            className="button compact"
            onClick={() => void action(false)}
            disabled={busy}
          >
            <Icon name="pause" size={14} />
            {busy ? "暂停中…" : "暂停"}
          </button>
        ) : recovery === "new_source" ? (
          <button
            className="button compact"
            onClick={() => void importSources()}
            disabled={busy}
          >
            <Icon name="folder" size={14} />
            重新选择来源
          </button>
        ) : recovery === "resume" ? (
          <button
            className="button compact"
            onClick={() => void action(true)}
            disabled={busy}
          >
            <Icon name="play" size={14} />
            {busy
              ? "恢复中…"
              : job.phase === "committing"
                ? "继续已核验快照"
                : "继续"}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function CoverageDetails({
  service,
  jobId,
}: {
  service: ScopedService;
  jobId: string;
}) {
  const result = useResource(
    () => service.importResult(jobId),
    [service, jobId],
  );
  if (result.loading) return <Loading label="正在读取导入覆盖记录" />;
  if (result.error)
    return <ErrorNotice error={result.error} retry={result.refresh} />;
  const coverage = result.data!.coverage;
  const notes = coverageNotes(coverage);
  return (
    <section className="import-coverage">
      <h3>本次覆盖</h3>
      <p>
        {coverage.conversations.toLocaleString()} 段会话 ·{" "}
        {coverage.events.toLocaleString()} 条可导入记录
      </p>
      {notes.length > 0 && (
        <ul>
          {notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      )}
      <p>
        导入完成表示已处理当前可识别内容，不表示附件原件、隐藏内容或网站全部历史已保存。
      </p>
    </section>
  );
}
