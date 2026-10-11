import test from 'node:test';
import assert from 'node:assert/strict';
import { validateArguments, makeCapsule } from '../protocol.js';

test('网页接力保留显式角色、时间和当前路径过滤', () => {
  for (const role of ['user', 'assistant', 'tool', 'system']) {
    const args = { query: '合成决定', target: 'events', event_filter: { role, occurred_from: '2026-01-01T00:00:00Z', occurred_until: '2026-02-01T08:00:00+08:00', on_current_path: false } };
    assert.equal(validateArguments('search', args), args);
  }
  assert.doesNotThrow(() => validateArguments('search', { query: '合成', event_filter: { role: null, occurred_from: null, occurred_until: null, on_current_path: null } }));
});
test('不把事件角色过滤当Memory事实等级，也不允许注入权限', () => {
  for (const event_filter of [[], 'user', { role: 'developer' }, { role: 'USER' }, { on_current_path: 'false' }, { scope: 'private' }, { occurred_from: 'yesterday' }, { occurred_from: '2026-02-02T00:00:00Z', occurred_until: '2026-02-01T00:00:00Z' }]) {
    assert.throws(() => validateArguments('search', { query: '合成', event_filter }));
  }
  for (const target of ['memories', 'views']) assert.throws(() => validateArguments('search', { query: '合成', target, event_filter: { role: 'user' } }));
});
test('相邻来源读取是显式布尔选项，目录不伪装事件关系', () => {
  for (const action of ['read', 'sources']) {
    assert.doesNotThrow(() => validateArguments(action, { refs: ['event:evt_synthetic'], include_adjacent: true }));
    assert.throws(() => validateArguments(action, { refs: ['event:evt_synthetic'], include_adjacent: 'true' }));
  }
  assert.throws(() => validateArguments('read', { refs: ['view:nav/_root'], include_adjacent: true }));
  assert.doesNotThrow(() => validateArguments('read', { refs: ['view:nav/_root'], include_adjacent: false }));
});
test('接力资料保留说话角色和过滤说明，不推断用户已采纳助手建议', () => {
  const capsule = makeCapsule({ action: 'bootstrap', request_id: 'synthetic' }, { results: [{ role: 'assistant', text: '合成建议' }] }, { nonce: 'synthetic-nonce', session_ref: 'chatgpt:synthetic' });
  assert.match(capsule.text, /"role": "assistant"/);
  assert.match(capsule.text, /event_filter/);
  assert.match(capsule.text, /助手建议不等于用户决定/);
});
