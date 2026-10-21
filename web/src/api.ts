// 与本地服务通信：同源 Cookie 鉴权，出错只抛中文消息。

export type Role = 'user' | 'assistant' | 'tool' | 'system';

export interface RecordRow {
  ref: string;
  kind: 'event' | 'memory';
  text: string;
  text_truncated?: boolean;
  role?: Role;
  occurred_at?: string | null;
  state?: string;
  evidence?: string;
  session_ref?: string | null;
  conversation_title?: string;
  platform?: string;
  time_note?: string;
  captured_at?: string | null;
  content_retained?: boolean;
  retention?: string;
}

/** read / sources 返回的是记录信封，正文在 record 里。 */
export interface RecordEnvelope {
  ref: string;
  record?: {
    content?: string;
    occurred_at?: string | null;
    captured_at?: string | null;
    time_note?: string;
    [key: string]: unknown;
  };
  role?: string;
  platform?: string;
  conversation_title?: string;
  content_retained?: boolean;
  retention?: string;
}

export function toRow(envelope: RecordEnvelope): RecordRow {
  const record = envelope.record ?? {};
  return {
    ref: envelope.ref,
    kind: envelope.ref.startsWith('memory:') ? 'memory' : 'event',
    text: typeof record.content === 'string' ? record.content : '',
    role: envelope.role as Role | undefined,
    occurred_at: record.occurred_at ?? null,
    captured_at: record.captured_at ?? null,
    time_note: record.time_note,
    platform: envelope.platform,
    conversation_title: envelope.conversation_title,
    content_retained: envelope.content_retained,
    retention: envelope.retention,
  };
}

export interface VaultInfo {
  session_id: string;
  root: string;
  display_name: string;
  scopes: string[];
  event_count: number;
  memory_count: number;
  health: { ok?: boolean; [key: string]: unknown };
}

export interface IndexStatus {
  configured: boolean;
  complete?: boolean;
  provider?: string;
  model?: string;
  dimensions?: number;
  documents?: number;
  by_kind?: Record<string, number>;
  reason?: string;
  scope?: string[];
}

export interface SettingsView {
  embedding: {
    provider: 'local' | 'cloud';
    endpoint: string;
    model: string;
    dimensions: number;
    has_key: boolean;
  };
  llm: {
    base_url: string;
    model: string;
    max_output_tokens: number;
    has_key: boolean;
  };
}

export interface ListResponse {
  results: RecordRow[];
  total: number;
  truncated: boolean;
  coverage?: { semantic_search?: string };
}

export interface StateResponse {
  vault: VaultInfo;
  scope: string;
  index: IndexStatus;
  settings: SettingsView | null;
  settings_error?: string;
}

export interface MemoryRow {
  id: string;
  revision: number;
  content: string;
  text_truncated: boolean;
  status: string;
  protected: boolean;
  evidence: string;
  source_refs: string[];
  hidden: boolean;
  can_restore: boolean;
  updated_at: string;
}

export interface DreamChange {
  operation: string;
  before: { id: string; revision: number; status: string; protected: boolean; content: string } | null;
  after: { id: string; revision: number; status: string; protected: boolean; content: string; evidence: string; source_refs: string[] } | null;
  diagnostics?: string[];
}

export interface DreamPreview {
  preview_id: string;
  review: {
    can_apply: boolean;
    requires_protected_approval: boolean;
    already_applied?: boolean;
    diagnostics?: string[];
    changes: DreamChange[];
  };
}

async function call<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    throw new Error(`服务返回的不是 JSON（HTTP ${response.status}）`);
  }
  if (!response.ok) {
    const message = (data as { error?: string } | null)?.error;
    throw new Error(message || `请求失败（HTTP ${response.status}）`);
  }
  return data as T;
}

export const api = {
  state: () => call<StateResponse>('/api/state'),
  setScope: (scope: string) => call<{ ok: boolean }>('/api/scope', { scope }),
  search: (query: string, target: string) => call<ListResponse>('/api/search', { query, target }),
  browse: (target: string) => call<ListResponse>('/api/browse', { target }),
  read: (ref: string) =>
    call<{ results: RecordEnvelope[]; truncated: boolean }>('/api/read', { ref }),
  sources: (ref: string) =>
    call<{ results: RecordEnvelope[]; truncated: boolean }>('/api/sources', { ref }),
  memories: (includeHidden: boolean, offset = 0) =>
    call<{ memories: MemoryRow[]; total: number; next_offset: number | null }>('/api/memories', {
      includeHidden,
      offset,
    }),
  background: () => call<Record<string, unknown>>('/api/background'),
  settings: () => call<SettingsView>('/api/settings'),
  saveSettings: (patch: unknown) => call<SettingsView>('/api/settings', patch),
  resetSettings: () => call<SettingsView>('/api/settings/reset', {}),
  buildIndex: () => call<Record<string, unknown>>('/api/index/build', {}),
  openVault: (path: string, create: boolean) =>
    call<{ vault: VaultInfo; scope: string }>('/api/vault/open', { path, create }),
  connect: () => call<{ config: unknown; binary: string }>('/api/connect'),
  dreamSources: (since: string, until: string) =>
    call<{ count: number; total_bytes: number; samples: RecordRow[]; llm_ready: { base_url: boolean } }>(
      '/api/dream/sources',
      { since, until },
    ),
  dreamPreview: (since: string, until: string) =>
    call<{ preview: DreamPreview; source_count: number; memory_count: number }>('/api/dream/preview', {
      since,
      until,
    }),
  dreamApply: (previewId: string, approveProtected: boolean) =>
    call<{ receipt: { changes: unknown[] } }>('/api/dream/apply', { previewId, approveProtected }),
};

export const roleLabel: Record<string, string> = {
  user: '用户原话',
  assistant: 'AI 回复',
  tool: '工具结果',
  system: '系统消息',
};

export const stateLabel: Record<string, string> = {
  active: '有效',
  tentative: '待确认',
  stale: '已过期',
  retracted: '已撤回',
  captured: '原始记录',
  superseded: '已被替代',
};

export const evidenceLabel: Record<string, string> = {
  user_explicit: '用户明确表达',
  UserExplicit: '用户明确表达',
  observed: '观察所得',
  Observed: '观察所得',
  assistant_suggestion: 'AI 建议，尚非用户事实',
  AssistantSuggestion: 'AI 建议，尚非用户事实',
};

export function formatTime(value?: string | null): string {
  if (!value) return '时间未知';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '时间未知';
  return date.toLocaleString('zh-CN', { hour12: false });
}
