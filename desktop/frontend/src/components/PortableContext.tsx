import { useEffect, useRef, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, type AppError } from "../service/contracts";
import { evidenceLabel, scopeLabel } from "../service/types";
import { revalidateAndCopy, utf8Bytes, type PortableContext, type ReplyPreview } from "../service/portable";
import { Badge, Dialog, ErrorNotice, Prose } from "./common";

export interface ContextChoice { ref: string; label: string }
export function PortableContextDialog({ service, choices, initialGoal, close }: {
  service: ScopedService; choices: ContextChoice[]; initialGoal: string; close: () => void;
}) {
  const [goal, setGoal] = useState(initialGoal);
  const [refs, setRefs] = useState(choices.slice(0, 8).map(row => row.ref));
  const [preview, setPreview] = useState<PortableContext | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const lock = useRef(false), active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, [service]);
  const prepare = () => service.prepareContext(refs, goal.trim());
  const run = async (copy: boolean) => {
    if (lock.current || !refs.length || !goal.trim()) return;
    lock.current = true; setBusy(true); setError(null); setStatus("");
    try {
      if (copy && preview) {
        const result = await revalidateAndCopy(preview, prepare, text => service.copy(text), () => active.current);
        if (result) {
          setPreview(result.current);
          setStatus(result.copied ? "已复制。请到任意 AI 粘贴，检查后由你发送。" : "资料已变化，尚未复制。请核对更新后的预览，再点击复制。");
        }
      } else {
        const result = await prepare();
        if (active.current) setPreview(result);
      }
    } catch (reason) { if (active.current) { setError(appError(reason)); setPreview(null); } }
    finally { lock.current = false; if (active.current) setBusy(false); }
  };
  const invalidate = () => { setPreview(null); setStatus(""); setError(null); };
  return <Dialog title="带到任意 AI" close={close} busy={busy} wide>
    <div className="modal-body portable-context">
      <p>在 {scopeLabel(service.scope)} 准备资料。无需安装扩展或登录网站，复制后在常用 AI 中粘贴并自行发送。</p>
      <label className="field-label" htmlFor="portable-goal">这次想问什么</label>
      <textarea id="portable-goal" className="text-area" value={goal} disabled={busy} onChange={e => { setGoal(e.target.value); invalidate(); }} />
      {utf8Bytes(goal.trim()) > 4096 && <p role="alert">问题最多 4,096 字节，请缩短后再准备。</p>}
      <fieldset disabled={busy}><legend>相关来源 · 已选 {refs.length} / 8 条</legend>
        {choices.map(choice => <label className="portable-choice" key={choice.ref}>
          <input type="checkbox" checked={refs.includes(choice.ref)} disabled={!refs.includes(choice.ref) && refs.length >= 8}
            onChange={e => { setRefs(e.target.checked ? [...refs, choice.ref] : refs.filter(ref => ref !== choice.ref)); invalidate(); }} />
          <span>{choice.label}<small>{choice.ref}</small></span>
        </label>)}
      </fieldset>
      <p className="context-note">不会复制：未勾选的 {choices.length - refs.length} 条结果、其他历史消息、附件原件及附件正文。稳定背景会一并带入，完整内容见下方预览；片段不代表完整会话或同一分支。</p>
      {error && <ErrorNotice error={error} />}
      {preview && <PortablePreview preview={preview} />}
      {status && <p role="status">{status}</p>}
    </div>
    <div className="modal-footer"><button className="button" onClick={close} disabled={busy}>关闭</button>
      <button className="button primary" disabled={busy || !refs.length || !goal.trim() || utf8Bytes(goal.trim()) > 4096} onClick={() => void run(!!preview)}>
        {busy ? "正在核对资料…" : preview ? "核对并复制上下文" : "预览将复制的内容"}
      </button></div>
  </Dialog>;
}
export function PortablePreview({ preview }: { preview: PortableContext }) {
  return <section aria-label="上下文复制预览">
    <h3>{preview.included_count} 条来源 · {utf8Bytes(preview.text).toLocaleString()} 字节 UTF-8</h3>
    <ul>{preview.records.map(record => <li key={record.ref}>
      <Badge>{record.role === "user" ? "用户原话" : record.role === "assistant" ? "AI 回答，非用户事实" : evidenceLabel(record.evidence)}</Badge>{" "}
      {record.conversation_title || record.text.slice(0, 60) || record.ref}{record.truncated && " · 正文仅节选"}{!record.content_retained && " · 未保留正文"}
    </li>)}</ul>
    {preview.pending_refs.length > 0 && <p>预算内未复制的引用：{preview.pending_refs.join("、")}</p>}
    {preview.truncated && <p className="context-note">部分资料或稳定背景已截短，未带入的尾部不能据此补全。</p>}
    <label className="field-label" htmlFor="portable-preview">完整复制内容</label>
    <textarea id="portable-preview" className="text-area portable-preview" value={preview.text} readOnly />
  </section>;
}

export function PasteReplyDialog({ service, close, saved }: { service: ScopedService; close: () => void; saved: (ref: string) => void }) {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<ReplyPreview | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false), active = useRef(true), previewToken = useRef<string | null>(null);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      if (previewToken.current) void service.cancelAiReply(previewToken.current).catch(() => {});
    };
  }, [service]);
  const dismiss = async () => {
    if (lock.current) return;
    lock.current = true; setBusy(true); setError(null);
    try {
      if (previewToken.current) await service.cancelAiReply(previewToken.current);
      if (active.current) { previewToken.current = null; close(); }
    } catch (reason) { if (active.current) setError(appError(reason)); }
    finally { lock.current = false; if (active.current) setBusy(false); }
  };
  const run = async () => {
    if (lock.current || !text.trim()) return;
    lock.current = true; setBusy(true); setError(null);
    try {
      if (!preview) {
        const result = await service.previewAiReply(text);
        if (result.role !== "assistant" || result.automatic_memory !== false || result.scope !== service.scope || result.session_id !== service.workspace.session_id)
          throw new Error("回答预览的来源或空间不一致，未保存。");
        previewToken.current = result.preview_id;
        if (active.current) setPreview(result);
        else await service.cancelAiReply(result.preview_id);
      } else {
        const result = await service.confirmAiReply(preview.preview_id);
        if (active.current) saved(result.ref);
      }
    } catch (reason) { if (active.current) setError(appError(reason)); }
    finally { lock.current = false; if (active.current) setBusy(false); }
  };
  return <Dialog title="粘贴 AI 回答" close={() => void dismiss()} busy={busy} wide>
    <div className="modal-body">
      <p>手动粘贴任意 AI 的普通回答，不需要特殊格式。只保存为 {scopeLabel(service.scope)} 的助手来源，可检索、可查原文，不自动整理成你的事实。</p>
      <p className="context-note">来源平台、原会话、父子关系与发生时间保持未核验；不会读取剪贴板、请求模型或改动网页草稿。</p>
      {error && <ErrorNotice error={error} />}
      {preview ? <><Badge>AI 回答，非用户事实</Badge><p>{preview.redacted ? "需要隐藏的内容已脱敏，请检查后确认。" : "检查将保存的文字，再确认保存。"}</p><Prose text={preview.content} /></> : <>
        <label className="field-label" htmlFor="pasted-ai-reply">AI 回答原文</label>
        <textarea id="pasted-ai-reply" className="text-area" value={text} disabled={busy} onChange={e => setText(e.target.value)} autoFocus />
        <p>{utf8Bytes(text).toLocaleString()} / 65,536 字节</p>
      </>}
    </div>
    <div className="modal-footer"><button className="button" onClick={() => void dismiss()} disabled={busy}>取消</button>
      <button className="button primary" disabled={busy || !text.trim() || utf8Bytes(text) > 65536} onClick={() => void run()}>{busy ? "正在处理…" : preview ? "确认为 AI 回答保存" : "预览回答"}</button></div>
  </Dialog>;
}
