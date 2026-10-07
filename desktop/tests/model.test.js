import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newState, activateVault, resetScope, recordText, recordRef, nativeInstructions, scopeOptions, displayDate } from '../ui/model.js';
test('切换资料库清除所有旧结果、选中来源和写入预览', () => {
  const s = newState(); Object.assign(s, { results: [1], selected: {}, sources: [1], importPreview: {}, dreamPreview: {}, selectedRefs: ['event:old'], query: 'old', page: 'dream' });
  activateVault(s, { session_id: 'new', scopes: ['work'] });
  assert.equal(s.scope, 'work'); assert.equal(s.page, 'home'); assert.equal(s.epoch, 1);
  assert.deepEqual([s.results,s.sources,s.selectedRefs], [[],[],[]]);
  assert.deepEqual([s.selected,s.importPreview,s.dreamPreview], [null,null,null]); assert.equal(s.query, '');
});
test('切换范围使异步结果失效并废弃跨范围整理来源', () => {
  const s = newState(); s.selectedRefs = ['event:personal']; s.importPreview = {}; resetScope(s, 'work');
  assert.equal(s.epoch, 1); assert.equal(s.scope, 'work'); assert.equal(s.importPreview, null); assert.deepEqual(s.selectedRefs, []);
});
test('参考值与空正文不会成为 HTML', () => { assert.equal(recordText({text:'<script>bad</script>'}), '<script>bad</script>'); assert.equal(recordRef(null), ''); });
test('命令指引对引号和 shell 代换保留原样', () => {
  const command = nativeInstructions("/tmp/a'$(bad)", 'personal');
  assert.ok(command.includes("'/tmp/a'\\''$(bad)'")); assert.ok(command.includes("--scope 'personal'"));
});
test('范围选项去重且不会由旧记录注入另一个 Vault', () => assert.deepEqual(scopeOptions({scopes:['work','work']},'work'), ['work','personal']));
test('缺失与无效日期显式呈现未知', () => { assert.equal(displayDate(undefined), '时间未知'); assert.equal(displayDate('invalid'), '时间未知'); });

test('切换资料库或范围后不保留旧交接正文和客户端配置',()=>{for(const change of [s=>activateVault(s,{session_id:'new',scopes:['work']}),s=>resetScope(s,'work')]){const s=newState();Object.assign(s,{conversations:[{}],conversation:{},conversationRows:[{}],continuation:{text:'private'},clientConfig:'old'});change(s);assert.deepEqual(s.conversations,[]);assert.deepEqual(s.conversationRows,[]);assert.equal(s.continuation,null);assert.equal(s.clientConfig,null);}});
