import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseAction, strictJson, validateAction, routeFor, reserveRequest, makeCapsule, MAX_REQUESTS } from '../protocol.js';
const fixture = JSON.parse(await readFile(new URL('./fixtures/actions.json', import.meta.url)));
const action = (changes = {}) => ({ ...structuredClone(fixture.valid), ...changes });
const fence = (value) => '```recallcard-action\n' + JSON.stringify(value) + '\n```';
test('仅解析完整的当前会话 action，支持中文与 CRLF', () => {
  assert.deepEqual(parseAction(fence(action()).replace(/\n/g, '\r\n'), fixture.session), fixture.valid);
});
test('不执行未结束的流式块、不接受尾随说明或多个块', () => {
  const full = fence(action());
  for (const input of [full.slice(0, -3), full.slice(0, -20), '说明\n' + full, full + '\n更多', full + '\n' + full, JSON.stringify(action()), '```json\n{}\n```']) assert.throws(() => parseAction(input, fixture.session));
});
test('JSON 重复键、转义重复键、原型字段均拒绝', () => {
  for (const input of ['{"a":1,"a":2}', '{"a":1,"\\u0061":2}', '{"x":{"__proto__":{}}}', '{"x":{"constructor":1}}']) assert.throws(() => strictJson(input));
  assert.deepEqual(strictJson('{"a":[{"x":"quote \\\" / \\\\"},true,null,-1.5e2]}'), {a:[{x:'quote " / \\'},true,null,-150]});
});
test('schema 拒绝旧 nonce、错误会话、非字符串 id、未知字段及命令', () => {
  for (const changes of [{nonce:'old'}, {session_ref:'other'}, {request_id:123}, {request_id:'x/y'}, {protocol:'recallcard/1'}, {command:'ls'}, ...fixture.unsupported.map((name) => ({action:name}))]) assert.throws(() => validateAction(action(changes), fixture.session));
});
test('四个只读能力、批量引用和参数边界', () => {
  for (const name of ['bootstrap', 'read', 'sources']) {
    const args = name === 'bootstrap' ? {} : {refs:['event:evt_demo', 'memory:mem_demo@1']};
    assert.equal(validateAction(action({action:name, arguments:args}), fixture.session).action, name);
  }
  for (const args of [{query:'x', limit:0}, {query:'x', limit:21}, {query:'x', scope:'private'}, {query:'x', target:'files'}, {query:'x', budget_tokens:999999}, {query:'x', cursor:0}, {query:'x', as_of:'yesterday'}, {query:' '}, {query:'x'.repeat(2049)}]) assert.throws(() => validateAction(action({arguments:args}), fixture.session));
  for (const ref of fixture.bad_refs) assert.throws(() => validateAction(action({action:'read', arguments:{refs:[ref]}}), fixture.session));
  assert.throws(() => validateAction(action({action:'sources', arguments:{refs:['view:profile']}}), fixture.session));
  assert.throws(() => validateAction(action({action:'read', arguments:{refs:[]}}), fixture.session));
});
test('超大、过深 JSON 被拒绝', () => {
  assert.throws(() => parseAction('x'.repeat(16385), fixture.session));
  assert.throws(() => strictJson('['.repeat(15)+'0'+']'.repeat(15)));
});
test('重复请求、频率、会话上限不会通过淘汰旧记录绕开', () => {
  const state = reserveRequest({...fixture.session, used:[]}, action(), 10000);
  assert.throws(() => reserveRequest(state, action(), 12000), /已处理/);
  assert.throws(() => reserveRequest(state, action({request_id:'r_two'}), 10500), /频繁/);
  assert.throws(() => reserveRequest({...state, used:Array.from({length:MAX_REQUESTS}, (_,i)=>`r_${i}`)}, action({request_id:'new'}), 12000), /上限/);
});
test('只允许 ChatGPT 主站及受支持会话路径', () => {
  for (const url of ['https://chatgpt.com/', 'https://chatgpt.com/c/abc-123?x=1', 'https://chatgpt.com/g/g-demo/c/123']) assert.match(routeFor(url), /^https:\/\/chatgpt.com/);
  for (const url of ['http://chatgpt.com/c/123','https://chatgpt.com.evil.test/','https://evil.test/','https://chatgpt.com/auth/login','https://u:p@chatgpt.com/','file:///tmp/chatgpt.com']) assert.throws(() => routeFor(url));
});
test('上下文标记来源、人工发送状态；不把输出变成 HTML 或执行代码', () => {
  const capsule = makeCapsule(action(), {text:'<script>alert(1)</script>', sources:['event:evt_demo']}, fixture.session);
  assert.equal(capsule.delivery, 'prepared');
  assert.match(capsule.text, /recallcard_context/);
  assert.match(capsule.text, /synthetic/);
  assert.throws(() => makeCapsule(action(), {text:'私'.repeat(40000)}, fixture.session), /64 KiB/);
});

test('层级目录引用、导航检索与游标遵循核心边界，不开放路径或配置', () => {
  const valid = ['view:nav/_root', 'view:nav/_unfiled', 'view:nav/topics/java/lab2', 'view:nav/labels/synthetic', 'view:nav/' + Array(5).fill('a'.repeat(48)).join('/')];
  for (const ref of valid) {
    const next = action({ action: 'read', arguments: { refs: [ref], cursor: 'n1:opaque:1', budget_bytes: 4096 } });
    assert.deepEqual(validateAction(next, fixture.session), next);
    assert.throws(() => validateAction(action({ action: 'sources', arguments: { refs: [ref] } }), fixture.session));
    assert.throws(() => validateAction(action({ action: 'read', arguments: { refs: [ref], offset_bytes: 0 } }), fixture.session));
  }
  for (const ref of ['view:nav/', 'view:nav/../secret', 'view:nav/topics//java', 'view:nav/%2e%2e', 'view:nav/topics\\java', 'view:nav/UPPER', 'view:nav/_root/child', 'view:nav/' + 'a'.repeat(49), 'view:nav/a/b/c/d/e/f/g', 'view:nav/' + Array(6).fill('a'.repeat(48)).join('/')]) assert.throws(() => validateAction(action({ action: 'read', arguments: { refs: [ref] } }), fixture.session));
  assert.throws(() => validateAction(action({ action: 'read', arguments: { refs: ['view:nav/_root', 'event:evt_synthetic'] } }), fixture.session));
  for (const target of ['views', 'all']) validateAction(action({ arguments: { query: '合成目录', target, include_navigation: true } }), fixture.session);
  for (const arguments_ of [{ query: '合成', target: 'memories', include_navigation: true }, { query: '合成', include_navigation: 'true' }, { query: '合成', semantic_config: '/private/config.json' }, { query: '合成', scope: 'secret' }]) assert.throws(() => validateAction(action({ arguments: arguments_ }), fixture.session));
});

test('bootstrap示例直接读取当前暴露目录，并如实说明语义不可用与手动发送', () => {
  const capsule = makeCapsule(action({ action: 'bootstrap', arguments: {} }), { navigation_root: 'view:nav/_root', coverage: { semantic_search: 'unavailable' } }, fixture.session);
  const requestText = capsule.text.slice(capsule.text.indexOf('```recallcard-action'));
  const next = parseAction(requestText, fixture.session);
  assert.equal(next.action, 'read'); assert.deepEqual(next.arguments.refs, ['view:nav/_root']);
  assert.match(capsule.text, /实际暴露的引用/); assert.match(capsule.text, /unavailable/); assert.match(capsule.text, /最终发送始终由用户点击/);
});
