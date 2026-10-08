import type { AppError, JobState } from "./contracts";
export interface MemoryTimeouts { connect_seconds: number; read_seconds: number; operation_seconds: number; }
export const validMemoryTimeouts = (value: MemoryTimeouts): boolean =>
  [value.connect_seconds, value.read_seconds, value.operation_seconds].every(Number.isSafeInteger) &&
  value.connect_seconds >= 1 && value.connect_seconds <= 120 &&
  value.read_seconds >= 1 && value.read_seconds <= value.operation_seconds &&
  value.operation_seconds >= value.connect_seconds && value.operation_seconds <= 1800;
export interface MemoryConfig {
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
