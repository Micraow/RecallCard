import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const nativeContracts = JSON.parse(await readFile(new URL('../native-ui-contracts.json', import.meta.url), 'utf8'));
import { fixture } from './harness.mjs';
import { backgroundMemories, backgroundPage, hostile, vault } from './fixtures.mjs';

const checkFor = id => `[data-memory-id="${id}"] input[type="checkbox"]`;
const rowFor = (ui, id) => ui.one(`[data-memory-id="${id}"]`);
async function openBackground(ui) { await ui.openVault(); await ui.navigate('随身背景'); }
async function choose(ui, id = 'mem_background', value = true) {
  ui.check(checkFor(id), value);
  await ui.click('检查随身背景变更');
}

test('随身背景显示核心实际包装和待确认原因，隐藏撤回内容缺席，全文出处按需读取', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  const expected = backgroundPage();
  assert.equal(ui.one('[aria-label="当前实际随身背景"]').value, expected.copy_text);
  assert.equal(ui.one(checkFor('mem_carried')).checked, true);
  assert.equal(ui.one(checkFor('mem_background')).checked, false);
  assert.equal(ui.one(checkFor('mem_tentative')).disabled, true);
  for (const id of ['mem_hidden', 'mem_retracted']) assert.equal(ui.document.querySelector(`[data-memory-id="${id}"]`), null);
  assert.match(rowFor(ui, 'mem_tentative').textContent, /待确认.*AI 建议尚非用户事实/);
  assert.doesNotMatch(ui.one('#content').textContent, /已隐藏的个人记忆|已撤回的记忆/);
  assert.doesNotMatch(ui.one('#content').textContent, /bootstrap/);
  assert.equal(ui.native.count('background_memory_source'), 0);
  await ui.click('查看全文与出处', rowFor(ui, 'mem_background'));
  assert.match(ui.one('.background-layout').textContent, /记忆全文.*用户明确表达/);
  await ui.click('查看背景出处 1');
  assert.deepEqual(ui.native.matching('background_memory_source')[0].payload,
    { sessionId: vault.session_id, scope: 'personal', memoryId: 'mem_background', eventId: 'evt_synthetic' });
  assert.match(ui.one('.background-source').textContent, /用户原话.*synthetic.*2026.*用户原话：请保留来源/);
  ui.noWrites();
});

test('加入前展示实际保存后背景与保护；取消最终确认不写入，最终点击仅写一次', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  ui.check(checkFor('mem_background'));
  assert.equal(ui.document.querySelector('#background-current .primary'), null, '未保存选择不能复制旧背景');
  ui.noWrites();
  await ui.click('检查随身背景变更');
  const review = ui.native.data.pendingBackground;
  assert.equal(ui.one('[aria-label="保存后的实际随身背景"]').value, review.copy_text_after);
  assert.match(ui.one('#background-review').textContent, /未保护 → 已保护/);
  assert.equal(ui.document.querySelector('#background-protected-approval'), null);
  ui.noWrites();
  await ui.click('保存随身背景选择'); ui.noWrites();
  await ui.click('取消', ui.modal()); ui.noWrites();
  await ui.click('保存随身背景选择');
  const confirm = ui.button('确认保存选择', ui.modal());
  confirm.click(); confirm.click(); await ui.idle();
  assert.deepEqual(ui.native.matching('confirm_background_change'), [{ command: 'confirm_background_change',
    payload: { sessionId: vault.session_id, scope: 'personal', previewId: 'synthetic-background', approveProtected: false } }]);
  assert.equal(ui.one('[aria-label="当前实际随身背景"]').value, review.copy_text_after);
  assert.equal(ui.one(checkFor('mem_background')).checked, true);
  const saved = ui.native.data.backgroundMemories.find(item => item.id === 'mem_background');
  assert.equal(saved.protected, true);
  assert.equal(saved.evidence, 'user_explicit');
  assert.equal(saved.status, 'active');
  assert.deepEqual(saved.source_refs, backgroundMemories[0].source_refs);
  assert.equal(saved.recorded_at, backgroundMemories[0].recorded_at);
});

test('受保护背景移除需要独立未勾选批准，保留记忆保护并可重新加入', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  assert.equal(ui.one('#background-protected-approval').checked, false);
  assert.match(ui.one('#background-review').textContent, /已保护 → 已保护/);
  await ui.click('保存随身背景选择');
  assert.equal(ui.modal().open, false); ui.noWrites();
  ui.check('#background-protected-approval');
  await ui.click('保存随身背景选择');
  assert.match(ui.modal().textContent, /保留记忆正文与现有保护.*随时重新选择/);
  await ui.click('确认保存选择', ui.modal());
  const saved = ui.native.data.backgroundMemories.find(item => item.id === 'mem_carried');
  assert.equal(saved.protected, true);
  assert.equal(saved.content, backgroundMemories[1].content);
  assert.deepEqual(saved.labels, ['长期偏好']);
  assert.equal(ui.one(checkFor('mem_carried')).checked, false);
  await choose(ui, 'mem_carried', true);
  assert.equal(ui.one('#background-protected-approval').checked, false, '再次加入不能沿用旧批准');
});

test('改变选择撤销旧审阅、批准和最终确认回调；旧回调不能写入', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  ui.check('#background-protected-approval'); await ui.click('保存随身背景选择');
  const oldConfirm = ui.button('确认保存选择', ui.modal());
  // 程序化触发事件，验证旧回调保护；不声称可以点击真实原生模态层背后的元素。
  ui.check(checkFor('mem_background'));
  await ui.click('放弃更改', ui.modal());
  assert.equal(ui.document.querySelector('#background-review'), null);
  oldConfirm.click(); await ui.idle();
  assert.match(ui.one('#notice').textContent, /确认已失效/);
  ui.noWrites();
  await ui.click('检查随身背景变更');
  assert.equal(ui.document.querySelector('#background-protected-approval'), null);
});

test('取消或撤回未保存勾选会重读当前背景，旧审阅和批准消失', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  ui.check('#background-protected-approval');
  const reads = ui.native.count('read_background');
  await ui.click('取消本次选择');
  assert.equal(ui.native.count('read_background'), reads + 1);
  assert.equal(ui.document.querySelector('#background-review'), null);
  assert.equal(ui.one(checkFor('mem_carried')).checked, true);
  ui.check(checkFor('mem_background'));
  assert.equal(ui.one(checkFor('mem_background')).disabled, false, '可以撤回尚未保存的选择');
  ui.check(checkFor('mem_background'), false); await ui.idle();
  assert.equal(ui.one(checkFor('mem_background')).checked, false);
  ui.button('复制当前随身背景'); ui.noWrites();
});

test('复制前重读版本和完整包装，原生剪贴板收到的就是已展示文本', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  const shown = ui.one('[aria-label="当前实际随身背景"]').value;
  await ui.click('复制当前随身背景');
  assert.equal(ui.native.count('read_background'), 2);
  assert.deepEqual(ui.native.matching('write_clipboard'), [{ command: 'write_clipboard', payload: { text: shown } }]);
  assert.match(shown, /^recallcard\.context\/1\n/);
  assert.deepEqual(ui.native.calls.slice(-2).map(call => call.command), ['read_background', 'write_clipboard']);
});

for (const [name, alter] of [
  ['版本', page => { page.background.bootstrap_version = 'changed'; }],
  ['内层正文', page => { page.background.stable_text += '\nchanged'; }],
  ['包装正文', page => { page.copy_text += '\nchanged'; }],
]) test(`复制重读发现${name}变化时移除旧文本和复制入口`, async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  const fresh = backgroundPage(); alter(fresh);
  ui.native.next('read_background', fresh);
  await ui.click('复制当前随身背景');
  assert.equal(ui.native.count('write_clipboard'), 0);
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.match(ui.one('#content').textContent, /资料或权限已改变/);
  ui.button('重新读取随身背景');
});

test('重读权限失败时不能继续复制旧背景，重新读取后才能恢复', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  ui.native.fail('read_background', '当前范围已不可访问');
  await ui.click('复制当前随身背景');
  assert.equal(ui.native.count('write_clipboard'), 0);
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.match(ui.one('#content').textContent, /当前范围已不可访问/);
  await ui.click('重新读取随身背景');
  ui.button('复制当前随身背景');
});

test('切换范围撤销全文、出处、审阅和最终确认；空范围提示从记忆开始', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  await ui.click('查看背景出处 1');
  ui.check('#background-protected-approval'); await ui.click('保存随身背景选择');
  const oldConfirm = ui.button('确认保存选择', ui.modal());
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle(); await ui.click('放弃更改', ui.modal());
  assert.equal(ui.document.querySelector('#background-review'), null);
  assert.equal(ui.document.querySelector('.background-source'), null);
  await ui.click('选择背景');
  assert.match(ui.one('#content').textContent, /这个范围还没有记忆可选.*还没有选择随身记忆/);
  assert.doesNotMatch(ui.one('[aria-label="当前实际随身背景"]').value, /已经选择的稳定背景/);
  oldConfirm.click(); await ui.idle(); ui.noWrites();
  assert.match(ui.one('#notice').textContent, /确认已失效/);
});

test('切换资料库后旧审阅失效，重新进入背景重新读取', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  ui.check('#background-protected-approval');
  await ui.click('切换资料库');
  await ui.click('放弃更改', ui.modal());
  ui.native.next('choose_vault', { ...vault, session_id: 'second-vault', display_name: '第二合成资料库' });
  await ui.click('打开已有资料库', ui.modal());
  assert.equal(ui.document.querySelector('#background-review'), null);
  await ui.navigate('随身背景');
  assert.equal(ui.native.matching('read_background').at(-1).payload.sessionId, 'second-vault');
  assert.equal(ui.document.querySelector('#background-protected-approval'), null);
  ui.noWrites();
});

test('来源/内容恶意标记保持纯文本，截断及后页已有选择不被误说为空', async t => {
  const ui = await fixture(t);
  const page = backgroundPage();
  page.candidates = [{ ...page.candidates[0], content: hostile, text_truncated: true }];
  page.selected_count = 1; page.total = 31; page.next_offset = 30;
  page.background.truncated = true;
  page.copy_text += `\n${hostile}`;
  ui.native.next('read_background', page);
  await openBackground(ui);
  assert.match(ui.one('#background-current').textContent, /只带上部分背景/);
  assert.doesNotMatch(ui.one('#background-current').textContent, /还没有选择随身记忆/);
  assert.equal(ui.one('[aria-label="当前实际随身背景"]').value, page.copy_text);
  assert.match(ui.one('.background-list').textContent, /这里只显示片段/);
  assert.equal(ui.one('#content').querySelectorAll('img, script, iframe, [onerror], [onclick]').length, 0);
  ui.native.next('read_background_page', { ...backgroundPage(), next_offset: null });
  await ui.click('下一页记忆');
  assert.deepEqual(ui.native.matching('read_background_page')[0].payload,
    { sessionId: vault.session_id, scope: 'personal', offset: 30 });
});

test('普通标签编辑隐藏并保留背景成员标记，不能靠输入该标记新增成员', async t => {
  const ui = await fixture(t);
  ui.native.data.memory.labels = ['bootstrap', '原标签'];
  await ui.openVault(); await ui.navigate('记忆管理');
  ui.one('.memory-list .result-card').click(); await ui.idle();
  assert.doesNotMatch(ui.one('#content').textContent, /bootstrap/);
  assert.match(ui.one('#content').textContent, /每次接续带上.*已选择/);
  await ui.click('修改正文、标签与保护');
  assert.equal(ui.one('#memory-labels').value, '原标签');
  ui.fill('#memory-labels', '新标签\nbootstrap');
  await ui.click('检查变更与影响');
  assert.deepEqual(ui.native.matching('review_memory_edit')[0].payload.edit.labels, ['新标签', 'bootstrap']);
  assert.doesNotMatch(ui.one('#memory-review').textContent, /bootstrap/);
  ui.noWrites();
});

test('旧资料中已选择但未保护的记忆可以明确启用保护后带上', async t => {
  const ui = await fixture(t);
  ui.native.data.backgroundMemories[1].protected = false;
  await openBackground(ui);
  assert.equal(ui.one(checkFor('mem_carried')).checked, true);
  await ui.click('启用并保护这条背景');
  await ui.click('检查随身背景变更');
  assert.match(ui.one('#background-review').textContent, /未保护 → 已保护/);
  assert.deepEqual(ui.native.matching('review_background_change')[0].payload,
    { sessionId: vault.session_id, scope: 'personal', id: 'mem_carried', revision: 3, include: true });
  ui.noWrites();
});

for (const [command, action] of [
  ['review_background_change', async ui => {
    ui.check(checkFor('mem_background')); await ui.click('检查随身背景变更');
  }],
  ['background_memory_source', async ui => {
    await ui.click('查看全文与出处', rowFor(ui, 'mem_background')); await ui.click('查看背景出处 1');
  }],
]) test(`${command}发现资料变化时移除旧背景、详情及审阅`, async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  ui.native.fail(command, '记忆或来源已经改变');
  await action(ui);
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.equal(ui.document.querySelector('#background-review'), null);
  assert.match(ui.one('#content').textContent, /记忆或来源已经改变/);
  ui.noWrites();
});

test('保存失败立即撤销旧批准和复制入口，返回后必须重新审阅', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui, 'mem_carried', false);
  ui.check('#background-protected-approval'); await ui.click('保存随身背景选择');
  ui.native.fail('confirm_background_change', '来源或权限已改变');
  await ui.click('确认保存选择', ui.modal());
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.equal(ui.document.querySelector('#background-review'), null);
  assert.match(ui.one('#content').textContent, /来源或权限已改变/);
  await ui.click('重新读取随身背景');
  await choose(ui, 'mem_carried', false);
  assert.equal(ui.one('#background-protected-approval').checked, false);
});

test('继续会话入口先撤销未使用的背景审阅，再进入已有会话流程', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  const cancellations = ui.native.count('cancel_previews');
  await ui.click('带上背景继续会话');
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.equal(ui.native.count('cancel_previews'), cancellations + 1);
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.equal(ui.native.count('list_conversations'), 2);
  ui.noWrites();
});

test('勾选后查看同一记忆全文不丢失选择，审阅仍使用原来的目标版本', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  ui.check(checkFor('mem_background'));
  await ui.click('查看全文与出处', rowFor(ui, 'mem_background'));
  assert.equal(ui.one(checkFor('mem_background')).checked, true);
  await ui.click('查看背景出处 1');
  await ui.click('检查随身背景变更');
  assert.equal(ui.native.matching('review_background_change')[0].payload.id, 'mem_background');
  ui.noWrites();
});

test('保存已完成而随后刷新失败时，明确报告已保存且不重用旧复制文本', async t => {
  const ui = await fixture(t);
  await openBackground(ui); await choose(ui);
  await ui.click('保存随身背景选择');
  ui.native.fail('read_background', '读取暂时失败');
  await ui.click('确认保存选择', ui.modal());
  assert.equal(ui.native.count('confirm_background_change'), 1);
  assert.match(ui.one('#content').textContent, /选择已保存，但重新读取失败/);
  assert.equal(ui.document.querySelector('#background-current'), null);
  await ui.click('重新读取随身背景');
  assert.equal(ui.one(checkFor('mem_background')).checked, true);
  assert.equal(ui.native.count('confirm_background_change'), 1);
});

test('已选但过期的记忆仍可移除，未带上原因不被误称为长度限制', async t => {
  const ui = await fixture(t);
  ui.native.data.backgroundMemories[1].valid_to = '2020-01-01T00:00:00Z';
  await openBackground(ui);
  assert.equal(ui.one(checkFor('mem_carried')).checked, true);
  assert.equal(ui.one(checkFor('mem_carried')).disabled, false);
  assert.match(rowFor(ui, 'mem_carried').textContent, /已过有效期/);
  assert.doesNotMatch(rowFor(ui, 'mem_carried').textContent, /长度有限/);
  await choose(ui, 'mem_carried', false);
  assert.equal(ui.native.matching('review_background_change')[0].payload.include, false);
  assert.equal(ui.one('#background-protected-approval').checked, false);
  ui.noWrites();
});

test('已经展示的背景出处被隐藏后，再次读取会清除旧原文和复制入口', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  await ui.click('查看全文与出处', rowFor(ui, 'mem_background'));
  await ui.click('查看背景出处 1');
  assert.match(ui.one('.background-source').textContent, /用户原话：请保留来源/);
  ui.native.data.backgroundMemories[0].hidden = true;
  await ui.click('查看背景出处 1');
  assert.equal(ui.document.querySelector('.background-source'), null);
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.doesNotMatch(ui.one('#content').textContent, /用户原话：请保留来源/);
  assert.match(ui.one('#content').textContent, /来源已隐藏或撤回/);
  assert.equal(ui.native.count('managed_memory_source'), 0, '背景页不能使用可主动读取隐藏资料的管理接口');
  ui.noWrites();
});

test('旧候选勾选后被隐藏，查看全文使用背景专用接口且清除旧选择和正文', async t => {
  const ui = await fixture(t);
  await openBackground(ui);
  ui.check(checkFor('mem_background'));
  ui.native.data.backgroundMemories[0].hidden = true;
  await ui.click('查看全文与出处', rowFor(ui, 'mem_background'));
  assert.equal(ui.native.count('background_memory'), 1);
  assert.equal(ui.native.count('managed_memory'), 0, '普通背景页不得通过管理接口读取隐藏全文');
  assert.equal(ui.document.querySelector('#background-current'), null);
  assert.equal(ui.document.querySelector('.background-list'), null);
  assert.doesNotMatch(ui.one('#content').textContent, /用户明确要求每次附上来源/);
  assert.match(ui.one('#content').textContent, /记忆已隐藏或撤回/);
  ui.noWrites();
});

// 原生闭环同一合同：范围切换先清空旧背景，用户重新打开后才显示新范围内容。
test('原生背景切范围路径：全部记忆重置后明确重开，往返个人内容保持一致', async t => {
  const ui = await fixture(t); await openBackground(ui);
  const contract = nativeContracts.scope_background;
  const personal = ui.one('[aria-label="当前实际随身背景"]').value;
  for (const scope of ['work', 'personal']) {
    ui.fill(contract.scope_selector, scope, 'change'); await ui.idle();
    assert.equal(ui.one(contract.reset_tab).getAttribute('aria-pressed'), 'true');
    assert.equal(ui.document.querySelector('[aria-label="当前实际随身背景"]'), null);
    ui.one(contract.background_tab).click(); await ui.idle();
    const shown = ui.one('[aria-label="当前实际随身背景"]').value;
    assert.equal(ui.native.matching('read_background').at(-1).payload.scope, scope);
    if (scope === 'work') {
      assert.equal(ui.document.querySelector('.background-candidate'), null);
      assert.doesNotMatch(shown, /已经选择的稳定背景/);
    } else assert.equal(shown, personal);
  }
  ui.noWrites();
});
