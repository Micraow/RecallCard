/** 合成连接场景只由 demo.html 引入，不访问本机配置、凭据或用户资料。 */
import type { AgentClient, ConnectionEntry, ConnectionGrant, ConnectionHealth, ConnectionSetupPlan, PairingRequest } from '../service/connections';
const now = '2026-10-08T08:32:00Z';
export function createConnectionDemo(mode: string) {
  let entries: ConnectionEntry[] = [];
  let pairings: PairingRequest[] = [];
  let offline = mode === 'connections-offline';
  const plans = new Map<string, ConnectionSetupPlan>();
  const health = new Map<string, ConnectionHealth>();
  const pair = () => { pairings = [{ request_id: 'pair_demo_browser', host_identity: 'a'.repeat(32), installation_id: '11111111-1111-4111-8111-111111111111', platform: 'chatgpt', requested_at: now, expires_at: '2099-01-01T00:00:00Z', recall_scope_cap: ['personal'], capture_scope_cap: ['personal'] }]; };
  const configure = (grant: ConnectionGrant, expectedRevision?: number | null): ConnectionEntry => {
    const existing = entries.find(entry => entry.grant.host_identity === grant.host_identity && entry.grant.installation_id === grant.installation_id && entry.grant.platform === grant.platform);
    if (expectedRevision !== undefined && expectedRevision !== (existing?.permission_revision ?? null)) throw new Error('连接授权已变化，请刷新后再确认');
    if (existing && !existing.revoked && JSON.stringify(existing.grant) === JSON.stringify(grant)) return existing;
    const entry: ConnectionEntry = { id: existing?.id || `conn_demo_${grant.client_kind}_${entries.length}`, grant, permission_revision: (existing?.permission_revision || 0) + 1, state: 'configured_unverified', configured_at: now, last_handshake_at: null, last_bootstrap_at: null, last_read_at: null, last_capture_at: null, last_error: null, revoked: false };
    entries = [...entries.filter(row => row.id !== entry.id), entry];
    const priorHealth = health.get(entry.id);
    if (priorHealth) health.set(entry.id, { ...priorHealth, permission_revision: entry.permission_revision, readiness: grant.auto_recall ? 'awaiting_host' : 'paused', verification_scope: 'none', last_read_at: null, last_capture_at: null, message: grant.auto_recall ? '授权已更新，等待新的客户端读取。' : '自动准备已暂停。' });
    return entry;
  };
  const hostRead = () => { for (const entry of entries) { entry.state = 'read_succeeded'; entry.last_read_at = now; entry.last_handshake_at = now; const checked = health.get(entry.id); if (checked) health.set(entry.id, { ...checked, readiness: 'read_verified', verification_scope: 'client_request', last_read_at: now, message: '合成客户端已请求读取；不代表模型理解了所有内容。' }); } };
  if (mode === 'pairing' || mode === 'slow-pairing') pair();
  if (mode === 'connected') {
    configure({ client_kind: 'browser', host_identity: 'a'.repeat(32), installation_id: '11111111-1111-4111-8111-111111111111', platform: 'chatgpt', recall_scopes: ['personal'], capture_scopes: ['personal'], provider_disclosure: true, auto_recall: true, auto_capture: true });
    hostRead();
    entries[0].last_capture_at = now;
  }
  Object.assign(window, { __DEMO_PAIR__: pair, __DEMO_HOST_READ__: hostRead, __DEMO_CONNECTIONS_OFFLINE__: (value: boolean) => { offline = value; } });
  return async (command: string, args: Record<string, unknown>): Promise<{ handled: boolean; value?: unknown }> => {
    if (!['connection_inventory', 'connection_configure', 'connection_approve_pairing', 'connection_revoke', 'choose_connection_project', 'connection_setup_plan', 'apply_connection_setup', 'connection_health', 'verify_connection', 'browser_setup_info', 'open_browser_extension_directory'].includes(command)) return { handled: false };
    const scope = String(args.scope || 'personal');
    if (offline) throw new Error('连接状态暂不可读取，请检查本机服务后重新检测');
    let value: unknown;
    if (command === 'browser_setup_info') value = { distribution: 'unpublished_test_package', extension_dir: mode === 'missing-extension' ? null : '/合成示例/RecallCard/extension', available: mode !== 'missing-extension', open_directory_available: mode !== 'missing-extension', store_url: null, fixed_extension_id: null, instructions: [] };
    if (command === 'open_browser_extension_directory') value = null;
    if (command === 'connection_inventory') value = { entries: entries.filter(entry => [...entry.grant.recall_scopes, ...entry.grant.capture_scopes].includes(scope)), pending_pairings: pairings.filter(request => [...request.recall_scope_cap, ...request.capture_scope_cap].includes(scope)), supported_protocols: ['synthetic'], identity_notice: '合成浏览器安装绑定，不代表网站账号认证。' };
    if (command === 'connection_configure' || command === 'connection_approve_pairing') {
      value = configure(args.grant as unknown as ConnectionGrant, args.expectedRevision as number | null);
      if (command === 'connection_approve_pairing') pairings = pairings.filter(request => request.request_id !== args.requestId);
    }
    if (command === 'connection_revoke') {
      const entry = entries.find(row => row.id === args.id)!;
      if (!entry || args.expectedRevision !== entry.permission_revision) throw new Error('连接授权已变化，请刷新后重试');
      entry.revoked = true; entry.state = 'revoked'; entry.permission_revision++;
      value = entry;
    }
    if (command === 'choose_connection_project') value = mode === 'cancel-project' ? null : '/合成示例/projects/atlas';
    if (command === 'connection_setup_plan') {
      const client = args.client as AgentClient;
      const grant: ConnectionGrant = { client_kind: client, host_identity: `${client}-project-demo`, installation_id: null, platform: client, recall_scopes: [scope], capture_scopes: [], provider_disclosure: true, auto_recall: true, auto_capture: false };
      const existing = entries.find(entry => entry.grant.host_identity === grant.host_identity && entry.grant.platform === client && entry.grant.recall_scopes.includes(scope));
      const plan: ConnectionSetupPlan = { plan_id: `plan_demo_${client}_${plans.size}`, expires_at: '2099-01-01T00:00:00Z', client, scope, project_dir: String(args.projectDir), binary_path: '/合成示例/bin/recallcard', connection_id: existing?.id || `conn_demo_${client}_${entries.length}`, permission_revision: existing?.permission_revision ?? null, grant_required: !existing, proposed_grant: grant, server_key: 'recallcard-demo', host_verified: false, files: [{ path: `${String(args.projectDir)}/${client === 'codex' ? '.codex/config.toml' : '.mcp.json'}`, before_digest: null, after_digest: 'synthetic_digest', managed_addition: '合成配置，保留现有文件内容\n--connection-id synthetic-connection', changed: true }, { path: `${String(args.projectDir)}/${client === 'codex' ? 'AGENTS.md' : 'CLAUDE.md'}`, before_digest: 'synthetic_before', after_digest: 'synthetic_after', managed_addition: '合成项目入口：按连接 ID 读取生成的上下文。', changed: true }], notices: ['合成示例仅验证界面，不会写入任何本机项目文件。'] };
      plans.set(plan.plan_id, plan); value = plan;
    }
    if (command === 'apply_connection_setup') {
      const plan = plans.get(String(args.planId));
      if (!plan) throw new Error('安装计划不存在，请重新选择项目');
      if (mode === 'install-error') throw new Error('安装结果暂不可确认，请检测当前连接');
      const entry = configure(plan.proposed_grant);
      const checked: ConnectionHealth = { connection_id: entry.id, permission_revision: entry.permission_revision, installation: 'configured', readiness: mode === 'verify-failed' ? 'local_unavailable' : 'read_verified', capture: 'disabled', checked_at: now, last_read_at: null, last_capture_at: null, verification_scope: mode === 'verify-failed' ? 'none' : 'local_cli', message: mode === 'verify-failed' ? '配置已写入，但本机读取组件暂不可用。' : '本机只读试验返回了结果，尚未收到宿主的读取。' };
      health.set(entry.id, checked); value = { connection_id: entry.id, configuration: 'written', paths: plan.files.map(file => file.path), health: mode === 'slow-verify-error' ? null : checked, verification_error: mode === 'slow-verify-error' ? { message: '本机读取程序暂不可用；配置已写入，请重新试读。', code: 'storage' } : null, notices: plan.notices };
    }
    if (command === 'connection_health' || command === 'verify_connection') {
      const id = String(args.connectionId);
      const previous = health.get(id);
      if (!previous) throw new Error('尚未找到这个项目的已安装连接');
      const entry = entries.find(row => row.id === id)!;
      if (command === 'verify_connection' && (entry.revoked || !entry.grant.auto_recall || !entry.grant.recall_scopes.includes(scope))) throw new Error('此连接尚未允许当前范围的读取');
      const checked: ConnectionHealth = { ...previous, permission_revision: entry.permission_revision, readiness: entry.revoked ? 'revoked' : !entry.grant.auto_recall ? 'paused' : command === 'verify_connection' ? 'read_verified' : previous.readiness, verification_scope: command === 'verify_connection' && !entry.last_read_at ? 'local_cli' : previous.verification_scope, message: command === 'verify_connection' ? '本机只读试验返回了结果，尚未收到宿主的读取。' : previous.message };
      health.set(id, checked); value = checked;
    }
    return { handled: true, value };
  };
}
