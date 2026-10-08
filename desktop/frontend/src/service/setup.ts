import type { AppError } from "./contracts";
import type { BuildInfo } from "./types";
import type { MemoryConfig, MemoryJob, RuntimeStatus } from "./runtime";
export interface CredentialStatus {
  present: boolean;
  storage: "os_protected" | "session_only" | "environment" | "unavailable";
  lifetime:
    | "until_deleted_from_os_store"
    | "background_service_exit"
    | "process_environment"
    | "not_configured";
  os_protected_available: boolean | null;
  message: string;
}
export interface LocalServiceStatus {
  schema: string;
  running: boolean;
  pid: number | null;
  started_at: string | null;
  heartbeat_at: string | null;
  imports_pending: boolean;
  last_memory_job: MemoryJob | null;
  error: AppError | null;
  credential: CredentialStatus;
  build?: BuildInfo;
  binary_hash?: string | null;
}
export interface ModelSetupSnapshot {
  runtime: RuntimeStatus;
  credential: CredentialStatus;
  service: LocalServiceStatus | null;
  service_error: AppError | null;
}
export interface ModelSetupRequest {
  config: MemoryConfig;
  apiKey: string | null;
  credentialStorage: "os_protected" | "session_only";
}
export function validModelTarget(endpoint: string, model: string): boolean {
  try {
    const url = new URL(endpoint);
    return (
      endpoint.startsWith("https://") &&
      endpoint.length <= 2048 &&
      !/[^\x21-\x7e]|[\\]/.test(endpoint) &&
      !/[\x00-\x1f\x7f]/.test(model) &&
      url.protocol === "https:" &&
      !!url.hostname &&
      url.pathname !== "/" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      model.trim().length > 0 &&
      model.length <= 256
    );
  } catch {
    return false;
  }
}
