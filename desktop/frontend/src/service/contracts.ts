/** 序列化合同对应 crates/recallcard/src/application/contract.rs。
 * 业务含义由 Rust 定义；这里不实现导入、任务恢复或权限算法。
 */
export type JobState =
  | "queued"
  | "running"
  | "paused"
  | "needs_input"
  | "failed"
  | "completed"
  | "cancelled";
export type ImportPhase = "preflight" | "parsing" | "committing" | "finished";
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
  schema: "recallcard.application-job/1";
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
export const isRunning = (job: JobStatus) =>
  job.state === "running" || job.state === "queued";
export function validateJobs(value: unknown): JobStatus[] {
  if (!Array.isArray(value))
    throw new Error("任务响应格式不受支持，请更新桌面与本地服务。");
  for (const job of value) {
    if (
      job?.schema !== "recallcard.application-job/1" ||
      typeof job.job_id !== "string" ||
      !job.progress ||
      ![
        "queued",
        "running",
        "paused",
        "needs_input",
        "failed",
        "completed",
        "cancelled",
      ].includes(job.state)
    ) {
      throw new Error("任务合同版本不一致，本次未显示不可靠的状态。");
    }
  }
  return value as JobStatus[];
}
export function appError(value: unknown): AppError {
  if (typeof value === "object" && value !== null && "message" in value) {
    const item = value as Partial<AppError>;
    return {
      code: item.code || "service",
      message: String(item.message),
      action: item.action || "",
      file_name: item.file_name || null,
      member: item.member || null,
      retryable: item.retryable === true,
      committed_events: item.committed_events || 0,
    };
  }
  return {
    code: "service",
    message: typeof value === "string" ? value : "本地操作未完成，请重试。",
    action: "",
    file_name: null,
    member: null,
    retryable: false,
    committed_events: 0,
  };
}

/** 错误恢复按钮只表示服务允许的动作，不用「继续」掩盖需要重新选来源的问题。 */
export function importRecovery(
  job: JobStatus,
): "new_source" | "resume" | "none" {
  if (
    job.error &&
    [
      "invalid_json",
      "invalid_archive",
      "invalid_conversation",
      "unsupported_format",
      "source_changed",
      "resource_limit",
    ].includes(job.error.code)
  )
    return "new_source";
  return job.can_resume ? "resume" : "none";
}

export interface ImportCoverage {
  expanded_bytes: number;
  archive_entries: number;
  conversations: number;
  events: number;
  ignored_values: number;
  ignored_files: number;
  hidden_fragments: number;
  unsupported_fragments: number;
  omitted_messages: number;
  file_references: number;
  attachment_payloads_retained?: number;
  attachment_payloads_missing?: number;
  unsupported_archive_sources?: string[];
  citations: number;
  trace_placeholders: number;
}
export interface ImportResult {
  job: JobStatus;
  coverage: ImportCoverage;
}
export function coverageNotes(coverage: ImportCoverage): string[] {
  const fields: [
    Exclude<keyof ImportCoverage, "unsupported_archive_sources">,
    string,
  ][] = [
    ["file_references", "条附件引用（按消息计数，非独立原件数）"],
    ["attachment_payloads_retained", "条附件引用已保留原始字节，未提取或索引正文"],
    ["attachment_payloads_missing", "条附件引用在所提供归档中未找到原件"],
    ["citations", "条引用记录"],
    ["trace_placeholders", "条工具记录仅保留类型，没有工具输出正文"],
    ["hidden_fragments", "个隐藏推理片段未收集"],
    ["unsupported_fragments", "个不支持的片段未导入"],
    ["omitted_messages", "条消息未导入"],
    ["ignored_files", "个非会话文件未导入"],
    ["ignored_values", "个不支持的数据项未导入"],
  ];
  const notes = fields
    .filter(([key]) => (coverage[key] ?? 0) > 0)
    .map(([key, label]) => `${(coverage[key] ?? 0).toLocaleString()} ${label}`);
  const unsupported = new Set(coverage.unsupported_archive_sources ?? []);
  if (unsupported.has("group_chats")) notes.push("群聊记录在当前支持范围外，已跳过");
  if (unsupported.has("pages")) notes.push("Pages 在当前支持范围外，已跳过");
  if ([...unsupported].some((source) => !["group_chats", "pages"].includes(source))) {
    notes.push("另有未支持的归档来源，本轮未导入");
  }
  return notes;
}
