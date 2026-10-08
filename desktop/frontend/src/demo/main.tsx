/** 仅由 demo.html 引入。生产 index.html 不引用此模块，演示内容不进入发行包。 */
import { createRoot } from 'react-dom/client';
import { App } from '../App';
import { RecallService, type Transport } from '../service/client';
import type { Memory, MemoryReview, Conversation, EventRecord } from '../service/types';
import type { JobStatus } from '../service/contracts';
import '../styles.css';

const now = '2026-10-08T08:32:00Z';
const entries = [
  ['项目交接：保留决定，也保留原因', '交接材料先写清当前目标、已作决定和仍未验证的假设。\n\n阅读者应能沿着来源找到当时为什么这样决定，而不只看到一个结论。设计文档、代码变更与讨论记录需要能互相追溯。\n\n示例项目「Atlas」当前聚焦离线阅读与跨设备连续性。验证一个完整任务的体验，优先于增加更多独立按钮。', ['工作方式', '项目协作']],
  ['研究笔记采用先证据、后结论的结构', '读论文时先记录研究问题、数据范围和实验限制，再形成自己的判断。没有原文支持的推断应单独标注。', ['学习', '研究']],
  ['图表说明要包含单位和比较基线', '展示结果时，坐标轴单位、样本范围、对照组和时间窗口应出现在图表旁边，而不是藏进注释。', ['表达偏好']],
  ['Atlas 的离线体验是当前重点', '出行场景优先验证：断网前准备内容、离线打开、恢复网络后同步位置。进度冲突需要可解释。', ['项目', 'Atlas']],
  ['周末步行路线先考虑公共交通', '路线草案先比较公共交通的接驳和返程时间。徒步长度只是其中一个参考，不代替天气与交通检查。', ['生活', '规划']],
  ['技术方案要写出失败时怎么办', '除了正常路径，方案需要列出中断、重复操作、版本变化和撤回权限后的行为。恢复路径同样是设计的一部分。', ['工作方式']],
  ['临时日程不能直接当长期偏好', '一次晚间开会只是一次安排。除非明确要求，否则不要由单次选择推断持续的可用时间。', ['待核对']],
  ['书籍批注保留页码与原始位置', '阅读批注应能回到原书所在位置。版本不同导致页码改变时，段落引用可以帮助再次定位。', ['阅读']],
  ['旧方案已不再用于当前项目', '早期设计使用手动导出流程。项目方向改变后，该结论应退出当前召回，同时保留历史出处。', ['历史']],
] as const;
const memories: Memory[] = entries.map(([title, body, labels], i) => ({ id: `mem_demo_${i + 1}`, content: `${title}\n\n${body}`, revision: i === 0 ? 3 : 1, status: i === 6 ? 'tentative' : 'active', evidence: i === 6 ? 'observed' : 'user_explicit', protected: i === 0, hidden: i === 8, can_restore: i === 8, source_refs: [`evt_demo_${i + 1}`, `evt_demo_${i + 11}`], updated_at: `2026-10-0${8 - i % 5}T08:32:00Z`, recorded_at: '2026-09-18T09:00:00Z', authority: 'user', scope: 'personal', labels: [...labels] }));
const sources: Conversation[] = [
  ['Atlas：离线阅读的端到端验证', 'chatgpt', 42], ['研究笔记与实验结论的写法', 'deepseek', 28], ['跨设备进度同步方案', 'chatgpt', 67], ['图表与论文阅读记录', 'claude', 16], ['周末路线草案', 'deepseek', 24], ['失败恢复与任务队列', 'chatgpt', 52], ['批注定位与格式选择', 'deepseek', 19], ['项目计划回顾', 'chatgpt', 36],
].map(([title, platform, count], i) => ({ session_ref: `source_demo_${i + 1}`, title: String(title), platform: String(platform), source_url: null, message_count: Number(count), captured_at: `2026-10-0${8 - i % 5}T08:32:00Z`, coverage: 'partial' }));
const events = new Map<string, EventRecord>();
memories.forEach((memory, i) => memory.source_refs.forEach((id, k) => events.set(id, { id, content: k === 0 ? `这段是用于界面验证的合成原话。\n\n${memory.content.split('\n\n').slice(1).join('\n\n')}` : '这是另一段合成的讨论记录，用于验证来源展开、角色与原始时间。请把结论与支持它的原文一起留下。', role: k === 0 ? 'user' : 'assistant', occurred_at: k === 0 ? '2026-10-07T14:25:00Z' : '2026-09-28T09:18:00Z', captured_at: now, source: { platform: k === 0 ? 'chatgpt' : 'deepseek', conversation_id: `demo_${i}`, message_id: id }, metadata: { conversation_title: k === 0 ? 'Atlas 的交接与体验检查' : '项目文档结构与原始依据' } })));
const completeJob: JobStatus = { schema: 'recallcard.application-job/1', job_id: 'job_demo_complete', request_id: 'request_demo_complete', kind: 'import', scope: 'personal', state: 'completed', phase: 'finished', progress: { source_bytes_read: 28744901, expanded_bytes_read: 38890423, files_processed: 1, files_total: 1, conversations: 8, events_staged: 344, events_total: 344, events_processed: 344, events_added: 284, events_duplicates: 60 }, error: null, can_resume: false, created_at: '2026-10-08T08:10:00Z', updated_at: '2026-10-08T08:11:00Z' };
const mode = new URLSearchParams(location.search).get('state') || 'normal';
let jobs: JobStatus[] = mode === 'empty' ? [] : [completeJob];
if (mode === 'failed') jobs = [{ ...completeJob, job_id: 'job_demo_failed', state: 'failed', phase: 'parsing', can_resume: true, progress: { ...completeJob.progress, events_total: null, events_processed: 0, events_added: 0, events_duplicates: 0, files_processed: 0 }, error: { code: 'invalid_json', message: '导出包中的会话文件未能完整解析', action: '重新下载完整的官方导出包，再添加来源。当前错误出现在会话边界附近。', file_name: 'chat-history-demo.zip', member: 'conversations.json', retryable: true, committed_events: 0 } }, completeJob];
if (mode === 'running') jobs = [{ ...completeJob, job_id: 'job_demo_running', state: 'running', phase: 'parsing', can_resume: false, progress: { ...completeJob.progress, events_total: null, events_processed: 0, events_added: 0, events_duplicates: 0, files_processed: 0 } }];
let pending: MemoryReview | null = null;
const calls: { command: string; args: Record<string, unknown> }[] = [];
Object.assign(window, { __DEMO_CALLS__: calls });
const transport: Transport = { async invoke<T>(command: string, args = {}): Promise<T> {
  calls.push({ command, args });
  await new Promise(resolve => setTimeout(resolve, command === 'import_sources' ? 350 : 55));
  const value = await invoke(command, args); return structuredClone(value) as T;
} };
async function invoke(command: string, args: Record<string, unknown>): Promise<unknown> {
  const rows = mode === 'empty' ? [] : memories;
  const scope = String(args.scope || 'personal');
  if (command === 'restore_workspace') return { vault: { session_id: 'demo_session', root: '/合成示例/RecallCard', display_name: '合成示例', scopes: ['personal', 'project:atlas'], event_count: 284, memory_count: memories.length, health: {} }, scope: 'personal' };
  if (command === 'remember_workspace' || command === 'cancel_previews' || command === 'write_clipboard') return null;
  if (command === 'choose_vault') return null;
  if (command === 'manage_memories') { const filtered = rows.filter(row => args.includeHidden || !row.hidden).filter((_, index) => scope === 'personal' || index === 3); return { memories: filtered, total: filtered.length, next_offset: null }; }
  if (command === 'managed_memory_view') { const memory = memories.find(row => row.id === args.id)!; return { memory, hidden: memory.hidden, can_restore: memory.can_restore }; }
  if (command === 'managed_memory_source') return events.get(String(args.eventId));
  if (command === 'list_conversations') return { conversations: mode === 'empty' ? [] : sources, total: mode === 'empty' ? 0 : sources.length, next_offset: null, note: '合成示例，不代表真实资料' };
  if (command === 'conversation_messages') { const source = sources.find(row => row.session_ref === args.conversationRef)!; const offset = Number(args.offset || 0); const messages = Array.from({ length: Math.min(4, source.message_count - offset) }, (_, i) => ({ ref: `event:evt_message_${offset + i}`, role: i % 2 === 0 ? 'user' : 'assistant', text: i % 2 === 0 ? '我们需要把当前的设计决定整理清楚。请先确认目标、已验证的行为和仍然开放的问题，不要把一个可点击的界面当作完整产品。\n\n导入后的资料应该马上可以查找。整理可以稍后完成，但不能让用户维护一串复制粘贴步骤。' : '可以按「目标、已验证事实、未验证假设、下一步」组织交接。\n\n本地存档保留原始内容；每条记忆回到证据。应用将后台任务的实际进度与需要用户决定的事分开呈现。', occurred_at: i === 3 ? null : '2026-10-07T14:25:00Z', captured_at: now, text_truncated: false, source: { platform: source.platform }, branch: { parent_ref: i ? `event:evt_message_${offset + i - 1}` : null, is_branch_end: false, relationship_known: true, child_count: 1, gap_before: false } })); return { ...source, messages, total: source.message_count, next_offset: offset + messages.length < source.message_count ? offset + messages.length : null, offset, order_known: true, order_kind: 'single_branch', coverage: '合成示例' }; }
  if (command === 'application_jobs') { if (mode === 'unavailable') throw { code: 'storage', message: '本地任务状态暂不可读取', action: '重新打开空间后重试', retryable: true, committed_events: 0 }; return jobs; }
  if (command === 'import_sources') { if (mode === 'cancel') return null; if (mode === 'pick-error') throw { code: 'permission_denied', message: '没有读取所选文件的权限', action: '请重新选择可读取的本机文件', file_name: '导出示例.zip', retryable: false, committed_events: 0 }; const job = { ...completeJob, job_id: 'job_demo_new', request_id: args.requestId, state: 'running', phase: 'parsing', progress: { ...completeJob.progress, events_total: null, events_processed: 0, events_added: 0, events_duplicates: 0, files_processed: 0 } } as JobStatus; jobs = [job, ...jobs.filter(item => item.job_id !== job.job_id)]; return job; }
  if (command === 'application_job_pause' || command === 'application_job_resume') { const job = jobs.find(item => item.job_id === args.jobId)!; job.state = command.endsWith('pause') ? 'paused' : 'running'; job.can_resume = job.state === 'paused'; job.error = null; return job; }
  if (command === 'search_records') return { results: rows.filter(row => !row.hidden && row.content.includes(String(args.query))).map(row => ({ ref: `memory:${row.id}`, kind: 'memory', text: row.content, state: row.status, evidence: row.evidence, occurred_at: row.updated_at })), truncated: false };
  if (command === 'review_memory_edit' || command === 'review_memory_visibility') { const memory = memories.find(row => row.id === args.id)!; pending = { preview_id: 'review_demo', operation: command === 'review_memory_edit' ? 'edit' : args.restore ? 'restore' : 'forget', before: memory, after: command === 'review_memory_edit' ? args.edit as MemoryReview['after'] : null, affected_events: command === 'review_memory_edit' ? 0 : 2, affected_memories: 1, requires_protected_approval: memory.protected, warning: command === 'review_memory_edit' ? '保存新的记忆版本，原始对话保持不变。' : '相关记忆与原始来源将退出检索和后续整理；文件不删除，可检查后恢复。' }; return pending; }
  if (command === 'confirm_memory_change') { if (!pending) throw new Error('确认已过期'); if (pending.requires_protected_approval && !args.approveProtected) throw new Error('请确认受保护内容的修改'); const index = memories.findIndex(row => row.id === pending!.before.id); if (pending.after) memories[index] = { ...memories[index], ...pending.after, revision: memories[index].revision + 1 }; else { memories[index].hidden = pending.operation === 'forget'; memories[index].can_restore = pending.operation === 'forget'; } pending = null; return {}; }
  if (command === 'prepare_client_config') return { mcpServers: { recallcard: { command: '/合成示例/recallcard', args: ['--vault', '/合成示例/RecallCard', 'mcp', '--scope', scope] } } };
  if (command === 'install_browser_connection') return { registered: true, registration: '/合成示例/native-host.json', capture_enabled: args.allowCapture, note: '示例注册完成，尚无浏览器调用回执。' };
  if (command === 'memory_runtime_status') return { schema: 'recallcard.memory-runtime/1', state: 'unconfigured', message: '后台整理尚未配置；已保存来源仍可搜索', config: { schema: 'recallcard.memory-runtime/1', enabled: false, paused: false, scope, provider: null, consent: null, budget: { max_calls_per_month: 100, max_reserved_tokens_per_month: 1000000, max_output_tokens_per_call: 4096, max_request_bytes_per_call: 1048576 }, quiet_seconds: 30, batch_size: 16, max_projection_bytes: 262144 }, usage: { month: '2026-10', reserved_calls: 0, reserved_tokens: 0, reported_input_tokens: 0, reported_output_tokens: 0, calls_with_unknown_usage: 0 }, jobs: [], raw_search_available: true, budget_note: 'token 预留是资源上限，不是准确费用' };
  throw new Error(`合成入口未实现命令：${command}`);
}
createRoot(document.getElementById('root')!).render(<App service={new RecallService(transport, true)} />);
