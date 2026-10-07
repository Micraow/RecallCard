import test from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate } from 'node:timers/promises';
import { fixture } from './harness.mjs';
import { vault, hostile, selectedSearchResults, selectedContextRecords, selectedContextResponse } from './fixtures.mjs';

const goal = '核对几次讨论里的方案，再继续实现';
const checks = ui => [...ui.document.querySelectorAll('.results input[type="checkbox"]')];
const checked = ui => checks(ui).map((node, index) => node.checked ? index : -1).filter(index => index >= 0);
const pane = ui => ui.one('#selection-context-panel');
const shown = ui => ui.one('[aria-label="交接内容预览"]', pane(ui));
const copyCount = ui => ui.native.count('write_clipboard');
const prepareCalls = ui => ui.native.matching('prepare_selected_context');
function choose(ui, index, value = true) {
  const node = checks(ui)[index]; assert.ok(node, `第 ${index + 1} 条资料必须可选择`);
  assert.equal(node.disabled, false);
  if (node.checked !== value) node.click();
}
function noPreview(ui) {
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.button('复制交接内容', pane(ui)).disabled, true);
}
async function search(ui, query = goal) { ui.fill('#query', query); await ui.click('查找'); }
async function open(t, { count = 10, query = goal, configure } = {}) {
  const ui = await fixture(t);
  ui.native.data.searchResults = structuredClone(selectedSearchResults.slice(0, count));
  configure?.(ui.native);
  await ui.openVault(); await search(ui, query);
  return ui;
}
async function carry(ui) { await ui.click('带走这些资料'); }
function replay(ui, node, type = 'click') {
  node.disabled = false;
  node.dispatchEvent(new ui.window.Event(type, { bubbles: true, cancelable: true }));
}

test('跨会话选文：同一双栏默认带上列表前三条，以实际查询为目标且只读预览', async t => {
  const ui = await open(t);
  const initialNavigation = ui.one('#navigation').textContent;
  await carry(ui);
  assert.equal(ui.one('#navigation').textContent, initialNavigation);
  assert.equal(ui.one('#navigation').querySelectorAll('button').length, 2);
  assert.equal(ui.one('#location').textContent, '搜索');
  assert.equal(ui.one('.results-layout').contains(pane(ui)), true);
  assert.equal(ui.one('#selection-context-panel h2').textContent, '带到另一个AI');
  assert.equal(checks(ui).length, 10);
  assert.deepEqual(checked(ui), [0, 1, 2]);
  for (const node of checks(ui)) {
    assert.equal(node.closest('button'), null, '独立的选择框不能嵌入阅读按钮');
    assert.match(node.getAttribute('aria-label'), /^带上资料：/);
  }
  assert.match(pane(ui).textContent, /列表顺序/);
  assert.match(pane(ui).textContent, /调整|勾选/);
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, goal);
  assert.deepEqual(prepareCalls(ui).map(call => call.payload), [{
    sessionId: vault.session_id, scope: 'personal',
    references: selectedContextRecords.slice(0, 3).map(record => record.ref), goal, budgetTokens: 32768,
  }]);
  assert.ok(shown(ui).value.includes(selectedContextRecords[0].text));
  assert.equal(shown(ui).readOnly, true);
  assert.equal(copyCount(ui), 0);
  assert.equal(ui.native.count('open_deepseek'), 0);
  assert.deepEqual(ui.shownDialogs, []);
  ui.noWrites();
});

test('跨会话选文：少量结果只选现有资料，空结果不能准备空交接', async t => {
  const ui = await open(t, { count: 1 }); await carry(ui);
  assert.deepEqual(checked(ui), [0]);
  assert.deepEqual(prepareCalls(ui)[0].payload.references, [selectedContextRecords[0].ref]);
  await ui.click('返回搜索结果');
  ui.native.data.searchResults = []; await search(ui);
  const openButton = [...ui.document.querySelectorAll('button')].find(node => node.textContent === '带走这些资料');
  assert.ok(!openButton || openButton.disabled, '没有结果时入口应隐藏或禁用');
  if (openButton) { replay(ui, openButton); await ui.idle(); }
  assert.equal(prepareCalls(ui).length, 1);
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  ui.noWrites();
});

test('跨会话选文：最多八条，恢复勾选仍按列表顺序，伪造第九次选择也无效', async t => {
  const ui = await open(t); await carry(ui);
  for (let index = 3; index < 8; index++) choose(ui, index);
  assert.deepEqual(checked(ui), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.match(pane(ui).textContent, /8/);
  const ninth = checks(ui)[8];
  ninth.disabled = false; ninth.click();
  assert.deepEqual(checked(ui), [0, 1, 2, 3, 4, 5, 6, 7]);
  choose(ui, 0, false); choose(ui, 8); choose(ui, 8, false); choose(ui, 0);
  noPreview(ui);
  assert.equal(prepareCalls(ui).length, 1, '调整选择不得自动重新读取或复制');
  await ui.click('准备交接内容');
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(0, 8).map(record => record.ref));
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：取消勾选或修改目标立即移除旧正文与复制权限', async t => {
  const ui = await open(t); await carry(ui);
  const oldCopy = ui.button('复制交接内容', pane(ui));
  choose(ui, 0, false); noPreview(ui);
  replay(ui, oldCopy); await ui.idle();
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0);
  await ui.click('准备交接内容');
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(1, 3).map(record => record.ref));
  ui.fill('[aria-label="接下来要做什么"]', '新目标：核对来源再决定'); noPreview(ui);
  assert.equal(prepareCalls(ui).length, 2);
  await ui.click('准备交接内容');
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '新目标：核对来源再决定');
  for (const index of [1, 2]) choose(ui, index, false);
  noPreview(ui);
  assert.equal(ui.button('准备交接内容').disabled, true);
  replay(ui, ui.button('准备交接内容')); await ui.idle();
  assert.equal(prepareCalls(ui).length, 3); assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：返回搜索保持已读结果、查询、类型和列表位置，不写入资料', async t => {
  const ui = await open(t);
  ui.fill('[aria-label="资料类型"]', 'events', 'change'); await ui.idle();
  ui.one('.results .result-card:first-child').click(); await ui.idle();
  const selectedRef = ui.one('.results .result-card.selected').dataset.reference;
  ui.one('.results.list-scroll').scrollTop = 289;
  ui.one('.reading-pane .reader-scroll').scrollTop = 145;
  await carry(ui); const oldCopy = ui.button('复制交接内容', pane(ui));
  await ui.click('返回搜索结果');
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  assert.equal(ui.one('#query').value, goal);
  assert.equal(ui.one('[aria-label="资料类型"]').value, 'events');
  assert.equal(ui.one('.results .selected').dataset.reference, selectedRef);
  assert.equal(ui.one('.results.list-scroll').scrollTop, 289);
  assert.equal(ui.one('.reading-pane .reader-scroll').scrollTop, 145);
  replay(ui, oldCopy); await ui.idle();
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：角色、待确认记忆和出处分开标注，恶意文本只能成为纯文本', async t => {
  const ui = await open(t, { query: hostile, configure(native) {
    native.data.searchResults[0].conversation_title = hostile;
    native.data.selectedContextRecords[0].conversation_title = hostile;
    native.data.selectedContextRecords[0].text = hostile;
  } });
  await carry(ui);
  assert.match(pane(ui).textContent, /用户原话/);
  assert.match(pane(ui).textContent, /AI 回复/);
  assert.match(pane(ui).textContent, /待确认/);
  assert.match(pane(ui).textContent, /整理.*记忆|记忆.*整理/);
  assert.match(pane(ui).textContent, /原话|原文/);
  assert.ok(shown(ui).value.includes(hostile));
  assert.equal(ui.one('#content').querySelectorAll('img,script,iframe').length, 0);
  assert.equal(ui.window.__injected, undefined);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：预算不够时说明未带上的资料，预览只显示实际可复制部分', async t => {
  const ui = await open(t);
  const result = { ...selectedContextResponse({ records: selectedContextRecords.slice(0, 1), goal }),
    selected_count: 3, included_count: 1, pending_refs: selectedContextRecords.slice(1, 3).map(record => record.ref),
    selected_refs: selectedContextRecords.slice(0, 3).map(record => record.ref),
    truncated: true, partial: true };
  ui.native.next('prepare_selected_context', result); await carry(ui);
  assert.equal(shown(ui).value, result.text);
  assert.match(pane(ui).textContent, /1\s*\/\s*3|选.*3.*带.*1|带.*1.*选.*3/);
  assert.match(pane(ui).textContent, /未带|未包含|没带|长度|预算/);
  assert.doesNotMatch(pane(ui).textContent, /800\s*token/i, '保守字节预算不能冒充精确 token 数');
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：复制先重新只读核验，双击只产生一次逐字复制', async t => {
  const ui = await open(t); await carry(ui);
  const preview = shown(ui).value;
  const fresh = selectedContextResponse({ goal });
  assert.equal(fresh.text, preview);
  const release = ui.native.hold('prepare_selected_context');
  const copy = ui.button('复制交接内容', pane(ui)); copy.click();
  replay(ui, copy);
  await setImmediate();
  assert.equal(prepareCalls(ui).length, 2); assert.equal(copyCount(ui), 0);
  release(fresh); await ui.idle();
  assert.deepEqual(ui.native.matching('write_clipboard').map(call => call.payload), [{ text: preview }]);
  const commands = ui.native.calls.map(call => call.command);
  assert.ok(commands.lastIndexOf('prepare_selected_context') < commands.indexOf('write_clipboard'));
  assert.equal(ui.button('复制交接内容', pane(ui)).closest('.continuation-scroll'), null);
  ui.noWrites();
});

test('跨会话选文：复制时源正文变化拒绝旧预览并撤销复制', async t => {
  const ui = await open(t); await carry(ui);
  ui.native.data.selectedContextRecords[0].text = '外部已改变的原文';
  await ui.click('复制交接内容');
  assert.equal(copyCount(ui), 0); noPreview(ui);
  assert.match(ui.one('#notice').textContent, /改变|变化|重新/); ui.noWrites();
});

for (const error of ['所选资料已遗忘或不再可见', '当前资料范围权限已撤回', '资料库会话已失效']) {
  test(`跨会话选文：重新核验出现“${error}”时清除旧正文，不复制`, async t => {
    const ui = await open(t); await carry(ui);
    ui.native.fail('prepare_selected_context', error); await ui.click('复制交接内容');
    assert.equal(copyCount(ui), 0); noPreview(ui);
    assert.ok(ui.one('#notice').textContent.includes(error)); ui.noWrites();
  });
}

test('跨会话选文：背景版本变化时拒绝曾经显示的交接内容', async t => {
  const ui = await open(t); await carry(ui);
  const revised = selectedContextResponse({ goal });
  revised.background.bootstrap_version = 'synthetic-background-next-version';
  ui.native.next('prepare_selected_context', revised); await ui.click('复制交接内容');
  assert.equal(copyCount(ui), 0); noPreview(ui);
  assert.match(ui.one('#notice').textContent, /改变|变化|重新/); ui.noWrites();
});

test('跨会话选文：重新检索清除选择和预览，旧复制与选择控件不能跨查询重放', async t => {
  const ui = await open(t); await carry(ui);
  choose(ui, 4); await ui.click('准备交接内容');
  const oldCopy = ui.button('复制交接内容', pane(ui)); const oldChoice = checks(ui)[0];
  ui.native.data.searchResults = structuredClone(selectedSearchResults.slice(5));
  const release = ui.native.hold('search_records');
  ui.fill('#query', '另一个查询'); ui.button('查找').click();
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  replay(ui, oldCopy); replay(ui, oldChoice, 'change');
  release({ results: ui.native.data.searchResults, truncated: false }); await ui.idle();
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  await carry(ui);
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(5, 8).map(record => record.ref));
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '另一个查询');
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：改范围清空预览和选择，旧控件不能带入另一范围', async t => {
  const ui = await open(t); await carry(ui);
  const oldCopy = ui.button('复制交接内容', pane(ui));
  choose(ui, 5);
  ui.native.data.searchResults = structuredClone(selectedSearchResults.slice(5));
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  replay(ui, oldCopy); await ui.idle(); assert.equal(copyCount(ui), 0);
  await carry(ui);
  assert.equal(prepareCalls(ui).at(-1).payload.scope, 'work');
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(5, 8).map(record => record.ref));
  ui.noWrites();
});

test('跨会话选文：改开资料库后旧 session 的交接控件不可重放', async t => {
  const ui = await open(t); await carry(ui);
  const oldCopy = ui.button('复制交接内容', pane(ui));
  const replacement = { ...vault, session_id: 'synthetic-next-vault', display_name: '另一个合成资料库' };
  ui.native.next('choose_vault', replacement);
  ui.one('#switch-vault').click(); await ui.click('打开已有资料库', ui.modal());
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  replay(ui, oldCopy); await ui.idle();
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0);
  await search(ui, '新资料库查询'); await carry(ui);
  assert.equal(prepareCalls(ui).at(-1).payload.sessionId, replacement.session_id);
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '新资料库查询'); ui.noWrites();
});

test('跨会话选文：显式保存记忆后清除旧交接，旧复制按钮不能恢复预览', async t => {
  const ui = await open(t); await carry(ui);
  const oldCopy = ui.button('复制交接内容', pane(ui));
  ui.noWrites();
  await ui.navigate('记忆');
  ui.one('.memory-list .result-card').click(); await ui.idle();
  await ui.click('修改正文、标签与保护');
  ui.fill('#memory-content', '用户明确审阅后修改的合成记忆');
  await ui.click('检查变更与影响');
  ui.check('#memory-protected-approval'); await ui.click('继续确认');
  await ui.click('确认执行', ui.modal());
  assert.equal(ui.native.count('confirm_memory_change'), 1);
  replay(ui, oldCopy); await ui.idle();
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0);
  assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  await search(ui); await carry(ui);
  assert.deepEqual(checked(ui), [0, 1, 2]);
  assert.equal(ui.native.count('confirm_memory_change'), 1);
});

test('跨会话选文：准备尚未返回时旧目标与选择控件不能污染待返回的内容', async t => {
  const ui = await open(t); await carry(ui);
  const oldGoal = ui.one('[aria-label="接下来要做什么"]');
  const oldChoice = checks(ui)[0];
  const release = ui.native.hold('prepare_selected_context');
  ui.button('准备交接内容').click();
  oldGoal.value = '迟到的旧控件目标'; replay(ui, oldGoal, 'input');
  oldChoice.checked = false; replay(ui, oldChoice, 'change');
  release(selectedContextResponse({ goal })); await ui.idle();
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, goal);
  assert.deepEqual(checked(ui), [0, 1, 2]);
  assert.equal(shown(ui).value, selectedContextResponse({ goal }).text);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：尚未提交的新搜索词不能冒充当前结果的交接目标', async t => {
  const ui = await open(t);
  ui.fill('#query', '输入框里尚未检索的新词'); await carry(ui);
  assert.equal(prepareCalls(ui).at(-1).payload.goal, goal);
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, goal);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：异步检索期间的迟到输入不能把旧结果冒充为新查询', async t => {
  const ui = await open(t);
  const release = ui.native.hold('search_records');
  ui.fill('#query', '正在等待的查询'); ui.button('查找').click();
  const query = ui.one('#query'); query.value = '检索尚未完成时的新词'; replay(ui, query, 'input');
  release({ results: selectedSearchResults.slice(5), truncated: false }); await ui.idle();
  const rendered = ui.document.querySelectorAll('.results .result-card');
  if (rendered.length) {
    await carry(ui);
    assert.equal(prepareCalls(ui).at(-1).payload.goal, '正在等待的查询', '交接目标必须对应实际查询响应，不能引用后来的未检索输入');
    assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(5, 8).map(record => record.ref));
  } else assert.equal(ui.document.querySelector('#selection-context-panel'), null);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：调整选择可回列表并聚焦选择框，再打开保留勾选', async t => {
  const ui = await open(t); await carry(ui);
  assert.equal(ui.one('.results-layout').classList.contains('show-detail'), true);
  await ui.click('调整所选资料');
  assert.equal(ui.one('.results-layout').classList.contains('show-detail'), false);
  assert.equal(ui.document.activeElement, checks(ui)[0]);
  assert.deepEqual(checked(ui), [0, 1, 2]);
  assert.equal(prepareCalls(ui).length, 1);
  choose(ui, 1, false); choose(ui, 4);
  await carry(ui);
  assert.equal(ui.one('.results-layout').classList.contains('show-detail'), true);
  assert.deepEqual(checked(ui), [0, 2, 4]);
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, [0, 2, 4].map(index => selectedContextRecords[index].ref));
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：预算未容纳任何所选资料时禁止复制，旧禁用控件重放也无效', async t => {
  const ui = await open(t);
  const result = { ...selectedContextResponse({ records: [], goal }),
    selected_refs: selectedContextRecords.slice(0, 3).map(record => record.ref), selected_count: 3,
    pending_refs: selectedContextRecords.slice(0, 3).map(record => record.ref), truncated: true, partial: true };
  ui.native.next('prepare_selected_context', result); await carry(ui);
  assert.equal(shown(ui).value, result.text);
  assert.match(pane(ui).textContent, /0\s*\/\s*3/);
  assert.match(pane(ui).textContent, /3 条所选资料没有放入/);
  assert.equal(ui.button('复制交接内容').disabled, true);
  replay(ui, ui.button('复制交接内容')); await ui.idle();
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：结果含独立选择框时，上下键仍沿整个列表移动阅读焦点', async t => {
  const ui = await open(t); await carry(ui);
  const cards = [...ui.document.querySelectorAll('.results button.result-card')];
  cards[0].focus();
  cards[0].dispatchEvent(new ui.window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(ui.document.activeElement, cards[1]);
  cards[1].dispatchEvent(new ui.window.KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }));
  assert.equal(ui.document.activeElement, cards[0]);
  cards[0].dispatchEvent(new ui.window.KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }));
  assert.equal(ui.document.activeElement, cards[0]);
  assert.deepEqual(checked(ui), [0, 1, 2]);
  assert.equal(prepareCalls(ui).length, 1); assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：核对原文及相邻消息再返回，保留手选引用和手写目标并重读', async t => {
  const ui = await open(t); await carry(ui);
  choose(ui, 1, false); choose(ui, 4);
  const typedGoal = '先核对几次讨论的分歧，再按我的选择继续';
  ui.fill('[aria-label="接下来要做什么"]', typedGoal); await ui.click('准备交接内容');
  const oldCopy = ui.button('复制交接内容', pane(ui));
  await ui.click('核对原文与出处', ui.one('[data-context-reference="event:evt_selected_0"]'));
  ui.native.next('event_location', { conversation_ref: ui.native.data.conversations[0].session_ref, offset: 0 });
  await ui.click('查看相邻消息'); await ui.click('返回之前的阅读');
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  replay(ui, oldCopy); await ui.idle(); assert.equal(copyCount(ui), 0);
  await carry(ui);
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, [0, 2, 4].map(index => selectedContextRecords[index].ref));
  assert.equal(prepareCalls(ui).at(-1).payload.goal, typedGoal);
  assert.deepEqual(checked(ui), [0, 2, 4]);
  ui.noWrites();
});

test('跨会话选文：核对后返回只恢复当前可见的同版本引用，不替补其他资料', async t => {
  const ui = await open(t); await carry(ui);
  choose(ui, 1, false); choose(ui, 4);
  ui.fill('[aria-label="接下来要做什么"]', '保留我的目标，只用仍可核对的资料'); await ui.click('准备交接内容');
  await ui.click('核对原文与出处', ui.one('[data-context-reference="event:evt_selected_0"]'));
  ui.native.next('event_location', { conversation_ref: ui.native.data.conversations[0].session_ref, offset: 0 });
  await ui.click('查看相邻消息');
  ui.native.data.hiddenRefs = [selectedContextRecords[2].ref, selectedContextRecords[4].ref];
  await ui.click('返回之前的阅读'); await carry(ui);
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, [selectedContextRecords[0].ref]);
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '保留我的目标，只用仍可核对的资料');
  assert.equal(pane(ui).querySelectorAll('.selected-context-record').length, 1);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：新查询失败立即撤去旧列表，重试成功后才可带走新资料', async t => {
  const ui = await open(t); await carry(ui); await ui.click('返回搜索结果');
  let rejectSearch;
  ui.native.next('search_records', new Promise((resolve, reject) => { rejectSearch = reject; }));
  ui.fill('#query', '另一次查找'); ui.button('查找').click();
  assert.equal(ui.document.querySelectorAll('.results .result-card').length, 0);
  rejectSearch(new Error('合成查询读取失败')); await ui.idle();
  assert.equal(ui.document.querySelectorAll('.results .result-card').length, 0);
  assert.equal([...ui.document.querySelectorAll('button')].some(node => node.textContent === '带走这些资料'), false);
  assert.match(ui.one('.results').textContent, /查找未完成|查找没有完成/);
  assert.doesNotMatch(ui.one('.results').textContent, /没有找到匹配资料/);
  ui.native.data.searchResults = structuredClone(selectedSearchResults.slice(5));
  await ui.click('重试查找'); await carry(ui);
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '另一次查找');
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(5, 8).map(record => record.ref));
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：出处失败不显示无来源结论，重读成功后才展示记忆和确切出处数', async t => {
  const ui = await open(t); await carry(ui);
  ui.native.fail('read_sources', '合成出处读取失败');
  await ui.click('核对原文与出处', ui.one('[data-context-reference="memory:mem_selected@4"]'));
  assert.match(ui.one('#notice').textContent, /合成出处读取失败/);
  assert.equal(ui.document.querySelector('.source-details summary'), null);
  assert.equal(ui.document.querySelector('.reading-pane .body-text'), null);
  const memoryButton = ui.one('.results [data-reference="memory:mem_selected@4"]');
  memoryButton.click(); await ui.idle();
  assert.equal(ui.one('.source-details summary').textContent, '原始出处 · 0 条');
  assert.ok(ui.one('.reading-pane .body-text').textContent.includes(selectedContextRecords[2].text));
  assert.equal(ui.native.count('read_sources'), 2);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：勾选、取消和触及八条上限后保留原选择框的键盘焦点', async t => {
  const ui = await open(t); await carry(ui); await ui.click('调整所选资料');
  assert.equal(ui.document.activeElement, checks(ui)[0]);
  checks(ui)[0].click();
  assert.equal(ui.document.activeElement, checks(ui)[0]);
  assert.equal(checks(ui)[0].checked, false);
  checks(ui)[0].click();
  assert.equal(ui.document.activeElement, checks(ui)[0]);
  assert.equal(checks(ui)[0].checked, true);
  for (let index = 3; index < 8; index++) choose(ui, index);
  checks(ui)[8].focus(); checks(ui)[8].click();
  assert.equal(ui.document.activeElement, checks(ui)[8]);
  assert.equal(checks(ui)[8].checked, false);
  assert.deepEqual(checked(ui), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：返回时原选择全部失效，保留目标但不自动改选其他结果', async t => {
  const ui = await open(t); await carry(ui);
  ui.fill('[aria-label="接下来要做什么"]', '不要替我换成其他资料'); await ui.click('准备交接内容');
  await ui.click('核对原文与出处', ui.one('[data-context-reference="event:evt_selected_0"]'));
  ui.native.next('event_location', { conversation_ref: ui.native.data.conversations[0].session_ref, offset: 0 });
  await ui.click('查看相邻消息');
  ui.native.data.hiddenRefs = selectedContextRecords.slice(0, 3).map(record => record.ref);
  await ui.click('返回之前的阅读');
  const before = prepareCalls(ui).length; await carry(ui);
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, '不要替我换成其他资料');
  assert.deepEqual(checked(ui), []);
  assert.equal(ui.button('准备交接内容').disabled, true);
  noPreview(ui); assert.equal(prepareCalls(ui).length, before);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：原文仅返回片段且未读出处时，明确待读取并可重读', async t => {
  const ui = await open(t);
  ui.native.next('read_record', { results: [], truncated: true, pending_refs: [selectedContextRecords[2].ref] });
  ui.one('.results [data-reference="memory:mem_selected@4"]').click(); await ui.idle();
  assert.equal(ui.native.count('read_sources'), 0);
  assert.equal(ui.document.querySelector('.source-details summary'), null);
  assert.match(ui.one('.reading-pane').textContent, /出处尚未读取/);
  await ui.click('重新读取资料与出处');
  assert.equal(ui.native.count('read_sources'), 1);
  assert.equal(ui.one('.source-details summary').textContent, '原始出处 · 0 条');
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：出处响应因预算不完整且暂未容纳来源，不冒称零条出处', async t => {
  const ui = await open(t);
  ui.native.next('read_sources', { results: [], truncated: true });
  ui.one('.results [data-reference="memory:mem_selected@4"]').click(); await ui.idle();
  assert.equal(ui.one('.source-details summary').textContent, '原始出处尚未读出');
  assert.match(ui.one('.source-details').textContent, /不是完整的来源数量/);
  assert.doesNotMatch(ui.one('.source-details').textContent, /0 条/);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

async function inspectAdjacentAndLeave(ui) {
  await ui.click('核对原文与出处', ui.one('[data-context-reference="event:evt_selected_0"]'));
  ui.native.next('event_location', { conversation_ref: ui.native.data.conversations[0].session_ref, offset: 0 });
  await ui.click('查看相邻消息');
}

test('跨会话选文：返回时相同记忆升级版本，不把未审阅的新版本偷换进选择', async t => {
  const ui = await open(t); await carry(ui);
  ui.fill('[aria-label="接下来要做什么"]', '先核对我看过的版本'); await ui.click('准备交接内容');
  await inspectAdjacentAndLeave(ui);
  ui.native.data.searchResults[2].ref = 'memory:mem_selected@5';
  ui.native.data.selectedContextRecords[2].ref = 'memory:mem_selected@5';
  await ui.click('返回之前的阅读'); await carry(ui);
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, selectedContextRecords.slice(0, 2).map(record => record.ref));
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '先核对我看过的版本');
  assert.equal(checks(ui)[2].checked, false);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：返回查找失败后重试保留用户草稿，不恢复旧预览与旧复制控件', async t => {
  const ui = await open(t); await carry(ui);
  choose(ui, 1, false); choose(ui, 4);
  ui.fill('[aria-label="接下来要做什么"]', '返回后继续这个手写目标'); await ui.click('准备交接内容');
  const oldCopy = ui.button('复制交接内容', pane(ui));
  await inspectAdjacentAndLeave(ui);
  ui.native.fail('search_records', '合成返回读取失败'); await ui.click('返回之前的阅读');
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.document.querySelectorAll('.results .result-card').length, 0);
  replay(ui, oldCopy); await ui.idle(); assert.equal(copyCount(ui), 0);
  await ui.click('重试查找'); await carry(ui);
  assert.equal(prepareCalls(ui).at(-1).payload.goal, '返回后继续这个手写目标');
  assert.deepEqual(prepareCalls(ui).at(-1).payload.references, [0, 2, 4].map(index => selectedContextRecords[index].ref));
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：原始出处仍在读取时不先展示记忆，完成后正文和出处一起出现', async t => {
  const ui = await open(t); await carry(ui);
  const release = ui.native.hold('read_sources');
  ui.button('核对原文与出处', ui.one('[data-context-reference="memory:mem_selected@4"]')).click();
  await setImmediate();
  assert.equal(ui.document.querySelector('.reading-pane .body-text'), null);
  assert.equal(ui.document.querySelector('#selection-context-preview'), null);
  release({ results: [{ events: [{ ref: 'event:evt_selected_0', role: 'user', content: '已核对来源', conversation_title: '来源会话' }] }], truncated: false });
  await ui.idle();
  assert.equal(ui.one('.source-details > summary').textContent, '原始出处 · 1 条');
  assert.ok(ui.one('.reading-pane > .reader-scroll > .body-text').textContent.includes(selectedContextRecords[2].text));
  assert.match(ui.one('.source-details').textContent, /已核对来源/);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：同会话的选择框用位置、角色和原话短摘录明确区分', async t => {
  const ui = await open(t); await carry(ui);
  const names = checks(ui).map(node => node.getAttribute('aria-label'));
  assert.equal(new Set(names).size, names.length);
  assert.match(names[0], /^带上资料：第 1 条，用户原话，合成会话 1，/);
  assert.match(names[1], /^带上资料：第 2 条，AI 回复，合成会话 1，/);
  assert.ok(names[0].includes(selectedContextRecords[0].text));
  assert.ok(names[1].includes(selectedContextRecords[1].text));
  assert.match(names[2], /第 3 条，整理记忆/);
  assert.doesNotMatch(names[2], /^带上资料：第 3 条，用户原话，/);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});

test('跨会话选文：选择框的长标题和正文只保留短摘录，恶意标记不能成为节点', async t => {
  const ui = await open(t, { configure(native) {
    native.data.searchResults[0].conversation_title = hostile + '长标题'.repeat(100);
    native.data.searchResults[0].text = hostile + '长正文'.repeat(100);
  } });
  await carry(ui);
  const name = checks(ui)[0].getAttribute('aria-label');
  assert.ok(name.length < 160);
  assert.match(name, /第 1 条，用户原话/);
  assert.equal(ui.one('.results').querySelectorAll('img,script,iframe').length, 0);
  assert.equal(copyCount(ui), 0); ui.noWrites();
});
