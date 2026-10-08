import { useEffect, useRef, useState } from 'react';
import type { ScopedService } from '../service/client';
import { appError, type AppError } from '../service/contracts';
import { connectionLabel, connectionName, sameConnectionGrant, healthLabel, healthTone, parseConnectionCode, type ConnectionEntry, type ConnectionGrant, type PairingRequest } from '../service/connections';
import { usePollingResource } from '../service/hooks';
import { dateLabel, platformLabel, scopeLabel } from '../service/types';
import { Badge, Dialog, ErrorNotice, Loading } from './common';
import { Icon } from './Icon';

type Editing = { kind: 'pair'; request: PairingRequest; previous?: ConnectionEntry } | { kind: 'entry'; entry: ConnectionEntry } | { kind: 'manual' };
export function ConnectionInventoryView({ service, refreshToken = 0, reviewRequestId = '', reviewOpened }: { service: ScopedService; refreshToken?: number; reviewRequestId?: string; reviewOpened?: () => void }) {
  const inventory = usePollingResource(() => service.connections(), [service, refreshToken]);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [revoking, setRevoking] = useState<ConnectionEntry | null>(null);
  const [busyId, setBusyId] = useState('');
  const [error, setError] = useState<AppError | null>(null);
  const lock = useRef(false);
  useEffect(() => {
    if (!reviewRequestId) return;
    let current = true;
    service.connections().then(result => {
      if (!current) return;
      inventory.setData(result);
      const request = result.pending_pairings.find(item => item.request_id === reviewRequestId);
      if (request) setEditing({ kind: 'pair', request, previous: result.entries.find(entry => entry.grant.host_identity === request.host_identity && entry.grant.installation_id === request.installation_id && entry.grant.platform === request.platform) });
      else setError(appError('这个连接请求已过期或已经处理，请在对应扩展重新发起。'));
      reviewOpened?.();
    }).catch(reason => { if (current) { setError(appError(reason)); reviewOpened?.(); } });
    return () => { current = false; };
  }, [service, reviewRequestId]);

  const updated = (entry: ConnectionEntry, requestId?: string) => {
    inventory.setData(previous => previous ? { ...previous, entries: [...previous.entries.filter(row => row.id !== entry.id), entry], pending_pairings: previous.pending_pairings.filter(request => request.request_id !== requestId) } : previous);
    inventory.refresh();
  };
  const toggle = async (entry: ConnectionEntry, field: 'auto_capture' | 'auto_recall') => {
    if (lock.current) return; lock.current = true; setBusyId(entry.id); setError(null);
    try { updated(await service.configureConnection({ ...entry.grant, [field]: !entry.grant[field] }, entry.permission_revision)); }
    catch (reason) { setError(appError(reason)); inventory.refresh(); }
    finally { lock.current = false; setBusyId(''); }
  };
  const revoke = async () => {
    if (!revoking || lock.current) return; lock.current = true; setBusyId(revoking.id); setError(null);
    try { updated(await service.revokeConnection(revoking.id, revoking.permission_revision)); setRevoking(null); }
    catch (reason) { setError(appError(reason)); }
    finally { lock.current = false; setBusyId(''); }
  };
  return <section className="connection-inventory" aria-label="连接授权与实际状态">
    <div className="section-heading"><h2>连接授权与实际状态</h2><button className="text-button" onClick={inventory.refresh}><Icon name="refresh" size={13} />刷新</button></div>
    {inventory.loading ? <Loading label="正在读取本机连接" /> : inventory.error ? <ErrorNotice error={inventory.error} retry={inventory.refresh} /> : inventory.data && <>
      {inventory.data.pending_pairings.length > 0 && <div className="pairing-requests" aria-label="待批准的连接请求">{inventory.data.pending_pairings.map(request => <div className="pairing-request" key={request.request_id}><Icon name="link" size={18} /><div><strong>{platformLabel(request.platform)} 请求连接</strong><p>浏览器安装 {request.installation_id.slice(0, 8)} · 到期 {dateLabel(request.expires_at, true)}</p><span>尚未授予读取或保存资料的权限</span></div><button className="button primary" onClick={() => setEditing({ kind: 'pair', request, previous: inventory.data!.entries.find(entry => entry.grant.host_identity === request.host_identity && entry.grant.installation_id === request.installation_id && entry.grant.platform === request.platform) })}>审阅连接请求</button></div>)}</div>}
      {inventory.data.entries.length > 0 ? <div className="registered-connections">{inventory.data.entries.map(entry => <div className="registered-connection" key={entry.id} data-connection-state={entry.state}>
        <div className="registered-heading"><Icon name={entry.grant.client_kind === 'browser' ? 'globe' : 'code'} size={20} /><h3>{connectionName(entry)}</h3><span>{entry.grant.client_kind === 'browser' ? `安装 ${entry.grant.installation_id?.slice(0, 8) || '未核验'}` : entry.grant.installation_id ? `项目 ${entry.grant.installation_id.slice(0, 8)}` : '本地 Agent'}</span>{entry.grant.client_kind === 'browser' && <Badge tone={entry.state === 'failed' ? 'warning' : entry.state === 'read_succeeded' ? 'success' : 'muted'}>{connectionLabel(entry.state)}</Badge>}</div>
        <div className="connection-permissions"><span>读取：{entry.grant.recall_scopes.length ? entry.grant.recall_scopes.map(scopeLabel).join('、') : '未授权'}</span><span>保存：{entry.grant.capture_scopes.length ? entry.grant.capture_scopes.map(scopeLabel).join('、') : '未授权'}</span></div>
        <dl className="connection-observations"><div><dt>最近本机握手</dt><dd>{entry.last_handshake_at ? dateLabel(entry.last_handshake_at, true) : '尚无回执'}</dd></div><div><dt>最近背景读取</dt><dd>{entry.last_bootstrap_at ? dateLabel(entry.last_bootstrap_at, true) : '尚无回执'}</dd></div><div><dt>最近按需读取</dt><dd>{entry.last_read_at ? dateLabel(entry.last_read_at, true) : '尚无回执'}</dd></div><div><dt>最近原话保存</dt><dd>{entry.last_capture_at ? dateLabel(entry.last_capture_at, true) : '尚无回执'}</dd></div></dl>
        {entry.last_error && <p className="connection-operation-error" role="status">{entry.last_error}</p>}
        {entry.grant.client_kind !== 'browser' && <ConnectionHealthView service={service} entry={entry} />}
        <div className="connection-controls">{!entry.revoked && <>{entry.grant.recall_scopes.includes(service.scope) && <button className="button compact" disabled={busyId === entry.id || !entry.grant.provider_disclosure} onClick={() => void toggle(entry, 'auto_recall')}><Icon name={entry.grant.auto_recall ? 'pause' : 'play'} size={13} />{entry.grant.auto_recall ? '暂停自动准备' : '开启自动准备'}</button>}{entry.grant.capture_scopes.includes(service.scope) && <button className="button compact" disabled={busyId === entry.id} onClick={() => void toggle(entry, 'auto_capture')}><Icon name={entry.grant.auto_capture ? 'pause' : 'play'} size={13} />{entry.grant.auto_capture ? '暂停自动保存' : '开启自动保存'}</button>}</>}<button className="text-button" disabled={busyId === entry.id} onClick={() => setEditing({ kind: 'entry', entry })}>{entry.revoked ? '重新授权' : '检查权限'}</button>{!entry.revoked && <button className="text-button" disabled={busyId === entry.id} onClick={() => setRevoking(entry)}>撤销访问</button>}</div>
      </div>)}</div> : <p className="connection-empty">尚无此范围的授权。打开扩展，选择「请求桌面连接」，申请会出现在这里。</p>}
      <p className="coverage-note">{inventory.data.identity_notice}读取、捕获和记忆整理分别记录，不以一个「已同步」状态替代。</p>
      <details className="manual-connection"><summary>无法请求连接？</summary><p>仅在自动请求不可用时，从扩展复制连接编号，再在此审阅范围。</p><button className="button compact" onClick={() => setEditing({ kind: 'manual' })}>手动连接浏览器</button></details>
    </>}
    {error && <ErrorNotice error={error} />}
    {editing && <ConnectionPermissionDialog service={service} editing={editing} close={() => setEditing(null)} saved={(entry, requestId) => { updated(entry, requestId); setEditing(null); }} />}
    {revoking && <Dialog title="撤销这个连接的访问？" close={() => setRevoking(null)} busy={!!busyId}><div className="modal-body"><p>将停止此连接的读取、自动准备和保存。已经保存到本机的来源会保留。</p><p>已写入网站的草稿不会自动撤回。若需要，请同时在原网页移除它。</p>{error && <ErrorNotice error={error} />}</div><div className="modal-footer"><button className="button" onClick={() => setRevoking(null)} disabled={!!busyId}>取消</button><button className="button danger" onClick={() => void revoke()} disabled={!!busyId}>{busyId ? '正在撤销…' : '确认撤销访问'}</button></div></Dialog>}
  </section>;
}

function ConnectionPermissionDialog({ service, editing, close, saved }: { service: ScopedService; editing: Editing; close: () => void; saved: (entry: ConnectionEntry, requestId?: string) => void }) {
  const existing = editing.kind === 'entry' ? editing.entry : editing.kind === 'pair' ? editing.previous : undefined;
  const request = editing.kind === 'pair' ? editing.request : undefined;
  const initial = existing?.grant;
  const [code, setCode] = useState('');
  const [platform, setPlatform] = useState<ConnectionGrant['platform']>(request?.platform || initial?.platform || 'chatgpt');
  const [read, setRead] = useState(initial?.recall_scopes.includes(service.scope) || false);
  const [capture, setCapture] = useState(initial?.capture_scopes.includes(service.scope) || false);
  const [autoRead, setAutoRead] = useState(!existing?.revoked && !!initial?.auto_recall);
  const [autoCapture, setAutoCapture] = useState(!existing?.revoked && !!initial?.auto_capture);
  const [disclosed, setDisclosed] = useState(!existing?.revoked && !!initial?.provider_disclosure);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const parsed = editing.kind === 'manual' ? parseConnectionCode(code) : null;
  const kind = initial?.client_kind || 'browser';
  const host = request?.host_identity || initial?.host_identity || parsed?.host_identity || '';
  const installation = request?.installation_id || initial?.installation_id || parsed?.installation_id || null;
  const canRead = !request || request.recall_scope_cap.includes(service.scope);
  const canCapture = kind === 'browser' && (!request || request.capture_scope_cap.includes(service.scope));
  const saveLock = useRef(false);
  const candidateGrant: ConnectionGrant = { client_kind: kind, host_identity: host, installation_id: installation, platform, recall_scopes: read ? [service.scope] : [], capture_scopes: capture ? [service.scope] : [], provider_disclosure: read && disclosed, auto_recall: read && autoRead, auto_capture: capture && autoCapture };
  const permissionChanged = editing.kind !== 'entry' || !existing || existing.revoked || !sameConnectionGrant(candidateGrant, existing.grant);
  const ready = !!host && (kind !== 'browser' || !!installation) && (read || capture) && (!read || disclosed) && permissionChanged;
  const resetConsent = () => { setRead(false); setCapture(false); setAutoRead(false); setAutoCapture(false); setDisclosed(false); };
  const save = async () => {
    if (!ready || saveLock.current) return; saveLock.current = true; setBusy(true); setError(null);
    const grant = candidateGrant;
    try { const result = request ? await service.approvePairing(request.request_id, grant, existing?.permission_revision ?? null) : await service.configureConnection(grant, existing?.permission_revision ?? null); saved(result, request?.request_id); }
    catch (reason) { setError(appError(reason)); } finally { saveLock.current = false; setBusy(false); }
  };
  return <Dialog title={request ? '审阅连接请求' : editing.kind === 'manual' ? '手动连接浏览器' : '检查连接权限'} close={close} busy={busy} wide><div className="modal-body permission-dialog"><fieldset className="permission-fields" disabled={busy}>
    {error && <ErrorNotice error={error} />}
    {editing.kind === 'manual' ? <><label className="field-label" htmlFor="connection-code">扩展弹窗中的连接编号</label><input id="connection-code" className="field-input" autoComplete="off" spellCheck={false} value={code} onChange={event => { setCode(event.target.value); resetConsent(); }} placeholder="recallcard-connect/1:…" /><label className="field-label" htmlFor="connection-site">允许接入的网站</label><select id="connection-site" className="field-input" value={platform} onChange={event => { setPlatform(event.target.value as ConnectionGrant['platform']); resetConsent(); }}><option value="chatgpt">ChatGPT</option><option value="deepseek">DeepSeek</option></select></> : <div className="model-destination"><Icon name={kind === 'browser' ? 'globe' : 'code'} size={21} /><div><strong>{platform === 'claude_code' ? 'Claude Code' : platformLabel(platform)}</strong><span>{kind === 'browser' ? `浏览器安装 ${installation}` : installation ? `项目标识 ${installation}` : '本地 Agent'}</span><span>本次请求的客户端与网站固定，不能替换为其他接收方。</span></div></div>}
    <div className="scope-chip"><Icon name="shield" size={15} />当前范围：{scopeLabel(service.scope)}</div>
    <label className="checkbox-row"><input type="checkbox" disabled={!canRead} checked={read} onChange={event => { setRead(event.target.checked); if (!event.target.checked) setAutoRead(false); }} />允许读取此范围的背景、记忆与原始来源</label>
    {read && <label className="checkbox-row sub-permission"><input type="checkbox" checked={autoRead} onChange={event => setAutoRead(event.target.checked)} />{kind !== 'browser' ? '允许自动加载背景，并执行只读查询' : '自动准备背景，并处理已授权的只读查询'}</label>}
    {kind === 'browser' && <><label className="checkbox-row"><input type="checkbox" disabled={!canCapture} checked={capture} onChange={event => { setCapture(event.target.checked); if (!event.target.checked) setAutoCapture(false); }} />允许将这个网站的对话保存到此范围</label>{capture && <label className="checkbox-row sub-permission"><input type="checkbox" checked={autoCapture} onChange={event => setAutoCapture(event.target.checked)} />在对话稳定结束后自动保存</label>}</>}
    {read && <div className="provider-disclosure"><strong>接收方：{kind !== 'browser' ? `${platform === 'claude_code' ? 'Claude Code' : 'Codex'} 使用的模型服务` : platformLabel(platform)}</strong><p>{kind === 'browser' ? '即使最后发送仍由你点击，资料进入网页可见草稿后，该网站就可能读取这些内容。' : '读取结果会进入这个宿主的模型上下文，并可能提供给它配置的模型服务。'}</p><label className="checkbox-row"><input type="checkbox" checked={disclosed} onChange={event => setDisclosed(event.target.checked)} />我同意将上述范围的资料提供给这个接收方</label></div>}
    <p className="field-hint">{kind === 'browser' ? '浏览器绑定扩展安装与网站，不等于核验登录账号。账号切换或共享设备时，请检查并暂停不再适用的授权。' : '请核对本地 Agent 实际配置的模型接收方。切换提供方后，应重新审阅授权；配置完成不代表已实际调用。'}</p>
    {editing.kind === 'entry' && !permissionChanged && <p className="field-hint">当前权限没有变化，关闭即可。读取回执与现有接入会保持有效。</p>}
    {!canRead || !canCapture && kind === 'browser' ? <p className="field-hint">部分权限不在当前本机组件的能力范围内，不能在此扩大。</p> : null}
  </fieldset></div><div className="modal-footer"><button className="button" onClick={close} disabled={busy}>取消</button><button className="button primary" onClick={() => void save()} disabled={!ready || busy}>{busy ? '正在保存…' : request ? '批准此连接' : '保存这次授权'}</button></div></Dialog>;
}

function ConnectionHealthView({ service, entry }: { service: ScopedService; entry: ConnectionEntry }) {
  const health = usePollingResource(() => service.connectionHealth(entry.id), [service, entry.id, entry.permission_revision]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const lock = useRef(false);
  const verify = async () => {
    if (lock.current) return;
    lock.current = true; setBusy(true); setError(null);
    try { health.setData(await service.verifyConnection(entry.id)); health.refresh(); }
    catch (reason) { setError(appError(reason)); }
    finally { lock.current = false; setBusy(false); }
  };
  return <div className="connection-health-detail">
    {health.error || error ? <ErrorNotice error={(health.error || error)!} retry={() => { setError(null); health.refresh(); }} retryLabel="重新检测" /> : health.loading || health.data?.permission_revision !== entry.permission_revision ? <span className="muted">正在检测项目接入…</span> : health.data && <><Badge tone={healthTone(health.data)}>{healthLabel(health.data)}</Badge><span>{health.data.message}</span></>}
    {!entry.revoked && <button className="text-button" disabled={busy || !entry.grant.auto_recall || !entry.grant.provider_disclosure} onClick={() => void verify()}>{busy ? '正在试读…' : '本机试读'}</button>}
  </div>;
}
