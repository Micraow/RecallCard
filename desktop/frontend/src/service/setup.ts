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
  const readyScopes=page.coverage?.scopes?.filter(scope=>scope.state==='ready')||[];
  if(page.coverage?.local_event_semantic) return [
    (page.coverage.semantic_query_encoding==='cache'||readyScopes.length>0&&readyScopes.every(scope=>scope.query_encoding==='cache'))?'本次使用已缓存的本机问题向量。':'本次问题已发送到你批准的本机编码服务。',
    '普通对话已直接参与本机语义检索，无需先整理成 Memory；记忆条目按词法补充，此入口没有云回退。',
    '每条原话展示得分最高的一处片段。角色、时间、否定与后续纠正仍需回到来源核对。',
    ...(page.truncated?['候选或片段达到数量/文字上限，未展示的来源不代表不存在。']:[])];
  if(page.coverage?.cloud_fallback===false) return [
    page.coverage.event_search==='local_only'?'本机普通对话索引尚未可用或未覆盖全部范围；现有 Memory 仅按词法补充。':'尚未配置普通对话本机索引，本次只在本地按关键词寻找原话和记忆。',
    '此入口不会改用云编码；未命中或未覆盖不表示没有相关历史。',
    ...(page.truncated?['候选、覆盖或文字预算有限，请核对状态并回到来源。']:[])];
  const notes = [semanticSearchNote(page.coverage) || "本次未提供语义检索状态，不能据此确认语义覆盖。"];
  if (page.coverage?.lexical_match === "natural_question") notes.push("按问题寻找字面候选；否定、相对时间和同义改写仍需核对原话，候选不等于答案。");
  notes.push("当前入口的语义候选只覆盖已建索引的记忆；原始聊天仍通过词法查找，尚不保证换个说法也能找到。");
  if (page.truncated) notes.push("本次结果达到数量或文字预算上限，未展示的来源并不代表不存在；可换用更具体的关键词继续找。");
  return notes;
}

export interface LocalEventSetup {
 endpoint:string; model:string; revision:string; tokenizer:string; dimensions:number;
 query_instruction:string; document_instruction:string; events_approved:boolean; query_approved:boolean;
 chunk_bytes:number; disk_cap_mib:number;
}
export interface LocalEventStatus {state:string;message:string;settings:LocalEventSetup|null;index_state?:string;source_count?:number;chunk_count?:number;database_bytes?:number;pending_sources?:number;}
export interface LocalEventProgress {state:string;complete:boolean;processed_sources:number;total_sources:number;}
export function validLocalEventSettings(value:LocalEventSetup):boolean {
 return validLocalSemanticTarget(value) && !!value.tokenizer.trim() && value.tokenizer.length<=256 && !/[\x00-\x1f\x7f]/.test(value.tokenizer)
 && Number.isInteger(value.chunk_bytes) && value.chunk_bytes>=256 && value.chunk_bytes<=8192
 && Number.isInteger(value.disk_cap_mib) && value.disk_cap_mib>=16 && value.disk_cap_mib<=512 && (value.events_approved||value.query_approved);
}
