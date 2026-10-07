import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { preview, vault } from './fixtures.mjs';

async function openPreview(ui, archive = false) {
  if (!archive) ui.native.next('pick_import', { ...preview, file_name: 'synthetic.json' });
  await ui.click('选择文件并预览');
  if (archive) {
    ui.check('[aria-label="选择会话：合成第一会话"]');
    await ui.click('预览所选会话');
  }
  return ui.button('确认导入 3 条记录');
}

async function ready(t, archive = false) {
  const ui = await fixture(t);
  await ui.openVault(); await ui.navigate('添加资料');
  const confirm = await openPreview(ui, archive);
  return { ui, confirm };
}

async function replay(ui, node) {
  // 直接触发保留节点的事件，证明防护不是只依赖 disabled 或节点被替换。
  node.disabled = false;
  node.dispatchEvent(new ui.window.MouseEvent('click', { bubbles: true }));
  await ui.idle();
}

for (const archive of [false, true]) {
  const format = archive ? 'ZIP' : 'JSON';
  test(`${format} 取消发生在唯一写入按钮之前，不打开确认窗口且旧按钮失效`, async t => {
    const { ui, confirm } = await ready(t, archive);
    const modal = t.mock.method(ui.window.HTMLDialogElement.prototype, 'showModal');
    ui.noWrites();
    await ui.click('取消这次导入');
    await replay(ui, confirm);
    assert.equal(modal.mock.callCount(), 0);
    assert.equal(ui.document.querySelector('.file-preview'), null);
    assert.equal(ui.document.querySelector('.archive-selection'), null);
    ui.noWrites();
    const fresh = await openPreview(ui, archive);
    const release = ui.native.hold('confirm_import');
    fresh.click(); fresh.dispatchEvent(new ui.window.MouseEvent('click'));
    assert.equal(ui.native.count('confirm_import'), 1);
    assert.equal(modal.mock.callCount(), 0, '唯一确认按钮直接提交，不调用 showModal');
    release({ events_added: 3, events_seen: 3 }); await ui.idle();
    await replay(ui, fresh);
    assert.equal(ui.native.count('confirm_import'), 1);
  });

  test(`${format} 确认时文件改变后清除预览，重新选择和审查后才可重试`, async t => {
    const { ui, confirm } = await ready(t, archive);
    ui.native.fail('confirm_import', '文件已改变或被替换，请重新选择文件并审查');
    await ui.click('确认导入 3 条记录');
    assert.equal(ui.native.count('confirm_import'), 1);
    assert.match(ui.one('#notice').textContent, /文件已改变/);
    assert.equal(ui.one('#location').textContent, '导入会话');
    assert.equal(ui.document.querySelector('.file-preview'), null);
    assert.equal(ui.document.querySelector('.archive-selection'), null);
    assert.equal(ui.document.querySelector('.import-batch-summary'), null);
    await replay(ui, confirm);
    assert.equal(ui.native.count('confirm_import'), 1);
    await openPreview(ui, archive);
    await replay(ui, confirm);
    assert.equal(ui.native.count('confirm_import'), 1, '新预览也不恢复旧按钮权限');
    await ui.click('确认导入 3 条记录');
    assert.equal(ui.native.count('confirm_import'), 2);
    assert.equal(ui.one('#location').textContent, '会话');
    assert.equal(ui.modal().open, false);
  });
}

const invalidate = [
  ['离开页面', async ui => ui.navigate('会话')],
  ['修改格式', async ui => { ui.fill('#import-format', 'manual-jsonl', 'change'); await ui.idle(); }],
  ['修改导入范围', async ui => { ui.fill('#import-scope', 'work', 'change'); await ui.idle(); }],
  ['切换全局范围', async ui => { ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle(); }],
  ['切换资料库', async ui => {
    ui.native.next('choose_vault', { ...vault, session_id: 'other-session', display_name: '另一资料库' });
    ui.one('#switch-vault').click(); await ui.click('打开已有资料库', ui.modal());
  }],
  ['同页重新渲染', async ui => ui.navigate('添加资料')],
  ['重新选择文件', async ui => openPreview(ui)],
  ['取消失败', async ui => { ui.native.fail('cancel_previews', '取消失败，请重新选择文件'); await ui.click('取消这次导入'); }],
];

for (const [label, change] of invalidate) {
  test(`${label}以后保留的旧导入按钮不得写入`, async t => {
    const { ui, confirm } = await ready(t);
    await change(ui);
    await replay(ui, confirm);
    ui.noWrites();
    assert.equal(ui.modal().open, false);
  });
}

test('ZIP 返回选择并重新预览后旧按钮失效，新按钮仍只需一次确认', async t => {
  const { ui, confirm } = await ready(t, true);
  await ui.click('返回会话选择'); await replay(ui, confirm); ui.noWrites();
  await ui.click('预览所选会话'); await replay(ui, confirm); ui.noWrites();
  await ui.click('确认导入 3 条记录');
  assert.equal(ui.native.count('confirm_import'), 1);
  assert.equal(ui.modal().open, false);
});

test('ZIP 导入失败后不能重放旧请求，重新预览后才能显式重试', async t => {
  const { ui, confirm } = await ready(t, true);
  ui.native.fail('confirm_import', '写入失败，请重试');
  await ui.click('确认导入 3 条记录');
  assert.match(ui.one('#notice').textContent, /写入失败/);
  assert.equal(ui.document.querySelector('.file-preview:not(.archive-selection)'), null);
  assert.ok(ui.one('.archive-selection'));
  await replay(ui, confirm);
  assert.equal(ui.native.count('confirm_import'), 1);
  await ui.click('预览所选会话'); await replay(ui, confirm);
  assert.equal(ui.native.count('confirm_import'), 1);
  await ui.click('确认导入 3 条记录');
  assert.equal(ui.native.count('confirm_import'), 2);
  assert.equal(ui.one('#location').textContent, '会话');
});
