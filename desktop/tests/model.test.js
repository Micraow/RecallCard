import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newState, activateVault, resetScope, recordText, recordRef, scopeOptions, displayDate } from '../ui/model.js';

test('切换资料库清除所有旧结果、选中来源和写入预览', () => {
  const s = newState(); Object.assign(s, { results: [1], selected: {}, sources: [1], dreamPreview: {}, selectedRefs: ['event:old'], query: 'old', page: 'dream' });
  activateVault(s, { session_id: 'new', scopes: ['work'] });
  assert.equal(s.scope, 'work'); assert.equal(s.page, 'search'); assert.equal(s.epoch, 1);
  assert.deepEqual([s.results, s.sources, s.selectedRefs], [[], [], []]);
  assert.deepEqual([s.selected, s.dreamPreview], [null, null]); assert.equal(s.query, '');
});

test('切换范围使异步结果失效并废弃跨范围整理来源', () => {
  const s = newState(); s.selectedRefs = ['event:personal']; resetScope(s, 'work');
  assert.equal(s.epoch, 1); assert.equal(s.scope, 'work'); assert.deepEqual(s.selectedRefs, []);
});

test('参考值与空正文不会成为 HTML', () => {
  assert.equal(recordText({ text: '<script>bad</script>' }), '<script>bad</script>');
  assert.equal(recordRef(null), '');
});

test('范围选项去重且不会由旧记录注入另一个 Vault', () => assert.deepEqual(scopeOptions({ scopes: ['work', 'work'] }, 'work'), ['work', 'personal']));

test('缺失与无效日期显式呈现未知', () => {
  assert.equal(displayDate(undefined), '时间未知');
  assert.equal(displayDate('invalid'), '时间未知');
});

test('切换资料库或范围后不保留客户端配置', () => {
  for (const change of [s => activateVault(s, { session_id: 'new', scopes: ['work'] }), s => resetScope(s, 'work')]) {
    const s = newState(); Object.assign(s, { clientConfig: 'old' });
    change(s); assert.equal(s.clientConfig, null);
  }
});

test('资料库与范围切换撤销完整整理任务、结果正文和待确认审查', () => {
  for (const change of [s => activateVault(s, { session_id: 'new', scopes: ['work'] }), s => resetScope(s, 'work')]) {
    const state = newState();
    Object.assign(state, { dreamTask: { text: '旧范围完整原文' }, dreamResultText: '旧范围模型结果', dreamPreview: { preview_id: 'old' }, selectedRefs: ['event:old'] });
    change(state);
    assert.equal(state.dreamTask, null);
    assert.equal(state.dreamResultText, '');
    assert.equal(state.dreamPreview, null);
    assert.deepEqual(state.selectedRefs, []);
  }
});
