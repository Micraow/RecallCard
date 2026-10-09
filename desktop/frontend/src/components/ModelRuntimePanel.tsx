import { useEffect, useRef, useState } from 'react';
import type { ScopedService } from '../service/client';
import { appError, type AppError } from '../service/contracts';
import { usePollingResource } from '../service/hooks';
import type { ModelSetupRequest, ModelSetupSnapshot } from '../service/setup';
import { dateLabel } from '../service/types';
import { canRetryMemoryJob, runtimeLabel, type MemoryJob } from '../service/runtime';
import { Badge, Dialog, ErrorNotice, Loading } from './common';
import { Icon } from './Icon';
import { ModelSetupDialog } from './ModelSetup';

export function ModelRuntimePanel({ service, settings = false, refreshToken = 0 }: { service: ScopedService; settings?: boolean; refreshToken?: number }) {
  const status = usePollingResource(() => service.setupStatus(), [service, refreshToken]);
  const [editing, setEditing] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [stopRequested, setStopRequested] = useState(false);
  const [operationError, setOperationError] = useState<AppError | null>(service.modelLastError ? appError(service.modelLastError) : null);
  const [pending, setPending] = useState(!!service.modelPending);
  useEffect(() => {
    if (!service.modelPending) return;
    let current = true;
    service.modelPending.then(result => { if (current) { status.setData(result); setPending(false); if (result.service_error) setOperationError(result.service_error); status.refresh(); } }, reason => { if (current) { setPending(false); setOperationError(appError(reason)); status.refresh(); } });
    return () => { current = false; };
  }, [service]);
  const submit = async (request: ModelSetupRequest): Promise<ModelSetupSnapshot> => {
    setPending(true); setOperationError(null);
    try {
      const result = await service.configureModel(request);
      status.setData(result); status.refresh();
      if (result.service_error) throw result.service_error;
      setEditing(false); return result;
    } catch (reason) { setOperationError(appError(reason)); throw reason; }
    finally { setPending(false); }
  };
  const stop = async () => {
    setStopRequested(true); setOperationError(null); setStopping(false);
    try { const result = await service.stopService(); status.setData(previous => previous ? { ...previous, service: result } : previous); if (!result.running) setStopRequested(false); status.refresh(); }
    catch (reason) { setStopRequested(false); setOperationError(appError(reason)); }
  };
  useEffect(() => { if (status.data?.service && !status.data.service.running) setStopRequested(false); }, [status.data?.service?.running]);
  if (status.loading) return <Loading label="正在核对本机服务与模型状态" />;
  if (status.error) return <ErrorNotice error={status.error} retry={status.refresh} />;
  const snapshot = status.data!;
  const runtime = snapshot.runtime;
  const running = snapshot.service?.running === true;
  const requestActive = pending || !!service.modelPending;
  const initial = service.modelDraft ? { ...snapshot, runtime: { ...runtime, config: service.modelDraft } } : snapshot;
  return <section className={`model-runtime-panel ${settings ? 'settings-model-panel' : ''}`} aria-label="后台整理与本机服务">
    <div className="runtime-summary"><span className="runtime-icon"><Icon name="moon" size={22} /></span><div className="runtime-summary-main"><div className="runtime-title"><h2>后台记忆整理</h2><Badge tone={runtime.state === 'failed' || runtime.state === 'needs_input' ? 'warning' : 'muted'}>{runtimeLabel(runtime.state)}</Badge></div><p>{runtime.message}</p><div className="runtime-facts"><span>{running ? '本机服务运行中' : '本机服务未运行'}</span><span>{snapshot.credential.present ? '已提供凭据' : '尚无可用凭据'}</span><span>{runtime.jobs.some(job => job.state === 'completed' && job.progress.provider_calls > 0) ? '已有整理完成记录' : '模型响应尚未验证'}</span></div></div><button className="button" disabled={requestActive || stopRequested} onClick={() => setEditing(true)}>{runtime.config.provider ? '检查模型设置' : '连接模型'}</button></div>
    {requestActive && <p className="context-note" role="status"><span className="spinner" />此资料库的模型设置正在更新，请等待实际结果。不会重复提交密钥。</p>}
    {stopRequested && <p className="context-note" role="status">已请求停止，正在等待安全边界；尚未确认服务退出。</p>}
    {operationError && <ErrorNotice error={operationError} />}
    {snapshot.service_error && !operationError && <ErrorNotice error={snapshot.service_error} />}
    {snapshot.service?.error && <ErrorNotice error={snapshot.service.error} />}
    {settings && <div className="model-setting-details"><dl><div><dt>模型目的地</dt><dd>{runtime.config.provider ? `${runtime.config.provider.model} · ${runtime.config.provider.endpoint}` : '尚未配置，不会默认把资料发送给供应商'}</dd></div><div><dt>密钥方式</dt><dd>{snapshot.credential.message}</dd></div><div><dt>实际服务心跳</dt><dd>{running && snapshot.service?.heartbeat_at ? dateLabel(snapshot.service.heartbeat_at, true) : '没有运行中的服务回执'}</dd></div><div><dt>本月资源使用</dt><dd>{runtime.usage.reserved_calls} / {runtime.config.budget.max_calls_per_month} 次预留请求 · {runtime.usage.reserved_tokens.toLocaleString()} / {runtime.config.budget.max_reserved_tokens_per_month.toLocaleString()} tokens</dd></div></dl><p className="field-hint">{runtime.budget_note}</p><div className="model-service-controls"><button className="button compact" disabled={!running || requestActive || stopRequested} onClick={() => setStopping(true)}><Icon name="pause" size={14} />暂停并停止本机服务</button>{!running && <span>服务未运行，已保存的原话仍可查找。</span>}</div><details className="advanced-details"><summary>高级启动方式</summary><p>可以由受保护的启动环境提供 RECALLCARD_DREAM_API_KEY。不要把密钥写进资料库、源码或普通配置文件。此方式不表示已经保存在系统凭据存储。</p></details></div>}
    {runtime.jobs.length > 0 && <div className="memory-runtime-jobs"><div className="section-heading"><h2>最近整理任务</h2><span>{runtime.jobs.length} 项</span></div>{[...runtime.jobs].sort((a,b)=>b.created_at.localeCompare(a.created_at)).slice(0,10).map(job => <MemoryJobRow key={job.job_id} service={service} job={job} refresh={status.refresh} />)}</div>}
    {editing && <ModelSetupDialog key={runtime.config.scope} initial={initial} scope={service.scope} inspect={target => service.inspectModel(target)} submit={submit} close={() => setEditing(false)} />}
    {stopping && <Dialog title="暂停并停止本机服务？" close={() => setStopping(false)}><div className="modal-body"><p>将暂停整个资料库的后台任务，并在安全边界停止服务。已保存的来源和记忆会保留。</p><p>仅当前服务持有的会话密钥会在服务退出后失效；系统中已安全保存的密钥不会被删除。只关闭窗口不会清除会话密钥。</p></div><div className="modal-footer"><button className="button" onClick={() => setStopping(false)}>取消</button><button className="button primary" onClick={() => void stop()}>确认暂停并停止</button></div></Dialog>}
  </section>;
}
function MemoryJobRow({ service, job, refresh }: { service: ScopedService; job: MemoryJob; refresh: () => void }) {
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const submitting = useRef(false);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => { if (!submitting.current) { inFlight.current = false; setBusy(false); } }, [job]);
  const controls = async (action: 'pause' | 'resume' | 'retry' | 'cancel') => {
    if (inFlight.current) return;
    inFlight.current = true; submitting.current = true; setBusy(true); setError(null);
    try {
      await service.controlMemoryJob(job.job_id, action);
      // 收到新任务状态之前保持禁用，避免成功响应与轮询更新之间重发。
    } catch (reason) {
      setError(appError(reason));
    } finally {
      submitting.current = false; refresh();
    }
  };
  const label = ({queued:'等待整理',running:'正在整理',paused:'已暂停',needs_input:'需要处理',failed:'未完成',completed:'已完成',cancelled:'已取消'})[job.state];
  const phase = ({preflight:'检查资料',parsing:'读取来源',preparing:'准备有界来源',executing:'等待模型响应',validating:'核对证据与版本',committing:'保存记忆',indexing:'更新检索',finished:'处理结束'})[job.phase];
  return <div className="memory-runtime-job"><div className="job-title"><strong>{phase}</strong><Badge tone={job.state==='needs_input'||job.state==='failed'?'warning':job.state==='completed'?'success':'muted'}>{label}</Badge><span className="row-date">{dateLabel(job.updated_at,true)}</span></div><p>读取 {job.progress.sources_selected} 条来源 · 已保存 {job.progress.memories_committed} 条记忆 · 跳过 {job.progress.sources_skipped} 条来源</p>{job.error && <ErrorNotice error={job.error} />}{canRetryMemoryJob(job) && <p className="field-hint">修复连接或额度后可重试；重试可能再次产生请求费用，上次预留预算不会自动退还。</p>}{error && <ErrorNotice error={error} />}<div className="connection-controls">{job.state==='running'||job.state==='queued'?<button className="button compact" disabled={busy} onClick={()=>void controls('pause')}>暂停任务</button>:job.state==='paused'?<button className="button compact" disabled={busy} onClick={()=>void controls('resume')}>继续任务</button>:canRetryMemoryJob(job)?<button className="button compact" disabled={busy} onClick={()=>void controls('retry')}>重试任务</button>:null}{['needs_input','failed','paused'].includes(job.state)&&<button className="text-button" disabled={busy} onClick={()=>void controls('cancel')}>取消此候选任务</button>}</div></div>;
}
