import "./connections.css";
import { useRef, useState } from "react";
import type { ScopedService } from "../service/client";
import { appError, type AppError } from "../service/contracts";
import { scopeLabel, type BrowserRegistration } from "../service/types";
import { parseConnectionCode, type AgentClient } from "../service/connections";
import { usePollingResource, useResource } from "../service/hooks";
import { ConnectionInventoryView } from "../components/ConnectionInventory";
import { AgentSetupDialog } from "../components/AgentSetup";
import { Badge, Dialog, ErrorNotice } from "../components/common";
import { Icon } from "../components/Icon";
import { navigate } from "../App";

export function ConnectionsPage({ service }: { service: ScopedService }) {
  const [installing, setInstalling] = useState(false);
  const [agent, setAgent] = useState<AgentClient | null>(null);
  const [revision, setRevision] = useState(0);
  const [reviewRequestId, setReviewRequestId] = useState('');
  const changed = () => setRevision(value => value + 1);
  return <div className="standard-page connections-page">
    <button className="text-button connection-back" onClick={() => navigate("background")}><Icon name="back" size={15} />返回首页</button>
    <div className="page-heading"><div><h1>连接你常用的 AI</h1><p>接好一次，授权范围内自动准备背景。你可以随时检查、暂停或撤销。</p></div></div>
    <div className="scope-bar"><Icon name="shield" size={18} /><div><strong>{scopeLabel(service.scope)}</strong><span>本页的授权只适用于这个范围。</span></div><Badge>只读召回 · 分开授权保存</Badge></div>
    <section className="connection-options" aria-label="开始连接">
      <article className="connection-option"><span className="connection-logo browser-logo"><Icon name="globe" size={25} /></span><div><h2>浏览器<Badge tone="warning">开发测试版</Badge></h2><p>ChatGPT、DeepSeek · 尚未上架商店</p><span>需先手动加载测试扩展；当前还不能自动发现和配对。</span></div><button className="button" onClick={() => setInstalling(true)}>安装测试扩展<Icon name="arrow" size={14} /></button></article>
      <article className="connection-option"><span className="connection-logo agent-logo"><Icon name="code" size={25} /></span><div><h2>Codex</h2><p>项目内接入</p><span>检查并合并项目配置，让新会话按需读取本机背景。</span></div><button className="button" onClick={() => setAgent('codex')}>连接 Codex<Icon name="arrow" size={14} /></button></article>
      <article className="connection-option"><span className="connection-logo agent-logo"><Icon name="code" size={25} /></span><div><h2>Claude Code</h2><p>项目内接入</p><span>配置只读工具与会话入口，在开始、恢复后加载背景。</span></div><button className="button" onClick={() => setAgent('claude_code')}>连接 Claude Code<Icon name="arrow" size={14} /></button></article>
    </section>
    <ConnectionInventoryView service={service} refreshToken={revision} reviewRequestId={reviewRequestId} reviewOpened={() => setReviewRequestId('')} />
    <div className="connection-bottom-note"><Icon name="shield" size={16} /><p>连接状态分别记录配置、本机试读和客户端读取。没有回执时，页面会明确显示等待，不把“已配置”当作“已读到”。</p></div>
    {installing && <BrowserBridgeDialog service={service} close={() => setInstalling(false)} connected={requestId => { setInstalling(false); setReviewRequestId(requestId); changed(); }} />}
    {agent && <AgentSetupDialog service={service} client={agent} close={() => setAgent(null)} changed={changed} />}
  </div>;
}
function BrowserBridgeDialog({ service, close, connected }: { service: ScopedService; close: () => void; connected: (requestId: string) => void }) {
  const setup = useResource(() => service.browserSetupInfo(), [service]);
  const [opened, setOpened] = useState(false);
  const [copiedManager, setCopiedManager] = useState(false);
  const [registration, setRegistration] = useState<BrowserRegistration | null>(null);
  const [code, setCode] = useState('');
  const [manualId, setManualId] = useState('');
  const [browser, setBrowser] = useState('chrome');
  const [capture, setCapture] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const lock = useRef(false);
  const parsed = parseConnectionCode(code);
  const extensionId = parsed?.host_identity || manualId;
  const inventory = usePollingResource(() => service.connections(), [service], 3000);
  const matchingRequest = inventory.data?.pending_pairings.find(request => request.host_identity === extensionId && (!parsed || request.installation_id === parsed.installation_id));
  const prepare = async () => {
    if (lock.current || !/^[a-p]{32}$/.test(extensionId)) return;
    lock.current = true; setBusy(true); setError(null);
    try { const result = await service.browserConnection(extensionId, browser, capture); if (result) { setRegistration(result); inventory.refresh(); } }
    catch (reason) { setError(appError(reason)); }
    finally { lock.current = false; setBusy(false); }
  };
  const openDirectory = async () => {
    if (lock.current || !setup.data?.open_directory_available) return;
    lock.current = true; setBusy(true); setError(null);
    try { await service.openBrowserExtensionDirectory(); setOpened(true); }
    catch (reason) { setError(appError(reason)); }
    finally { lock.current = false; setBusy(false); }
  };
  const copyManager = async () => {
    if (lock.current) return;
    lock.current = true; setBusy(true); setError(null);
    try { await service.copy(browser === 'brave' ? 'brave://extensions' : 'chrome://extensions'); setCopiedManager(true); }
    catch (reason) { setError(appError(reason)); }
    finally { lock.current = false; setBusy(false); }
  };
  return <Dialog title="安装浏览器测试扩展" close={close} busy={busy} wide><div className="modal-body browser-setup"><fieldset className="browser-setup-fields" disabled={busy}>
    <ol className="setup-steps" aria-label="浏览器连接步骤"><li className={registration ? 'complete' : 'current'}><span>{registration ? <Icon name="check" size={13} /> : '1'}</span>安装与登记</li><li className={registration ? 'current' : ''}><span>2</span>请求并授权</li><li><span>3</span>首次读取</li></ol>
    {error && <ErrorNotice error={error} />}
    {registration ? <>
      <div className="setup-result"><span className="setup-result-icon written"><Icon name="check" size={22} /></span><div><h3>本机桥已登记</h3><p>现在回到浏览器的 RecallCard 扩展，点击「请求桌面连接」。</p></div></div>
      <div className="scope-chip"><Icon name="shield" size={15} />接下来一次确认：{scopeLabel(service.scope)}</div>
      {inventory.error ? <ErrorNotice error={inventory.error} retry={inventory.refresh} retryLabel="重新检测请求" /> : matchingRequest ? <div className="browser-pairing-found" role="status"><Icon name="link" size={20} /><div><strong>已收到这个扩展的连接请求</strong><p>下一步审阅网站、读取和保存权限。</p></div></div> : <div className="browser-waiting" role="status"><span className="spinner" /><div><strong>等待扩展发来请求</strong><p>此页会自动检测。若扩展仍提示本机桥不可用，请检查浏览器选择和扩展编号。</p></div></div>}
      <details className="advanced-details"><summary>登记详情与排查</summary><p>{registration.note}</p><p className="path-value">{registration.registration}</p><p>登记不是连接成功。批准后，真实读取或保存发生时才会产生回执。</p></details>
    </> : <>
      <div className="browser-release-status"><Badge tone="warning">开发测试版 · 尚未上架商店</Badge><p>当前使用发行包附带的扩展，需要手动加载并配对。商店安装后的自动发现尚未提供。</p></div>
      {setup.loading ? <p className="field-hint">正在检查安装包中的扩展…</p> : setup.error ? <ErrorNotice error={setup.error} retry={setup.refresh} /> : setup.data && <>
        {!setup.data.available && <p className="context-note">当前安装中未找到可核验的测试扩展。请使用包含 extension 文件夹的完整发行包。</p>}
        <ol className="browser-install-instructions"><li><strong>找到扩展文件夹</strong><p>{opened ? '已请求在文件管理器中打开。请选择这个文件夹，不要选择整个应用目录。' : '使用下面的按钮定位此安装包里的 extension 文件夹。'}</p>{setup.data.extension_dir && <p className="path-value">{setup.data.extension_dir}</p>}</li><li><strong>在浏览器中加载测试扩展</strong><p>打开扩展管理页，开启开发者模式，选择「加载已解压的扩展程序」，再选中上面的文件夹。</p><button className="text-button" onClick={() => void copyManager()} disabled={busy}>{copiedManager ? '管理页地址已复制' : '复制扩展管理页地址'}<Icon name="external" size={13} /></button></li><li><strong>完成开发版配对</strong><p>加载后打开 RecallCard 扩展。已有本机桥的安装可直接点击「请求桌面连接」；首次开发安装需要下方高级配对。</p></li></ol>
      </>}
      <label className="field-label" htmlFor="browser-kind">测试扩展所在浏览器</label><select className="field-input" id="browser-kind" value={browser} onChange={event => { setBrowser(event.target.value); setCopiedManager(false); }}><option value="chrome">Chrome</option><option value="chromium">Chromium</option><option value="brave">Brave</option></select>
      <details className="advanced-details browser-manual-setup"><summary>开发版手动配对（高级）</summary>
        <p>在扩展的「备用：手动填写连接编号」中点击「复制连接编号」，粘贴到这里。登记时会显示本机确认。</p>
        <label className="field-label" htmlFor="browser-connection-code">扩展连接编号</label><input id="browser-connection-code" className="field-input" value={code} onChange={event => { setCode(event.target.value.trim()); setManualId(''); }} placeholder="recallcard-connect/1:…" autoComplete="off" spellCheck={false} />
        {code && !parsed && <p className="field-hint" role="status">请粘贴完整连接编号，包含扩展 ID 和本机安装 ID。</p>}
        <label className="checkbox-row"><input type="checkbox" checked={capture} onChange={event => setCapture(event.target.checked)} />让本机桥支持保存对话（下一步再批准具体网站和范围）</label>
        <details className="advanced-details"><summary>连接编号不可用？填写扩展 ID</summary><label className="field-label" htmlFor="extension-id">RecallCard 扩展 ID</label><input id="extension-id" className="field-input" value={manualId} onChange={event => { setManualId(event.target.value.trim()); setCode(''); }} placeholder="浏览器扩展管理页中的 32 位 ID" autoComplete="off" spellCheck={false} /></details>
        <p className="field-hint">本机桥图形化登记当前支持 Linux。编号只绑定扩展安装，不核验网站登录账号。</p>
        <button className="button" onClick={() => void prepare()} disabled={busy || !/^[a-p]{32}$/.test(extensionId)}>{busy ? '正在登记…' : '登记本机桥并继续'}</button>
      </details>

    </>}
  </fieldset></div><div className="modal-footer"><button className="button" onClick={() => { if (registration) { setRegistration(null); setError(null); } else close(); }} disabled={busy}>{registration ? '返回' : '取消'}</button>{registration ? <button className="button primary" onClick={matchingRequest && !inventory.error ? () => connected(matchingRequest.request_id) : inventory.refresh} disabled={busy || inventory.loading}>{matchingRequest && !inventory.error ? '审阅这个连接' : '检测连接请求'}</button> : <button className="button primary" onClick={() => void openDirectory()} disabled={busy || !setup.data?.open_directory_available}>{busy ? '正在打开…' : '打开测试扩展文件夹'}</button>}</div></Dialog>;
}
