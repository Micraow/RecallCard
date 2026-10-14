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
  const compactMetadata=page.coverage?.presentation==='compact_index_metadata_full_source_via_fetch';
  const omitted=page.coverage?.budget_omitted_by_kind;
  const omittedKinds=([['event','普通对话'],['attachment','附件'],['memory','记忆']] as const).flatMap(([kind,label])=>{
    const count=omitted?.[kind];
    return typeof count==='number'&&Number.isSafeInteger(count)&&count>0?[`${label} ${count} 项`]:[];
  });
  const budgetNotes=[
    ...(compactMetadata?['本次返回采用紧凑索引元数据，保留片段正文和来源定位信息；完整来源可按引用读取。']:[]),
    ...(omittedKinds.length>0?[`本次返回预算省略了${omittedKinds.join('、')}候选；未展示的来源不代表不存在，也不保证每种来源类型均有保留。`]:[]),
    ...(page.coverage?.budget_limited===true&&!compactMetadata&&omittedKinds.length===0?['本次返回受到预算限制，请按已知引用读取来源核对。']:[]),
  ];
  // 已有明确预算说明时，不把仅精简索引元数据误报为正文或来源被截断。
  const genericTruncation=page.truncated&&budgetNotes.length===0;
  const attachments=page.coverage?.attachments;
  // 各范围是语义索引来源的覆盖依据；顶部的计数/上限仍属于另一条词法扫描路线。
  const scopedAttachments=page.coverage?.scopes?.flatMap(scope=>scope.attachments&&scope.attachments.status!=='not_approved'&&scope.attachments.status!=='not_requested'?[scope.attachments]:[])||[];
  const attachmentSemantic=attachments?.semantic===true||scopedAttachments.some(item=>item.semantic===true);
  const scopedAttachmentLimited=scopedAttachments.some(item=>item.limited===true);
  const scopedAttachmentSkipped=scopedAttachments.some(item=>(item.skipped||0)>0);
  const scopedAttachmentPartial=scopedAttachmentLimited||scopedAttachmentSkipped||scopedAttachments.some(item=>item.status==='partial');
  const attachmentPartial=attachments?.status==='partial'||attachments?.limited===true||(attachments?.skipped||0)>0||scopedAttachmentPartial;
  const attachmentNotes=(page.results.some(row=>row.kind==='attachment')||attachmentPartial||attachmentSemantic)?[
    attachmentSemantic?'本次至少一个独立批准范围的附件正文已参与本机语义检索；仅包含已保留且核验通过的安全 UTF-8 文本，不含 PDF 提取或 OCR。':'本次附件正文仅在本机按词法查找已保留且核验通过的安全 UTF-8 文本，未使用附件语义向量；不含 PDF 提取或 OCR。',
    '附件内容是独立来源，不当作上传者原话；父事件须在当前范围可见、未撤回且未被修订替换。',
    ...(attachments?.limited?['附件扫描达到文件数量或总字节上限，部分原件尚未检查；缩小读取范围，或按已知附件引用单独读取。换关键词不会解除这项扫描上限。']:[]),
    ...((attachments?.skipped||0)>0?['部分附件因格式不支持、原件缺失、损坏或单文件超限未参与检索；请检查完整导出原件，未命中不能当作附件内容不存在。']:[]),
    ...(scopedAttachmentPartial?['部分范围的附件语义索引来源覆盖不完整；未纳入索引的附件不代表原件不存在，也不能据此推断其他范围已获批准或全部覆盖。']:[]),
    ...(scopedAttachmentLimited?['附件语义索引的来源检查达到文件数量或总字节上限，部分原件尚未检查；缩小读取范围，或按已知附件引用单独读取。换关键词不会解除这项来源检查上限。']:[]),
    ...(scopedAttachmentSkipped?['部分附件因格式不支持、原件缺失、损坏或单文件超限未进入语义索引；请检查完整导出原件，未命中不能当作附件内容不存在。']:[]),
  ]:[];
  const readyScopes=page.coverage?.scopes?.filter(scope=>scope.state==='ready')||[];
  const fallbackScopes=page.coverage?.scopes?.filter(scope=>scope.fusion==='lexical_only')||[];
  const directFallback=page.coverage?.fusion==='lexical_only';
  const fallbackLexical=[...(directFallback&&page.coverage?.lexical?[page.coverage.lexical]:[]),...fallbackScopes.flatMap(scope=>scope.lexical?[scope.lexical]:[])];
  const fallbackNotes=directFallback||fallbackScopes.length>0?[
    '部分资料范围的本机编码服务不可用，仅返回来源已核验的词法候选；这些范围没有语义分数，也不会改用云编码。',
    ...(fallbackLexical.some(item=>item.status==='limited_query')?['降级范围的问题超出词法预算，本次未能执行这两条候选路线；没有截短或改写问题。']:[]),
    ...(fallbackLexical.some(item=>item.status==='unsupported_query')?['降级范围的问题没有受支持的词法词项，本次没有可用的候选路线。']:[]),
    ...(fallbackLexical.some(item=>item.candidates_truncated)?['降级范围的词法候选达到扫描或数量预算，未展示的来源仍可能存在。']:[]),
    '字面命中仍需核对角色、时间、否定和后续纠正；未命中不表示没有相关历史。',
  ]:[];
  const lexical=[...(!directFallback&&page.coverage?.lexical?[page.coverage.lexical]:[]),...readyScopes.flatMap(scope=>scope.lexical?[scope.lexical]:[])];
  if(page.coverage?.local_event_semantic) return [
    ...budgetNotes, ...attachmentNotes, ...fallbackNotes,
    (page.coverage.semantic_query_encoding==='cache'||readyScopes.length>0&&readyScopes.every(scope=>scope.query_encoding==='cache'))?'本次使用已缓存的本机问题向量。':'本次问题已发送到你批准的本机编码服务。',
    (attachmentSemantic?'本次独立批准的资料已参与本机语义检索':'普通对话已直接参与本机语义检索')+'，无需先整理成 Memory；记忆条目按词法补充，此入口没有云回退。',
    (attachmentSemantic?'每项资料':'每条原话')+'展示一处来源已核验的匹配片段，可能由词法或语义通道选中。角色、时间、否定与后续纠正仍需回到来源核对。',
    ...(lexical.some(item=>item.status==='available')?[(attachmentSemantic?'已批准资料的':'普通对话')+'词法索引已独立寻找完整编号和汉字词组候选，并结合语义排序；字面匹配不代表事实正确。']:[]),
    ...(lexical.some(item=>item.status==='legacy_unavailable')?['部分范围仍使用旧语义索引；显式刷新'+(attachmentSemantic?'已批准资料':'普通对话')+'索引后可补建词法通道并复用已有向量。']:[]),
    ...(lexical.some(item=>item.status==='limited_query')?['本次问题超出词法通道预算，完整问题仍用于语义检索；没有截短或改写问题。']:[]),
    ...(lexical.some(item=>item.status==='unsupported_query')?['本次问题没有受支持的词法词项，仍按完整问题进行语义检索。']:[]),
    ...(lexical.some(item=>item.candidates_truncated)?['词法候选达到本次扫描或数量预算，未展示的精确来源仍可能存在。']:[]),
    '相似候选可能完全无关；尚未校准相关性或可回答性，不能把返回结果当作已经记起事实。',
    ...(page.coverage.candidate_verification_limited||readyScopes.some(scope=>scope.verification_limited)?['来源核验受到时间或资源门槛限制，部分候选尚未核验；请缩小范围或稍后重试。']:[]),
    ...(genericTruncation?['候选、片段或核验达到本次预算，未展示的来源不代表不存在。']:[])];
  if(fallbackNotes.length>0) return [...budgetNotes,...attachmentNotes,...fallbackNotes];
  if(page.coverage?.cloud_fallback===false) return [
    ...budgetNotes, ...attachmentNotes,
    page.coverage.event_search==='local_only'?'本机普通对话索引尚未可用或未覆盖全部范围；现有 Memory 仅按词法补充。':'尚未配置普通对话本机索引，本次只在本地按关键词寻找原话和记忆。',
    '此入口不会改用云编码；未命中或未覆盖不表示没有相关历史。',
    ...(page.coverage.lexical_match==='natural_question'?['按问题寻找字面候选；否定、相对时间和同义改写仍需核对原话，候选不等于答案。']:[]),
    ...(genericTruncation?['候选、覆盖或文字预算有限，请核对状态并回到来源。']:[])];
  const notes = [...budgetNotes, ...attachmentNotes, semanticSearchNote(page.coverage) || "本次未提供语义检索状态，不能据此确认语义覆盖。"];
  if (page.coverage?.lexical_match === "natural_question") notes.push("按问题寻找字面候选；否定、相对时间和同义改写仍需核对原话，候选不等于答案。");
  notes.push("当前入口的语义候选只覆盖已建索引的记忆；原始聊天仍通过词法查找，尚不保证换个说法也能找到。");
  if (genericTruncation) notes.push(attachmentPartial ? "本次附件覆盖不完整，未展示的来源并不代表不存在；请先查看上方附件状态。" : "本次结果达到数量或文字预算上限，未展示的来源并不代表不存在；可换用更具体的关键词继续找。");
  return notes;
}

export interface LocalEventSetup {
 endpoint:string; model:string; revision:string; tokenizer:string; dimensions:number;
 query_instruction:string; document_instruction:string; events_approved:boolean; attachments_approved:boolean; query_approved:boolean;
 chunk_bytes:number; disk_cap_mib:number;
}
// 旧版本状态缺少附件批准；它不能从普通对话或问题批准中继承。
export type LocalEventSettingsSnapshot = Omit<LocalEventSetup,'attachments_approved'> & {attachments_approved?:boolean};
export function normalizeLocalEventSettings(value:LocalEventSettingsSnapshot):LocalEventSetup {
 return {...value,attachments_approved:value.attachments_approved===true};
}
export interface LocalEventStatus {state:string;message:string;settings:LocalEventSettingsSnapshot|null;index_state?:string;source_count?:number;chunk_count?:number;database_bytes?:number;pending_sources?:number;}
export interface LocalEventProgress {state:string;complete:boolean;processed_sources:number;total_sources:number;}
export function validLocalEventSettings(value:LocalEventSetup):boolean {
 return validLocalSemanticTarget(value) && !!value.tokenizer.trim() && value.tokenizer.length<=256 && !/[\x00-\x1f\x7f]/.test(value.tokenizer)
 && Number.isInteger(value.chunk_bytes) && value.chunk_bytes>=256 && value.chunk_bytes<=8192
 && Number.isInteger(value.disk_cap_mib) && value.disk_cap_mib>=16 && value.disk_cap_mib<=512 && (value.events_approved||value.attachments_approved===true||value.query_approved);
}
