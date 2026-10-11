import { useEffect, useRef, useState } from 'react';
import type { ScopedService } from '../service/client';
import type { ConnectionEntry } from '../service/connections';
import { appError, type AppError } from '../service/contracts';
import { scopeLabel, type BrowserRegistration } from '../service/types';
import { Dialog, ErrorNotice } from './common';

export function BrowserReconnectDialog({service, entry, close, changed}: {
  service: ScopedService; entry: ConnectionEntry; close: () => void; changed: () => void;
}) {
  // 授权记录不含浏览器型号，不能从网站名称猜测，更不能静默登记到 Chrome。
  const recovered = service.browserReconnectState(entry.id);
  const [browser, setBrowser] = useState(recovered?.browser || '');
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(!!service.browserReconnectPending);
  const [error, setError] = useState<AppError | null>(recovered?.error ? appError(recovered.error) : null);
  const [cancelled, setCancelled] = useState(!!recovered && !recovered.pending && !recovered.result && !recovered.error);
  const [result, setResult] = useState<BrowserRegistration | null>(recovered?.result || null);
  const lock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {mounted.current=true;return () => {mounted.current=false;};}, []);
  useEffect(() => {
    const pending=service.browserReconnectPending;
    if (!pending) return;
    let current=true;
    pending.then(updated => {if(current && service.browserReconnectState(entry.id)) {setResult(updated);setCancelled(!updated);changed();}},reason => {if(current && service.browserReconnectState(entry.id))setError(appError(reason));}).finally(() => {if(current)setBusy(false);});
    return () => {current=false;};
  }, [service, entry.id]);
  const finish = () => {service.dismissBrowserReconnect(entry.id);close();};
  const apply = async () => {
    if (lock.current || !confirmed || !browser || result) return;
    lock.current = true; setBusy(true); setError(null); setCancelled(false);
    try {
      const updated = await service.refreshBrowserConnection(entry, browser);
      if (!mounted.current) return;
      if (updated) { setResult(updated); changed(); }
      else { setCancelled(true); setConfirmed(false); }
    } catch (reason) { if(mounted.current) {setError(appError(reason));setConfirmed(false);} }
    finally { lock.current = false; if(mounted.current)setBusy(false); }
  };
  return <Dialog title="更新已有浏览器的本机桥" close={finish} busy={busy} wide>
    <div className="modal-body browser-setup">
      {error && <ErrorNotice error={error}/>}
      {busy && <p role="status">已有本机桥更新正在进行，请等待系统确认结果。离开页面不会再次登记。</p>}
      {result ? <>
        <h3>本机桥文件已更新，等待扩展重新连接</h3>
        <p role="status">登记成功只说明配置已写入，还没有核验扩展是否加载了本次语义设置。</p>
        <ol><li>回到对应浏览器的 RecallCard 扩展，点击「重新连接当前对话」，再点击「检查本机连接」；若仍显示旧状态，在扩展管理页重新加载这个扩展并刷新原网页。</li><li>再次打开浮窗执行目录或检索读取，检查它显示的语义状态。目录/词法可读不代表 embedding 已就绪。</li><li>若提示配置过期，确认更新的是同一浏览器、资料库与范围；若提示凭据或预算问题，回到设置处理相应提示。</li></ol>
        <p>原连接的权限、暂停选择和历史回执均保留。没有新的请求时，旧“最近读取成功”不能作为本次重新连接的验收。</p>
        <details><summary>更新详情</summary><p>{result.note}</p><p className="path-value">{result.registration}</p></details>
      </> : <>
        <p>用于云/本机语义配置改变后重新生成连接文件。无需重新寻找扩展 ID 或重复批准读取范围。</p>
        <dl><div><dt>扩展 ID</dt><dd className="path-value">{entry.grant.host_identity}</dd></div><div><dt>本次本机桥范围</dt><dd>{scopeLabel(service.scope)}</dd></div><div><dt>保存能力</dt><dd>{entry.grant.capture_scopes.includes(service.scope) ? '保留此范围已有保存能力' : '不启用保存能力'}</dd></div></dl>
        <p>这不会修改连接授权或恢复已暂停的自动读取/保存。登记仍需下一步系统确认。</p>
        <p className="context-note">同一浏览器共用一个 RecallCard 本机桥；本次更新会把它指向此资料库和当前范围。若还使用其他资料库或范围，请先确认当前选择。</p>
        <label className="field-label" htmlFor="reconnect-browser">这个扩展所在的浏览器</label>
        <select id="reconnect-browser" className="field-input" disabled={busy} value={browser} onChange={event => {setBrowser(event.target.value);setConfirmed(false);}}><option value="">请选择，不自动猜测</option><option value="chrome">Chrome</option><option value="chromium">Chromium</option><option value="brave">Brave</option></select>
        <label className="checkbox-label"><input type="checkbox" disabled={busy || !browser} checked={confirmed} onChange={event => setConfirmed(event.target.checked)}/>我确认更新上述浏览器的本机桥，并保留现有连接权限</label>
        {cancelled && <p role="status">已取消系统登记，未确认任何文件更新。需要继续时请重新确认。</p>}
      </>}
    </div>
    <div className="modal-footer"><button className="button" disabled={busy} onClick={finish}>{result ? '关闭' : '取消'}</button>{!result && <button className="button primary" disabled={busy || !confirmed || !browser} onClick={() => void apply()}>{busy ? '正在确认并更新' : '继续系统确认并更新'}</button>}</div>
  </Dialog>;
}
