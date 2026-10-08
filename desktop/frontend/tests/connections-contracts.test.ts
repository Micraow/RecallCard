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
