import { useEffect, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, type AppError } from "../service/contracts";
import { useResource } from "../service/hooks";
import {
  dateLabel,
  evidenceLabel,
  eventText,
  platformLabel,
  statusLabel,
  titleOf,
  type Memory,
  type MemoryReview,
} from "../service/types";
import { Icon } from "../components/Icon";
import {
  Badge,
  Dialog,
  EmptyState,
  ErrorNotice,
  Loading,
  Prose,
} from "../components/common";
import { navigate } from "../App";

type Filter = "current" | "tentative" | "hidden";
export function MemoriesPage({
  service,
  selected,
  refresh,
  changed,
}: {
  service: ScopedService;
  selected: string;
  refresh: number;
  changed: () => void;
}) {
  const [filter, setFilter] = useState<Filter>("current");
  const [offset, setOffset] = useState(0);
  const records = useResource(
    () => service.memories(offset, filter),
    [service, offset, filter, refresh],
  );
  const rows = records.data?.memories || [];
  const selectedId = selected || rows[0]?.id;
  return (
    <div className={`split-view ${selected ? "has-selection" : ""}`}>
      <section className="collection-panel" aria-label="记忆列表">
        <div className="collection-heading">
          <h1>记忆</h1>
          {records.data && (
            <span className="muted count-label">
              {records.data.total} 条{filter === "hidden" ? "（含历史）" : ""}
            </span>
          )}
        </div>
        <div className="segmented-tabs" aria-label="记忆筛选">
          {(
            [
              ["current", "当前"],
              ["tentative", "待确认"],
              ["hidden", "已隐藏 / 历史"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              className={filter === id ? "active" : ""}
              aria-pressed={filter === id}
              onClick={() => {
                setFilter(id);
                setOffset(0);
                navigate("memories");
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="collection-scroll">
          {records.loading ? (
            <Loading />
          ) : records.error ? (
            <ErrorNotice error={records.error} retry={records.refresh} />
          ) : rows.length ? (
            <div className="record-list">
              {rows.map((row) => (
                <button
                  key={row.id}
                  className={`memory-row ${selectedId === row.id ? "selected" : ""}`}
                  onClick={() => navigate("memories", row.id)}
                  aria-current={selectedId === row.id ? "true" : undefined}
                >
                  <div className="row-title">
                    <span>{titleOf(row.content)}</span>
                    {row.protected && <Icon name="shield" size={14} />}
                  </div>
                  <p>
                    {row.content.split("\n").slice(1).join(" ").trim() ||
                      row.content}
                  </p>
                  <div className="row-meta">
                    <span
                      className={`tiny-dot ${row.status === "tentative" ? "warning" : ""}`}
                    />
                    {row.hidden ? "已退出召回" : statusLabel(row.status)}
                    <span className="meta-separator">·</span>
                    {row.source_refs.length} 条来源
                    <span className="row-date">
                      {dateLabel(row.updated_at)}
                    </span>
                  </div>
                </button>
              ))}
            </div>
          ) : (
            <div className="small-empty">
              <Icon
                name={filter === "current" ? "memory" : "check"}
                size={24}
              />
              <h3>
                {filter === "current"
                  ? "还没有记忆"
                  : filter === "tentative"
                    ? "没有待确认记忆"
                    : "没有隐藏或历史记忆"}
              </h3>
              <p>
                {filter === "current"
                  ? "已有对话仍可在来源中阅读和搜索。"
                  : "需要核对的变化与历史记录会出现在相应分类。"}
              </p>
            </div>
          )}
        </div>
        {records.data && (
          <div className="pagination">
            <span>按更新时间排序</span>
            <div>
              <button
                className="icon-button"
                aria-label="上一页记忆"
                disabled={offset === 0 || records.loading}
                onClick={() => setOffset(Math.max(0, offset - 30))}
              >
                <Icon name="back" size={16} />
              </button>
              <button
                className="icon-button"
                aria-label="下一页记忆"
                disabled={records.data.next_offset === null || records.loading}
                onClick={() => setOffset(records.data!.next_offset!)}
              >
                <Icon name="arrow" size={16} />
              </button>
            </div>
          </div>
        )}
      </section>
      <section className="reader-panel" aria-label="记忆阅读区">
        {selectedId ? (
          <MemoryReader
            key={selectedId}
            service={service}
            id={selectedId}
            refresh={refresh}
            changed={() => {
              changed();
              records.refresh();
            }}
          />
        ) : (
          <EmptyState icon="memory" title="把有用的背景留在这里">
            <p>
              记忆会保留出处和版本。你可以阅读、纠正，
              <br />
              也可以让一条内容退出后续召回。
            </p>
            <button className="text-button" onClick={() => navigate("sources")}>
              查看已保存的来源 <Icon name="arrow" size={15} />
            </button>
          </EmptyState>
        )}
      </section>
    </div>
  );
}
function MemoryReader({
  service,
  id,
  refresh,
  changed,
}: {
  service: ScopedService;
  id: string;
  refresh: number;
  changed: () => void;
}) {
  const record = useResource(() => service.memory(id), [service, id, refresh]);
  const [edit, setEdit] = useState<{
    mode: "edit" | "forget" | "restore";
    memory: Memory;
  } | null>(null);
  const beginEdit = (mode: "edit" | "forget" | "restore", memory: Memory) => {
    navigate("memories", id);
    setEdit({ mode, memory });
  };
  const memory = record.data;
  const current = memory && ["active", "tentative"].includes(memory.status);
  return (
    <>
      {record.loading ? (
        <Loading />
      ) : record.error ? (
        <ErrorNotice error={record.error} retry={record.refresh} />
      ) : (
        memory && (
          <>
            <div className="reader-toolbar">
              <button
                className="text-button mobile-back"
                onClick={() => navigate("memories")}
              >
                <Icon name="back" size={15} />
                记忆列表
              </button>
              <span className="reader-kind">
                <Icon name="memory" size={15} />
                记忆
              </span>
              <div className="reader-actions">
                {!memory.hidden && current && (
                  <button
                    className="button quiet compact"
                    onClick={() => beginEdit("edit", memory)}
                  >
                    <Icon name="edit" size={15} />
                    纠正内容
                  </button>
                )}
                {(current || memory.hidden) && (
                  <button
                    className="button quiet compact"
                    disabled={memory.hidden && !memory.can_restore}
                    title={
                      memory.hidden && !memory.can_restore
                        ? "此条受其他来源规则影响，无法单独恢复"
                        : undefined
                    }
                    onClick={() =>
                      beginEdit(memory.hidden ? "restore" : "forget", memory)
                    }
                  >
                    {memory.hidden
                      ? memory.can_restore
                        ? "检查恢复影响"
                        : "由来源规则隐藏"
                      : "停止召回"}
                  </button>
                )}
                {!current && !memory.hidden && <Badge>历史版本只读</Badge>}
              </div>
            </div>
            <article className="memory-article">
              <div className="article-status">
                <Badge
                  tone={
                    memory.hidden
                      ? "muted"
                      : memory.status === "tentative"
                        ? "warning"
                        : "success"
                  }
                >
                  {memory.hidden ? "已退出召回" : statusLabel(memory.status)}
                </Badge>
                <span>{evidenceLabel(memory.evidence)}</span>
                {memory.protected && (
                  <span className="protected-label">
                    <Icon name="shield" size={13} />
                    受保护
                  </span>
                )}
              </div>
              <h1>{titleOf(memory.content, 100)}</h1>
              <div className="article-byline">
                更新于 {dateLabel(memory.updated_at, true)}
                <span>版本 {memory.revision}</span>
              </div>
              <Prose
                text={
                  memory.content.includes("\n")
                    ? memory.content.split("\n").slice(1).join("\n").trim() ||
                      memory.content
                    : memory.content
                }
              />
              {memory.time_note && (
                <p className="context-note">时间说明：{memory.time_note}</p>
              )}
              {memory.labels?.length > 0 && (
                <div className="label-list">
                  {memory.labels.map((label) => (
                    <Badge key={label}>{label}</Badge>
                  ))}
                </div>
              )}
              <MemorySources service={service} memory={memory} />
              <footer className="article-footer">
                <Icon name="shield" size={15} />
                <span>纠正会保存新版本；原始来源保持可追溯。</span>
              </footer>
            </article>
          </>
        )
      )}
      {edit && (
        <MemoryChangeDialog
          service={service}
          memory={edit.memory}
          mode={edit.mode}
          close={() => setEdit(null)}
          changed={() => {
            setEdit(null);
            changed();
            record.refresh();
          }}
        />
      )}
    </>
  );
}
function MemorySources({
  service,
  memory,
}: {
  service: ScopedService;
  memory: Memory;
}) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const refs = showAll ? memory.source_refs : memory.source_refs.slice(0, 3);
  const sources = useResource(
    () => Promise.all(refs.map((ref) => service.memorySource(memory.id, ref))),
    [service, memory.id, memory.revision, showAll],
  );
  return (
    <section className="evidence-section">
      <div className="section-heading">
        <h2>来源依据</h2>
        <span>{memory.source_refs.length} 条</span>
      </div>
      {sources.loading ? (
        <Loading label="正在读取来源" />
      ) : sources.error ? (
        <ErrorNotice error={sources.error} retry={sources.refresh} />
      ) : (
        sources.data?.map((event, index) => (
          <div key={event.id} className="evidence-item">
            <button
              className="evidence-summary"
              onClick={() =>
                setExpanded(expanded === event.id ? null : event.id)
              }
              aria-expanded={expanded === event.id}
            >
              <span className="source-number">{index + 1}</span>
              <div>
                <strong>
                  {event.metadata?.conversation_title ||
                    titleOf(eventText(event), 42)}
                </strong>
                <span>
                  {platformLabel(event.source.platform)} ·{" "}
                  {event.role === "user"
                    ? "用户"
                    : event.role === "assistant"
                      ? "AI 助手"
                      : "工具 / 系统"}{" "}
                  · {dateLabel(event.occurred_at)}
                </span>
              </div>
              <Icon name="chevron" size={15} />
            </button>
            {expanded === event.id && (
              <div className="evidence-quote">
                <Prose text={eventText(event)} />
                <p>原始时间：{dateLabel(event.occurred_at, true)}</p>
              </div>
            )}
          </div>
        ))
      )}
      {memory.source_refs.length > 3 && (
        <button
          className="text-button evidence-more"
          onClick={() => setShowAll(!showAll)}
        >
          {showAll
            ? "收起来源"
            : `查看全部 ${memory.source_refs.length} 条来源`}
        </button>
      )}
    </section>
  );
}
export function MemoryChangeDialog({
  service,
  memory,
  mode,
  close,
  changed,
  quickSave = false,
}: {
  service: ScopedService;
  memory: Memory;
  quickSave?: boolean;
  mode: "edit" | "forget" | "restore";
  close: () => void;
  changed: () => void;
}) {
  const [content, setContent] = useState(memory.content);
  const [reason, setReason] = useState("");
  const [review, setReview] = useState<MemoryReview | null>(null);
  const [approved, setApproved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const title =
    mode === "edit"
      ? "纠正这条记忆"
      : mode === "forget"
        ? "停止召回这条记忆"
        : "恢复召回";
  const dismiss = () => {
    void service.cancelPreviews().catch(() => {});
    close();
  };
  useEffect(() => {
    if (mode === "restore") void prepare();
  }, []);
  async function prepare() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await (mode === "edit"
        ? service.reviewEdit(memory, content)
        : service.reviewVisibility(memory.id, mode === "restore", reason));
      if (quickSave && mode === "edit" && !result.requires_protected_approval) {
        await service.confirmMemory(result.preview_id, false);
        changed();
      } else setReview(result);
    } catch (value) {
      setError(appError(value));
    } finally {
      setBusy(false);
    }
  }
  async function confirm() {
    if (!review || busy) return;
    setBusy(true);
    setError(null);
    try {
      await service.confirmMemory(review.preview_id, approved);
      changed();
    } catch (value) {
      setError(appError(value));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog title={title} close={dismiss} busy={busy} wide>
      <div className="modal-body">
        {error && <ErrorNotice error={error} />}
        {review ? (
          <>
            <p>{review.warning}</p>
            {review.after && (
              <div className="review-content">
                <span className="eyebrow">将保存的内容</span>
                <Prose text={review.after.content} />
              </div>
            )}
            {mode !== "edit" && (
              <div className="impact-summary">
                <strong>{review.affected_memories} 条记忆</strong>
                <span>和 {review.affected_events} 条相关原文可能受到影响</span>
              </div>
            )}
            {review.requires_protected_approval && (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={approved}
                  onChange={(event) => setApproved(event.target.checked)}
                />
                我已检查，允许本次修改受保护的内容
              </label>
            )}
          </>
        ) : mode === "edit" ? (
          <>
            <p>修正事实或补充细节。保存前会检查来源与当前版本。</p>
            <label className="field-label" htmlFor="memory-content">
              记忆内容
            </label>
            <textarea
              id="memory-content"
              className="text-area memory-editor"
              value={content}
              onChange={(event) => setContent(event.target.value)}
            />
          </>
        ) : mode === "forget" ? (
          <>
            <p>
              停止后，相关内容会退出检索和后续整理。下一步会列出受影响的记忆和来源。
            </p>
            <label className="field-label" htmlFor="forget-reason">
              原因
            </label>
            <textarea
              id="forget-reason"
              className="text-area"
              placeholder="例如：计划已取消，不再适用"
              value={reason}
              onChange={(event) => setReason(event.target.value)}
            />
          </>
        ) : (
          <Loading label="正在检查恢复影响" />
        )}
      </div>
      <div className="modal-footer">
        <button className="button" onClick={dismiss} disabled={busy}>
          取消
        </button>
        {review ? (
          <button
            className={`button ${mode === "forget" ? "danger" : "primary"}`}
            onClick={() => void confirm()}
            disabled={busy || (review.requires_protected_approval && !approved)}
          >
            {busy
              ? "正在保存…"
              : mode === "edit"
                ? "保存新版本"
                : mode === "forget"
                  ? "确认停止召回"
                  : "确认恢复"}
          </button>
        ) : (
          mode !== "restore" && (
            <button
              className="button primary"
              onClick={() => void prepare()}
              disabled={
                busy ||
                !(mode === "edit"
                  ? content.trim() && content !== memory.content
                  : reason.trim())
              }
            >
              {busy ? "正在保存…" : quickSave && mode === "edit" ? "保存修改" : "检查更改"}
            </button>
          )
        )}
      </div>
    </Dialog>
  );
}
