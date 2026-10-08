import { useRef, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, isRunning, importRecovery, type AppError, type JobStatus } from "../service/contracts";
import { usePollingResource, useResource } from "../service/hooks";
import { dateLabel, scopeLabel } from "../service/types";
import { connectionLabel, connectionName, recentConnectionReceipts } from "../service/connections";
import { runtimeLabel } from "../service/runtime";
import { Badge, Dialog, ErrorNotice, Loading, Prose } from "../components/common";
import { Icon } from "../components/Icon";
import { navigate } from "../App";
import "./background.css";

export function BackgroundPage({ service, refresh, changed, jobs, jobsError, importError, importing, importSources }: {
  service: ScopedService; refresh: number; changed: () => void; jobs: JobStatus[]; jobsError: AppError | null;
  importError: AppError | null; importing: boolean; importSources: () => Promise<void>;
}) {
  const records = useResource(() => Promise.all([service.memories(), service.conversations(), service.memories(0, "tentative")]), [service, refresh]);
  const inventory = usePollingResource(() => service.connections(), [service]);
  const runtime = usePollingResource(() => service.runtime(), [service], 8000);
  const [adding, setAdding] = useState(false);
  const [saved, setSaved] = useState("");
  const active = jobs.find(isRunning);
  const failed = jobs.find(job => job.state === "failed" || job.state === "needs_input");
  const connections = inventory.error ? [] : inventory.data?.entries.filter(entry => !entry.revoked) || [];
  const requests = inventory.error ? [] : inventory.data?.pending_pairings || [];
  const receipts = recentConnectionReceipts(inventory.error ? [] : connections);
  const totalSources = records.data?.[1].total;
  const totalMemories = records.data?.[0].total;
  const tentativeCount = records.data?.[2].total || 0;
  const needsConnections = connections.filter(entry => ["failed", "needs_setup", "configured_unverified"].includes(entry.state));
  const runtimeNeedsInput = !runtime.error && runtime.data && ["unconfigured", "paused", "failed", "needs_input"].includes(runtime.data.state);
  const completedMemoryJobs = (runtime.data?.jobs || []).filter(job => job.state === "completed").sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 2);
  const attentionCount = requests.length + needsConnections.length + (failed ? 1 : 0) + (runtimeNeedsInput ? 1 : 0) + (tentativeCount ? 1 : 0);
  const openConnections = () => navigate("connections");
  return <div className="background-page home-page">
    <header className="home-heading">
      <div><span className="eyebrow">{scopeLabel(service.scope)}</span><h1>让下一次对话接得上</h1><p>接好常用的 AI，授权范围内的背景就能自动准备。</p></div>
      <button className="button" onClick={() => setAdding(true)}><Icon name="plus" size={15} />补充最新情况</button>
    </header>
    {saved && <p className="background-saved" role="status"><Icon name="check" size={15} />{saved}</p>}
    <section className="home-connection-panel" aria-label="连接健康">
      <div className="home-panel-heading"><div><h2>你的 AI 连接</h2><p>一次确认范围，之后无需每轮挑选上下文。</p></div><button className="text-button" onClick={openConnections}>管理连接<Icon name="chevron" size={14} /></button></div>
      {inventory.loading ? <Loading label="正在检查连接" /> : inventory.error ? <ErrorNotice error={inventory.error} retry={inventory.refresh} retryLabel="重新检测" /> : connections.length ?
        <div className="home-connection-list">{connections.slice(0, 4).map(entry => <button className="home-connection-item" key={entry.id} onClick={openConnections} data-connection-state={entry.state}>
          <span className="home-client-icon"><Icon name={entry.grant.client_kind === "browser" ? "globe" : "code"} size={19} /></span>
          <span><strong>{connectionName(entry)}</strong><small>{entry.last_error || (entry.grant.auto_recall ? "已授权自动准备背景" : "自动准备未开启")}</small></span>
          <Badge tone={entry.state === "read_succeeded" ? "success" : ["failed", "needs_setup"].includes(entry.state) ? "warning" : "muted"}>{connectionLabel(entry.state)}</Badge><Icon name="chevron" size={14} />
        </button>)}</div> : <div className="home-connect-start"><span className="home-connect-symbol"><Icon name="link" size={26} /></span><div><h3>先接好一个常用的 AI</h3><p>浏览器里的 ChatGPT、DeepSeek，或本地 Codex、Claude Code。</p></div><button className="button primary" onClick={openConnections}>连接 AI<Icon name="arrow" size={15} /></button></div>}
      <div className="home-connection-footer"><Icon name="shield" size={14} /><span>读取回执只证明客户端请求过本机资料。网页里的最后发送仍由你点击。</span></div>
    </section>
    <div className="home-columns">
      <section className="home-receipts" aria-label="最近增强回执">
        <div className="section-heading"><h2>最近增强回执</h2><span>实际发生过的读取与保存</span></div>
        {inventory.loading ? <Loading label="正在读取回执" /> : inventory.error ? <p className="home-empty-note">连接状态暂不可读，暂不展示旧回执。可在上方重新检测。</p> : receipts.length ? <ol className="receipt-list">{receipts.slice(0, 5).map(receipt => <li key={receipt.id}><span className="receipt-mark"><Icon name={receipt.kind === "capture" ? "sources" : "book"} size={15} /></span><div><strong>{receipt.title}</strong><p>{receipt.client} · {receipt.description}</p><time dateTime={receipt.at}>{dateLabel(receipt.at, true)}</time></div></li>)}</ol> : <div className="home-empty-receipts"><Icon name="activity" size={24} /><h3>还没有增强回执</h3><p>连接完成并实际读取后，这里会显示客户端请求过的本机背景。</p><button className="text-button" onClick={openConnections}>去完成连接<Icon name="arrow" size={14} /></button></div>}
        {!runtime.error && completedMemoryJobs.map(job => <div className="memory-receipt" key={job.job_id}><Icon name="memory" size={16} /><div><strong>记忆整理已完成</strong><p>{job.progress.sources_committed} 条来源 · {job.progress.memories_committed} 条记忆 · {dateLabel(job.updated_at, true)}</p></div><button className="text-button" onClick={() => navigate("activity")}>查看</button></div>)}
      </section>
      <section className="home-attention" aria-label="需要处理">
        <div className="section-heading"><h2>需要处理</h2>{attentionCount > 0 && <Badge tone="warning">{attentionCount}</Badge>}</div>
        {requests.length > 0 && <button className="home-action-row" onClick={openConnections}><Icon name="link" size={17} /><span><strong>{requests.length} 个连接请求待批准</strong><small>确认接收方和一次性范围授权</small></span><Icon name="chevron" size={13} /></button>}
        {needsConnections.length > 0 && <button className="home-action-row" onClick={openConnections}><Icon name="alert" size={17} /><span><strong>{needsConnections.length} 个连接还需完成验证</strong><small>继续安装、检测或试读</small></span><Icon name="chevron" size={13} /></button>}
        {runtime.loading ? <Loading label="正在检查自动整理" /> : runtime.error ? <ErrorNotice error={runtime.error} retry={runtime.refresh} /> : runtimeNeedsInput && <button className="home-action-row" onClick={() => navigate("settings")}><Icon name="memory" size={17} /><span><strong>自动整理{runtimeLabel(runtime.data!.state)}</strong><small>{runtime.data!.state === "unconfigured" ? "选择模型并确认资料范围后启用" : runtime.data!.message}</small></span><Icon name="chevron" size={13} /></button>}
        {tentativeCount > 0 && <button className="home-action-row" onClick={() => navigate("memories", "", "", "tentative")}><Icon name="book" size={17} /><span><strong>{tentativeCount} 条记忆待核对</strong><small>查看存在冲突或尚未确认的内容</small></span><Icon name="chevron" size={13} /></button>}
        {failed && <button className="home-action-row" onClick={() => navigate("activity")}><Icon name="alert" size={17} /><span><strong>一次导入需要处理</strong><small>{failed.error?.message || "检查任务详情并继续"}</small></span><Icon name="chevron" size={13} /></button>}
        {!attentionCount && !inventory.loading && !runtime.loading && !records.loading && !inventory.error && !runtime.error && !records.error && !jobsError && <p className="home-empty-note">暂时没有需要你处理的事项。</p>}
        <button className="text-button home-activity-link" onClick={() => navigate("activity")}>查看全部处理记录<Icon name="arrow" size={14} /></button>
      </section>
    </div>
    {(active || jobsError || importError || failed?.error) && <section className="background-import" aria-label="导入进度">
      {jobsError && <ErrorNotice error={jobsError} />}
      {active && <div className="import-reading"><span className="spinner" /><div><strong>{active.phase === "committing" ? "正在保存你的对话" : "正在读取聊天记录"}</strong><p>{active.progress.events_added > 0 ? `已保存 ${active.progress.events_added.toLocaleString()} 条，可到原始资料中阅读` : `已识别 ${active.progress.events_staged.toLocaleString()} 条记录，正在核对文件`}</p></div><button className="text-button" onClick={() => navigate("activity")}>查看详情</button></div>}
      {(importError || failed?.error) && <ErrorNotice error={(importError || failed?.error)!} retryLabel="重新选择文件" retry={failed && importRecovery(failed) === "new_source" ? () => void importSources() : undefined} />}
    </section>}
    <section className="home-library" aria-label="本地资料">
      <div><h2>本地资料</h2><p>原话与记忆都保留在你的空间里，随时可以查找和纠正。</p></div>
      {records.loading ? <span className="muted">正在读取…</span> : records.error ? <ErrorNotice error={records.error} retry={records.refresh} /> : <div className="home-library-links"><button onClick={() => navigate("memories")}><strong>{totalMemories?.toLocaleString()}</strong><span>条记忆<Icon name="chevron" size={13} /></span></button><button onClick={() => navigate("sources")}><strong>{totalSources?.toLocaleString()}</strong><span>段原始资料<Icon name="chevron" size={13} /></span></button></div>}
      {totalSources === 0 && <button className="button" disabled={importing} onClick={() => void importSources()}><Icon name="download" size={15} />{importing ? "正在选择文件…" : "导入聊天记录"}</button>}
    </section>
    {adding && <AddUpdateDialog service={service} subject="" close={() => setAdding(false)} saved={() => { setAdding(false); setSaved("已保存你的补充，新的查找可以立即读到；原对话保持原样。"); changed(); records.refresh(); }} />}
  </div>;
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
