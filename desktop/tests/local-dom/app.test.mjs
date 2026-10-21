import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { hostile, memory, vault } from './fixtures.mjs';

async function editMemory(ui) {
  await ui.openVault();
  await ui.navigate('记忆管理');
  ui.one('.memory-list .result-card').click(); await ui.idle();
  await ui.click('修改正文、标签与保护');
  ui.fill('#memory-content', '纠正后的记忆正文');
}
async function reviewMemory(ui) { await ui.click('检查变更与影响'); }
function noExecutableMarkup(ui, root = ui.one('#content')) {
  assert.equal(root.querySelectorAll('img, script, iframe, [onerror], [onclick]').length, 0,
    '恶意内容不得成为 HTML 元素或行内事件属性');
  assert.equal(ui.window.__injected, undefined);
}

test('实际首页与资料入口存在，选择资料库以前没有原生写入', async t => {
  const ui = await fixture(t);
  assert.equal(ui.one('#content h1').textContent, '把以前的对话接着用');
  ui.button('创建新资料库'); ui.button('打开已有资料库');
  assert.deepEqual(
    [...ui.one('#navigation').querySelectorAll('button')].map(node => node.dataset.workspace),
    ['search', 'memories']);
  assert.deepEqual(ui.native.calls, [{ command: 'restore_workspace', payload: {} }]);
  ui.noAutomaticWrites();
});

test('受保护记忆默认未批准，检查与取消不写入，最终确认只发送已审阅的可编辑字段', async t => {
  const ui = await fixture(t);
  await editMemory(ui);
  ui.fill('#memory-labels', '新标签\n共同主题');
  ui.check('#memory-protected', false);
  await reviewMemory(ui);
  assert.deepEqual(ui.native.matching('review_memory_edit')[0].payload, {
    sessionId: vault.session_id, scope: 'personal', id: memory.id, revision: memory.revision,
    edit: { content: '纠正后的记忆正文', protected: false, labels: ['新标签', '共同主题'] },
  });
  assert.equal(ui.one('#memory-protected-approval').checked, false);
  assert.match(ui.one('#memory-review .before').textContent, /完整记忆正文/);
  assert.match(ui.one('#memory-review .after').textContent, /纠正后的记忆正文/);
  ui.noWrites();
  await ui.click('继续确认');
  assert.equal(ui.modal().open, false); ui.noWrites();
  assert.match(ui.one('#notice').textContent, /先勾选额外确认/);
  ui.check('#memory-protected-approval');
  await ui.click('继续确认'); ui.noWrites();
  assert.equal(ui.modal().open, true);
  await ui.click('取消', ui.modal()); ui.noWrites();
  await ui.click('继续确认');
  await ui.click('确认执行', ui.modal());
  assert.deepEqual(ui.native.matching('confirm_memory_change'), [{
    command: 'confirm_memory_change', payload: {
      sessionId: vault.session_id, previewId: 'synthetic-memory-edit', approveProtected: true,
    },
  }]);
  assert.equal(ui.document.querySelector('#memory-review'), null);
  assert.equal(ui.native.data.memory.evidence, memory.evidence);
  assert.deepEqual(ui.native.data.memory.source_refs, memory.source_refs);
});

for (const [name, change] of [
  ['正文', ui => ui.fill('#memory-content', '再次纠正的正文')],
  ['标签', ui => ui.fill('#memory-labels', '重新分类')],
  ['保护设置', ui => ui.check('#memory-protected', false)],
]) {
  test(`${name}变化后立即移除旧记忆审阅，新审阅不能沿用旧批准`, async t => {
    const ui = await fixture(t);
    await editMemory(ui); await reviewMemory(ui);
    ui.check('#memory-protected-approval');
    change(ui);
    assert.equal(ui.document.querySelector('#memory-review'), null);
    ui.noWrites();
    await reviewMemory(ui);
    assert.equal(ui.one('#memory-protected-approval').checked, false);
    await ui.click('继续确认');
    assert.equal(ui.modal().open, false); ui.noWrites();
  });
}

test('遗忘原因变更清除旧批准，重新检查仍展示共享来源影响范围', async t => {
  const ui = await fixture(t);
  await editMemory(ui); await ui.click('取消操作');
  await ui.click('设置遗忘规则');
  ui.fill('#memory-forget-reason', '已过期'); await reviewMemory(ui);
  ui.check('#memory-protected-approval');
  ui.fill('#memory-forget-reason', '需要重新判断');
  assert.equal(ui.document.querySelector('#memory-review'), null);
  await reviewMemory(ui);
  assert.equal(ui.one('#memory-protected-approval').checked, false);
  assert.match(ui.one('#memory-review').textContent, /受影响记忆3.*受影响原始记录2/);
  await ui.click('取消本次变更');
  assert.equal(ui.document.querySelector('#memory-review'), null);
  ui.noWrites();
});

test('已打开最终确认后输入变化，旧确认回调仍拒绝写入', async t => {
  const ui = await fixture(t);
  await editMemory(ui); await reviewMemory(ui);
  ui.check('#memory-protected-approval'); await ui.click('继续确认');
  const oldConfirm = ui.button('确认执行', ui.modal());
  // 有意程序化注入 input，验证异步/旧回调防线；不是模拟原生模态框背后的鼠标操作。
  ui.fill('#memory-content', '确认期间内容已改变');
  assert.equal(ui.document.querySelector('#memory-review'), null);
  oldConfirm.click(); await ui.idle();
  assert.match(ui.one('#notice').textContent, /确认已失效/);
  ui.noWrites();
});

test('整理结果也要求受保护批准，返回修改后批准重置，最终点击前不保存', async t => {
  const ui = await fixture(t);
  await ui.openVault(); await ui.navigate('整理记忆');
  ui.one('#dream-file-options > summary').click();
  await ui.click('选择结果并审阅');
  assert.equal(ui.one('#protected-approval').checked, false);
  await ui.click('保存这些记忆');
  assert.equal(ui.modal().open, false); ui.noWrites();
  ui.check('#protected-approval');
  await ui.click('返回修改结果');
  ui.fill('[aria-label="AI整理结果"]', '{"schema":"recallcard.dream-result/1"}');
  await ui.click('检查并预览结果');
  assert.equal(ui.one('#protected-approval').checked, false);
  ui.check('#protected-approval');
  await ui.click('保存这些记忆'); ui.noWrites();
  await ui.click('取消', ui.modal()); ui.noWrites();
  await ui.click('保存这些记忆');
  await ui.click('确认保存', ui.modal());
  assert.equal(ui.native.count('apply_dream'), 1);
  assert.deepEqual(ui.native.matching('apply_dream')[0].payload, {
    sessionId: vault.session_id, previewId: 'synthetic-dream', approveProtected: true,
  });
});
