import "./connections.css";
import { useRef, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, type AppError } from "../service/contracts";
import { scopeLabel, type BrowserRegistration } from "../service/types";
import { ConnectionInventoryView } from "../components/ConnectionInventory";
import { Badge, Dialog, ErrorNotice } from "../components/common";
import { Icon } from "../components/Icon";

export function ConnectionsPage({ service }: { service: ScopedService }) {
  const [installing, setInstalling] = useState(false);
  return <div className="standard-page connections-page">
    <div className="page-heading"><div><h1>把背景带到你使用的 AI</h1><p>配置一次，日常自动保存与按需读取。网页的最后发送始终由你点击。</p></div></div>
    <div className="scope-bar"><Icon name="shield" size={18} /><div><strong>{scopeLabel(service.scope)}</strong><span>本页只管理这个范围；个人资料不会随项目自动共享。</span></div><Badge>本地连接</Badge></div>
    <section className="connection-list" aria-label="开始连接">
      <div className="connection-row"><span className="connection-logo browser-logo"><Icon name="globe" size={26} /></span><div className="connection-info"><h2>浏览器：请求一次，在这里批准</h2><p>打开 ChatGPT 或 DeepSeek 中的 RecallCard 扩展，点击「请求桌面连接」。待批准请求会出现在下方，无需搬运对话或粘贴 JSON。</p><div className="capability-list"><span>稳定可见消息</span><span>自动准备草稿</span><span>最终手动发送</span></div></div></div>
      <div className="connection-row"><span className="connection-logo agent-logo"><Icon name="code" size={26} /></span><div className="connection-info"><h2>Claude Code：从稳定背景开始</h2><p>先批准当前范围，再生成绑定该连接的 MCP 与 SessionStart 配置。开始、恢复、压缩后可加载背景，细节由只读工具按需查询。</p></div></div>
    </section>
    <ConnectionInventoryView service={service} />
    <details className="advanced-details"><summary>首次使用：安装或修复浏览器本机桥</summary><p>只有扩展提示找不到本机桥时才需要这里。注册固定扩展与资料范围后，再从扩展发起配对并分别批准读取和保存。</p><button className="button" onClick={() => setInstalling(true)}>设置浏览器本机桥</button><p className="field-hint">这是本机传输安装步骤。注册完成不代表已授权自动接入，也不证明账号或网站适配已验证。</p></details>
    {installing && <BrowserBridgeDialog service={service} close={() => setInstalling(false)} />}
  </div>;
}
function BrowserBridgeDialog({ service, close }: { service: ScopedService; close: () => void }) {
  const [registration, setRegistration] = useState<BrowserRegistration | null>(null);
  const [extensionId, setExtensionId] = useState(""); const [browser, setBrowser] = useState("chrome");
  const [capture, setCapture] = useState(false); const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null); const lock = useRef(false);
  const prepare = async () => {
    if (lock.current || !/^[a-p]{32}$/.test(extensionId)) return;
    lock.current = true; setBusy(true); setError(null);
    try { const result = await service.browserConnection(extensionId, browser, capture); if (result) setRegistration(result); }
    catch (reason) { setError(appError(reason)); } finally { lock.current = false; setBusy(false); }
  };
  return <Dialog title="设置浏览器本机桥" close={close} busy={busy}><div className="modal-body">{error && <ErrorNotice error={error} />}
    <div className="scope-chip"><Icon name="shield" size={15} />固定范围：{scopeLabel(service.scope)}</div>
    {registration ? <><Badge tone="warning">本机注册完成 · 尚未验证读取</Badge><p>{registration.note}</p><p>回到扩展点击「请求桌面连接」，然后在本页批准具体权限。账号身份仍未核验。</p></> : <>
      <p>先安装 RecallCard 扩展，再填写浏览器扩展管理页中的准确 ID。注册前会显示系统确认。</p>
      <label className="field-label" htmlFor="browser-kind">浏览器</label><select className="field-input" id="browser-kind" value={browser} onChange={event => setBrowser(event.target.value)}><option value="chrome">Chrome</option><option value="chromium">Chromium</option><option value="brave">Brave</option></select>
      <label className="field-label" htmlFor="extension-id">RecallCard 扩展 ID</label><input id="extension-id" className="field-input" value={extensionId} onChange={event => setExtensionId(event.target.value.trim())} placeholder="浏览器扩展管理页中的 32 位 ID" autoComplete="off" spellCheck={false} />
      <label className="checkbox-row"><input type="checkbox" checked={capture} onChange={event => setCapture(event.target.checked)} />让此本机桥具备保存到当前范围的能力</label>
      <p className="context-note">这个能力上限不授予自动捕获权限。配对时仍由你选择是否读取、保存、自动准备以及向网站提供资料。当前图形化注册支持 Linux。</p>
    </>}
    </div><div className="modal-footer"><button className="button" onClick={close} disabled={busy}>{registration ? "完成" : "取消"}</button>{!registration && <button className="button primary" onClick={() => void prepare()} disabled={busy || !/^[a-p]{32}$/.test(extensionId)}>{busy ? "正在处理…" : "检查并注册本机桥"}</button>}</div>
  </Dialog>;
}
