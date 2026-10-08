import test from 'node:test';
import assert from 'node:assert/strict';
import { validateArguments, makeCapsule } from '../protocol.js';
import { relayState, setRelay, observeRelay, capsuleText, finalReceipt } from '../relay-state.js';
import '../clipboard-adapter.js';
const session = { nonce: 'a'.repeat(48), session_ref: 'chatgpt:synthetic', preview: { id: 'r_2', nonce: 'a'.repeat(48), text: '完整资料', delivery: 'draft' } };
const final = (changes = {}) => '合成本轮回答\n```recallcard-final\n' + JSON.stringify({ protocol: 'recallcard.final/1', nonce: session.nonce, session_ref: session.session_ref, after_request_id: 'r_2', ...changes }) + '\n```';
test('read/sources byte预算与游标前后端合同完整，禁止别名歧义及跨ref续读', () => {
  for (const action of ['read', 'sources']) {
    assert.deepEqual(validateArguments(action, { refs: ['event:evt_synthetic'], cursor: 'cursor_complete', budget_bytes: 32768 }), { refs: ['event:evt_synthetic'], cursor: 'cursor_complete', budget_bytes: 32768 });
    validateArguments(action, { refs: ['memory:mem_synthetic@1'], offset_bytes: 0, budget_tokens: 512 });
    for (const args of [{ budget_bytes: 4000, budget_tokens: 4000 }, { budget_bytes: 511 }, { budget_bytes: 32769 }, { offset_bytes: -1 }, { cursor: 'next', offset_bytes: 0 }, { refs: ['event:evt_a', 'event:evt_b'], cursor: 'next' }, { refs: ['view:profile'], cursor: 'next' }]) assert.throws(() => validateArguments(action, { refs: ['event:evt_synthetic'], ...args }));
  }
});
test('capsule逐字保留范围、快照、游标和pending_refs，不把partial称全文', () => {
  const result = { status: 'partial', text_range: { start: 0, end: 9, total: 18 }, snapshot: 'v1', next_cursor: 'next-1', pending_refs: ['event:evt_other'], text: '第一段' };
  const capsule = makeCapsule({ request_id: 'r_2', action: 'read' }, result, session);
  const parsed = JSON.parse(capsule.text.slice(capsule.text.indexOf('{')));
  assert.deepEqual(parsed.result, result);
  const bootstrap = makeCapsule({ request_id: 'boot', action: 'bootstrap' }, {}, session);
  assert.match(bootstrap.text, /budget_exhausted 应增加预算/); assert.match(bootstrap.text, /next_cursor 续读/);
});
test('草稿消失、拷贝、旧final、不同nonce和未完成回复均不能称结果就绪', () => {
  for (const observation of [null, { sent: [], last: { role: 'assistant', text: final(), complete: true, after_sent: true } }, { sent: [{ id: 'r_2', message_id: 'u2' }], last: { role: 'assistant', text: final(), complete: false, after_sent: true } }, { sent: [{ id: 'r_2', message_id: 'u2' }], last: { role: 'assistant', text: final(), complete: true, after_sent: false } }, { sent: [{ id: 'r_2', message_id: 'u2' }], last: { role: 'assistant', text: final({ nonce: 'wrong' }), complete: true, after_sent: true } }]) {
    const state = structuredClone(session); assert.equal(observeRelay(state, observation), false); assert.notEqual(relayState(state).phase, 'result_ready');
  }
});
test('只接受当前轮资料后的完整final，回执不改变授权或执行写动作', () => {
  const state = structuredClone(session);
  assert.equal(observeRelay(state, { sent: [{ id: 'r_2', message_id: 'u2' }], last: { role: 'assistant', text: final(), complete: true, after_sent: true } }), true);
  assert.equal(state.preview.delivery, 'observed_sent'); assert.equal(state.relay.phase, 'result_ready');
  for (const text of [final({ after_request_id: 'r_old' }), final() + '\n还没说完', final().slice(0, -3), '```recallcard-action\n{}\n```\n' + final()]) assert.equal(finalReceipt(text, session, 'r_2'), false);
  assert.match(capsuleText(session.preview), /^\[RecallCard a{48}:r_2\]/); assert.equal(state.automation, undefined);
});
test('暂停/恢复为显式状态，不能继承完成标识', () => {
  const state = structuredClone(session); setRelay(state, 'result_ready', '就绪'); setRelay(state, 'paused', '暂停', { paused: true });
  assert.equal(relayState(state).result_ready, false); assert.equal(relayState(state).paused, true); assert.throws(() => setRelay(state, 'sent', '无效'));
});
test('剪贴板接口仅显式活动页面写入；无readText、无权限申请或后台读取', async () => {
  const writes = []; const doc = { visibilityState: 'visible', hasFocus: () => true };
  const navigator = { clipboard: { writeText: async text => writes.push(text), readText: () => { throw new Error('不应读取'); } } };
  assert.deepEqual(globalThis.RecallCardClipboard.capability(doc, navigator), { write: true, read: 'paste_only', monitoring: false, permission_requested: false });
  await assert.rejects(globalThis.RecallCardClipboard.copy('合成数据', { doc, navigator, userGesture: false }));
  await globalThis.RecallCardClipboard.copy('合成数据', { doc, navigator, userGesture: true });
  await assert.rejects(globalThis.RecallCardClipboard.copy('隐藏数据', { doc: { ...doc, visibilityState: 'hidden' }, navigator, userGesture: true }));
  await assert.rejects(globalThis.RecallCardClipboard.copy('无焦点数据', { doc: { ...doc, hasFocus: () => false }, navigator, userGesture: true }));
  assert.deepEqual(writes, ['合成数据']);
});
