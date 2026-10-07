import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { conversation, vault, hostile } from './fixtures.mjs';

const leaf = (id, text) => ({ ref: `event:${id}`, role: 'assistant', text, occurred_at: null, branch: { parent_ref: 'event:root', is_branch_end: true, child_count: 0, gap_before: false, omitted_parent_nodes: 0 } });
const left = leaf('left', '路线甲：先完成可靠导入');
const right = leaf('right', '路线乙：另一种尚未采纳的建议');
const rows = [{ ref: 'event:root', role: 'user', text: '比较两种方案', occurred_at: null, branch: { is_branch_end: false } }, left, right];
const summary = { branch_count: 2, root_count: 1, selection_required: true, branches: [left, right].map(row => ({ ...row, branch_ref: row.ref })), branches_truncated: false, shown_branches: 2, has_gaps: false };
const messages = { messages: rows, total: 3, offset: 0, next_offset: null, order_known: false, order_kind: 'branch_forest', branch_summary: summary };
const handoff = { text: '仅含根消息与所选路线甲', message_count: 2, available_messages: 2, available_conversation_messages: 3, selected_branch_ref: left.ref, branch_count: 2, has_gaps: false };
async function open(ui, result = messages) { ui.native.next('conversation_messages', result); await ui.openVault(); }
async function continuePane(ui) { await ui.click('带到另一个AI'); }
function pick(ui, reference = left.ref) { ui.fill('#continuation-branch', reference, 'change'); }

test('分支：不猜测默认回答，明确选末端后准备和复制都携带相同引用', async t => {
  const ui = await fixture(t); await open(ui); await continuePane(ui);
  assert.equal(ui.one('#continuation-branch').value, '');
  assert.equal(ui.button('准备交接内容').disabled, true);
  ui.button('准备交接内容').disabled = false; ui.button('准备交接内容').click(); await ui.idle();
  assert.equal(ui.native.count('prepare_continuation'), 0);
  pick(ui); ui.native.next('prepare_continuation', handoff); await ui.click('准备交接内容');
  assert.deepEqual(ui.native.matching('prepare_continuation')[0].payload, { sessionId: vault.session_id, scope: 'personal', conversationRef: conversation.session_ref, goal: '', branchRef: left.ref });
  assert.equal(ui.one('[aria-label="交接内容预览"]').value, handoff.text);
  ui.native.next('prepare_continuation', handoff); await ui.click('复制交接内容');
  assert.equal(ui.native.matching('prepare_continuation')[1].payload.branchRef, left.ref);
  assert.equal(ui.native.matching('write_clipboard')[0].payload.text, handoff.text);
  ui.noWrites();
});

test('分支：更换选择立即失效旧预览，复制前仍核对最新内容', async t => {
  const ui = await fixture(t); await open(ui); await continuePane(ui); pick(ui);
  ui.native.next('prepare_continuation', handoff); await ui.click('准备交接内容');
  pick(ui, right.ref);
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.button('复制交接内容').disabled, true);
  const revised = { ...handoff, text: '路线乙预览', selected_branch_ref: right.ref, has_gaps: true };
  ui.native.next('prepare_continuation', revised); await ui.click('准备交接内容');
  assert.match(ui.one('#continuation-preview').textContent, /存在省略/);
  ui.native.next('prepare_continuation', { ...revised, text: '权限变化后的新内容' }); await ui.click('复制交接内容');
  assert.equal(ui.native.count('write_clipboard'), 0);
  assert.match(ui.one('#notice').textContent, /资料或权限已改变/);
});

test('分支：截断摘要以当前页叶端补充，普通文本不能生成HTML', async t => {
  const ui = await fixture(t);
  const other = { ...right, text: hostile };
  await open(ui, { ...messages, messages: [other], offset: 20, total: 140, branch_summary: { ...summary, branch_count: 120, branches: [summary.branches[0]], branches_truncated: true } });
  assert.equal(ui.one('.conversation-message .body-text').textContent, hostile);
  ui.one('.branch-end').click(); await ui.idle();
  assert.equal(ui.one('#continuation-branch').value, right.ref);
  assert.match(ui.one('#continuation-panel').textContent, /这里只列出部分末尾/);
  assert.equal(ui.one('#continuation-panel').querySelectorAll('img,script').length, 0);
  ui.native.next('prepare_continuation', { ...handoff, selected_branch_ref: right.ref }); await ui.click('准备交接内容');
  assert.equal(ui.native.matching('prepare_continuation')[0].payload.branchRef, right.ref);
});

test('分支：翻页保留明确选择，切换另一会话清除旧选择', async t => {
  const ui = await fixture(t); await open(ui, { ...messages, next_offset: 3, total: 5 }); await continuePane(ui); pick(ui);
  await ui.click('返回阅读');
  ui.native.next('conversation_messages', { ...messages, messages: [right], total: 5, offset: 3 }); await ui.click('后续消息');
  await continuePane(ui); assert.equal(ui.one('#continuation-branch').value, left.ref);
  await ui.click('返回阅读');
  ui.native.data.conversations = [{ ...conversation, session_ref: 'other-conversation', title: '其他会话' }];
  // 通过真实刷新列表后点击新的会话，不用修改事件处理器或业务状态。
  ui.native.next('conversation_messages', messages); await ui.click('刷新已保存会话');
  ui.native.next('conversation_messages', { ...messages, session_ref: 'other-conversation' });
  ui.one('.conversation-list .result-card').click(); await ui.idle(); await continuePane(ui);
  assert.equal(ui.one('#continuation-branch').value, '');
  assert.equal(ui.button('准备交接内容').disabled, true);
});

test('分支：范围改变清除选择；旧线性会话不多加一次选择', async t => {
  const ui = await fixture(t); await open(ui); await continuePane(ui); pick(ui);
  ui.native.next('list_conversations', { conversations: [conversation], total: 1, next_offset: null });
  ui.native.next('conversation_messages', messages); ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  await continuePane(ui); assert.equal(ui.one('#continuation-branch').value, '');
  await ui.click('返回阅读'); ui.native.next('list_conversations', { conversations: [conversation], total: 1, next_offset: null }); ui.native.next('conversation_messages', { messages: rows.slice(0, 2), total: 2, next_offset: null, order_known: true });
  await ui.click('刷新已保存会话'); await continuePane(ui);
  assert.equal(ui.document.querySelector('#continuation-branch'), null);
  ui.native.next('prepare_continuation', handoff); await ui.click('准备交接内容');
  assert.equal(Object.hasOwn(ui.native.matching('prepare_continuation').at(-1).payload, 'branchRef'), false);
});
