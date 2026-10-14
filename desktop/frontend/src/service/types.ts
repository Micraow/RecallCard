export interface BuildInfo {
  version: string;
  commit: string;
  dirty: boolean;
}
export interface Workspace {
  session_id: string;
  root: string;
  display_name: string;
  scopes: string[];
  event_count: number;
  memory_count: number;
  health: unknown;
}
export interface RestoredWorkspace {
  vault: Workspace;
  scope: string;
}
export interface MemoryRow {
  id: string;
  revision: number;
  content: string;
  text_truncated?: boolean;
  status: "active" | "tentative" | "stale" | "retracted" | "superseded";
  protected: boolean;
  evidence: string;
  source_refs: string[];
  hidden: boolean;
  can_restore?: boolean;
  updated_at: string;
}
export interface Memory extends MemoryRow {
  supersedes?: string[];
  labels: string[];
  scope: string;
  recorded_at: string;
  authority: string;
  time_note?: string;
  valid_from?: string | null;
  valid_to?: string | null;
}
/** 搜索返回带版本 ref，管理列表返回裸 ID。这里仅拆合同，ID 仍由核心校验。 */
export function parseMemoryTarget(target: string): {id: string; revision: number | null} | null {
  const match = /^(?:memory:)?(mem_[a-z0-9_]{1,128})(?:@([1-9][0-9]*))?$/.exec(target);
  if (!match) return null;
  const revision = match[2] ? Number(match[2]) : null;
  return revision !== null && !Number.isSafeInteger(revision) ? null : {id:match[1],revision};
}
/** 历史边只含裸 ID；不将非法路径或版本引用修饰成另一个合法目标。 */
export function memoryPredecessors(memory: Pick<Memory,'id'|'supersedes'>): {ids:string[]; invalid:number} {
  if(memory.supersedes === undefined) return {ids:[],invalid:0};
  if(!Array.isArray(memory.supersedes)) return {ids:[],invalid:1};
  const ids=new Set<string>();let invalid=0;
  for(const candidate of memory.supersedes){
    if(typeof candidate !== 'string'){invalid++;continue;}
    const target=parseMemoryTarget(candidate);
    if(!target || target.id!==candidate || target.revision!==null || candidate===memory.id){invalid++;continue;}
    ids.add(candidate);
  }
  return {ids:[...ids],invalid};
}
export interface MemoryPage {
  memories: MemoryRow[];
  total: number;
  next_offset: number | null;
}
export interface Source {
  platform: string;
  conversation_id?: string;
  message_id?: string;
  url?: string | null;
}
export interface EventRecord {
  id: string;
  content: string;
  parts?: { text?: string }[];
  role: string;
  occurred_at: string | null;
  captured_at: string;
  source: Source;
  metadata?: { conversation_title?: string };
}
export interface Conversation {
  session_ref: string;
  title: string;
  platform: string;
  source_url: string | null;
  message_count: number;
  captured_at: string;
  last_occurred_at?: string | null;
  coverage: string;
}
export interface ConversationPage {
  conversations: Conversation[];
  total: number;
  next_offset: number | null;
  note: string;
}
export interface SourceAssets {
  files: {
    source_file_id: string | null;
    name: string | null;
    byte_count: number | null;
    payload_status:
      | "retained"
      | "missing"
      | "available_not_retained"
      | "not_in_export";
    declared_media_type?: string | null;
    library_media_type?: string | null;
    detected_media_type?: string | null;
    declared_media_type_conflict?: boolean;
    object_sha256?: string | null;
    archive_name_alias?: string | null;
  }[];
  citations: { url: string; title: string | null }[];
  tool_trace: {
    type: string;
    payload_status: "not_in_export" | "references_only";
    reference_count?: number;
  }[];
  files_total: number;
  citations_total: number;
  trace_total: number;
  unsupported_total: number;
  truncated: boolean;
}
/** 只解释导入时记录的状态，不从文件名、类型或哈希推断原件已经存在。 */
export function attachmentStatus(
  status: SourceAssets["files"][number]["payload_status"],
) {
  switch (status) {
    case "retained":
      return { label: "原件已保留", detail: "已在本机保留原始字节" };
    case "missing":
      return { label: "原件缺失", detail: "所提供归档中未找到原件" };
    case "available_not_retained":
      return {
        label: "未保留原件",
        detail: "只读解析已发现原始字节，本次未保留",
      };
    case "not_in_export":
      return {
        label: "旧版引用记录",
        detail: "旧记录仅有元数据，可重新导入所提供归档以核对原件",
      };
    default:
      return { label: "状态未知", detail: "未提供可识别的原件状态" };
  }
}
export interface Message {
  ref: string;
  role: string;
  text: string;
  occurred_at: string | null;
  captured_at: string;
  text_truncated: boolean;
  source: Source;
  kind?: string;
  assets?: SourceAssets | null;
  branch?: {
    parent_ref: string | null;
    is_branch_end: boolean;
    relationship_known: boolean;
    child_count: number;
    gap_before: boolean;
    on_current_path?: boolean;
  };
}
export interface MessagesPage {
  messages: Message[];
  total: number;
  next_offset: number | null;
  offset: number;
  title: string;
  platform: string;
  session_ref: string;
  coverage: string;
  order_known: boolean;
  order_kind: string;
}
export interface SearchRow {
  source_ref?: string;
  score_kind?: string;
  retrieval_support?: {kind?:string;match_scope?:string;protected?:boolean;protected_anchors?:number;matched_protected_anchors?:number;limited?:boolean};
  role?: string | null;
  ref: string;
  kind: string;
  text: string;
  content?: string;
  snippet?: string;
  state?: string;
  evidence?: string;
  occurred_at?: string | null;
  conversation_ref?: string;
  conversation_title?: string;
  platform?: string;
  source_summary?: {
    count: number;
    sources: { conversation_title: string; platform: string }[];
  };
}
export interface EventLexicalCoverage {status?:string;version?:string|null;reason?:string;candidates_truncated?:boolean;query_terms?:number;postings_scanned?:number;query_terms_visited?:number;}
export interface SearchPage {
  coverage?: {attachments?:{status?:string;limited?:boolean;skipped?:number;semantic?:boolean};event_search?: string; cloud_fallback?: boolean; scopes?: {scope:string;state:string;query_encoding?:string;verification_limited?:boolean;lexical?:EventLexicalCoverage;fusion?:string;semantic?:{status?:string;reason?:string|null}}[]; lexical?:EventLexicalCoverage; fusion?:string; candidate_verification_limited?:boolean; local_event_semantic?: boolean; memory_search?: string; chunk_selection?: string; semantic_search?: string; semantic_error?: string; semantic_query_encoding?: string; semantic_coverage?: string; lexical_matching?: string; lexical_match?: string};
  results: SearchRow[];
  truncated?: boolean;
}
export interface MemoryReview {
  preview_id: string;
  operation: string;
  before: Memory;
  after: { content: string; protected: boolean; labels: string[] } | null;
  affected_events: number;
  affected_memories: number;
  requires_protected_approval: boolean;
  warning: string;
}
export interface ClientConfig {
  mcpServers: { recallcard: { command: string; args: string[] } };
}
export interface BrowserRegistration {
  registered: boolean;
  registration: string;
  capture_enabled: boolean;
  note: string;
}
export type Destination =
  | "background"
  | "memories"
  | "sources"
  | "connections"
  | "activity"
  | "settings"
  | "search";
export const scopeLabel = (scope: string) =>
  scope === "personal" ? "个人空间" : scope.replace(/^project[:/]/, "");
export const platformLabel = (platform: string) =>
  ({
    chatgpt: "ChatGPT",
    "chatgpt-export": "ChatGPT",
    deepseek: "DeepSeek",
    "deepseek-export": "DeepSeek",
    qwen: "Qwen",
    "qwen-export": "Qwen",
    claude: "Claude",
    "recallcard-desktop": "本地笔记",
    "manual-ai-paste": "手动粘贴的 AI 回答",
    codex: "Codex",
  })[platform.toLowerCase()] || platform;
export const evidenceLabel = (evidence: string) =>
  ({
    user_explicit: "明确表达",
    observed: "观察所得",
    assistant_suggestion: "AI 建议",
    UserExplicit: "明确表达",
    Observed: "观察所得",
    AssistantSuggestion: "AI 建议",
  })[evidence] || "证据未标注";
export const statusLabel = (status: string) =>
  ({
    active: "有效",
    tentative: "待确认",
    stale: "已过期",
    retracted: "已撤回",
    superseded: "已替代",
  })[status] || status;
export function dateLabel(date: string | null | undefined, full = false) {
  if (!date || Number.isNaN(new Date(date).getTime())) return "时间未知";
  return new Date(date).toLocaleString(
    "zh-CN",
    full
      ? {
          year: "numeric",
          month: "long",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
          hour12: false,
        }
      : { month: "short", day: "numeric" },
  );
}
export const titleOf = (content: string, max = 44) =>
  content
    .trim()
    .split(/\n/)[0]
    .replace(/^#+\s*/, "")
    .slice(0, max) || "未命名记忆";
export const eventText = (event: EventRecord) =>
  event.parts?.length
    ? event.parts.map((part) => part.text || "").join("\n")
    : event.content;
