import test from 'node:test';
import assert from 'node:assert/strict';
import { newState, activateVault, resetScope } from '../ui/model.js';
import { invalidateMemoryContent, preservedMemoryLabels } from '../ui/memory-management.js';
import { backgroundReviewKey, sameBackground } from '../ui/background.js';

test('随身背景复制需要相同有效版本、原文与核心包装，缺失字段不能算通过', () => {
  const shown = { bootstrap_version: 'v1', stable_text: '正文', copy_text: '来源包装\n正文' };
  assert.equal(sameBackground(shown, { ...shown }), true);
  for (const field of Object.keys(shown)) {
    assert.equal(sameBackground(shown, { ...shown, [field]: '变化' }), false);
    assert.equal(sameBackground({ ...shown, [field]: '' }, { ...shown, [field]: '' }), false);
  }
  assert.equal(sameBackground(null, null), false);
});

test('随身背景审阅绑定资料库、范围、版本和本次选择', () => {
  const state = newState();
  state.vault = { session_id: 'original' };
  state.background.draft = { id: 'mem_one', revision: 1, include: true };
  const before = backgroundReviewKey(state);
  for (const mutate of [
    s => { s.epoch++; }, s => { s.vault.session_id = 'other'; }, s => { s.scope = 'work'; },
    s => { s.background.draft.id = 'mem_two'; }, s => { s.background.draft.revision++; },
    s => { s.background.draft.include = false; }, s => { s.background.draft = null; },
  ]) { const changed = structuredClone(state); mutate(changed); assert.notEqual(backgroundReviewKey(changed), before); }
});

test('记忆变更、范围与资料库切换均撤销背景正文、出处和待确认选择', () => {
  for (const change of [invalidateMemoryContent, s => resetScope(s, 'work'), s => activateVault(s, { scopes: ['work'] })]) {
    const state = newState();
    Object.assign(state.background, { snapshot: { copy_text: '旧背景' }, rows: ['old'], source: { content: '旧出处' },
      selected: {}, selectedRow: {}, draft: {}, review: {}, reviewKey: 'old', loaded: true });
    change(state);
    assert.deepEqual([state.background.snapshot, state.background.source, state.background.selected, state.background.selectedRow,
      state.background.draft, state.background.review], Array(6).fill(null));
    assert.deepEqual(state.background.rows, []);
    assert.equal(state.background.loaded, false);
    assert.equal(state.background.reviewKey, '');
  }
});

test('普通标签编辑保留原有成员标记，也不能通过输入标记新增成员', () => {
  assert.deepEqual(preservedMemoryLabels('新标签\nbootstrap', ['旧标签']), ['新标签']);
  assert.deepEqual(preservedMemoryLabels('新标签', ['bootstrap', '旧标签']), ['新标签', 'bootstrap']);
});
