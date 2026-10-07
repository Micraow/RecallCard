import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './harness.mjs';
import { hostile, preview, vault } from './fixtures.mjs';

// 这里只模拟已公开的原生响应；导入行为、DOM 和事件来自未改写的应用 ES 模块。
const batch = {
  ...preview, preview_id: 'synthetic-multifile-preview', format: 'auto',
  files: [{ file_name: 'deepseek-history.json', byte_count: 1024 }, { file_name: 'chatgpt.zip', byte_count: 2048 }],
  byte_count: 3072, event_count: 6,
  conversations: [...preview.conversations, { ...preview.conversations[0], source_id: 'two', title: '第二份导出的会话' }],
};
const running = {
  job_id: 'synthetic-import-job', scope: 'personal', state: 'running',
  events_total: 6, events_processed: 0, events_added: 0, events_duplicates: 0,
  files_total: 2, conversations_total: 2, can_resume: false,
  message: '正在保存合成消息', created_at: '2026-10-07T10:00:00Z', updated_at: '2026-10-07T10:00:00Z',
};
const paused = { ...running, state: 'cancelled', events_processed: 2, events_added: 2, can_resume: true, message: '已暂停，之后可以继续' };
const completed = { ...running, state: 'completed', events_processed: 6, events_added: 5, events_duplicates: 1, message: '全部消息已处理' };
const pane = ui => ui.one('[aria-label="导入平台历史"]');

function controlledPolling(t, ui) {
  let sequence = 0;
  const timers = new Map();
  t.mock.method(ui.window, 'setTimeout', (callback, delay, ...args) => {
    const id = ++sequence; timers.set(id, { callback, delay, args }); return id;
  });
  t.mock.method(ui.window, 'clearTimeout', id => { timers.delete(id); });
  return {
    get count() { return timers.size; },
    fire() {
      assert.equal(timers.size, 1, '活动任务恰好保留一个待执行轮询');
      const [id, timer] = timers.entries().next().value;
      assert.equal(timer.delay, 600, '轮询采用 600 毫秒间隔，不进行忙循环');
      timers.delete(id);
      return timer.callback(...timer.args);
    },
  };
}
async function ready(t) {
  const ui = await fixture(t);
  const polling = controlledPolling(t, ui);
  await ui.openVault(); await ui.navigate('添加资料');
  return { ui, polling };
}
async function pick(ui, result = batch) {
  ui.native.next('pick_import_files', result);
  await ui.click('选择导出文件');
}
async function start(ui, result = running) {
  await pick(ui); ui.noWrites();
  ui.native.next('start_import_job', result);
  await ui.click('导入全部 6 条消息');
}
async function replay(ui, node) {
  node.disabled = false;
  node.dispatchEvent(new ui.window.MouseEvent('click', { bubbles: true }));
  await ui.idle();
}
async function switchVault(ui) {
  ui.native.next('choose_vault', { ...vault, session_id: 'synthetic-second-session', display_name: '另一合成资料库' });
  ui.one('#switch-vault').click(); await ui.click('打开已有资料库', ui.modal());
}

test('首次开始使用只准备默认本机位置，空库直接到多文件导入且不用选择目录', async t => {
  const ui = await fixture(t);
  const showModal = t.mock.method(ui.window.HTMLDialogElement.prototype, 'showModal');
  ui.noWrites();
  const release = ui.native.hold('open_default_workspace');
  const begin = ui.button('开始使用'); begin.click(); begin.click();
  assert.equal(ui.native.count('open_default_workspace'), 1);
  release({ ...vault, event_count: 0 }); await ui.idle();
  assert.equal(ui.one('#content h1').textContent, '导入会话');
  ui.button('选择导出文件');
  assert.equal(ui.native.count('choose_vault'), 0);
  assert.equal(ui.native.count('pick_import_files'), 0);
  assert.equal(showModal.mock.callCount(), 0);
  assert.deepEqual(ui.native.matching('open_default_workspace')[0].payload, {});
  // 这个按钮明确允许初始化默认资料库；消息导入仍须单独批准。
  ui.noWrites({ allowDefaultWorkspace: true });
});

test('开始使用发现已有记录时回到真实会话阅读，不自动创建导入任务', async t => {
  const ui = await fixture(t);
  ui.native.next('open_default_workspace', vault);
  await ui.click('开始使用');
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  assert.equal(ui.native.count('conversation_messages'), 1);
  ui.noWrites({ allowDefaultWorkspace: true });
});

test('默认位置读取失败保留开始入口，重试仍由用户明确点击', async t => {
  const ui = await fixture(t);
  ui.native.fail('open_default_workspace', '本机保存位置暂时无法打开');
  await ui.click('开始使用');
  assert.match(ui.one('#notice').textContent, /本机保存位置暂时无法打开/);
  assert.equal(ui.one('#vault-badge').textContent, '尚未打开资料库');
  assert.equal(ui.native.count('open_default_workspace'), 1);
  await ui.click('开始使用');
  assert.equal(ui.native.count('open_default_workspace'), 2);
  assert.equal(ui.one('#content h1').textContent, '导入会话');
  ui.noWrites({ allowDefaultWorkspace: true });
});

test('取消多文件选择后保持空导入入口，不创建任务或写入消息', async t => {
  const { ui, polling } = await ready(t);
  await pick(ui, null);
  assert.equal(ui.native.count('pick_import_files'), 1);
  assert.deepEqual(ui.native.matching('pick_import_files')[0].payload, { sessionId: vault.session_id, scope: 'personal', format: 'auto' });
  ui.button('选择导出文件');
  assert.equal(polling.count, 0); ui.noWrites();
});

test('文件无法解析时说明失败并保留重新选择入口，未启动任何任务', async t => {
  const { ui, polling } = await ready(t);
  ui.native.fail('pick_import_files', '导入文件无法完整解析；尚未写入任何消息');
  await ui.click('选择导出文件');
  assert.match(ui.one('#notice').textContent, /无法完整解析.*尚未写入/);
  ui.button('选择导出文件');
  assert.equal(polling.count, 0); ui.noWrites();
});

test('多文件预览展示合并数量、目标及纯文本样本，取消使旧批准按钮失效', async t => {
  const { ui } = await ready(t);
  await pick(ui, { ...batch, files: [{ ...batch.files[0], file_name: hostile }, batch.files[1]], warning: hostile, samples: [{ ...batch.samples[0], content: hostile }] });
  const section = pane(ui);
  assert.match(section.textContent, /2 个文件 · 2 个有消息的会话 · 6 条消息/);
  assert.match(section.textContent, /合成资料库.*个人资料/);
  assert.ok(section.textContent.includes(hostile));
  assert.equal(section.querySelectorAll('img, script, iframe, [onerror], [onclick]').length, 0);
  assert.equal(ui.one('.sample > p').textContent, hostile);
  assert.equal(ui.one('.sample > p').childNodes.length, 1);
  assert.equal(ui.one('.sample > p').firstChild.nodeType, ui.window.Node.TEXT_NODE);
  const old = ui.button('导入全部 6 条消息'); ui.noWrites();
  await ui.click('取消', section); await replay(ui, old);
  ui.button('选择导出文件');
  assert.equal(ui.document.querySelector('.sample'), null);
  assert.equal(ui.modal().open, false); ui.noWrites();
});

test('重新选择后旧取消按钮不能撤销新的待批准批次', async t => {
  const { ui } = await ready(t); await pick(ui);
  const oldCancel = ui.button('取消', pane(ui));
  await ui.click('取消', pane(ui));
  await pick(ui, { ...batch, preview_id: 'new-preview' });
  const cancellations = ui.native.count('cancel_previews');
  await replay(ui, oldCancel);
  ui.button('导入全部 6 条消息');
  assert.equal(ui.native.count('cancel_previews'), cancellations); ui.noWrites();
  ui.native.next('start_import_job', completed); await ui.click('导入全部 6 条消息');
  assert.equal(ui.native.matching('start_import_job')[0].payload.previewId, 'new-preview');
});

test('十万条有效预览仍只需一次批准，不在界面偷偷缩小导入范围', async t => {
  const { ui } = await ready(t);
  await pick(ui, { ...batch, event_count: 100000, conversations: [{ ...batch.conversations[0], event_count: 100000 }] });
  ui.noWrites();
  ui.native.next('start_import_job', { ...completed, events_total: 100000, events_processed: 100000, events_added: 100000, events_duplicates: 0 });
  await ui.click('导入全部 100000 条消息');
  assert.equal(ui.native.count('start_import_job'), 1);
  assert.equal(ui.one('progress').value, 100000);
  assert.equal(ui.one('progress').max, 100000);
});

for (const [name, changes] of [
  ['没有可导入消息', { event_count: 0 }],
  ['消息数量为负数', { event_count: -1 }],
  ['消息数量不是整数', { event_count: 1.5 }],
  ['消息数量超过允许上限', { event_count: 100001 }],
  ['批准标识缺失', { preview_id: '' }],
  ['文件列表为空', { files: [] }],
  ['文件数量超过允许上限', { files: Array.from({ length: 33 }, () => batch.files[0]) }],
  ['文件列表缺失', { files: null }],
  ['文件元数据无效', { files: [{ file_name: 'invalid.json', byte_count: -1 }] }],
  ['会话列表缺失', { conversations: null }],
  ['文字样本无效', { samples: [{ content: null }] }],
  ['脱敏数量超过消息总数', { redacted_event_count: 7 }],
  ['其他资料库的预览', { session_id: 'unexpected-session' }],
  ['其他范围的预览', { scope: 'work' }],
]) {
  test(`多文件预览${name}时不能提供批准写入入口`, async t => {
    const { ui } = await ready(t);
    await pick(ui, { ...batch, ...changes });
    assert.equal([...pane(ui).querySelectorAll('button')].some(node => /^导入全部 /.test(node.textContent) && !node.disabled), false,
      '不可验证的预览不得成为可点击的写入批准');
    assert.match(ui.one('#notice').textContent, /无法|无可|没有|无效|失效|重新|为空/);
    ui.button('选择导出文件'); ui.noWrites();
  });
}

test('多文件只需一次批准，双击及旧节点只启动一次且进度由真实轮询更新', async t => {
  const { ui, polling } = await ready(t);
  const showModal = t.mock.method(ui.window.HTMLDialogElement.prototype, 'showModal');
  await pick(ui); ui.noWrites();
  const approve = ui.button('导入全部 6 条消息');
  const release = ui.native.hold('start_import_job');
  approve.click(); approve.dispatchEvent(new ui.window.MouseEvent('click'));
  assert.equal(ui.native.count('start_import_job'), 1);
  release(running); await ui.idle(); await replay(ui, approve);
  assert.equal(ui.native.count('start_import_job'), 1);
  assert.deepEqual(ui.native.matching('start_import_job')[0].payload, { sessionId: vault.session_id, scope: 'personal', previewId: batch.preview_id });
  assert.equal(showModal.mock.callCount(), 0);
  assert.equal(ui.modal().open, false);
  assert.equal(ui.one('progress').value, 0);
  assert.equal(ui.one('progress').max, 6);
  assert.equal(ui.one('[aria-label="资料范围"]').disabled, true);
  assert.equal(ui.one('#switch-vault').disabled, true);
  ui.native.next('import_job_status', { ...running, events_processed: 3, events_added: 3 });
  await polling.fire();
  assert.match(pane(ui).textContent, /已处理 3 \/ 6 条消息/);
  assert.equal(ui.one('progress').value, 3);
  ui.native.next('import_job_status', completed); await polling.fire();
  assert.match(pane(ui).textContent, /导入完成/);
  assert.equal(ui.one('progress').value, 6);
  assert.equal(polling.count, 0);
  assert.equal(ui.one('#switch-vault').disabled, false);
  assert.equal(ui.native.count('confirm_import'), 0);
  assert.deepEqual(ui.native.matching('import_job_status').map(call => call.payload), [
    { sessionId: vault.session_id, scope: 'personal', jobId: running.job_id },
    { sessionId: vault.session_id, scope: 'personal', jobId: running.job_id },
  ]);
});

test('开始导入失败清除旧预览，重新选择审阅之前不能重放批准', async t => {
  const { ui } = await ready(t); await pick(ui);
  const old = ui.button('导入全部 6 条消息');
  ui.native.fail('start_import_job', '文件已改变，请重新选择并检查');
  await ui.click('导入全部 6 条消息'); await replay(ui, old);
  assert.equal(ui.native.count('start_import_job'), 1);
  assert.match(ui.one('#notice').textContent, /文件已改变/);
  ui.button('选择导出文件');
  await pick(ui); await replay(ui, old);
  assert.equal(ui.native.count('start_import_job'), 1);
  ui.native.next('start_import_job', completed); await ui.click('导入全部 6 条消息');
  assert.equal(ui.native.count('start_import_job'), 2);
  assert.match(pane(ui).textContent, /导入完成/);
});

for (const [label, change] of [
  ['离开导入页面', ui => ui.navigate('会话')],
  ['切换范围', async ui => { ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle(); }],
  ['切换资料库', switchVault],
  ['同页重新绘制', ui => ui.navigate('添加资料')],
]) {
  test(`多文件批准按钮在${label}后不能重放`, async t => {
    const { ui } = await ready(t); await pick(ui);
    const old = ui.button('导入全部 6 条消息');
    await change(ui); await replay(ui, old); ui.noWrites();
  });
}

test('暂停先显示停止中，再保留已保存数量；继续必须显式点击且重复点击只恢复一次', async t => {
  const { ui, polling } = await ready(t); await start(ui);
  ui.native.next('cancel_import_job', { ...running, state: 'cancelling', events_processed: 2, events_added: 2 });
  await ui.click('暂停导入');
  assert.match(pane(ui).textContent, /正在暂停/);
  assert.equal([...pane(ui).querySelectorAll('button')].some(node => node.textContent === '继续导入'), false);
  ui.native.next('import_job_status', paused); await polling.fire();
  assert.match(pane(ui).textContent, /已暂停.*已处理 2 \/ 6 条消息/);
  assert.equal(polling.count, 0);
  assert.equal(ui.native.count('resume_import_job'), 0);
  const resume = ui.button('继续导入');
  const release = ui.native.hold('resume_import_job'); resume.click(); resume.dispatchEvent(new ui.window.MouseEvent('click'));
  assert.equal(ui.native.count('resume_import_job'), 1);
  release({ ...running, events_processed: 2, events_added: 2 }); await ui.idle(); await replay(ui, resume);
  assert.equal(ui.native.count('resume_import_job'), 1);
  assert.deepEqual(ui.native.matching('resume_import_job')[0].payload, { sessionId: vault.session_id, scope: 'personal', jobId: running.job_id });
  assert.equal(polling.count, 1);
  assert.equal(ui.modal().open, false);
});

test('暂停请求失败后明确给出重读入口，仍可恢复可核实的任务进度', async t => {
  const { ui, polling } = await ready(t); await start(ui);
  ui.native.fail('cancel_import_job', '合成暂停请求失败');
  await ui.click('暂停导入');
  assert.match(ui.one('#notice').textContent, /合成暂停请求失败/);
  assert.match(pane(ui).textContent, /仍在进行|重新读取|无法核实/);
  ui.button('重新读取进度');
  ui.native.next('import_job_status', completed); await ui.click('重新读取进度');
  assert.match(pane(ui).textContent, /导入完成/);
  assert.equal(polling.count, 0);
  assert.equal(ui.native.count('start_import_job'), 1);
});

test('继续请求失败后保留暂停任务和明确重读入口，旧继续节点不可重试', async t => {
  const { ui, polling } = await ready(t); await start(ui, paused);
  const oldResume = ui.button('继续导入');
  ui.native.fail('resume_import_job', '合成继续请求失败'); await ui.click('继续导入');
  assert.match(ui.one('#notice').textContent, /合成继续请求失败/);
  assert.equal(ui.one('progress').value, 2);
  ui.button('重新读取进度');
  await replay(ui, oldResume);
  assert.equal(ui.native.count('resume_import_job'), 1);
  assert.equal(polling.count, 0);
  ui.native.next('import_job_status', { ...running, events_processed: 3, events_added: 3 });
  await ui.click('重新读取进度');
  assert.equal(ui.one('progress').value, 3);
  assert.equal(polling.count, 1);
});

test('恢复之后暂停页的旧导入其他文件按钮不能清除正在运行的任务', async t => {
  const { ui, polling } = await ready(t); await start(ui, paused);
  const oldOther = ui.button('导入其他文件');
  ui.native.next('resume_import_job', { ...running, events_processed: 2, events_added: 2 });
  await ui.click('继续导入'); await replay(ui, oldOther);
  assert.match(pane(ui).textContent, /正在导入.*已处理 2 \/ 6 条消息/);
  ui.button('暂停导入');
  assert.equal([...pane(ui).querySelectorAll('button')].some(node => node.textContent === '选择导出文件'), false);
  assert.equal(polling.count, 1);
});

test('恢复历史以后旧查看并继续按钮不能把运行任务退回暂停', async t => {
  const { ui, polling } = await ready(t);
  ui.native.next('list_import_jobs', [paused]); await ui.click('查看未完成的导入');
  const oldHistory = ui.button('查看并继续');
  ui.native.next('import_job_status', paused);
  await ui.click('查看并继续');
  ui.native.next('resume_import_job', { ...running, events_processed: 4, events_added: 4 });
  await ui.click('继续导入'); await replay(ui, oldHistory);
  assert.match(pane(ui).textContent, /正在导入.*已处理 4 \/ 6 条消息/);
  assert.equal(ui.one('progress').value, 4);
  assert.equal(polling.count, 1);
  assert.equal(ui.native.count('resume_import_job'), 1);
});

for (const state of ['interrupted', 'failed', 'cancelled']) {
  test(`历史 ${state} 任务只读取和展示，用户点击继续后才恢复`, async t => {
    const { ui, polling } = await ready(t);
    ui.native.next('list_import_jobs', [{ ...paused, state }]);
    await ui.click('查看未完成的导入');
    assert.match(ui.one('.import-history-row').textContent, /2 个文件 · 已处理 2 \/ 6 条/);
    assert.equal(polling.count, 0); ui.noWrites();
    ui.native.next('import_job_status', { ...paused, state });
    await ui.click('查看并继续');
    assert.equal(ui.one('progress').value, 2); ui.noWrites();
    assert.deepEqual(ui.native.matching('import_job_status')[0].payload, { sessionId: vault.session_id, scope: 'personal', jobId: running.job_id });
    ui.native.next('resume_import_job', running); await ui.click('继续导入');
    assert.equal(ui.native.count('resume_import_job'), 1);
    assert.equal(polling.count, 1);
    assert.equal(ui.native.count('start_import_job'), 0);
  });
}

test('查看历史时任务已完成，以重新核实的状态为准且不再显示继续按钮', async t => {
  const { ui, polling } = await ready(t);
  ui.native.next('list_import_jobs', [paused]); await ui.click('查看未完成的导入');
  ui.native.next('import_job_status', completed); await ui.click('查看并继续');
  assert.match(pane(ui).textContent, /导入完成.*已处理 6 \/ 6 条消息/);
  assert.equal([...pane(ui).querySelectorAll('button')].some(node => node.textContent === '继续导入'), false);
  assert.equal(polling.count, 0); ui.noWrites();
});

test('历史状态重新核实失败不显示可恢复批准，重新读取成功后再继续', async t => {
  const { ui } = await ready(t);
  ui.native.next('list_import_jobs', [paused]); await ui.click('查看未完成的导入');
  ui.native.fail('import_job_status', '合成历史状态读取失败'); await ui.click('查看并继续');
  assert.match(ui.one('#notice').textContent, /合成历史状态读取失败/);
  assert.equal(ui.document.querySelector('progress'), null);
  assert.equal([...pane(ui).querySelectorAll('button')].some(node => node.textContent === '继续导入'), false);
  ui.noWrites();
  ui.native.next('import_job_status', paused); await ui.click('查看并继续');
  assert.equal(ui.one('progress').value, 2);
  ui.button('继续导入'); ui.noWrites();
});

test('恢复历史中仍活动的导入只轮询状态，不重复启动或恢复任务', async t => {
  const { ui, polling } = await ready(t);
  ui.native.next('list_import_jobs', [{ ...running, events_processed: 3, events_added: 3 }]);
  await ui.click('查看未完成的导入');
  assert.equal(ui.one('progress').value, 3); ui.noWrites();
  ui.native.next('import_job_status', completed); await polling.fire();
  assert.match(pane(ui).textContent, /导入完成/);
  assert.equal(polling.count, 0); ui.noWrites();
});

test('历史中没有可继续任务时说明状态，不偷偷创建新导入', async t => {
  const { ui, polling } = await ready(t);
  ui.native.next('list_import_jobs', [completed]);
  await ui.click('查看未完成的导入');
  assert.match(pane(ui).textContent, /没有需要继续的导入/);
  assert.equal(ui.document.querySelector('.import-history-row'), null);
  assert.equal(polling.count, 0); ui.noWrites();
});

test('轮询失败保留已核实进度并提供重读和暂停，不假报失败或完成', async t => {
  const { ui, polling } = await ready(t);
  await start(ui, { ...running, events_processed: 2, events_added: 2 });
  ui.native.fail('import_job_status', '合成状态读取失败'); await polling.fire();
  assert.equal(ui.one('progress').value, 2);
  assert.match(pane(ui).textContent, /导入可能仍在进行/);
  ui.button('暂停导入'); ui.button('重新读取进度');
  assert.equal(polling.count, 0, '读取失败应等待用户选择，避免忙重试');
  ui.native.next('import_job_status', completed); await ui.click('重新读取进度');
  assert.match(pane(ui).textContent, /导入完成/);
  assert.equal(pane(ui).querySelector('.hint.warning'), null);
  assert.equal(ui.native.count('start_import_job'), 1);
});

for (const [label, result] of [
  ['其他范围', { ...running, scope: 'work' }],
  ['其他任务', { ...running, job_id: 'unexpected-job' }],
  ['超过消息总数', { ...running, events_processed: 7 }],
  ['非法任务状态', { ...running, state: 'unknown' }],
  ['新增重复计数与已处理数矛盾', { ...running, events_processed: 2, events_added: 1 }],
  ['未处理完却声明完成', { ...running, state: 'completed', events_processed: 2, events_added: 2 }],
]) {
  test(`轮询返回${label}时保留上次核实结果，不替换当前导入`, async t => {
    const { ui, polling } = await ready(t);
    await start(ui, { ...running, events_processed: 2, events_added: 2 });
    ui.native.next('import_job_status', result); await polling.fire();
    assert.equal(ui.one('progress').value, 2);
    assert.match(pane(ui).textContent, /无法读取|无法核实/);
    assert.equal(polling.count, 0);
    ui.native.next('import_job_status', completed); await ui.click('重新读取进度');
    assert.equal(ui.native.matching('import_job_status').at(-1).payload.jobId, running.job_id);
  });
}

test('暂停完成以后较早的轮询响应不能把任务变回运行中', async t => {
  const { ui, polling } = await ready(t); await start(ui);
  const release = ui.native.hold('import_job_status');
  const pending = polling.fire();
  assert.equal(ui.native.count('import_job_status'), 1);
  ui.native.next('cancel_import_job', paused); await ui.click('暂停导入');
  assert.match(pane(ui).textContent, /已暂停/);
  release({ ...running, events_processed: 1, events_added: 1 }); await pending;
  assert.match(pane(ui).textContent, /已暂停.*已处理 2 \/ 6 条消息/);
  assert.equal(ui.one('progress').value, 2);
  assert.equal(polling.count, 0);
  ui.button('继续导入');
});

test('恢复成功以后旧轮询不能回退进度或重新显示可继续按钮', async t => {
  const { ui, polling } = await ready(t); await start(ui);
  const release = ui.native.hold('import_job_status'); const pending = polling.fire();
  ui.native.next('cancel_import_job', paused); await ui.click('暂停导入');
  ui.native.next('resume_import_job', { ...running, events_processed: 4, events_added: 4 }); await ui.click('继续导入');
  release(paused); await pending;
  assert.match(pane(ui).textContent, /正在导入.*已处理 4 \/ 6 条消息/);
  assert.equal(ui.one('progress').value, 4);
  assert.equal(polling.count, 1);
  ui.button('暂停导入');
});

for (const [label, change, sessionId, scope] of [
  ['范围', async ui => { ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle(); }, vault.session_id, 'work'],
  ['资料库', switchVault, 'synthetic-second-session', 'personal'],
]) {
  test(`暂停后切换${label}，旧任务响应和旧继续按钮不会污染新上下文`, async t => {
    const { ui, polling } = await ready(t); await start(ui);
    const release = ui.native.hold('import_job_status'); const pending = polling.fire();
    ui.native.next('cancel_import_job', paused); await ui.click('暂停导入');
    const oldResume = ui.button('继续导入');
    await change(ui); await ui.navigate('添加资料');
    release(completed); await pending; await replay(ui, oldResume);
    ui.button('选择导出文件');
    assert.equal(ui.document.querySelector('progress'), null);
    assert.equal(ui.native.count('resume_import_job'), 0);
    assert.equal(polling.count, 0);
    await pick(ui, { ...batch, session_id: sessionId, scope });
    assert.deepEqual(ui.native.matching('pick_import_files').at(-1).payload, { sessionId, scope, format: 'auto' });
    ui.native.next('start_import_job', { ...completed, scope, job_id: 'second-context-job' });
    await ui.click('导入全部 6 条消息');
    assert.deepEqual(ui.native.matching('start_import_job').at(-1).payload, { sessionId, scope, previewId: batch.preview_id });
    assert.match(pane(ui).textContent, /导入完成/);
  });
}

test('暂停任务的已保存会话入口重新读取列表和原话，返回后仍可查看任务', async t => {
  const { ui } = await ready(t); await start(ui, paused);
  const reads = ui.native.count('list_conversations');
  await ui.click('查看已保存会话');
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.equal(ui.native.count('list_conversations'), reads + 1);
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  assert.equal(ui.native.matching('list_conversations').at(-1).payload.sessionId, vault.session_id);
  await ui.navigate('添加资料');
  assert.match(pane(ui).textContent, /已暂停/); ui.button('继续导入');
  assert.equal(ui.native.count('resume_import_job'), 0);
});

test('完成后查看已保存会话的读取失败不能残留旧正文', async t => {
  const { ui } = await ready(t); await start(ui, completed);
  ui.native.fail('list_conversations', '合成会话列表读取失败');
  await ui.click('查看已保存会话');
  assert.match(ui.one('#notice').textContent, /合成会话列表读取失败/);
  assert.equal(ui.document.querySelector('.conversation-reader .body-text'), null);
});

test('官方导出指南只打开已限定的 DeepSeek 入口，不自动选择文件或启动导入', async t => {
  const { ui } = await ready(t);
  assert.match(ui.one('.official-export-guide').textContent, /头像.*系统设置.*数据管理.*导出所有历史对话/);
  await ui.click('打开 DeepSeek');
  assert.deepEqual(ui.native.matching('open_deepseek'), [{ command: 'open_deepseek', payload: {} }]);
  assert.match(ui.one('#notice').textContent, /下载后回到这里选择文件/);
  assert.equal(ui.native.count('pick_import_files'), 0); ui.noWrites();
});
