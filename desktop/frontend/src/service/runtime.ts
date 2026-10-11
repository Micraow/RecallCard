import type { AppError, JobState } from "./contracts";
export interface MemoryTimeouts { connect_seconds: number; read_seconds: number; operation_seconds: number; }
export const validMemoryTimeouts = (value: MemoryTimeouts): boolean =>
  [value.connect_seconds, value.read_seconds, value.operation_seconds].every(Number.isSafeInteger) &&
  value.connect_seconds >= 1 && value.connect_seconds <= 120 &&
  value.read_seconds >= 1 && value.read_seconds <= value.operation_seconds &&
  value.operation_seconds >= value.connect_seconds && value.operation_seconds <= 1800;
export interface MemoryConfig {
  pipeline?: "legacy_single" | "staged_review";
  candidate_semantic_config?: string | null;
  index_maintenance_enabled?: boolean;
  schema: "recallcard.memory-runtime/1";
  credential_storage?:
    | "os_protected"
    | "session_only"
    | "environment"
    | "unavailable"
    | null;
  enabled: boolean;
  paused: boolean;
  scope: string;
  provider: { endpoint: string; model: string } | null;
  consent: {
    endpoint: string;
    model: string;
    scope: string;
    send_source_snapshots: boolean;
    send_memory_snapshots: boolean;
    auto_apply: boolean;
    accepted_at: string;
  } | null;
  budget: {
    mode?: "limited" | "unlimited";
    max_calls_per_month: number;
    max_reserved_tokens_per_month: number;
    max_output_tokens_per_call: number;
    max_request_bytes_per_call: number;
  };
  timeouts?: MemoryTimeouts;
  quiet_seconds: number;
  batch_size: number;
  max_projection_bytes: number;
}
export interface MemoryProgress {
  sources_selected: number;
  memories_read: number;
  sources_committed: number;
  memories_committed: number;
  sources_skipped: number;
  provider_calls: number;
  reserved_tokens: number;
  input_tokens: number | null;
  output_tokens: number | null;
  source_cursor: string | null;
  receipt_id: string | null;
}
export interface MemoryJob {
  schema: "recallcard.application-job/1";
  job_id: string;
  request_id: string;
  kind: "memory";
  scope: string;
  state: JobState;
  phase:
    | "preflight"
    | "parsing"
    | "preparing"
    | "executing"
    | "validating"
    | "committing"
    | "indexing"
    | "finished";
  progress: MemoryProgress;
  error: AppError | null;
  can_resume: boolean;
  created_at: string;
  updated_at: string;
}
/** 每页最多十项；过滤只影响显示，绝不改变任务或重新提交。 */
export function memoryJobWindow(jobs: readonly MemoryJob[], filter:'pending'|'all', page:number) {
  const pending = (job:MemoryJob) => !['completed','cancelled'].includes(job.state);
  const pendingCount=jobs.filter(pending).length;
  const selected=jobs.filter(job=>filter==='all'||pending(job)).sort((a,b)=>b.created_at.localeCompare(a.created_at)||a.job_id.localeCompare(b.job_id));
  const pages=Math.max(1,Math.ceil(selected.length/10));
  const current=Math.min(pages-1,Math.max(0,Number.isSafeInteger(page)?page:0));
  return {rows:selected.slice(current*10,current*10+10),total:selected.length,pendingCount,page:current,pages,start:selected.length?current*10+1:0,end:Math.min((current+1)*10,selected.length)};
}
export interface IndexRefreshStatus {
  state: string; message: string; generation: string | null; indexed_generation: string | null;
  ready_documents: number; total_documents: number; new_vectors: number; attempts: number; failures: number;
  error_code: string | null; can_retry: boolean; checked_at: string; retry_after: string | null;
}
export interface RuntimeStatus {
  schema: "recallcard.memory-runtime/1";
  state: string;
  message: string;
  config: MemoryConfig;
  usage: {
    month: string;
    reserved_calls: number;
    reserved_tokens: number;
    reported_input_tokens: number;
    reported_output_tokens: number;
    calls_with_unknown_usage: number;
  };
  jobs: MemoryJob[];
  raw_search_available: boolean;
  index_refresh?: IndexRefreshStatus | null;
  oversized_sources: number;
  budget_note: string;
}
export const runtimeLabel = (state: string) =>
  ({
    unconfigured: "尚未启用",
    paused: "已暂停",
    ready: "配置已就绪",
    needs_input: "需要处理",
    failed: "有任务失败",
  })[state] || "状态待检查";

// 需要用户判断证据或受保护内容的候选不能当作可重发的网络故障。
export const canRetryMemoryJob = (job: MemoryJob): boolean =>
  ["failed", "needs_input"].includes(job.state) &&
  job.can_resume && job.error?.retryable === true;

export interface ReviewMemory {
  id: string; revision: number; content: string; scope: string; status: string;
  evidence: string; source_refs: string[]; protected: boolean;
  supersedes?: string[]; navigation?: { path: string; title?: string }[];
}
export interface MemoryJobReview {
  job: MemoryJob;
  source_snapshots: { ref: string; event: import('./types').EventRecord }[];
  extraction?: { result: { proposals: { operation: string; content?: string; source_refs: string[] }[] } } | null;
  semantic_lookup?: { status: string; queries: number; cache_hits: number; local_encodings?: number; errors: string[]; memory_refs: string[]; directory_refs: string[] } | null;
  review: { job_id: string; result_hash: string; already_applied: boolean; can_apply: boolean; requires_protected_approval: boolean; changes: { operation: string; before: ReviewMemory | null; after: ReviewMemory | null }[]; diagnostics: string[]; split_history?: { target_ref: string; retired_ref: string; branch_refs: string[] }[] } | null;
  note: string;
}
export function validateMemoryJobReview(value: unknown, jobId: string, scope: string): MemoryJobReview {
  const data = value as MemoryJobReview | null;
  if (!data || data.job?.schema !== "recallcard.application-job/1" || data.job?.job_id !== jobId || data.job?.scope !== scope || !Array.isArray(data.source_snapshots) || (data.review && (!/^[a-f0-9]{64}$/.test(data.review.result_hash) || !Array.isArray(data.review.changes) || typeof data.review.can_apply !== 'boolean' || typeof data.review.requires_protected_approval !== 'boolean' || typeof data.review.already_applied !== 'boolean'))) throw new Error('候选审阅身份、范围或完整结果格式不一致，请重新读取');
  const available=new Set(data.source_snapshots.map(source=>{
    if(typeof source?.ref!=='string'||!source.ref.startsWith('event:')||source.ref.slice(6)!==source.event?.id||typeof source.event?.content!=='string')throw new Error('候选来源快照不完整，请重新读取');
    return source.event.id;
  }));
  if(data.review?.changes.some(change=>change.after?.source_refs.some(ref=>!available.has(ref.replace(/^event:/,'')))))throw new Error('候选缺少完整来源快照，请重新读取');
  return data;
}
export function canApproveMemoryReview(data: MemoryJobReview | null, confirmed: boolean, protectedApproved: boolean): boolean {
  const review=data?.review;
  return !!(confirmed && data && ['needs_input','failed'].includes(data.job.state) && review && review.can_apply && !review.already_applied && /^[a-f0-9]{64}$/.test(review.result_hash) && (!review.requires_protected_approval || protectedApproved));
}

export function monthlyBudgetLabel(budget: MemoryConfig["budget"]): string {
  return budget.mode === "unlimited" ? "无限（月度请求/token 不设配额，持续计量）" : `${budget.max_calls_per_month.toLocaleString()} 次请求 · ${budget.max_reserved_tokens_per_month.toLocaleString()} tokens`;
}
