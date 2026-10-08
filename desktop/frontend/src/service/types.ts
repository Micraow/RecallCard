export interface Workspace {
  session_id: string; root: string; display_name: string; scopes: string[];
  event_count: number; memory_count: number; health: unknown;
}
export interface RestoredWorkspace { vault: Workspace; scope: string }
export interface MemoryRow {
  id: string; revision: number; content: string; text_truncated?: boolean;
  status: 'active' | 'tentative' | 'stale' | 'retracted' | 'superseded';
  protected: boolean; evidence: string; source_refs: string[]; hidden: boolean;
  can_restore?: boolean; updated_at: string;
}
export interface Memory extends MemoryRow {
  labels: string[]; scope: string; recorded_at: string; authority: string;
  time_note?: string; valid_from?: string | null; valid_to?: string | null;
}
export interface MemoryPage { memories: MemoryRow[]; total: number; next_offset: number | null }
export interface Source { platform: string; conversation_id?: string; message_id?: string; url?: string | null }
export interface EventRecord {
  id: string; content: string; parts?: { text?: string }[]; role: string;
  occurred_at: string | null; captured_at: string; source: Source;
  metadata?: { conversation_title?: string };
}
export interface Conversation {
  session_ref: string; title: string; platform: string; source_url: string | null;
  message_count: number; captured_at: string; coverage: string;
}
export interface ConversationPage { conversations: Conversation[]; total: number; next_offset: number | null; note: string }
export interface Message {
  ref: string; role: string; text: string; occurred_at: string | null;
  captured_at: string; text_truncated: boolean; source: Source;
  branch?: { parent_ref: string | null; is_branch_end: boolean; relationship_known: boolean; child_count: number; gap_before: boolean };
}
export interface MessagesPage {
  messages: Message[]; total: number; next_offset: number | null; offset: number;
  title: string; platform: string; session_ref: string; coverage: string;
  order_known: boolean; order_kind: string;
}
export interface SearchRow {
  ref: string; kind: string; text: string; content?: string; snippet?: string;
  state?: string; evidence?: string; occurred_at?: string | null;
  conversation_ref?: string; conversation_title?: string; platform?: string;
  source_summary?: { count: number; sources: { conversation_title: string; platform: string }[] };
}
export interface SearchPage { results: SearchRow[]; truncated?: boolean }
export interface MemoryReview {
  preview_id: string; operation: string; before: Memory;
  after: { content: string; protected: boolean; labels: string[] } | null;
  affected_events: number; affected_memories: number;
  requires_protected_approval: boolean; warning: string;
}
export interface ClientConfig { mcpServers: { recallcard: { command: string; args: string[] } } }
export interface BrowserRegistration { registered: boolean; registration: string; capture_enabled: boolean; note: string }
export type Destination = 'memories' | 'sources' | 'connections' | 'activity' | 'settings' | 'search';
export const scopeLabel = (scope: string) => scope === 'personal' ? '个人空间' : scope.replace(/^project[:/]/, '');
export const platformLabel = (platform: string) => ({ chatgpt: 'ChatGPT', deepseek: 'DeepSeek', claude: 'Claude', 'recallcard-desktop': '本地笔记', 'codex': 'Codex' })[platform.toLowerCase()] || platform;
export const evidenceLabel = (evidence: string) => ({ user_explicit: '明确表达', observed: '观察所得', assistant_suggestion: 'AI 建议', UserExplicit: '明确表达', Observed: '观察所得', AssistantSuggestion: 'AI 建议' })[evidence] || '证据未标注';
export const statusLabel = (status: string) => ({ active: '有效', tentative: '待确认', stale: '已过期', retracted: '已撤回', superseded: '已替代' })[status] || status;
export function dateLabel(date: string | null | undefined, full = false) {
  if (!date || Number.isNaN(new Date(date).getTime())) return '时间未知';
  return new Date(date).toLocaleString('zh-CN', full ? { year: 'numeric', month: 'long', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false } : { month: 'short', day: 'numeric' });
}
export const titleOf = (content: string, max = 44) => (content.trim().split(/\n/)[0].replace(/^#+\s*/, '').slice(0, max) || '未命名记忆');
export const eventText = (event: EventRecord) => event.parts?.length ? event.parts.map(part => part.text || '').join('\n') : event.content;
