import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseTextFrame, newTextTicket, textHeader, webOpeningInstructions, mcpRecallInstructions, RECALL_CAPABILITIES } from '../text-relay-core.js';
const ticket = 'T_1234567890abcdef';
const frame = (action, body) => `${textHeader(ticket)}\n${typeof action === 'string' ? action : JSON.stringify(action)}${body === undefined ? '' : '\n\n' + body}\nEND RECALLCARD`;

test('纯帧只给出动作，三项传输明确且不执行工具', () => {
  assert.deepEqual(parseTextFrame(frame({action:'search',query:'以前如何安排合成提醒？'}), ticket), {ticket,action:'search',query:'以前如何安排合成提醒？'});
  for (const id of ['S1', 'P12']) assert.deepEqual(parseTextFrame(frame({action:'fetch',id}), ticket), {ticket,action:'fetch',id});
  assert.deepEqual(parseTextFrame(frame({action:'final'}, '已有证据有限。[S1]\n\n保留这一空行。'), ticket), {ticket,action:'final',body:'已有证据有限。[S1]\n\n保留这一空行。'});
  assert.equal(parseTextFrame(frame({action:'search',query:'合成'}).replace(/\n/gu,'\r\n')+'\r\n',ticket).action,'search');
});
test('旧票据、前后文字、引用、围栏、半帧、多帧不升级成请求', () => {
  const full = frame({action:'search',query:'合成'});
  for (const text of [' '+full, '\n'+full, '> '+full, '```text\n'+full+'\n```', '说明\n'+full, full+'\n说明', full+'\n\n', full.slice(0,-3), full+'\n'+full, full.replace('\n','\r'), full.replace('RECALLCARD/2','RECALLCARD/1')]) assert.throws(()=>parseTextFrame(text,ticket));
  assert.throws(()=>parseTextFrame(full,'T_aaaaaaaaaaaaaaaa'));
  assert.throws(()=>parseTextFrame(full,'not-a-ticket'));
});
test('严格JSON拒绝键重放、动作强制类型及越权字段', () => {
  for (const raw of ['{"action":"search","action":"final","query":"a"}', '{"action":"search","\\u0071uery":"a","query":"b"}', '{"action":"final","__proto__":{}}', '{"action":"final","scope":"other"}', '{"action":["search"]}', '{"action":{"toString":"search"}}', '{"action":null}', '[]', '{"action":"write"}', '{"action":"search"}', '{"action":"fetch","id":"S1","budget":999}', '{"action":"search","query":"x","arguments":{}}']) assert.throws(()=>parseTextFrame(frame(raw),ticket));
});
test('query按UTF8与整帧计数，空值、换行和过界不截断', () => {
  const allowed='中'.repeat(682)+'ab';
  assert.equal(new TextEncoder().encode(allowed).length,2048);
  assert.equal(parseTextFrame(frame({action:'search',query:allowed}),ticket).query,allowed);
  for (const query of ['', '  ', '中'.repeat(683), 'a\nb', '\u0000', '\u007f', '\ud800', '\udfff', 1, []]) assert.throws(()=>parseTextFrame(frame({action:'search',query}),ticket));
  assert.equal(parseTextFrame(frame({action:'search',query:'🧭'.repeat(512)}),ticket).query,'🧭'.repeat(512));
  // 合法字符串的转义表示仍受整个原始帧上限约束。
  assert.throws(()=>parseTextFrame(frame('{"action":"search","query":"'+'\\u0061'.repeat(1000)+'"}'),ticket));
});
test('只接受规范S/P语法，存在性与当前已发送集合由host继续验证', () => {
  for (const id of ['S0','S01','S1000000','s1','S1\n','P-1','event:evt_synthetic','rcs1|forged','https://example.invalid','/tmp/data',1]) assert.throws(()=>parseTextFrame(frame({action:'fetch',id}),ticket));
  // 语法模块绝不声称猜到的编号已获授权。
  assert.equal(parseTextFrame(frame({action:'fetch',id:'S999'}),ticket).id,'S999');
});
test('final必须正文非空、严格间隔和单帧；正文里的引用示例不递归执行', () => {
  for (const value of [frame({action:'final'}),frame({action:'final'},''),frame({action:'final'},' \n\t'),frame({action:'final'},'前文\nEND RECALLCARD\n后文'),frame({action:'final'},'前文\nRECALLCARD/2 T_1234567890abcdef\n后文'),frame({action:'search',query:'a'},'不允许正文')]) assert.throws(()=>parseTextFrame(value,ticket));
  const body='协议示例：\n> RECALLCARD/2 T_1234567890abcdef\n> {"action":"search","query":"不执行"}\n> END RECALLCARD';
  assert.equal(parseTextFrame(frame({action:'final'},body),ticket).body,body);
  assert.throws(()=>parseTextFrame(frame({action:'final'},'x'.repeat(65536)),ticket));
  assert.throws(()=>parseTextFrame(frame({action:'final'},'\ud800'),ticket));
});
test('96位票据和简短共同语义不要求MCP复制Web帧', () => {
  const tickets = Array.from({length:30},()=>newTextTicket());
  assert.equal(new Set(tickets).size,30);
  for(const value of tickets) assert.match(value,/^T_[A-Za-z0-9_-]{16}$/u);
  const web = webOpeningInstructions(ticket), mcp = mcpRecallInstructions();
  assert.ok(web.length<1000); assert.match(web,/发送由我完成/); assert.match(web,/本轮传输头：RECALLCARD\/2/);
  assert.ok(web.includes(RECALL_CAPABILITIES.common)); assert.ok(mcp.includes(RECALL_CAPABILITIES.common));
  assert.match(mcp,/bootstrap/); assert.match(mcp,/当前 MCP 使用完整定位符/);
});
test('能力正本与生成模块逐字段一致，顶层模块适合当前打包规则', () => {
  const source=JSON.parse(readFileSync(new URL('../../crates/recallcard/src/recall-capabilities.json',import.meta.url),'utf8'));
  assert.deepEqual(RECALL_CAPABILITIES,source);
  assert.equal(source.limits.core_query_bytes,4096);assert.equal(source.limits.web_query_bytes,2048);
});
