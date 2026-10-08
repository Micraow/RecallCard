export interface ConnectionGrant {
  client_kind: 'browser' | 'claude_code' | 'codex';
  host_identity: string;
  installation_id: string | null;
  platform: 'chatgpt' | 'deepseek' | 'claude_code' | 'codex';
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

export function connectionName(entry: ConnectionEntry): string {
  if (entry.grant.client_kind === 'claude_code') return 'Claude Code';
  return ({ chatgpt: 'ChatGPT', deepseek: 'DeepSeek', codex: 'Codex' } as Record<string, string>)[entry.grant.platform] || entry.grant.platform;
}
export interface ConnectionReceipt { id: string; client: string; kind: 'bootstrap' | 'read' | 'capture'; title: string; description: string; at: string }
/** 授权、配置与握手不是增强成功；记录只证明客户端请求过资料。 */
export function recentConnectionReceipts(entries: ConnectionEntry[]): ConnectionReceipt[] {
  return entries.flatMap(entry => {
    const receipts: ConnectionReceipt[] = [];
    const client = connectionName(entry);
    if (entry.last_bootstrap_at) receipts.push({ id: `${entry.id}:bootstrap`, client, kind: 'bootstrap', title: '收到会话背景读取', description: '只读背景入口返回了结果', at: entry.last_bootstrap_at });
    if (entry.last_read_at) receipts.push({ id: `${entry.id}:read`, client, kind: 'read', title: '收到按需读取', description: '查询了授权范围内的资料', at: entry.last_read_at });
    if (entry.last_capture_at) receipts.push({ id: `${entry.id}:capture`, client, kind: 'capture', title: '已保存原始对话', description: '保存回执，不代表已完成记忆整理', at: entry.last_capture_at });
    return receipts;
  }).sort((a, b) => b.at.localeCompare(a.at));
}


export type AgentClient = 'codex' | 'claude_code';
export interface ConnectionSetupPlan {
  plan_id: string;
  expires_at: string;
  client: AgentClient;
  connection_id: string;
  permission_revision: number | null;
  grant_required: boolean;
  proposed_grant: ConnectionGrant;
  scope: string;
  project_dir: string;
  binary_path: string;
  server_key: string;
  files: { path: string; before_digest: string | null; after_digest: string; managed_addition: string; changed: boolean; backup_path?: string | null }[];
  notices: string[];
  host_verified: false;
}
export interface ConnectionHealth {
  connection_id: string;
  permission_revision: number;
  installation: 'not_inspected' | 'not_installed' | 'configured' | 'configuration_changed';
  readiness: 'permission_required' | 'awaiting_host' | 'local_unavailable' | 'read_verified' | 'revoked' | 'paused';
  capture: 'disabled' | 'enabled_unverified' | 'verified';
  checked_at: string;
  last_read_at: string | null;
  last_capture_at: string | null;
  last_local_verified_at?: string | null;
  read_verification?: unknown;
  verification_scope: 'local_cli' | 'client_request' | 'none';
  message: string;
}
export interface ConnectionSetupResult {
  connection_id: string;
  configuration: 'written';
  paths: string[];
  health: ConnectionHealth | null;
  verification_error?: unknown;
  notices: string[];
}
export function healthLabel(health: ConnectionHealth): string {
  if (health.readiness === 'revoked') return '访问已撤销';
  if (health.readiness === 'paused') return '自动准备已暂停';
  if (health.installation === 'configuration_changed') return '配置有变化，需要检查';
  if (health.readiness === 'local_unavailable') return '本机试读未通过';
  if (health.readiness === 'permission_required') return '需要范围授权';
  if (health.readiness === 'read_verified' && health.verification_scope === 'client_request') return '收到客户端读取';
  if (health.readiness === 'read_verified' && health.verification_scope === 'local_cli') return '本机试读通过';
  return health.installation === 'configured' ? '配置已写入，等待首次读取' : health.installation === 'not_inspected' ? '尚未检查安装' : '等待安装';
}
export function validateSetupPlan(value: ConnectionSetupPlan, client: AgentClient, scope: string, projectDir: string): ConnectionSetupPlan {
  if (!value || value.client !== client || value.scope !== scope || value.project_dir !== projectDir || !value.plan_id || !value.connection_id || !Array.isArray(value.files) || !value.files.length || !value.proposed_grant || value.proposed_grant.client_kind !== client || value.proposed_grant.platform !== client || !Array.isArray(value.proposed_grant.recall_scopes) || !Array.isArray(value.proposed_grant.capture_scopes) || value.proposed_grant.recall_scopes.length !== 1 || value.proposed_grant.recall_scopes[0] !== scope || value.proposed_grant.capture_scopes.length !== 0 || !value.proposed_grant.auto_recall || !value.proposed_grant.provider_disclosure || value.host_verified !== false) throw new Error('安装预览与当前客户端或资料范围不一致，请重新选择项目');
  return value;
}

export function healthTone(health: ConnectionHealth): 'success' | 'warning' | 'muted' {
  if (health.installation === 'configuration_changed' || health.readiness === 'local_unavailable') return 'warning';
  if (health.readiness === 'read_verified' && health.verification_scope !== 'none' && health.installation === 'configured') return 'success';
  return 'muted';
}

export interface BrowserSetupInfo {
  distribution: 'unpublished_test_package';
  extension_dir: string | null;
  available: boolean;
  open_directory_available: boolean;
  store_url: null;
  fixed_extension_id: null;
  instructions: string[];
}

export function sameConnectionGrant(left: ConnectionGrant, right: ConnectionGrant): boolean {
  const scopesEqual = (a: string[], b: string[]) => a.length === b.length && [...a].sort().every((scope, index) => scope === [...b].sort()[index]);
  return left.client_kind === right.client_kind && left.host_identity === right.host_identity && left.installation_id === right.installation_id && left.platform === right.platform && scopesEqual(left.recall_scopes, right.recall_scopes) && scopesEqual(left.capture_scopes, right.capture_scopes) && left.provider_disclosure === right.provider_disclosure && left.auto_capture === right.auto_capture && left.auto_recall === right.auto_recall;
}
