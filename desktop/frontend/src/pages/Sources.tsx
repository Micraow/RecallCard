import { useEffect, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, type AppError, type JobStatus } from "../service/contracts";
import { useResource } from "../service/hooks";
import {
  dateLabel,
  platformLabel,
  type Conversation,
  type Message,
  type SourceAssets,
} from "../service/types";
import { Icon } from "../components/Icon";
import {
  Badge,
  EmptyState,
  ErrorNotice,
  Loading,
  Prose,
} from "../components/common";
import { JobRow } from "../components/Jobs";
import { navigate } from "../App";

export function SourcesPage({
  service,
  selected,
  focusRef = "",
  refresh,
  jobs,
  jobsError,
  importError,
  importing,
  importSources,
  reloadJobs,
}: {
  service: ScopedService;
  selected: string;
  focusRef?: string;
  refresh: number;
  jobs: JobStatus[];
  jobsError: AppError | null;
  importError: AppError | null;
  importing: boolean;
  importSources: () => Promise<void>;
  reloadJobs: () => void;
}) {
  const [offset, setOffset] = useState(0);
  const [platform, setPlatform] = useState("all");
  const sources = useResource(
    () => service.conversations(offset),
    [service, offset, refresh],
  );
  const all = sources.data?.conversations || [];
  const rows = all.filter(
    (row) => platform === "all" || row.platform === platform,
  );
  const selectedSource = selected || rows[0]?.session_ref;
  const visibleJobs = jobs.filter(
    (job) => job.state !== "completed" && job.state !== "cancelled",
  );
  const recentCompleted = jobs.find((job) => job.state === "completed");
  return (
    <div className={`sources-layout ${selected ? "has-selection" : ""}`}>
      {(importError ||
        jobsError ||
        visibleJobs.length > 0 ||
        recentCompleted) && (
        <section className="source-activity" aria-label="导入状态">
          {importError && (
            <ErrorNotice
              error={importError}
              retry={() => void importSources()}
            />
          )}
          {jobsError && <ErrorNotice error={jobsError} retry={reloadJobs} />}
          {visibleJobs.map((job) => (
            <JobRow
              key={job.job_id}
              job={job}
              service={service}
              refresh={reloadJobs}
              importSources={importSources}
              compact
            />
          ))}
          {visibleJobs.length === 0 && recentCompleted && (
            <div className="import-success" role="status">
              <Icon name="check" size={15} />
              <span>
                最近导入：新增{" "}
                {recentCompleted.progress.events_added.toLocaleString()}{" "}
                条消息，
                {recentCompleted.progress.events_duplicates.toLocaleString()}{" "}
                条重复已跳过
              </span>
              <button
                className="text-button"
                onClick={() => navigate("activity")}
              >
                查看记录
              </button>
            </div>
          )}
        </section>
      )}
      <div className="split-view">
        <section className="collection-panel" aria-label="来源列表">
          <div className="collection-heading">
            <h1>来源</h1>
            <span className="muted count-label">
              {sources.data ? `${sources.data.total} 段会话` : ""}
            </span>
          </div>
          <div className="collection-filter">
            <Icon name="globe" size={15} />
            <select
              aria-label="按来源平台筛选"
              value={platform}
              onChange={(event) => {
                setPlatform(event.target.value);
                navigate("sources");
              }}
            >
              <option value="all">全部平台</option>
              {[...new Set(all.map((row) => row.platform))].map((name) => (
                <option key={name} value={name}>
                  {platformLabel(name)}
                </option>
              ))}
            </select>
            <span>本页</span>
          </div>
          <div className="collection-scroll">
            {sources.loading ? (
              <Loading />
            ) : sources.error ? (
              <ErrorNotice error={sources.error} retry={sources.refresh} />
            ) : rows.length ? (
              <div className="record-list">
                {rows.map((source) => (
                  <SourceRow
                    key={source.session_ref}
                    source={source}
                    selected={source.session_ref === selectedSource}
                  />
                ))}
              </div>
            ) : (
              <div className="small-empty">
                <Icon name="sources" size={24} />
                <h3>还没有保存的来源</h3>
                <p>
                  用右上角「添加记录」导入官方导出。可以直接选择 ZIP，无需解压。
                </p>
              </div>
            )}
          </div>
          <div className="pagination">
            <span>仅显示本机可访问的内容</span>
            <div>
              <button
                className="icon-button"
                aria-label="上一页来源"
                disabled={offset === 0 || sources.loading}
                onClick={() => setOffset(Math.max(0, offset - 50))}
              >
                <Icon name="back" size={16} />
              </button>
              <button
                className="icon-button"
                aria-label="下一页来源"
                disabled={sources.data?.next_offset == null || sources.loading}
                onClick={() => setOffset(sources.data!.next_offset!)}
              >
                <Icon name="arrow" size={16} />
              </button>
            </div>
          </div>
        </section>
        <section className="reader-panel" aria-label="来源阅读区">
          {selectedSource ? (
            <ConversationReader
              key={selectedSource}
              service={service}
              reference={selectedSource}
              focusRef={focusRef}
              refresh={refresh}
            />
          ) : (
            <EmptyState icon="sources" title="所有背景，都有来处">
              <p>
                导入 ChatGPT、DeepSeek 的官方导出文件。
                <br />
                保存后即可搜索原话，不必先等待记忆整理。
              </p>
              <div className="supported-sources">
                <span>
                  C<span>ChatGPT</span>
                </span>
                <span>
                  D<span>DeepSeek</span>
                </span>
                <span>
                  <Icon name="file" size={17} />
                  <span>JSON / ZIP</span>
                </span>
              </div>
            </EmptyState>
          )}
        </section>
      </div>
    </div>
  );
}
function SourceRow({
  source,
  selected,
}: {
  source: Conversation;
  selected: boolean;
}) {
  return (
    <button
      className={`source-row ${selected ? "selected" : ""}`}
      onClick={() => navigate("sources", source.session_ref)}
      aria-current={selected ? "true" : undefined}
    >
      <span className={`platform-icon ${source.platform.toLowerCase()}`}>
        {platformLabel(source.platform).slice(0, 1)}
      </span>
      <div className="source-row-content">
        <div className="row-title">{source.title.trim() || "未命名会话"}</div>
        <p>
          {platformLabel(source.platform)}
          <span>·</span>
          {source.message_count} 条消息
        </p>
        <div className="row-meta">
          <span>已保存</span>
          <span className="row-date">{dateLabel(source.captured_at)} 导入</span>
        </div>
      </div>
    </button>
  );
}
function ConversationReader({
  service,
  reference,
  focusRef,
  refresh,
}: {
  service: ScopedService;
  reference: string;
  focusRef: string;
  refresh: number;
}) {
  const [offset, setOffset] = useState(0);
  const [locationError, setLocationError] = useState<AppError | null>(null);
  const [locating, setLocating] = useState(Boolean(focusRef));
  useEffect(() => {
    let current = true;
    setLocationError(null);
    setLocating(Boolean(focusRef));
    if (focusRef)
      service
        .eventLocation(focusRef)
        .then((result) => {
          if (!current) return;
          if (result.conversation_ref !== reference)
            throw new Error("这条原话所属会话已变化，请重新查找。");
          setOffset(result.offset);
        })
        .catch((reason) => {
          if (current) setLocationError(appError(reason));
        })
        .finally(() => {
          if (current) setLocating(false);
        });
    return () => {
      current = false;
    };
  }, [service, reference, focusRef]);
  const conversation = useResource(
    () => service.messages(reference, offset),
    [service, reference, offset, refresh],
  );
  return (
    <>
      {locationError && <ErrorNotice error={locationError} />}
      {conversation.loading || locating ? (
        <Loading />
      ) : conversation.error ? (
        <ErrorNotice error={conversation.error} retry={conversation.refresh} />
      ) : (
        conversation.data && (
          <>
            <div className="reader-toolbar">
              <button
                className="text-button mobile-back"
                onClick={() => navigate("sources")}
              >
                <Icon name="back" size={15} />
                来源列表
              </button>
              <span className="reader-kind">
                <Icon name="sources" size={15} />
                {platformLabel(conversation.data.platform)}
              </span>
              <Badge>已保存的原文</Badge>
            </div>
            <article className="conversation-article">
              <h1>{conversation.data.title.trim() || "未命名会话"}</h1>
              <div className="article-byline">
                <span>{conversation.data.total} 条可访问消息</span>
                <span>原始记录</span>
              </div>
              {!conversation.data.order_known && (
                <div className="context-note">
                  <Icon name="alert" size={15} />
                  <span>
                    {conversation.data.order_kind === "branch_forest"
                      ? "此会话包含多个回答分支，以下按已保存的关系展示；不视为单一对话。"
                      : "部分消息关系未知，以下为已保存的记录，不补写缺失内容。"}
                  </span>
                </div>
              )}
              <div className="message-list">
                {conversation.data.messages.map((message) => (
                  <MessageView
                    key={message.ref}
                    message={message}
                    focused={message.ref === focusRef}
                  />
                ))}
              </div>
              <div className="reader-pagination">
                <button
                  className="button compact"
                  disabled={offset === 0}
                  onClick={() => setOffset(0)}
                >
                  <Icon name="back" size={15} />
                  回到开头
                </button>
                <span>
                  {offset + 1}–{offset + conversation.data.messages.length} /{" "}
                  {conversation.data.total}
                </span>
                <button
                  className="button compact"
                  disabled={conversation.data.next_offset === null}
                  onClick={() => setOffset(conversation.data!.next_offset!)}
                >
                  继续阅读
                  <Icon name="arrow" size={15} />
                </button>
              </div>
              <p className="coverage-note">
                只包含已保存且当前可访问的内容。原始时间未知时，不用导入时间替代。
              </p>
            </article>
          </>
        )
      )}
    </>
  );
}
function MessageView({
  message,
  focused = false,
}: {
  message: Message;
  focused?: boolean;
}) {
  const role =
    message.role === "user"
      ? "用户"
      : message.role === "assistant"
        ? "AI 助手"
        : message.role === "tool"
          ? "工具"
          : "系统来源";
  return (
    <section
      className={`source-message ${focused ? "focused-message" : ""}`}
      data-reference={message.ref}
    >
      {focused && (
        <p className="source-position" role="status">
          已定位到这条原话
        </p>
      )}
      {message.branch?.on_current_path === false && (
        <p className="branch-marker">其他分支，不作为当前回答链</p>
      )}
      {message.branch?.gap_before && (
        <div className="branch-marker">这里存在缺失的上文</div>
      )}
      <header>
        <span className={`role-avatar ${message.role}`}>
          <Icon name={message.role === "user" ? "memory" : "code"} size={14} />
        </span>
        <strong>{role}</strong>
        <span>{dateLabel(message.occurred_at, true)}</span>
        {message.branch?.child_count && message.branch.child_count > 1 ? (
          <Badge tone="warning">分支点</Badge>
        ) : null}
      </header>
      {message.text ? (
        <Prose text={message.text} />
      ) : (
        !message.assets && (
          <p className="coverage-note">此记录没有可显示的文本正文。</p>
        )
      )}
      {message.assets && <MessageAssets assets={message.assets} />}
      {message.text_truncated && (
        <p className="context-note">此条为节选，完整原文仍保存在本机文件中。</p>
      )}
      {message.branch?.is_branch_end && (
        <span className="branch-end">此分支末端</span>
      )}
    </section>
  );
}

function MessageAssets({ assets }: { assets: SourceAssets }) {
  const sizeLabel = (bytes: number | null) =>
    bytes == null || !Number.isFinite(bytes) || bytes < 0
      ? "大小未提供"
      : bytes < 1024
        ? `${bytes} B`
        : bytes < 1048576
          ? `${(bytes / 1024).toFixed(1)} KB`
          : `${(bytes / 1048576).toFixed(1)} MB`;
  return (
    <div className="message-assets" aria-label="消息中的附件与引用">
      {assets.files.map((file, index) => (
        <div className="asset-row" key={`file-${index}`}>
          <Icon name="file" size={18} />
          <div>
            <strong>{file.name?.trim() || "未命名附件"}</strong>
            <p>{sizeLabel(file.byte_count)} · 导出未包含文件内容</p>
          </div>
          <Badge>仅元数据</Badge>
        </div>
      ))}
      {assets.citations.map((citation, index) => (
        <div className="asset-row citation-row" key={`citation-${index}`}>
          <Icon name="book" size={18} />
          <div>
            <strong>
              {citation.title?.trim() || citation.url || "未命名引用"}
            </strong>
            {citation.title?.trim() && (
              <p className="citation-url">{citation.url}</p>
            )}
            <p>仅保存引用信息，未下载网页正文</p>
          </div>
          <Badge>引用</Badge>
        </div>
      ))}
      {assets.tool_trace.map((trace, index) => (
        <div className="asset-row" key={`trace-${index}`}>
          <Icon name="code" size={18} />
          <div>
            <strong>{trace.type || "工具记录"}</strong>
            <p>
              {trace.payload_status === "references_only"
                ? trace.reference_count == null
                  ? "仅保留引用信息，没有完整工具正文"
                  : `仅有 ${trace.reference_count} 条相关引用，没有完整工具正文`
                : "导出未包含工具正文"}
            </p>
          </div>
          <Badge>工具记录</Badge>
        </div>
      ))}
      {assets.unsupported_total > 0 && (
        <p className="asset-coverage">
          另有 {assets.unsupported_total} 个未支持的片段，无法显示内容。
        </p>
      )}
      {assets.truncated && (
        <p className="asset-coverage">
          当前展示 {assets.files.length} / {assets.files_total} 个附件、
          {assets.citations.length} / {assets.citations_total} 条引用、
          {assets.tool_trace.length} / {assets.trace_total}{" "}
          条工具记录；其余元数据未在本页展开。
        </p>
      )}
    </div>
  );
}
