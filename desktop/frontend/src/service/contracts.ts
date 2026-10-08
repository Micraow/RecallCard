/** 序列化合同对应 crates/recallcard/src/application/contract.rs。
 * 业务含义由 Rust 定义；这里不实现导入、任务恢复或权限算法。
 */
export type JobState = 'queued' | 'running' | 'paused' | 'needs_input' | 'failed' | 'completed' | 'cancelled';
export type ImportPhase = 'preflight' | 'parsing' | 'committing' | 'finished';
export interface AppError {
  code: string;
  message: string;
  action: string;
  file_name: string | null;
  member: string | null;
  retryable: boolean;
  committed_events: number;
}
export interface ImportProgress {
  source_bytes_read: number;
  expanded_bytes_read: number;
  files_processed: number;
  files_total: number;
  conversations: number;
  events_staged: number;
  events_total: number | null;
  events_processed: number;
  events_added: number;
  events_duplicates: number;
}
export interface JobStatus {
  schema: 'recallcard.application-job/1';
  job_id: string;
  request_id: string;
  kind: string;
  scope: string;
  state: JobState;
  phase: ImportPhase;
  progress: ImportProgress;
  error: AppError | null;
  can_resume: boolean;
  created_at: string;
  updated_at: string;
}
export const isRunning = (job: JobStatus) => job.state === 'running' || job.state === 'queued';
export function validateJobs(value: unknown): JobStatus[] {
  if (!Array.isArray(value)) throw new Error('任务响应格式不受支持，请更新桌面与本地服务。');
  for (const job of value) {
    if (job?.schema !== 'recallcard.application-job/1' || typeof job.job_id !== 'string' || !job.progress || !['queued', 'running', 'paused', 'needs_input', 'failed', 'completed', 'cancelled'].includes(job.state)) {
      throw new Error('任务合同版本不一致，本次未显示不可靠的状态。');
    }
  }
  return value as JobStatus[];
}
export function appError(value: unknown): AppError {
  if (typeof value === 'object' && value !== null && 'message' in value) {
    const item = value as Partial<AppError>;
    return { code: item.code || 'service', message: String(item.message), action: item.action || '', file_name: item.file_name || null, member: item.member || null, retryable: item.retryable === true, committed_events: item.committed_events || 0 };
  }
  return { code: 'service', message: typeof value === 'string' ? value : '本地操作未完成，请重试。', action: '', file_name: null, member: null, retryable: false, committed_events: 0 };
}
