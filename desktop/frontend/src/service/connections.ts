export interface ConnectionGrant {
  client_kind: 'browser' | 'claude_code';
  host_identity: string;
  installation_id: string | null;
  platform: 'chatgpt' | 'deepseek' | 'claude_code';
  recall_scopes: string[];
  capture_scopes: string[];
  provider_disclosure: boolean;
  auto_capture: boolean;
  auto_recall: boolean;
}
export interface ConnectionEntry {
  id: string; grant: ConnectionGrant; permission_revision: number;
  state: 'configured_unverified' | 'reachable' | 'read_succeeded' | 'failed' | 'revoked' | 'needs_setup';
  configured_at: string; last_handshake_at: string | null; last_bootstrap_at: string | null;
  last_read_at: string | null; last_capture_at: string | null; last_error: string | null; revoked: boolean;
}
export interface PairingRequest {
  request_id: string; host_identity: string; installation_id: string; platform: 'chatgpt' | 'deepseek';
  requested_at: string; expires_at: string; recall_scope_cap: string[]; capture_scope_cap: string[];
}
export interface ConnectionInventory {
  entries: ConnectionEntry[]; pending_pairings: PairingRequest[]; supported_protocols: string[]; identity_notice: string;
}
export const connectionLabel = (state: ConnectionEntry['state']) => ({ configured_unverified: '已配置 · 尚未验证', reachable: '本机可达', read_succeeded: '最近读取成功', failed: '最近操作未完成', revoked: '已撤销', needs_setup: '需要重新设置' })[state];
export function parseConnectionCode(code: string): { host_identity: string; installation_id: string } | null {
  const match = /^recallcard-connect\/1:([a-p]{32}):([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})$/i.exec(code.trim());
  return match ? { host_identity: match[1].toLowerCase(), installation_id: match[2].toLowerCase() } : null;
}

export interface AgentConnectionConfigs {
  connection_id: string;
  registered: false;
  mcp: Record<string, unknown>;
  hooks: Record<string, unknown>;
  note: string;
}
export function validateAgentConfigs(value: unknown, id: string): AgentConnectionConfigs {
  const result = value as AgentConnectionConfigs | null;
  const mcp = result?.mcp as { mcpServers?: { recallcard?: { args?: unknown } } } | undefined;
  const hooks = result?.hooks as { hooks?: { SessionStart?: { hooks?: { args?: unknown }[] }[] } } | undefined;
  const bound = (args: unknown) => Array.isArray(args) && args.includes('--connection-id') && args[args.indexOf('--connection-id') + 1] === id;
  if (!result || result.connection_id !== id || result.registered !== false || !bound(mcp?.mcpServers?.recallcard?.args) || !bound(hooks?.hooks?.SessionStart?.[0]?.hooks?.[0]?.args)) throw new Error('接入组件没有返回绑定此连接的完整配置，请更新后重试');
  return result;
}
