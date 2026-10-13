import type { AppError } from "./contracts";
import type { BuildInfo, SearchPage } from "./types";
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

export interface LocalSemanticSetup {
  endpoint: string; model: string; dimensions: number; revision: string;
  query_instruction: string; document_instruction: string;
  query_approved: boolean; memory_approved: boolean; resume_background: boolean;
}
export interface SemanticSetupStatus {
  state: string; message: string; settings: LocalSemanticSetup | null; paused: boolean;
  index_refresh: import("./runtime").IndexRefreshStatus | null;
  service: LocalServiceStatus | null; service_error: AppError | null;
}
export function validLocalSemanticTarget(value: Pick<LocalSemanticSetup,"endpoint"|"model"|"dimensions"|"revision"|"query_instruction"|"document_instruction">): boolean {
  return /^http:\/\/(127\.0\.0\.1|\[::1\]):[0-9]+\/v1\/embeddings$/.test(value.endpoint)
    && value.endpoint.length<=2048 && (()=>{const port=Number(value.endpoint.match(/:([0-9]+)\/v1\/embeddings$/)?.[1]);return port>0&&port<=65535;})()
    && value.model.trim().length>0 && new TextEncoder().encode(value.model).length<=256 && !/[\x00-\x1f\x7f]/.test(value.model)
    && value.revision.trim().length>0 && new TextEncoder().encode(value.revision).length<=256 && !/[\x00-\x1f\x7f]/.test(value.revision)
    && Number.isSafeInteger(value.dimensions) && value.dimensions>=1 && value.dimensions<=8192
    && new TextEncoder().encode(value.query_instruction).length<=2048 && new TextEncoder().encode(value.document_instruction).length<=2048;
}

export function semanticSearchNote(coverage?: {semantic_search?: string; semantic_error?: string; semantic_query_encoding?: string}): string {
 if(!coverage)return '';
 if(coverage.semantic_search==='available')return (coverage.semantic_query_encoding==='local_encoded'?'本次新查询已在本机编码。':coverage.semantic_query_encoding==='cloud_encoded'?'本次新查询已按独立云批准编码。':'')+'已结合 Memory 语义候选；请打开来源核对事实，排序不代表答案准确率。';
 const recovery:Record<string,string>={key_unavailable:'独立云凭据不可读取，请在云向量设置核对系统存储或重新输入该目标凭据。',network_budget_exhausted:'本月云向量预算不足，请核对用量或明确调整预算模式。',cloud_approval_changed:'云向量批准或配置已改变，请按当前设置重新检索。',cloud_budget_busy:'预算记录正在更新，可稍后重试；此请求尚未发送。'};
 return coverage.semantic_error?`本次语义检索未可用（${coverage.semantic_error}），保留词法结果；${recovery[coverage.semantic_error]||'可在设置中检查语义提供器与索引状态。'}`:'尚未启用语义检索，本次使用词法匹配。';
}
export interface CloudEmbeddingSettings {
 endpoint:string; model:string; dimensions:number; revision:string; query_instruction:string; document_instruction:string;
 query_approved:boolean; memory_approved:boolean; budget_mode?:'limited'|'unlimited'; max_calls_per_month:number; max_bytes_per_month:number;
}
export interface CloudSemanticSetup {settings:CloudEmbeddingSettings;save_in_os:boolean;resume_background:boolean;}
export interface CloudSemanticStatus {
 state:string;message:string;paused:boolean;
 cloud:{grant:{revision:string;scope:string;settings:CloudEmbeddingSettings;accepted_at:string}|null;usage:{month:string;reserved_calls:number;reserved_bytes:number;query_calls:number;memory_calls:number};budget_note:string};
 index_refresh:import('./runtime').IndexRefreshStatus|null;credential:CredentialStatus|null;service_error:AppError|null;
}
export function validCloudSemanticSettings(value:CloudEmbeddingSettings):boolean {
 return validModelTarget(value.endpoint,value.model) && !new URL(value.endpoint).pathname.endsWith('/')
  && new URL(value.endpoint).hostname.split('.').every(label=>/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/i.test(label)) && new URL(value.endpoint).port!=='0'
  && Number.isSafeInteger(value.dimensions)&&value.dimensions>=1&&value.dimensions<=8192
  && value.revision.trim().length>0&&new TextEncoder().encode(value.revision).length<=256&&!/[\x00-\x1f\x7f]/.test(value.revision)
  && new TextEncoder().encode(value.query_instruction).length<=2048&&new TextEncoder().encode(value.document_instruction).length<=2048
  && (value.query_approved||value.memory_approved)
  && (!value.budget_mode||['limited','unlimited'].includes(value.budget_mode))
  && Number.isSafeInteger(value.max_calls_per_month)&&value.max_calls_per_month>=1&&value.max_calls_per_month<=1000000
  && Number.isSafeInteger(value.max_bytes_per_month)&&value.max_bytes_per_month>=1&&value.max_bytes_per_month<=4294967296;
}

export function recallCoverageNotes(page: SearchPage): string[] {
  const notes = [semanticSearchNote(page.coverage) || "本次未提供语义检索状态，不能据此确认语义覆盖。"];
  notes.push("当前入口的语义候选只覆盖已建索引的记忆；原始聊天仍通过词法查找，尚不保证换个说法也能找到。");
  if (page.truncated) notes.push("本次结果达到数量或文字预算上限，未展示的来源并不代表不存在；可换用更具体的关键词继续找。");
  return notes;
}
