import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activateVault, resetScope, newState, importSelectionStats, importCoverageLines } from '../ui/model.js';

test('ZIP 选择按原始会话编号计数，超限和未知编号不能预览', () => {
  const selection = { conversations: [{ source_id: 'one', event_count: 3000 }, { source_id: 'two', event_count: 2500 }] };
  assert.deepEqual(importSelectionStats(selection, []), { conversations: 0, events: 0, valid: false });
  assert.deepEqual(importSelectionStats(selection, ['one','one']), { conversations: 1, events: 3000, valid: true });
  assert.equal(importSelectionStats(selection, ['one','two']).valid, false);
  assert.equal(importSelectionStats(selection, ['one','missing']).valid, false);
});
test('覆盖范围明确显示未收集的隐藏推理、附件及跳过 JSON', () => {
  const lines = importCoverageLines({ invalid_json_files_skipped: 1, other_files_skipped: 2, markdown_files_skipped: 3, messages: { hidden_reasoning_messages_skipped: 4 }, notes: ['只保留选定分支'] });
  assert.ok(lines.includes('损坏 JSON 已跳过：1'));
  assert.ok(lines.includes('其他文件已跳过：2'));
  assert.ok(lines.includes('Markdown 文件已跳过：3'));
  assert.ok(lines.includes('隐藏推理消息未收集：4'));
  assert.equal(lines.at(-1), '只保留选定分支');
});
test('切换范围与资料库均清空 ZIP 清单、会话勾选与写入令牌', () => {
  for (const change of [s => resetScope(s,'work'), s => activateVault(s,{scopes:['work']})]) {
    const s = newState(); Object.assign(s,{importSelection:{selection_id:'old'}, importSelectedIds:['private'],importPreview:{preview_id:'old'}});
    change(s);assert.equal(s.importSelection,null);assert.deepEqual(s.importSelectedIds,[]);assert.equal(s.importPreview,null);
  }
});
