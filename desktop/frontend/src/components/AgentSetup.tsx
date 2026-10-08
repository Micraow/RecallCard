import { useEffect, useRef, useState } from 'react';
import type { ScopedService } from '../service/client';
import { appError, type AppError } from '../service/contracts';
import { healthLabel, healthTone, validateSetupPlan, type AgentClient, type ConnectionHealth, type ConnectionSetupPlan, type ConnectionSetupResult } from '../service/connections';
import { dateLabel, scopeLabel } from '../service/types';
import { Badge, Dialog, ErrorNotice } from './common';
import { Icon } from './Icon';

export function AgentSetupDialog({ service, client, close, changed }: { service: ScopedService; client: AgentClient; close: () => void; changed: () => void }) {
  const name = client === 'codex' ? 'Codex' : 'Claude Code';
  const recovered = service.connectionSetupState(client);
  const [project, setProject] = useState(recovered?.plan.project_dir || '');
  const [plan, setPlan] = useState<ConnectionSetupPlan | null>(recovered?.plan || null);
  const [result, setResult] = useState<ConnectionSetupResult | null>(recovered?.result || null);
  const [health, setHealth] = useState<ConnectionHealth | null>(recovered?.result?.health || null);
  const [disclosed, setDisclosed] = useState(false);
  const [busy, setBusy] = useState(service.connectionSetupPending ? '已有连接正在安装…' : '');
  const [error, setError] = useState<AppError | null>(recovered?.error ? appError(recovered.error) : null);
  const [pollError, setPollError] = useState<AppError | null>(null);
  const [attempted, setAttempted] = useState(!!recovered);
  const lock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const pending = service.connectionSetupPending;
    if (!pending) return;
    let current = true;
    pending.then(installed => { if (current) { if (service.connectionSetupState(client)) { setResult(installed); setHealth(installed.health); setAttempted(true); if (installed.verification_error) setError(appError(installed.verification_error)); } changed(); } }, reason => { if (current && service.connectionSetupState(client)) setError(appError(reason)); }).finally(() => { if (current) setBusy(''); });
    return () => { current = false; };
  }, [service]);
  const stage = attempted ? 3 : plan ? 2 : 1;
  const chooseProject = async () => {
    if (lock.current) return;
    lock.current = true; setBusy('正在选择项目…'); setError(null);
    try {
      const selected = await service.chooseConnectionProject();
      if (!selected || !mounted.current) return;
      setProject(selected); setDisclosed(false); setBusy('正在检查已有配置…');
      const preview = validateSetupPlan(await service.connectionSetupPlan(client, selected), client, service.scope, selected);
      if (mounted.current) setPlan(preview);
    } catch (reason) { if (mounted.current) setError(appError(reason)); }
    finally { lock.current = false; if (mounted.current) setBusy(''); }
  };
  const apply = async () => {
    if (lock.current || !plan || !disclosed || attempted) return;
    lock.current = true; setBusy('正在安装并试读…'); setError(null); setAttempted(true);
    try {
      const installed = await service.applyConnectionSetup(plan);
      if (mounted.current) { setResult(installed); setHealth(installed.health); if (installed.verification_error) setError(appError(installed.verification_error)); changed(); }
    } catch (reason) { if (mounted.current) { setError(appError(reason)); changed(); } }
    finally { lock.current = false; if (mounted.current) setBusy(''); }
  };
  const check = async (verify = false) => {
    if (lock.current || !plan) return;
    lock.current = true; setBusy(verify ? '正在执行本机试读…' : '正在检测安装状态…'); setError(null); setPollError(null);
    try { const checked = await (verify ? service.verifyConnection(plan.connection_id) : service.connectionHealth(plan.connection_id)); if (mounted.current) { setHealth(checked); service.recordConnectionHealth(client, checked); changed(); } }
    catch (reason) { if (mounted.current) setError(appError(reason)); }
    finally { lock.current = false; if (mounted.current) setBusy(''); }
  };
  useEffect(() => {
    if (!result || !plan) return;
    let current = true;
    const timer = setInterval(() => {
      if (lock.current || document.visibilityState === 'hidden') return;
      service.connectionHealth(plan.connection_id).then(value => { if (current) { setHealth(value); setPollError(null); service.recordConnectionHealth(client, value); if (value.readiness === 'read_verified') setError(null); } }).catch(reason => { if (current) setPollError(appError(reason)); });
    }, 4000);
    return () => { current = false; clearInterval(timer); };
  }, [service, result, plan]);
  const finish = () => { service.dismissConnectionSetup(client); close(); };
  return <Dialog title={`连接 ${name}`} close={finish} busy={!!busy} wide>
    <div className="modal-body agent-setup">
      <ol className="setup-steps" aria-label="连接步骤">{['选择项目', '确认范围', '安装与试读'].map((step, index) => <li key={step} className={stage === index + 1 ? 'current' : stage > index + 1 ? 'complete' : ''} aria-current={stage === index + 1 ? 'step' : undefined}><span>{stage > index + 1 ? <Icon name="check" size={13} /> : index + 1}</span>{step}</li>)}</ol>
      {error && <ErrorNotice error={error} />}
      {pollError && <ErrorNotice error={pollError} />}
      {stage === 1 && <>
        <div className="setup-intro"><span className="connection-logo agent-logo"><Icon name="code" size={27} /></span><div><h3>在这个项目里自动接上背景</h3><p>选择你使用 {name} 的项目文件夹。接着会检查已有设置，展示需要写入的文件。</p></div></div>
        <div className="setup-benefits"><div><Icon name="book" size={17} /><span>启动时读取稳定背景，细节按需检索</span></div><div><Icon name="shield" size={17} /><span>只读取一次批准的范围，可随时暂停或撤销</span></div><div><Icon name="folder" size={17} /><span>合并项目配置，保留已有内容</span></div></div>
        {project && <p className="setup-project-path">上次选择：{project}</p>}
      </>}
      {stage === 2 && plan && <>
        <div className="setup-scope"><Icon name="shield" size={20} /><div><strong>{scopeLabel(service.scope)}</strong><p>自动读取此范围的背景、记忆与原始来源</p></div><Badge>只读</Badge></div>
        <div className="setup-project"><span>接收方</span><strong>{name} 及它实际配置的模型服务</strong><span>项目</span><strong className="path-value">{plan.project_dir}</strong></div>
        <p className="setup-disclosure">读取结果会进入 {name} 的模型上下文。确认一次后，后续启动和查询会沿用此范围，无需每轮选择上下文。</p>
        <section className="setup-files"><h3>将合并这些文件</h3>{plan.files.map(file => <details key={file.path}><summary><Icon name="file" size={15} /><span className="path-value">{file.path}</span><Badge>{file.changed ? file.before_digest ? '合并' : '新建' : '无需修改'}</Badge></summary><pre>{file.managed_addition}</pre>{file.backup_path && <p className="field-hint path-value">原文件备份：{file.backup_path}</p>}</details>)}</section>
        {plan.notices.length > 0 && <div className="setup-notices">{plan.notices.map((notice, index) => <p key={index}>{notice}</p>)}</div>}
        <label className="checkbox-row setup-consent"><input type="checkbox" checked={disclosed} onChange={event => setDisclosed(event.target.checked)} />我允许此项目的 {name} 自动读取上述范围，并向其模型服务提供检索结果</label>
        <p className="field-hint">安装后会执行一次本机只读试验。它不能证明 {name} 已实际调用，客户端读取会单独记录。</p>
      </>}
      {stage === 3 && <>
        {busy && <div className="setup-working" role="status"><span className="spinner" />{busy}</div>}
        <div className="setup-result"><span className={`setup-result-icon ${result ? 'written' : ''}`}><Icon name={result ? 'check' : 'file'} size={22} /></span><div><h3>{result ? '项目接入配置已写入' : '正在确认安装结果'}</h3><p>{result ? '范围授权已经保存。以后在这个项目里沿用这次设置。' : '尚未确认写入结果。检测状态后再决定下一步，避免重复安装。'}</p></div></div>
        {health && !error && !pollError && <section className="setup-health" data-verification-scope={health.verification_scope}><Badge tone={healthTone(health)}>{healthLabel(health)}</Badge><p>{health.message}</p><span>检查于 {dateLabel(health.checked_at, true)}</span></section>}
        {result && <div className="setup-next"><h3>下一步：在此项目开始新会话</h3><p>重新打开 {name} 的项目会话，让它加载新配置。在连接页可以查看首次客户端读取回执。</p><p className="field-hint">本机试读成功与客户端真正读取分开显示；不据此声称模型已理解全部背景。</p></div>}
        {!!result?.notices.length && <div className="setup-notices">{result.notices.map((notice, index) => <p key={index}>{notice}</p>)}</div>}
        {result && <details className="advanced-details"><summary>查看已写入文件</summary>{result.paths.map(path => <p className="path-value" key={path}>{path}</p>)}</details>}
      </>}
    </div>
    <div className="modal-footer setup-footer">
      <button className="button" disabled={!!busy} onClick={() => { if (stage === 2) { setPlan(null); setDisclosed(false); setError(null); } else finish(); }}>{stage === 2 ? '返回' : stage === 3 ? '关闭' : '取消'}</button>
      {stage === 1 && <button className="button primary" disabled={!!busy} onClick={() => void chooseProject()}>{busy || '选择项目文件夹'}{!busy && <Icon name="folder" size={15} />}</button>}
      {stage === 2 && <button className="button primary" disabled={!!busy || !disclosed} onClick={() => void apply()}>授权并安装连接</button>}
      {stage === 3 && <button className="button primary" disabled={!!busy} onClick={() => void check(!!result && (!health || health.readiness === 'local_unavailable' || !!error))}>{busy || (result && (!health || health.readiness === 'local_unavailable' || !!error) ? '重新试读' : '检测连接状态')}</button>}
    </div>
  </Dialog>;
}
