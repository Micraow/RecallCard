import type { ScopedService } from '../service/client';
import type { AppError, JobStatus } from '../service/contracts';
import { ErrorNotice, Badge, EmptyState, Loading } from '../components/common';
import { Icon } from '../components/Icon';
import { JobRow } from '../components/Jobs';
import { scopeLabel } from '../service/types';
import { navigate } from '../App';
import { useResource } from '../service/hooks';
import { runtimeLabel } from '../service/runtime';

export function ActivityPage({ service, jobs, error, refresh }: { service: ScopedService; jobs: JobStatus[]; error: AppError | null; refresh: () => void }) {
  return <div className="standard-page"><div className="page-heading"><div><h1>活动</h1><p>后台工作、处理结果与需要你决定的事。</p></div><button className="button" onClick={refresh}><Icon name="refresh" size={15} />刷新</button></div><RuntimeStatus service={service} /><section className="activity-section"><div className="section-heading"><h2>导入记录</h2><span>{jobs.length ? `${jobs.length} 项` : ''}</span></div>{error ? <ErrorNotice error={error} retry={refresh} /> : jobs.length ? <div className="activity-list">{[...jobs].sort((a, b) => b.created_at.localeCompare(a.created_at)).map(job => <JobRow key={job.job_id} job={job} service={service} refresh={refresh} />)}</div> : <EmptyState icon="activity" title="这里会留下处理记录"><p>导入之后，在这里查看保存了什么、重复了多少，<br />以及需要恢复的任务。</p></EmptyState>}</section><p className="coverage-note">任务进度来自本地服务。暂停保留已经保存的内容；总量未知时不显示百分比。</p></div>;
}
function RuntimeStatus({ service }: { service: ScopedService }) {
  const runtime = useResource(() => service.runtime(), [service]);
  if (runtime.loading) return <Loading label="正在读取后台整理状态" />;
  if (runtime.error) return <ErrorNotice error={runtime.error} retry={runtime.refresh} />;
  const status = runtime.data!;
  return <section className="runtime-status"><div className="runtime-icon"><Icon name="moon" size={21} /></div><div><div className="runtime-title"><h2>后台记忆整理</h2><Badge tone={status.state === 'failed' || status.state === 'needs_input' ? 'warning' : 'muted'}>{runtimeLabel(status.state)}</Badge></div><p>{status.message}</p><span className="runtime-usage">本月预留 {status.usage.reserved_calls} 次请求 · {status.usage.reserved_tokens.toLocaleString()} tokens</span></div><button className="text-button" onClick={() => navigate('settings')}>查看设置<Icon name="arrow" size={15} /></button></section>;
}
export function SettingsPage({ service, chooseWorkspace }: { service: ScopedService; chooseWorkspace: () => void }) {
  const runtime = useResource(() => service.runtime(), [service]);
  return <div className="standard-page settings-page"><div className="page-heading"><div><h1>设置</h1><p>本地资料、访问范围与运行方式。</p></div></div><section className="settings-section"><h2>本地空间</h2><div className="setting-row"><div><h3>当前范围</h3><p>{scopeLabel(service.scope)}</p></div><Badge>范围隔离</Badge></div><div className="setting-row"><div><h3>资料保存位置</h3><p className="path-value">{service.workspace.root}</p><span className="muted">Event 与 Memory 文件是正本；索引和阅读视图可以重新生成。</span></div><button className="button" onClick={chooseWorkspace}>打开其他空间</button></div></section><section className="settings-section"><h2>后台整理与模型</h2>{runtime.loading ? <Loading label="正在读取模型设置" /> : runtime.error ? <ErrorNotice error={runtime.error} retry={runtime.refresh} /> : runtime.data && <><div className="setting-row"><div><h3>一次配置，按授权范围运行</h3><p>{runtime.data.message}</p><span className="muted">{runtime.data.config.provider ? `${runtime.data.config.provider.model} · ${runtime.data.config.provider.endpoint}` : '尚未设置模型目的地。不会默认发送资料给任何供应商。'}</span></div><Badge>{runtimeLabel(runtime.data.state)}</Badge></div><div className="setting-row"><div><h3>本月资源使用</h3><p>预留请求 {runtime.data.usage.reserved_calls} / {runtime.data.config.budget.max_calls_per_month} 次 · 预留 tokens {runtime.data.usage.reserved_tokens.toLocaleString()} / {runtime.data.config.budget.max_reserved_tokens_per_month.toLocaleString()}</p><span className="muted">{runtime.data.budget_note}</span></div></div><p className="coverage-note">当前为真实状态读取；模型配置与安全凭据入口尚在接入，不提供不会生效的启用开关。</p></>}</section><section className="settings-section"><h2>连接与数据边界</h2><div className="setting-row"><div><h3>AI 的访问范围</h3><p>客户端只读当前明确授权的范围。网页中的最后发送始终由你确认。</p></div><button className="button" onClick={() => navigate('connections')}>管理连接</button></div><div className="setting-row"><div><h3>同步不等于共享</h3><p>个人对话不会因为选择项目范围或配置 Git 而自动公开。</p></div><Icon name="shield" size={18} /></div></section><footer className="settings-footer"><span className="brand-mark small"><Icon name="memory" size={16} /></span>RecallCard <span>{__APP_VERSION__}</span><span>本地优先 · 跨 AI 上下文</span></footer></div>;
}
