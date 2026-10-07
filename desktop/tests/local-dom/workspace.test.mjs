import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { vault, preview } from './fixtures.mjs';
const conversation = { session_ref: 'browser-contract', title: '从 DeepSeek 继续', platform: 'deepseek', message_count: 1, captured_at: '2026-10-06T10:00:00Z', coverage: 'partial' };
const event = { id: 'browser-event', role: 'user', content: '决定用 Rust，助手的建议还没有确认。', occurred_at: null, captured_at: '2026-10-06T10:00:00Z', source: { platform: 'deepseek' } };
const row = { ref: 'event:browser-event', kind: 'event', text: event.content, role: 'user', conversation_ref: conversation.session_ref, conversation_title: conversation.title, platform: conversation.platform };
const messages = { messages: [{ ref: row.ref, role: 'user', text: event.content, occurred_at: null }], total: 1, next_offset: null, offset: 0, order_known: false };
const list = { conversations: [conversation], total: 1, next_offset: null };
async function openPopulated(ui) { ui.native.next('list_conversations', list); ui.native.next('conversation_messages', messages); await ui.openVault(); }
async function action(ui, selector) { ui.one(selector).click(); await ui.idle(); }
function fill(ui, selector, value) { ui.fill(selector, value); }
async function search(ui, query = 'Rust') {
  ui.native.next('search_records', { results: [row], truncated: false });
  fill(ui, '#query', query); await ui.click('查找');
  ui.native.next('read_record', { results: [{ ref: row.ref, record: event }], truncated: false });
  await action(ui, `[data-reference="${row.ref}"]`);
}

test('工作区：双入口、自动阅读、交接开关与目标失效', async t => {
  const ui = await fixture(t); await openPopulated(ui);
  assert.deepEqual([...ui.one('#navigation').querySelectorAll('button')].map(n => n.dataset.workspace), ['conversations', 'memories']);
  assert.equal(ui.one('.conversation-reader .reader-title h2').textContent, conversation.title);
  assert.equal(ui.one('.conversation-message .body-text').textContent, event.content);
  await action(ui, '[data-action="open-continuation"]');
  assert.equal(ui.button('复制交接内容').disabled, true);
  fill(ui, 'textarea[aria-label="接下来要做什么"]', '继续实现');
  await action(ui, '[data-action="close-continuation"]');
  assert.equal(ui.one('.conversation-message .body-text').textContent, event.content);
  await action(ui, '[data-action="open-continuation"]');
  assert.equal(ui.one('textarea[aria-label="接下来要做什么"]').value, '继续实现');
  ui.native.next('prepare_continuation', { text: '检查后的交接', message_count: 1, available_messages: 1 });
  await ui.click('准备交接内容');
  fill(ui, 'textarea[aria-label="接下来要做什么"]', '另一个目标');
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.button('复制交接内容').disabled, true);
  ui.noWrites();
});

test('工作区：搜索定位并返回原查询、筛选、阅读', async t => {
  const ui = await fixture(t); await openPopulated(ui); await search(ui);
  ui.native.next('event_location', { ref: row.ref, conversation_ref: conversation.session_ref, conversation_title: conversation.title, platform: 'deepseek', role: 'user', offset: 0, total: 1 });
  ui.native.next('conversation_messages', messages); await ui.click('查看相邻消息');
  assert.equal(ui.one('.located-message').dataset.reference, row.ref);
  ui.native.next('search_records', { results: [row], truncated: false });
  ui.native.next('read_record', { results: [{ ref: row.ref, record: event }], truncated: false });
  await ui.click('返回之前的阅读');
  assert.equal(ui.one('#query').value, 'Rust');
  assert.equal(ui.one('.reading-pane .body-text').textContent, event.content);
  ui.noWrites();
});

test('阅读安全：超预算只显示片段，提示不冒充全文', async t => {
  const ui = await fixture(t); await openPopulated(ui);
  ui.native.next('search_records', { results: [{ ...row, text: '部分搜索片段' }], truncated: false });
  fill(ui, '#query', 'Rust'); await ui.click('查找');
  ui.native.next('read_record', { results: [], truncated: true, pending_refs: [row.ref] });
  await action(ui, `[data-reference="${row.ref}"]`);
  assert.equal(ui.one('.reading-pane .body-text').textContent, '部分搜索片段');
  assert.match(ui.one('.reading-pane .hint').textContent, /部分内容/);
  assert.match(ui.one('#notice').textContent, /部分内容/);
  ui.noWrites();
});

test('导入成果：确认后定位本批，下一批重新选择文件', async t => {
  const ui = await fixture(t); await ui.openVault(); await action(ui, '#import-button');
  assert.equal(ui.one('#file-import-details').open, true);
  assert.equal(ui.one('#note-import-details').open, false);
  await ui.click('选择文件并预览');
  ui.check('[aria-label="选择会话：合成第一会话"]'); await ui.click('预览所选会话');
  await ui.click(`确认导入 ${preview.event_count} 条记录`);
  ui.native.next('confirm_import', { events_added: 3, events_seen: 3, events_duplicates: 0, conversations: [conversation] });
  ui.native.next('list_conversations', list); ui.native.next('conversation_messages', messages);
  await ui.click('确认导入', ui.modal());
  assert.equal(ui.one('#location').textContent, '会话');
  assert.match(ui.one('.import-batch-summary').textContent, /本批导入/);
  assert.equal(ui.one('.conversation-message .body-text').textContent, event.content);
  await action(ui, '#import-button');
  assert.equal(ui.document.querySelector('.archive-selection'), null);
  await ui.click('选择文件并预览');
  assert.equal(ui.one('[aria-label="选择会话：合成第一会话"]').checked, false);
  assert.equal(ui.native.count('confirm_import'), 1);
});

test('记忆工作区：出处先展开，未保存编辑经用户取舍才离开', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.navigate('记忆');
  await action(ui, '.memory-list .result-card');
  await action(ui, '.memory-sources > summary'); await ui.click('查看出处 1');
  assert.match(ui.one('.memory-source .body-text').textContent, /用户原话/);
  await ui.click('修改正文、标签与保护'); fill(ui, '#memory-content', '待保存的纠正');
  await ui.click('检查变更与影响'); await ui.navigate('会话');
  assert.equal(ui.modal().open, true); await ui.click('继续编辑', ui.modal());
  assert.equal(ui.one('#memory-content').value, '待保存的纠正');
  await ui.navigate('会话'); await ui.click('放弃更改', ui.modal()); await ui.navigate('记忆');
  assert.equal(ui.document.querySelector('#memory-review'), null);
  ui.noWrites();
});

test('范围隔离：取消保留旧范围，放弃才清空编辑和隐藏筛选', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.navigate('记忆');
  ui.check('#include-hidden-memories'); await ui.idle();
  await action(ui, '.memory-list .result-card'); await ui.click('修改正文、标签与保护');
  fill(ui, '#memory-content', '范围切换前的草稿'); await ui.click('检查变更与影响');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  assert.equal(ui.one('[aria-label="资料范围"]').value, 'personal');
  assert.equal(ui.modal().open, true); await ui.click('继续编辑', ui.modal());
  assert.equal(ui.one('#memory-content').value, '范围切换前的草稿');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.click('放弃更改', ui.modal());
  assert.equal(ui.one('[aria-label="资料范围"]').value, 'work');
  assert.equal(ui.one('#include-hidden-memories').checked, false);
  assert.equal(ui.document.querySelector('#memory-review'), null); ui.noWrites();
});

test('背景入口：两个子视图和全部记忆保留两个主入口', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.navigate('记忆');
  for (const name of ['background-selected', 'background-select']) {
    await action(ui, `[data-action="${name}"]`);
    assert.equal(ui.one('#location').textContent, '记忆');
    assert.equal(ui.one('#navigation').querySelectorAll('button').length, 2);
    assert.equal(ui.one('#navigation [data-workspace="memories"]').getAttribute('aria-current'), 'page');
  }
  await action(ui, '[data-action="memory-all"]');
  assert.equal(ui.document.querySelectorAll('.memory-list .result-card').length, 1); ui.noWrites();
});

test('返回会话保留选中项、目标和独立滚动，重新读取失败移除旧正文', async t => {
  const ui = await fixture(t); await ui.openVault();
  const selectedRef = ui.one('.conversation-list .result-card.selected').dataset.conversationRef;
  ui.one('.conversation-list').scrollTop = 146; ui.one('.reader-scroll').scrollTop = 327;
  await ui.click('带到另一个AI'); ui.fill('[aria-label="接下来要做什么"]', '继续核对长期方案');
  await ui.click('返回阅读');
  await ui.navigate('记忆'); await ui.navigate('会话');
  assert.equal(ui.one('.conversation-list .selected').dataset.conversationRef, selectedRef);
  assert.equal(ui.one('.conversation-list').scrollTop, 146);
  assert.equal(ui.one('.reader-scroll').scrollTop, 327);
  await ui.click('带到另一个AI');
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, '继续核对长期方案');
  await ui.click('返回阅读'); await ui.navigate('记忆');
  ui.native.fail('conversation_messages', '会话已不可见'); await ui.navigate('会话');
  assert.equal(ui.document.querySelector('.conversation-message'), null);
  assert.equal(ui.document.querySelector('[data-action="open-continuation"]'), null);
  assert.match(ui.one('#notice').textContent, /会话已不可见/); ui.noWrites();
});

test('搜索定位再返回保留查询、类型及列表位置，失效命中不会留下旧预览', async t => {
  const ui = await fixture(t); await ui.openVault(); ui.fill('#query', '核对来源'); await ui.click('查找');
  ui.fill('[aria-label="资料类型"]', 'events', 'change'); await ui.idle();
  await action(ui, '.results .result-card:first-child');
  const ref = ui.one('.results .result-card.selected').dataset.reference;
  ui.one('.results.list-scroll').scrollTop = 231; ui.one('.reading-pane .reader-scroll').scrollTop = 102;
  await ui.click('查看相邻消息'); await ui.click('返回之前的阅读');
  assert.equal(ui.one('#query').value, '核对来源'); assert.equal(ui.one('[aria-label="资料类型"]').value, 'events');
  assert.equal(ui.one('.results.list-scroll').scrollTop, 231); assert.equal(ui.one('.reading-pane .reader-scroll').scrollTop, 102);
  assert.equal(ui.one('.results .selected').dataset.reference, ref);
  await ui.click('查看相邻消息'); ui.native.data.hiddenRefs.push(ref); await ui.click('返回之前的阅读');
  assert.equal(ui.document.querySelector('.reading-pane .body-text'), null);
  assert.doesNotMatch(ui.one('.reading-pane').textContent, /决定先核对来源/); ui.noWrites();
});

test('固定交接操作在滚动区外；复制重验失败撤销旧正文和复制权限', async t => {
  const ui = await fixture(t); await ui.openVault();
  assert.equal(ui.one('[data-action="open-continuation"]').closest('.reader-scroll'), null);
  await ui.click('带到另一个AI'); ui.fill('[aria-label="接下来要做什么"]', '完成下一步');
  await ui.click('准备交接内容');
  const shown = ui.one('[aria-label="交接内容预览"]').value;
  assert.equal(ui.button('复制交接内容').closest('.continuation-scroll'), null);
  await ui.click('复制交接内容');
  assert.equal(ui.native.matching('write_clipboard').at(-1).payload.text, shown);
  assert.match(ui.one('#notice').textContent, /已复制.*个人资料/);
  ui.native.data.messages[0].text = '后台来源已经改变';
  await ui.click('复制交接内容');
  assert.equal(ui.native.count('write_clipboard'), 1);
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.button('复制交接内容').disabled, true);
  assert.match(ui.one('#notice').textContent, /资料或权限已改变/); ui.noWrites();
});

test('导入取消或失败停留原步骤，53个会话成果保持本批且所有行可达', async t => {
  const ui = await fixture(t); await ui.openVault(); await action(ui, '#import-button');
  await ui.click('选择文件并预览'); ui.check('[aria-label="选择会话：合成第一会话"]'); await ui.click('预览所选会话');
  await ui.click('确认导入 3 条记录'); await ui.click('取消', ui.modal());
  assert.equal(ui.one('#location').textContent, '导入会话'); assert.equal(ui.document.querySelector('.import-batch-summary'), null); ui.noWrites();
  await ui.click('确认导入 3 条记录'); ui.native.fail('confirm_import', '文件已改变'); await ui.click('确认导入', ui.modal());
  assert.equal(ui.one('#location').textContent, '导入会话'); assert.equal(ui.document.querySelector('.import-batch-summary'), null);
  await ui.click('选择文件并预览'); ui.check('[aria-label="选择会话：合成第一会话"]'); await ui.click('预览所选会话');
  const many = Array.from({ length: 53 }, (_, index) => ({ session_ref: `synthetic-${index}`, title: `${index + 1}. 合成会话 ${index % 5 === 0 ? '一个非常长的项目讨论标题'.repeat(6) : '继续核验'}`, platform: index % 2 ? 'deepseek' : 'chatgpt-export', message_count: 2, captured_at: index % 3 ? '2026-10-06T08:00:00Z' : null, coverage: 'partial' }));
  ui.native.data.conversations = many;
  ui.native.next('confirm_import', { events_added: 106, events_seen: 110, events_duplicates: 4, conversation_refs: many.map(item => item.session_ref), conversations: many });
  await ui.click('确认导入 3 条记录'); await ui.click('确认导入', ui.modal());
  assert.equal(ui.document.querySelectorAll('.conversation-list .result-card').length, 53);
  assert.match(ui.one('.import-batch-summary').textContent, /53 个会话.*新增 106 条.*重复 4 条/);
  assert.equal(ui.one('.conversation-list .selected').dataset.conversationRef, many[0].session_ref);
  const expectedOrder = many.map(item => item.session_ref); ui.native.data.conversations = [...many].reverse();
  await ui.navigate('记忆'); await ui.navigate('会话');
  assert.deepEqual([...ui.document.querySelectorAll('.conversation-list .result-card')].map(node => node.dataset.conversationRef), expectedOrder);
  assert.equal(ui.document.querySelectorAll('.conversation-list .result-card').length, 53, '返回本批时应重新读取后页并保留全部可访问成果');
  assert.ok(ui.native.matching('list_conversations').some(call => call.payload.offset === 50));
});

test('已选背景筛选包含后页已选项，同时保留普通记忆内部标签', async t => {
  const ui = await fixture(t);
  const seed = ui.native.data.backgroundMemories[0];
  ui.native.data.backgroundMemories = Array.from({ length: 34 }, (_, index) => ({ ...seed, id: `background-${index}`, content: `合成背景 ${index}`, labels: index === 33 ? ['bootstrap', '研究偏好'] : [], protected: index === 33 }));
  await ui.openVault(); await ui.navigate('记忆'); await ui.click('已选背景');
  assert.equal(ui.document.querySelectorAll('.background-candidate').length, 1);
  assert.equal(ui.one('.background-candidate').dataset.memoryId, 'background-33');
  assert.equal(ui.native.matching('read_background_page').at(-1).payload.offset, 30);
  assert.doesNotMatch(ui.one('#content').textContent, /bootstrap/); ui.noWrites();
});

test('键盘上下移动列表焦点，返回列表保留选中条目', async t => {
  const ui = await fixture(t); await ui.openVault();
  ui.native.data.conversations.push({ ...ui.native.data.conversations[0], session_ref: 'second', title: '第二段会话' });
  await ui.click('刷新已保存会话');
  const rows = [...ui.document.querySelectorAll('.conversation-list .result-card')]; rows[0].focus();
  rows[0].dispatchEvent(new ui.window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(ui.document.activeElement, rows[1]); rows[1].click(); await ui.idle();
  assert.equal(ui.one('.conversation-layout').classList.contains('show-detail'), true);
  await ui.click('返回列表');
  assert.equal(ui.one('.conversation-layout').classList.contains('show-detail'), false);
  assert.equal(ui.one('.conversation-list .selected').dataset.conversationRef, 'second');
});

test('保存背景清旧预览，回会话重验后恢复本次目标和新的会话标题', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.click('带到另一个AI');
  ui.fill('[aria-label="接下来要做什么"]', '保留我手写的下一步目标'); await ui.click('准备交接内容');
  await ui.click('设置随身背景');
  ui.check('[data-memory-id="mem_background"] input'); await ui.click('检查随身背景变更');
  await ui.click('保存随身背景选择'); await ui.click('确认保存选择', ui.modal());
  assert.equal(ui.document.querySelector('#continuation-preview'), null);
  ui.native.data.conversations[0].title = '当前可见记录的新标题';
  await ui.click('带上背景继续会话');
  assert.equal(ui.one('.reader-title h2').textContent, '当前可见记录的新标题');
  await ui.click('带到另一个AI');
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, '保留我手写的下一步目标');
  assert.equal(ui.document.querySelector('[aria-label="交接内容预览"]'), null);
  assert.equal(ui.button('复制交接内容').disabled, true);
});


test('移除背景后重新核验会话恢复目标，目标被遗忘则丢弃恢复指针', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.click('带到另一个AI');
  ui.fill('[aria-label="接下来要做什么"]', '只在当前任务保留的目标');
  await ui.click('设置随身背景');
  ui.check('[data-memory-id="mem_carried"] input', false); await ui.click('检查随身背景变更');
  ui.check('#background-protected-approval'); await ui.click('保存随身背景选择'); await ui.click('确认保存选择', ui.modal());
  const saved = [...ui.native.data.conversations]; ui.native.data.conversations = [];
  await ui.click('带上背景继续会话');
  assert.equal(ui.document.querySelector('.conversation-message'), null);
  assert.equal(ui.document.querySelector('#continuation-panel'), null);
  assert.match(ui.one('#notice').textContent, /会话已不可见/);
  ui.native.data.conversations = saved; await ui.navigate('记忆'); await ui.navigate('会话'); await ui.click('带到另一个AI');
  assert.equal(ui.one('[aria-label="接下来要做什么"]').value, '');
  assert.equal(ui.button('复制交接内容').disabled, true);
});

test('新工作区没有选中项时显示列表，失效来源不会困在窄窗详情', async t => {
  const ui = await fixture(t); await ui.openVault();
  await action(ui, '.conversation-list .result-card'); await ui.navigate('记忆');
  assert.equal(ui.one('.memory-layout').classList.contains('show-detail'), false);
  ui.fill('#query', '核对'); await ui.click('查找'); await action(ui, '.results .result-card:first-child');
  const ref = ui.one('.results .selected').dataset.reference;
  await ui.click('查看相邻消息'); ui.native.data.hiddenRefs.push(ref); await ui.click('返回之前的阅读');
  assert.equal(ui.one('.workspace-split').classList.contains('show-detail'), false);
});

test('整理差异可在原位核对原话，来源失效移除先前证据且不写入', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.navigate('整理记忆');
  await ui.click('选择结果并审阅');
  await ui.click('读取来源原话');
  assert.match(ui.one('.dream-evidence .body-text').textContent, /决定先核对来源/);
  assert.equal(ui.native.matching('read_record').at(-1).payload.reference, 'event:evt_synthetic');
  assert.equal(ui.one('#protected-approval').checked, false);
  ui.native.data.hiddenRefs.push('event:evt_synthetic'); await ui.click('读取来源原话');
  assert.equal(ui.document.querySelector('.dream-evidence .body-text'), null);
  assert.match(ui.one('#notice').textContent, /原始出处已不可见/); ui.noWrites();
});

test('不同会话、消息页和定位目标从顶部阅读，同对象返回保留位置', async t => {
  const ui = await fixture(t);
  const first = ui.native.data.conversations[0];
  ui.native.data.conversations.push({ ...first, session_ref: 'another-conversation', title: '另一段长会话' });
  ui.native.data.messages = Array.from({ length: 25 }, (_, index) => ({ ref: `event:page-${index}`, role: 'user', text: `第${index}条合成原话`, occurred_at: null }));
  await ui.openVault();
  ui.one('.conversation-reader .reader-scroll').scrollTop = 380;
  await action(ui, '[data-conversation-ref="another-conversation"]');
  assert.equal(ui.one('.conversation-reader .reader-scroll').scrollTop, 0);
  ui.one('.conversation-reader .reader-scroll').scrollTop = 240;
  await ui.click('后续消息');
  assert.equal(ui.native.matching('conversation_messages').at(-1).payload.offset, 20);
  assert.equal(ui.one('.conversation-reader .reader-scroll').scrollTop, 0);
  ui.one('.conversation-reader .reader-scroll').scrollTop = 160;
  await ui.navigate('记忆'); await ui.navigate('会话');
  assert.equal(ui.one('.conversation-reader .reader-scroll').scrollTop, 160);
  ui.native.next('event_location', { ref: 'event:page-3', conversation_ref: first.session_ref, conversation_title: first.title, offset: 3, total: 25 });
  ui.fill('#query', '来源'); await ui.click('查找'); await action(ui, '.results .result-card:first-child');
  await ui.click('查看相邻消息');
  assert.equal(ui.native.matching('conversation_messages').at(-1).payload.offset, 3);
  assert.equal(ui.one('.conversation-reader .reader-scroll').scrollTop, 0);
  ui.noWrites();
});

test('先浏览后页再导入，往返工作区不丢本批前页成果', async t => {
  const ui = await fixture(t);
  const seed = ui.native.data.conversations[0];
  const old = Array.from({ length: 55 }, (_, index) => ({ ...seed, session_ref: `old-${index}`, title: `旧会话 ${index}` }));
  ui.native.data.conversations = old; await ui.openVault(); await ui.click('更多会话');
  assert.equal(ui.native.matching('list_conversations').at(-1).payload.offset, 50);
  await action(ui, '#import-button'); await ui.click('选择文件并预览');
  ui.check('[aria-label="选择会话：合成第一会话"]'); await ui.click('预览所选会话');
  const batch = Array.from({ length: 53 }, (_, index) => ({ ...seed, session_ref: `new-${index}`, title: `本批会话 ${index}` }));
  ui.native.data.conversations = [...batch, ...old];
  ui.native.next('confirm_import', { events_added: 106, events_seen: 106, events_duplicates: 0, conversations: batch });
  await ui.click('确认导入 3 条记录'); await ui.click('确认导入', ui.modal());
  await ui.navigate('记忆'); await ui.navigate('会话');
  assert.deepEqual([...ui.document.querySelectorAll('.conversation-list [data-conversation-ref]')].map(n => n.dataset.conversationRef), batch.map(c => c.session_ref));
  const offsets = ui.native.matching('list_conversations').slice(-2).map(c => c.payload.offset);
  assert.deepEqual(offsets, [0, 50]);
});

test('查看实际背景明确展开正文，重绘保留开关且从当前区域顶部开始', async t => {
  const ui = await fixture(t); await ui.openVault(); await ui.navigate('记忆'); await ui.click('选择背景');
  assert.equal(ui.one('#background-current').open, false);
  ui.one('.background-detail .reader-scroll').scrollTop = 400;
  await ui.click('查看实际背景');
  assert.equal(ui.one('#background-current').open, true);
  assert.equal(ui.one('.background-detail .reader-scroll').scrollTop, 0);
  assert.equal(ui.one('.background-layout').classList.contains('show-detail'), true);
  assert.ok(ui.one('[aria-label="当前实际随身背景"]').value);
  ui.check('[data-memory-id="mem_background"] input');
  assert.equal(ui.one('#background-current').open, true);
  await action(ui, '#background-current > summary');
  assert.equal(ui.one('#background-current').open, false);
  await ui.click('查看实际背景');
  assert.equal(ui.one('#background-current').open, true);
  ui.noWrites();
});

test('自动导入选中后打开接续进入详情状态，缩窄时不会退回列表', async t => {
  const ui = await fixture(t); await ui.openVault(); await action(ui, '#import-button');
  await ui.click('选择文件并预览'); ui.check('[aria-label="选择会话：合成第一会话"]');
  await ui.click('预览所选会话'); await ui.click('确认导入 3 条记录'); await ui.click('确认导入', ui.modal());
  assert.equal(ui.one('.conversation-layout').classList.contains('show-detail'), false);
  await ui.click('带到另一个AI');
  assert.equal(ui.one('.conversation-layout').classList.contains('show-detail'), true);
  assert.ok(ui.one('#continuation-panel'));
  await ui.click('返回阅读');
  assert.equal(ui.one('.conversation-layout').classList.contains('show-detail'), true);
});
