import { useRef, useState } from "react";
import type { ScopedService } from "../service/client";
import {
  appError,
  isRunning,
  importRecovery,
  type AppError,
  type JobStatus,
} from "../service/contracts";
import { useResource } from "../service/hooks";
import {
  dateLabel,
  platformLabel,
  titleOf,
  type Conversation,
  type Memory,
  type MemoryRow,
} from "../service/types";
import {
  Badge,
  Dialog,
  ErrorNotice,
  Loading,
  Prose,
} from "../components/common";
import { Icon } from "../components/Icon";
import { MemoryChangeDialog } from "./Memories";
import { navigate } from "../App";
import "./background.css";

export function BackgroundPage({
  service,
  refresh,
  changed,
  jobs,
  jobsError,
  importError,
  importing,
  importSources,
}: {
  service: ScopedService;
  refresh: number;
  changed: () => void;
  jobs: JobStatus[];
  jobsError: AppError | null;
  importError: AppError | null;
  importing: boolean;
  importSources: () => Promise<void>;
}) {
  const records = useResource(
    () => Promise.all([service.memories(), service.conversations()]),
    [service, refresh],
  );
  const [adding, setAdding] = useState<string | null>(null);
  const [editing, setEditing] = useState<Memory | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [saved, setSaved] = useState("");
  const [connecting, setConnecting] = useState(false);
  const active = jobs.find(isRunning);
  const failed = jobs.find(
    (job) => job.state === "failed" || job.state === "needs_input",
  );
  const memories = records.data?.[0].memories || [];
  const conversations = records.data?.[1].conversations || [];
  const hasData =
    (records.data?.[0].total || 0) + (records.data?.[1].total || 0) > 0;
  const edit = async (row: MemoryRow) => {
    setError(null);
    try {
      setEditing(await service.memory(row.id));
    } catch (reason) {
      setError(appError(reason));
    }
  };
  return (
    <div className="background-page">
      {(active || jobsError || importError || failed) && (
        <section className="background-import" aria-label="导入进度">
          {jobsError && <ErrorNotice error={jobsError} />}
          {active && (
            <div className="import-reading">
              <span className="spinner" />
              <div>
                <strong>
                  {active.phase === "committing"
                    ? "正在保存你的对话"
                    : "正在读取聊天记录"}
                </strong>
                <p>
                  {active.progress.events_added > 0
                    ? `已保存 ${active.progress.events_added.toLocaleString()} 条，下面已可阅读`
                    : `已识别 ${active.progress.events_staged.toLocaleString()} 条记录，正在核对文件`}
                </p>
              </div>
              <button
                className="text-button"
                onClick={() => navigate("activity")}
              >
                查看详情
              </button>
            </div>
          )}
          {(importError || failed?.error) && (
            <ErrorNotice
              error={(importError || failed?.error)!}
              retryLabel="重新选择文件"
              retry={
                failed && importRecovery(failed) === "new_source"
                  ? () => void importSources()
                  : undefined
              }
            />
          )}
        </section>
      )}
      {saved && (
        <p className="background-saved" role="status">
          <Icon name="check" size={15} />
          {saved}
        </p>
      )}
      {error && <ErrorNotice error={error} />}
      {records.loading ? (
        <Loading label="正在打开已有背景" />
      ) : records.error ? (
        <ErrorNotice error={records.error} retry={records.refresh} />
      ) : !hasData ? (
        <div className="background-start">
          <span className="empty-icon">
            <Icon name="sources" size={29} />
          </span>
          <h1>把已有的对话带进来</h1>
          <p>
            选择 ChatGPT 或 DeepSeek 的官方导出。
            <br />
            文件保存后就能阅读、查找，不用先配置模型。
          </p>
          <button
            className="button primary"
            disabled={importing}
            onClick={() => void importSources()}
          >
            <Icon name="download" size={17} />
            {importing ? "正在选择文件…" : "导入聊天记录"}
          </button>
          <span>直接选择 ZIP 或 JSON，无需解压或挑选格式</span>
        </div>
      ) : (
        <>
          <header className="background-heading">
            <div>
              <span className="eyebrow">你的本地背景</span>
              <h1>接着聊，从这里开始</h1>
              <p>
                已有的决定、近况与原话都能回到出处。连接后，ChatGPT
                可以按需查找。
              </p>
            </div>
            <button
              className="button primary"
              onClick={() => setConnecting(true)}
            >
              <Icon name="link" size={16} />
              连接 ChatGPT
            </button>
          </header>
          <div className="background-current">
            <div>
              <Icon name="shield" size={15} />
              <span>ChatGPT 待连接</span>
              <span>授权和真实读取尚未验证</span>
            </div>
            <button className="text-button" onClick={() => setAdding("")}>
              补充最新情况
              <Icon name="plus" size={14} />
            </button>
          </div>
          {memories.length > 0 && (
            <section className="background-section">
              <div className="section-heading">
                <h2>已保存的记忆</h2>
                <span>重要的内容可以在这里纠正</span>
              </div>
              <div className="background-facts">
                {memories.slice(0, 3).map((memory) => (
                  <article className="background-fact" key={memory.id}>
                    <div className="fact-heading">
                      <h3>{titleOf(memory.content, 80)}</h3>
                      {memory.status === "tentative" && (
                        <Badge tone="warning">待核对</Badge>
                      )}
                    </div>
                    <p>
                      {memory.content.includes("\n")
                        ? memory.content.split("\n").slice(1).join("\n").trim()
                        : memory.content}
                    </p>
                    <footer>
                      <button
                        className="text-button"
                        onClick={() => navigate("memories", memory.id)}
                      >
                        {memory.source_refs.length} 条出处{" "}
                        <Icon name="chevron" size={13} />
                      </button>
                      <span>{dateLabel(memory.updated_at)} 更新</span>
                      <button
                        className="text-button"
                        onClick={() => void edit(memory)}
                      >
                        <Icon name="edit" size={13} />
                        纠正
                      </button>
                    </footer>
                  </article>
                ))}
              </div>
            </section>
          )}
          <section className="background-section">
            <div className="section-heading">
              <h2>{memories.length ? "最近保存的对话" : "从这些原话继续"}</h2>
              <button
                className="text-button"
                onClick={() => navigate("sources")}
              >
                查看全部 {records.data?.[1].total} 段
                <Icon name="arrow" size={14} />
              </button>
            </div>
            <div className="background-conversations">
              {conversations.slice(0, 4).map((conversation) => (
                <ConversationContext
                  key={conversation.session_ref}
                  service={service}
                  conversation={conversation}
                  update={() =>
                    setAdding(conversation.title.trim() || "这段对话")
                  }
                />
              ))}
            </div>
          </section>
          <p className="background-boundary">
            原话保留当时的时间和说法，不会被冒充成已经提炼的新记忆。有变化时，补充一句最新情况即可让之后的查找读到它。
          </p>
        </>
      )}
      {adding !== null && (
        <AddUpdateDialog
          service={service}
          subject={adding}
          close={() => setAdding(null)}
          saved={() => {
            setAdding(null);
            setSaved("已保存你的补充，新的查找可以立即读到；原对话保持原样。");
            changed();
            records.refresh();
          }}
        />
      )}
      {editing && (
        <MemoryChangeDialog
          service={service}
          memory={editing}
          mode="edit"
          quickSave
          close={() => setEditing(null)}
          changed={() => {
            setEditing(null);
            setSaved("背景已更新，新版本保留了出处。");
            changed();
            records.refresh();
          }}
        />
      )}
      {connecting && (
        <ChatGPTEntry service={service} close={() => setConnecting(false)} />
      )}
    </div>
  );
}
function ConversationContext({
  service,
  conversation,
  update,
}: {
  service: ScopedService;
  conversation: Conversation;
  update: () => void;
}) {
  const messages = useResource(
    () =>
      service.messages(
        conversation.session_ref,
        Math.max(0, conversation.message_count - 8),
      ),
    [service, conversation.session_ref, conversation.message_count],
  );
  const textRows = (messages.data?.messages || []).filter(
    (message) =>
      message.role === "user" &&
      message.text.trim() &&
      message.branch?.on_current_path !== false,
  );
  const excerpt = textRows[textRows.length - 1];
  return (
    <article className="context-conversation">
      <div className="context-conversation-heading">
        <span className={`platform-icon ${conversation.platform}`}>
          {platformLabel(conversation.platform).slice(0, 1)}
        </span>
        <div>
          <h3>{conversation.title.trim() || "未命名会话"}</h3>
          <p>
            {platformLabel(conversation.platform)} ·{" "}
            {conversation.last_occurred_at
              ? dateLabel(conversation.last_occurred_at, true)
              : "原始时间未提供"}
          </p>
        </div>
      </div>
      {messages.loading ? (
        <p className="context-excerpt muted">正在打开原话…</p>
      ) : messages.error ? (
        <ErrorNotice error={messages.error} retry={messages.refresh} />
      ) : excerpt ? (
        <div className="context-excerpt">
          <span>
            {conversation.platform === "recallcard-desktop"
              ? "本地补充"
              : "你的原话"}
          </span>
          <p>
            {excerpt.text.length > 280
              ? `${excerpt.text.slice(0, 280)}…`
              : excerpt.text}
          </p>
        </div>
      ) : (
        <p className="context-excerpt muted">
          这段记录中的附件、工具或其他分支信息，可在出处中查看。
        </p>
      )}
      <footer>
        <button
          className="text-button"
          onClick={() =>
            navigate(
              "sources",
              conversation.session_ref,
              "",
              excerpt?.ref || "",
            )
          }
        >
          回到出处
          <Icon name="arrow" size={14} />
        </button>
        <button className="text-button" onClick={update}>
          补充近况
        </button>
      </footer>
    </article>
  );
}
function AddUpdateDialog({
  service,
  subject,
  close,
  saved,
}: {
  service: ScopedService;
  subject: string;
  close: () => void;
  saved: () => void;
}) {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<{
    preview_id: string;
    content: string;
    redacted: boolean;
  } | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const save = async () => {
    if (lock.current || !text.trim()) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      const item =
        preview ||
        (await service.previewNote(
          subject
            ? `关于「${subject}」的最新情况：\n\n${text.trim()}`
            : text.trim(),
        ));
      if (item.redacted && !preview) {
        setPreview(item);
        return;
      }
      setPreview(item);
      await service.confirmNote(item.preview_id);
      saved();
    } catch (reason) {
      setError(appError(reason));
    } finally {
      lock.current = false;
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={subject ? `补充「${subject}」的近况` : "补充最新情况"}
      close={close}
      busy={busy}
      wide
    >
      <div className="modal-body">
        {error && <ErrorNotice error={error} />}
        <p>
          直接写现在的情况，例如“报告已经交付，只剩图表勘误”。这会保存为你的新补充，不改写原对话。
        </p>
        {preview ? (
          <>
            <p className="context-note">
              {preview.redacted
                ? "检测到需要隐藏的内容，请检查将保存的文字。"
                : "保存结果尚未确认，请检查这些文字后重试同一笔保存。"}
            </p>
            <Prose text={preview.content} />
          </>
        ) : (
          <>
            <label className="field-label" htmlFor="latest-update">
              最新情况
            </label>
            <textarea
              id="latest-update"
              className="text-area"
              value={text}
              onChange={(event) => setText(event.target.value)}
              autoFocus
              placeholder="写一句现在的情况…"
            />
          </>
        )}
      </div>
      <div className="modal-footer">
        <button className="button" onClick={close} disabled={busy}>
          取消
        </button>
        <button
          className="button primary"
          disabled={busy || !text.trim()}
          onClick={() => void save()}
        >
          {busy ? "正在保存…" : preview ? "确认保存这些文字" : "保存近况"}
        </button>
      </div>
    </Dialog>
  );
}
function ChatGPTEntry({
  service,
  close,
}: {
  service: ScopedService;
  close: () => void;
}) {
  const plan = useResource(() => service.chatgptPlan(), [service]);
  return (
    <Dialog title="让 ChatGPT 接上这些背景" close={close} wide>
      <div className="modal-body">
        <p>
          ChatGPT 通过原生只读连接查找本机背景，不需要把对话搬进另一个聊天窗口。
        </p>
        {plan.loading ? (
          <Loading label="正在检查本机读取入口" />
        ) : plan.error ? (
          <ErrorNotice error={plan.error} retry={plan.refresh} />
        ) : (
          <>
            <div className="chatgpt-connection-state">
              <Icon name="link" size={20} />
              <div>
                <strong>
                  {plan.data?.local_readiness === "ready"
                    ? "本机读取已准备"
                    : "本机读取待授权"}
                </strong>
                <p>ChatGPT 官方连接与真实读取尚未验证</p>
              </div>
              <Badge>待连接</Badge>
            </div>
            <p>
              连接后，资料正本仍在本机；ChatGPT
              实际检索到的背景、原文和出处会提供给
              OpenAI。只读连接不能改写这些资料。
            </p>
            <p className="context-note">
              还需在你长期使用的电脑上完成官方连接。当前未设置官方账号授权、隧道或凭据。
            </p>
            <details className="advanced-details">
              <summary>为什么仍需一次官方连接</summary>
              <p>
                使用 OpenAI Secure MCP Tunnel 将现有本机读取工具连接到
                ChatGPT；无需开放本机 HTTP
                端口。账号和持续访问授权需要你明确确认。
              </p>
            </details>
          </>
        )}
      </div>
      <div className="modal-footer">
        <button className="button" onClick={close}>
          继续查看本机背景
        </button>
      </div>
    </Dialog>
  );
}
