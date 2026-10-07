import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activateVault, newState, resetScope } from '../ui/model.js';
import { canEditMemory, invalidateMemoryContent, memoryLabels, memoryReviewKey } from '../ui/memory-management.js';

test('只有可见且有效或待确认的记忆可以编辑', () => {
  for (const status of ['active', 'tentative']) {
    assert.equal(canEditMemory({ status }, { hidden: false }), true);
    assert.equal(canEditMemory({ status }, { hidden: true }), false);
  }
  for (const status of ['superseded', 'retracted', 'unknown']) assert.equal(canEditMemory({ status }, { hidden: false }), false);
  assert.equal(canEditMemory(null, null), false);
});

test('预览绑定会话、范围、版本与完整表单，任何变化都需要重新审阅', () => {
  const original = newState();
  original.vault = { session_id: 'one' };
  Object.assign(original.memory, { selected: { id: 'first', revision: 1 }, mode: 'edit',
    draft: { content: '旧正文', labels: '工作', protected: true } });
  const before = memoryReviewKey(original);
  for (const mutate of [
    s => { s.vault.session_id = 'two'; }, s => { s.scope = 'work'; }, s => { s.epoch++; },
    s => { s.memory.selected.id = 'second'; }, s => { s.memory.selected.revision++; },
    s => { s.memory.mode = 'forget'; }, s => { s.memory.reason = '已过期'; },
    s => { s.memory.draft.content = '新正文'; }, s => { s.memory.draft.labels = '个人'; },
    s => { s.memory.draft.protected = false; },
  ]) {
    const state = structuredClone(original); mutate(state); assert.notEqual(memoryReviewKey(state), before);
  }
});

test('记忆变更废弃旧检索、来源、交接正文和整理结果，仅保留主动的隐藏筛选', () => {
  const state = newState();
  Object.assign(state, { results: ['旧检索'], selected: {}, sources: ['原文'], selectedRefs: ['source'],
    dreamTask: { text: '旧任务' }, dreamPreview: {}, dreamResultText: '旧结果',
    importPreview: {}, importSelection: {}, importSelectedIds: ['old'], notePreview: {},
    conversations: [{}], conversation: {}, conversationRows: [{}], continuation: { text: '旧交接' },
    continuationGoal: '旧目标', conversationListNext: 30 });
  Object.assign(state.memory, { includeHidden: true, selected: {}, draft: {}, review: {}, source: {} });
  invalidateMemoryContent(state);
  assert.deepEqual([state.results, state.sources, state.selectedRefs, state.conversationRows, state.conversations, state.importSelectedIds], [[], [], [], [], [], []]);
  assert.deepEqual([state.selected, state.dreamTask, state.dreamPreview, state.importPreview, state.importSelection,
    state.notePreview, state.conversation, state.continuation, state.memory.selected, state.memory.draft, state.memory.review, state.memory.source], Array(12).fill(null));
  assert.equal(state.dreamResultText, ''); assert.equal(state.continuationGoal, '');
  assert.equal(state.conversationListNext, null); assert.equal(state.memory.includeHidden, true);
});

test('切换资料库或范围不保留隐藏筛选、全文、来源、草稿或批准预览', () => {
  for (const change of [s => resetScope(s, 'work'), s => activateVault(s, { scopes: ['work'] })]) {
    const state = newState();
    Object.assign(state.memory, { includeHidden: true, rows: ['old'], selected: { content: '私密正文' },
      source: { content: '私密出处' }, draft: {}, review: { preview_id: 'old' }, reviewKey: 'old', reason: 'old' });
    change(state);
    assert.equal(state.memory.includeHidden, false); assert.deepEqual(state.memory.rows, []);
    assert.deepEqual([state.memory.selected, state.memory.source, state.memory.draft, state.memory.review], [null, null, null, null]);
    assert.equal(state.memory.reviewKey, ''); assert.equal(state.memory.reason, '');
  }
});

test('标签按行清理重复，逗号和标记是标签原文', () => {
  assert.deepEqual(memoryLabels(' 项目,读书 \n \n项目,读书\n<img src=x>\n工作'), ['项目,读书', '<img src=x>', '工作']);
});
