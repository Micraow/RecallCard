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
