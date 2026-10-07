import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { hostile, memory, preview, selection, vault } from './fixtures.mjs';

async function chooseImport(ui, chosen) {
  if (chosen) ui.native.next('pick_import', chosen);
  await ui.openVault();
  await ui.navigate('添加资料');
  ui.one('#file-import-details > summary').click();
  await ui.click('选择文件并预览');
}
async function previewFirst(ui) {
  ui.check('[aria-label="选择会话：合成第一会话"]');
  await ui.click('预览所选会话');
}
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
  assert.equal(ui.one('#content h1').textContent, '你的对话与记忆');
  ui.button('创建新资料库'); ui.button('打开已有资料库');
  assert.equal(ui.one('#navigation').querySelectorAll('button').length, 2);
  await ui.navigate('添加资料');
  assert.equal(ui.modal().open, true);
  ui.button('创建新资料库', ui.modal()); ui.button('打开已有资料库', ui.modal());
  await ui.click('取消', ui.modal());
  assert.equal(ui.modal().open, false);
  assert.equal(ui.native.calls.length, 0);
  ui.noWrites();
});

test('导入来源与时间增加段落后，恶意正文仍只有一个纯文本正文节点', async t => {
  const ui = await fixture(t);
  await chooseImport(ui, {
    ...preview, file_name: hostile, warning: hostile,
    samples: [{ ...preview.samples[0], content: hostile }],
  });
  assert.equal(ui.one('.file-name').textContent, hostile);
  const sample = ui.one('.file-preview .sample');
  assert.equal(sample.querySelectorAll('p').length, 3, '角色之外同时保留来源时间、来源会话和正文');
  // 复现旧选择器的真实歧义；测试不能把 .sample p 静默取第一个当正文。
  assert.throws(() => ui.one('.sample p'), /实际匹配到 3 个节点/);
  const body = ui.one('.sample > p:last-child');
  assert.equal(body.textContent, hostile);
  assert.equal(body.childNodes.length, 1);
  assert.equal(body.firstChild.nodeType, ui.window.Node.TEXT_NODE);
  assert.equal(body.children.length, 0);
  assert.match(sample.textContent, /chatgpt-export.*原始时间：.*2023/);
  assert.match(sample.textContent, /来源会话：合成第一会话/);
  assert.equal(ui.one('.sample .tag').textContent, '用户');
  ui.button('确认导入 3 条记录'); ui.button('取消这次导入');
  noExecutableMarkup(ui); ui.noWrites();
});

test('ZIP 会话默认全不选，显示角色计数及跳过范围，恶意标题不生成 HTML', async t => {
  const ui = await fixture(t);
  await chooseImport(ui);
  const checks = [...ui.document.querySelectorAll('.archive-selection input[type="checkbox"]')];
  assert.equal(checks.length, 2);
  assert.ok(checks.every(node => node.checked === false));
  const start = ui.button('预览所选会话');
  assert.equal(start.disabled, true);
  start.click(); await ui.idle();
  assert.equal(ui.native.count('preview_import_selection'), 0);
  const text = ui.one('.archive-selection').textContent;
  for (const value of ['2 个会话 / 5 条消息', '用户 1 · 助手 1 · 工具 1',
    'Markdown 文件已跳过：1', '隐藏推理消息未收集：1', '附件原件不导入', hostile]) {
    assert.ok(text.includes(value), `应展示：${value}`);
  }
  noExecutableMarkup(ui); ui.noWrites();
});

test('仅提交所选会话，预览保留来源与目的地，一次确认直接写入且重复点击只写一次', async t => {
  const ui = await fixture(t);
  await chooseImport(ui); await previewFirst(ui);
  assert.deepEqual(ui.native.matching('preview_import_selection')[0].payload, {
    sessionId: vault.session_id, selectionId: selection.selection_id, sourceIds: ['one'],
  });
  const samples = [...ui.document.querySelectorAll('.file-preview .sample')];
  assert.deepEqual(samples.map(node => node.querySelector('.tag').textContent), ['用户', '助手', '工具']);
  for (const sample of samples) {
    assert.match(sample.textContent, /chatgpt-export.*2023/);
    assert.match(sample.textContent, /来源会话：合成第一会话/);
    assert.equal(ui.one('p:last-child', sample).children.length, 0);
  }
  assert.match(ui.one('.file-preview').textContent, /写入资料库合成资料库.*写入范围personal/);
  ui.noWrites();
  const finish = ui.button('确认导入 3 条记录');
  const release = ui.native.hold('confirm_import');
  finish.click(); finish.click();
  assert.equal(ui.native.count('confirm_import'), 1);
  assert.equal(finish.disabled, true);
  assert.equal(ui.modal().open, false, '预览按钮之后不再打开同义确认窗口');
  release({ events_added: 3, events_seen: 3 }); await ui.idle();
  finish.disabled = false; finish.click(); await ui.idle();
  assert.equal(ui.native.count('confirm_import'), 1, '完成后保留的旧按钮也不能重放写入');
  assert.deepEqual(ui.native.matching('confirm_import')[0].payload, {
    sessionId: vault.session_id, previewId: preview.preview_id,
  });
  assert.equal(ui.modal().open, false);
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.equal(ui.document.querySelector('.archive-selection'), null);
  await ui.navigate('添加资料'); await ui.click('选择文件并预览');
  assert.ok([...ui.document.querySelectorAll('.archive-selection input')].every(node => !node.checked));
  assert.equal(ui.button('预览所选会话').disabled, true);
});

test('ZIP 全选超限后禁止预览，缩小批次时只传余下所选会话', async t => {
  const ui = await fixture(t);
  const large = structuredClone(selection);
  large.conversations.forEach(conversation => { conversation.event_count = 3000; });
  large.coverage.events_available = 6000;
  await chooseImport(ui, large);
  await ui.click('选择全部可导入会话');
  assert.equal(ui.button('预览所选会话').disabled, true);
  assert.match(ui.one('.archive-selection').textContent, /超过 5000/);
  ui.button('预览所选会话').click(); await ui.idle();
  assert.equal(ui.native.count('preview_import_selection'), 0);
  ui.one('.archive-selection label:nth-of-type(2) input').click();
  await ui.click('预览所选会话');
  assert.deepEqual(ui.native.matching('preview_import_selection')[0].payload.sourceIds, ['one']);
  ui.noWrites();
});

test('ZIP 返回选择与改格式都会撤销旧确认界面，取消后不恢复清单', async t => {
  const ui = await fixture(t);
  await chooseImport(ui); await previewFirst(ui);
  await ui.click('返回会话选择');
  assert.deepEqual(ui.native.matching('return_import_selection')[0].payload, {
    sessionId: vault.session_id, selectionId: selection.selection_id,
  });
  assert.equal(ui.document.querySelectorAll('.file-name').length, 1);
  assert.equal(ui.document.querySelectorAll('.file-preview .sample .tag').length, 0);
  ui.fill('#import-format', 'manual-jsonl', 'change'); await ui.idle();
  assert.equal(ui.document.querySelector('.archive-selection'), null);
  assert.equal(ui.document.querySelector('.file-preview'), null);
  await ui.click('选择文件并预览');
  await ui.click('取消这次导入');
  await ui.navigate('概览'); await ui.navigate('添加资料');
  assert.equal(ui.document.querySelector('.archive-selection'), null);
  ui.noWrites();
});

test('ZIP 原文件改变时显示失败并撤销会话清单和确认入口', async t => {
  const ui = await fixture(t);
  await chooseImport(ui);
  ui.check('[aria-label="选择会话：合成第一会话"]');
  ui.native.fail('preview_import_selection', '文件已改变或被替换，请重新选择文件并审查');
  await ui.click('预览所选会话');
  assert.match(ui.one('#notice').textContent, /文件已改变/);
  assert.equal(ui.document.querySelector('.archive-selection'), null);
  assert.equal(ui.document.querySelector('.file-preview'), null);
  ui.noWrites();
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
