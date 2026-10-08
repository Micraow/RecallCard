import assert from 'node:assert/strict';
import test from 'node:test';
import { connectionLabel, parseConnectionCode, validateAgentConfigs } from '../src/service/connections.ts';
const id = 'conn_synthetic';
const configs = () => ({ connection_id: id, registered: false, mcp: { mcpServers: { recallcard: { args: ['mcp', '--connection-id', id] } } }, hooks: { hooks: { SessionStart: [{ hooks: [{ args: ['agent-hook', '--connection-id', id] }] }] } }, note: '合成配置，未安装' });
test('连接编号只接受固定扩展和本机安装身份，不推导账号认证', () => {
  const valid = `recallcard-connect/1:${'a'.repeat(32)}:11111111-1111-4111-8111-111111111111`;
  assert.equal(parseConnectionCode(valid)?.installation_id, '11111111-1111-4111-8111-111111111111');
  for (const value of ['a'.repeat(32), valid + ':grant=true', valid.replace('connect/1', 'connect/2'), valid.replace('11111111-', 'invalid-')]) assert.equal(parseConnectionCode(value), null);
});
test('MCP与生命周期片段必须同时固定当前连接ID，旧宽配置被拒绝', () => {
  assert.equal(validateAgentConfigs(configs(), id).registered, false);
  const missing = configs(); missing.hooks.hooks.SessionStart[0].hooks[0].args = ['agent-hook', '--scope', 'personal'];
  assert.throws(() => validateAgentConfigs(missing, id), /完整配置/);
  assert.throws(() => validateAgentConfigs(configs(), 'conn_other'), /完整配置/);
  assert.throws(() => validateAgentConfigs({ ...configs(), registered: true }, id), /完整配置/);
});
test('配置、握手和成功读取不合并为已同步或模型已理解', () => {
  assert.match(connectionLabel('configured_unverified'), /尚未验证/);
  assert.equal(connectionLabel('reachable'), '本机可达');
  assert.equal(connectionLabel('read_succeeded'), '最近读取成功');
  assert.equal(connectionLabel('needs_setup'), '需要重新设置');
});

import { healthLabel, healthTone, recentConnectionReceipts, validateSetupPlan, type ConnectionEntry, type ConnectionHealth, type ConnectionSetupPlan } from '../src/service/connections.ts';
const health: ConnectionHealth = { connection_id: id, permission_revision: 1, installation: 'configured', readiness: 'read_verified', capture: 'disabled', checked_at: '2026-10-08T08:32:00Z', last_read_at: null, last_capture_at: null, verification_scope: 'local_cli', message: '本机试读，不代表宿主调用' };
const grant = { client_kind: 'codex', host_identity: 'synthetic-codex', installation_id: null, platform: 'codex', recall_scopes: ['personal'], capture_scopes: [], provider_disclosure: true, auto_capture: false, auto_recall: true } as const;
const plan = (): ConnectionSetupPlan => ({ plan_id: 'plan_synthetic', expires_at: '2099-01-01T00:00:00Z', client: 'codex', connection_id: id, permission_revision: null, grant_required: true, proposed_grant: { ...grant, recall_scopes: [...grant.recall_scopes], capture_scopes: [] }, scope: 'personal', project_dir: '/synthetic/project', binary_path: '/synthetic/bin/recallcard', server_key: 'recallcard', files: [{ path: '/synthetic/project/AGENTS.md', before_digest: null, after_digest: 'synthetic', managed_addition: '合成增量', changed: true }], notices: [], host_verified: false });
test('本机试读和客户端读取的文案严格分开', () => {
  assert.equal(healthLabel(health), '本机试读通过');
  assert.equal(healthLabel({ ...health, verification_scope: 'client_request' }), '收到客户端读取');
  assert.equal(healthLabel({ ...health, verification_scope: 'none', readiness: 'awaiting_host' }), '配置已写入，等待首次读取');
  assert.equal(healthLabel({ ...health, installation: 'not_inspected', verification_scope: 'none', readiness: 'awaiting_host' }), '尚未检查安装');
});
test('配置变化、暂停、撤销或无证据不显示绿色成功', () => {
  assert.equal(healthTone(health), 'success');
  assert.equal(healthTone({ ...health, installation: 'configuration_changed' }), 'warning');
  assert.equal(healthTone({ ...health, readiness: 'local_unavailable' }), 'warning');
  for (const readiness of ['revoked', 'paused', 'awaiting_host'] as const) assert.equal(healthTone({ ...health, readiness }), 'muted');
  assert.equal(healthTone({ ...health, verification_scope: 'none' }), 'muted');
});
test('仅配置与握手没有增强回执，捕获不标为记忆整理', () => {
  const entry: ConnectionEntry = { id, grant: { ...grant, recall_scopes: ['personal'], capture_scopes: [] }, permission_revision: 1, state: 'reachable', configured_at: health.checked_at, last_handshake_at: health.checked_at, last_bootstrap_at: null, last_read_at: null, last_capture_at: null, last_error: null, revoked: false };
  assert.deepEqual(recentConnectionReceipts([entry]), []);
  const receipts = recentConnectionReceipts([{ ...entry, last_read_at: '2026-10-08T08:30:00Z', last_capture_at: health.checked_at }]);
  assert.equal(receipts[0].kind, 'capture');
  assert.match(receipts[0].description, /不代表已完成记忆整理/);
  assert.equal(receipts[1].title, '收到按需读取');
});
test('安装预览固定客户端、项目和唯一范围，拒绝跨范围或伪造宿主验证', () => {
  assert.equal(validateSetupPlan(plan(), 'codex', 'personal', '/synthetic/project').plan_id, 'plan_synthetic');
  for (const changed of [{ ...plan(), client: 'claude_code' }, { ...plan(), scope: 'project:other' }, { ...plan(), project_dir: '/other' }, { ...plan(), host_verified: true }, { ...plan(), proposed_grant: { ...plan().proposed_grant, recall_scopes: ['personal', 'project:other'] } }]) assert.throws(() => validateSetupPlan(changed as ConnectionSetupPlan, 'codex', 'personal', '/synthetic/project'), /不一致/);
});

import { sameConnectionGrant } from '../src/service/connections.ts';
test('原样查看权限不生成新授权版本，范围顺序不算变化', () => {
  const current = plan().proposed_grant;
  assert.equal(sameConnectionGrant(current, { ...current }), true);
  assert.equal(sameConnectionGrant({ ...current, recall_scopes: ['personal', 'project:a'] }, { ...current, recall_scopes: ['project:a', 'personal'] }), true);
  assert.equal(sameConnectionGrant(current, { ...current, auto_recall: false }), false);
  assert.equal(sameConnectionGrant(current, { ...current, recall_scopes: ['project:a'] }), false);
});
