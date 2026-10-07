import test from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate } from 'node:timers/promises';
import { fixture } from './harness.mjs';
import { conversation, conversationMessages, vault } from './fixtures.mjs';

// 所有响应均为公开结构的合成资料。应用仍从原文件加载；恢复选择与 DOM 逻辑不在测试中重写。
const restored = { vault, scope: 'personal' };
const interrupted = {
  job_id: 'synthetic-recovered-job', scope: 'personal', state: 'interrupted',
  events_total: 6, events_processed: 2, events_added: 2, events_duplicates: 0,
  files_total: 2, conversations_total: 2, can_resume: true,
  message: '上次导入已中断；已保存消息仍在资料库中',
  created_at: '2026-10-06T10:00:00Z', updated_at: '2026-10-07T10:00:00Z',
};
const completed = { ...interrupted, state: 'completed', events_processed: 6,
  events_added: 5, events_duplicates: 1, can_resume: false, message: '全部合成消息已处理' };
const pane = ui => ui.one('[aria-label="导入平台历史"]');
const commandNames = ui => ui.native.calls.map(call => call.command);

function noAutomaticImport(ui) {
  ui.noAutomaticWrites();
  assert.equal(ui.native.count('pick_import_files'), 0, '恢复不得自动打开文件选择');
  assert.equal(ui.native.count('choose_vault'), 0, '恢复不得自动打开目录选择');
  assert.equal(ui.native.count('cancel_import_job'), 0, '恢复不得自动暂停或修改任务');
  assert.equal(ui.native.count('open_deepseek'), 0, '恢复不得自动打开网站');
}
function noResumeControl(ui) {
  assert.equal([...ui.document.querySelectorAll('button')].some(node => node.textContent === '继续导入'), false);
}
async function replay(ui, node) {
  node.disabled = false;
  node.dispatchEvent(new ui.window.MouseEvent('click', { bubbles: true }));
  await ui.idle();
}

test('首次启动没有最近资料库时只读检查，不创建默认库、不保存设置或弹窗', async t => {
  const ui = await fixture(t);
  assert.deepEqual(ui.native.calls, [{ command: 'restore_workspace', payload: {} }]);
  assert.equal(ui.one('#content h1').textContent, '把以前的对话接着用');
  assert.equal(ui.one('#vault-badge').textContent, '尚未打开资料库');
  for (const label of ['开始使用', '打开已有资料库', '创建新资料库']) ui.button(label);
  assert.equal(ui.modal().open, false);
  assert.deepEqual(ui.shownDialogs, []);
  noAutomaticImport(ui);
});

test('已有资料库启动恢复后自动读取真实会话原文，不再保存最近位置', async t => {
  const ui = await fixture(t, { restoreResponse: restored });
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.match(ui.one('#vault-badge').textContent, /合成资料库/);
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源，再继续实现/);
  assert.equal(ui.native.count('restore_workspace'), 1);
  assert.deepEqual(ui.native.matching('list_import_jobs'), [{ command: 'list_import_jobs', payload: { sessionId: vault.session_id, scope: 'personal' } }]);
  assert.deepEqual(ui.native.matching('conversation_messages')[0].payload, {
    sessionId: vault.session_id, scope: 'personal', conversationRef: conversation.session_ref, offset: 0,
  });
  assert.ok(commandNames(ui).indexOf('list_import_jobs') < commandNames(ui).indexOf('list_conversations'));
  assert.deepEqual(ui.shownDialogs, []);
  noAutomaticImport(ui);
});

for (const scope of ['personal', 'work']) {
  test(`恢复时保持保存的 ${scope} 范围，不能用资料库首个范围覆盖`, async t => {
    const other = scope === 'personal' ? 'work' : 'personal';
    const ui = await fixture(t, {
      restoreResponse: { vault: { ...vault, scopes: [other, scope] }, scope },
      configureNative(native) {
        native.next('list_conversations', { conversations: [conversation], total: 1, next_offset: null });
        native.next('conversation_messages', { ...conversation, messages: conversationMessages, total: 2, offset: 0, next_offset: null, order_known: true });
      },
    });
    assert.equal(ui.one('[aria-label="资料范围"]').value, scope);
    for (const command of ['list_import_jobs', 'list_conversations', 'conversation_messages']) {
      assert.equal(ui.native.matching(command).length, 1);
      assert.equal(ui.native.matching(command)[0].payload.scope, scope);
      assert.equal(ui.native.matching(command)[0].payload.sessionId, vault.session_id);
    }
    assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
    noAutomaticImport(ui);
  });
}

for (const reason of ['最近资料库位置不存在，请重新选择', '最近资料库设置损坏，无法恢复', '资料库不完整，不能打开']) {
  test(`恢复失败「${reason}」保留开始入口，不默默建立替代库`, async t => {
    const ui = await fixture(t, { restoreError: reason });
    assert.ok(ui.one('#notice').textContent.includes(reason));
    assert.equal(ui.one('#vault-badge').textContent, '尚未打开资料库');
    assert.equal(ui.one('#content h1').textContent, '把以前的对话接着用');
    assert.equal(ui.document.querySelector('.conversation-reader'), null);
    assert.equal(ui.native.count('list_conversations'), 0);
    assert.equal(ui.native.count('list_import_jobs'), 0);
    assert.deepEqual(ui.shownDialogs, []);
    noAutomaticImport(ui);
    await ui.openVault();
    assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
    assert.equal(ui.native.count('remember_workspace'), 1, '恢复失败后由用户明确打开才记住新位置');
  });
}

for (const [name, response] of [
  ['缺少资料库', { scope: 'personal' }],
  ['缺少会话标识', { ...restored, vault: { ...vault, session_id: '' } }],
  ['范围为空白', { ...restored, scope: '  ' }],
  ['范围包含控制字符', { ...restored, scope: 'personal\nwork' }],
]) {
  test(`启动返回${name}时拒绝绑定资料，不查询会话或改写设置`, async t => {
    const ui = await fixture(t, { restoreResponse: response });
    assert.match(ui.one('#notice').textContent, /无法核实|无效|重新选择/);
    assert.equal(ui.one('#vault-badge').textContent, '尚未打开资料库');
    assert.equal(ui.native.count('list_import_jobs'), 0);
    assert.equal(ui.native.count('list_conversations'), 0);
    assert.equal(ui.native.count('conversation_messages'), 0);
    noAutomaticImport(ui);
  });
}

test('启动恢复尚未返回时禁用创建和打开，重复旧入口不能抢建资料库', async t => {
  const ui = await fixture(t, { holdStartup: true });
  assert.equal(ui.native.count('restore_workspace'), 1);
  for (const label of ['开始使用', '打开已有资料库', '创建新资料库']) {
    const node = ui.button(label);
    assert.equal(node.disabled, true, '启动读尚未结束时不能并发更换资料库');
    node.dispatchEvent(new ui.window.MouseEvent('click', { bubbles: true }));
  }
  assert.deepEqual(commandNames(ui), ['restore_workspace']);
  ui.releaseStartup(restored); await ui.idle();
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  assert.equal(ui.native.count('restore_workspace'), 1);
  noAutomaticImport(ui);
});

for (const [label, command, create] of [
  ['打开已有资料库', 'choose_vault', false],
  ['创建新资料库', 'choose_vault', true],
  ['开始使用', 'open_default_workspace', undefined],
]) {
  test(`用户明确「${label}」成功后才保存本次 session 与范围`, async t => {
    const ui = await fixture(t);
    const chosen = { ...vault, session_id: 'synthetic-explicit-open', scopes: ['work', 'personal'] };
    const release = ui.native.hold(command);
    ui.button(label).click();
    assert.equal(ui.native.count('remember_workspace'), 0, '打开操作尚未成功不得提前记住位置');
    release(chosen); await ui.idle();
    assert.deepEqual(ui.native.matching('remember_workspace'), [{ command: 'remember_workspace', payload: { sessionId: chosen.session_id, scope: 'work' } }]);
    if (create !== undefined) assert.equal(ui.native.matching(command)[0].payload.create, create);
    assert.equal(ui.one('[aria-label="资料范围"]').value, 'work');
    assert.equal(ui.modal().open, false);
    ui.noWrites({ allowDefaultWorkspace: command === 'open_default_workspace' });
  });
}

for (const command of ['choose_vault', 'open_default_workspace']) {
  test(`${command} 的最近位置保存失败只提示，已打开资料仍可读`, async t => {
    const ui = await fixture(t);
    ui.native.next(command, vault);
    ui.native.fail('remember_workspace', '合成设置位置不可写，不能记住资料库');
    await ui.click(command === 'choose_vault' ? '打开已有资料库' : '开始使用');
    assert.match(ui.one('#notice').textContent, /不能记住|无法记住|未能记住|保存.*失败/);
    assert.match(ui.one('#vault-badge').textContent, /合成资料库/);
    assert.equal(ui.one('#content h1').textContent, '会话');
    assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
    assert.equal(ui.native.count('remember_workspace'), 1);
    await ui.navigate('记忆'); await ui.navigate('会话');
    assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
    ui.noWrites({ allowDefaultWorkspace: command === 'open_default_workspace' });
  });
}

test('取消选择资料库与打开失败不会保存最近位置', async t => {
  const ui = await fixture(t);
  ui.native.next('choose_vault', null); await ui.openVault();
  assert.equal(ui.native.count('remember_workspace'), 0);
  ui.native.fail('choose_vault', '合成资料库读取失败'); await ui.openVault();
  assert.match(ui.one('#notice').textContent, /合成资料库读取失败/);
  assert.equal(ui.native.count('remember_workspace'), 0);
  ui.noAutomaticWrites();
});

test('范围保存挂起时连续切换只串行保存首次与最新选择，过期范围不再调用原生设置', async t => {
  const ui = await fixture(t, { restoreResponse: { ...restored, vault: { ...vault, scopes: ['personal', 'work', 'research'] } } });
  const release = ui.native.hold('remember_workspace');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  assert.deepEqual(ui.native.matching('remember_workspace').map(call => call.payload), [{ sessionId: vault.session_id, scope: 'work' }]);
  ui.fill('[aria-label="资料范围"]', 'personal', 'change'); await ui.idle();
  ui.fill('[aria-label="资料范围"]', 'research', 'change'); await ui.idle();
  assert.equal(ui.native.count('remember_workspace'), 1, '前一保存尚未返回时，后续设置不得并发覆盖');
  release(null); await ui.idle();
  assert.deepEqual(ui.native.matching('remember_workspace').map(call => call.payload), [
    { sessionId: vault.session_id, scope: 'work' },
    { sessionId: vault.session_id, scope: 'research' },
  ]);
  assert.equal(ui.one('[aria-label="资料范围"]').value, 'research');
  assert.deepEqual(ui.shownDialogs, []);
  ui.noWrites();
});

test('旧范围设置保存挂起后明确改开另一资料库，最后保存属于新 session', async t => {
  const ui = await fixture(t, { restoreResponse: restored });
  const release = ui.native.hold('remember_workspace');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  const next = { ...vault, session_id: 'synthetic-replacement-session', display_name: '新选择的合成资料库' };
  ui.native.next('choose_vault', next);
  ui.one('#switch-vault').click();
  ui.button('打开已有资料库', ui.modal()).click(); await setImmediate();
  assert.equal(ui.native.count('choose_vault'), 1);
  assert.equal(ui.native.count('remember_workspace'), 1);
  release(null); await ui.idle();
  assert.deepEqual(ui.native.matching('remember_workspace').map(call => call.payload), [
    { sessionId: vault.session_id, scope: 'work' },
    { sessionId: next.session_id, scope: 'personal' },
  ]);
  assert.match(ui.one('#vault-badge').textContent, /新选择的合成资料库/);
  assert.equal(ui.one('[aria-label="资料范围"]').value, 'personal');
  assert.equal(ui.native.matching('conversation_messages').at(-1).payload.sessionId, next.session_id);
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  ui.noWrites();
});

for (const state of ['interrupted', 'cancelled']) {
  test(`启动发现 ${state} 导入只恢复进度，明确继续、双击及旧按钮合计只恢复一次`, async t => {
    const current = { ...interrupted, state };
    const ui = await fixture(t, { restoreResponse: restored, importJobs: [current] });
    assert.equal(ui.one('#content h1').textContent, '导入会话');
    assert.equal(ui.one('progress').value, 2);
    assert.equal(ui.one('progress').max, 6);
    assert.match(pane(ui).textContent, /已处理 2 \/ 6 条消息/);
    assert.equal(ui.native.count('list_conversations'), 0);
    assert.equal(ui.native.count('conversation_messages'), 0);
    assert.equal(ui.native.count('import_job_status'), 0, '暂停任务没有自动启动轮询');
    assert.deepEqual(ui.shownDialogs, []);
    noAutomaticImport(ui);
    const button = ui.button('继续导入');
    const release = ui.native.hold('resume_import_job');
    button.click(); button.dispatchEvent(new ui.window.MouseEvent('click', { bubbles: true }));
    assert.deepEqual(ui.native.matching('resume_import_job'), [{ command: 'resume_import_job', payload: { sessionId: vault.session_id, scope: 'personal', jobId: current.job_id } }]);
    release(completed); await ui.idle(); await replay(ui, button);
    assert.equal(ui.native.count('resume_import_job'), 1);
    assert.equal(ui.native.count('start_import_job'), 0);
    assert.match(pane(ui).textContent, /导入完成.*已处理 6 \/ 6 条消息/);
    assert.equal(ui.native.count('remember_workspace'), 0);
  });
}

test('按原生清单顺序恢复最新可继续任务，旧任务不抢占', async t => {
  const latest = { ...interrupted, job_id: 'latest-unfinished', events_processed: 4, events_added: 4,
    created_at: '2026-10-07T11:00:00Z', updated_at: '2026-10-07T11:00:00Z', message: '应显示这个最新可恢复任务' };
  const ui = await fixture(t, { restoreResponse: restored, importJobs: [
    latest,
    { ...interrupted, job_id: 'middle-unfinished', created_at: '2026-10-07T10:30:00Z', updated_at: '2026-10-07T10:30:00Z' },
    interrupted,
  ] });
  assert.equal(ui.one('progress').value, 4);
  assert.match(pane(ui).textContent, /应显示这个最新可恢复任务/);
  noAutomaticImport(ui);
  ui.native.next('resume_import_job', { ...completed, job_id: latest.job_id });
  await ui.click('继续导入');
  assert.equal(ui.native.matching('resume_import_job')[0].payload.jobId, latest.job_id);
});

test('最新任务已完成时不把较早暂停任务拉回首页，用户仍能从历史入口查看', async t => {
  const jobs = [{ ...completed, job_id: 'latest-completed', created_at: '2026-10-07T12:00:00Z' }, interrupted];
  const ui = await fixture(t, { restoreResponse: restored, importJobs: jobs });
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  assert.deepEqual(ui.shownDialogs, []);
  noAutomaticImport(ui);
  await ui.navigate('添加资料');
  ui.native.next('list_import_jobs', jobs); await ui.click('查看导入记录');
  const history = [...ui.document.querySelectorAll('.import-history-row')];
  assert.equal(history.length, 2);
  const unfinished = history.find(node => node.textContent.includes('已处理 2 / 6 条'));
  assert.ok(unfinished);
  ui.native.next('import_job_status', interrupted); await ui.click('查看这次导入', unfinished);
  assert.equal(ui.one('progress').value, 2);
  ui.button('继续导入'); noAutomaticImport(ui);
});

test('启动发现仍在运行的任务优先显示状态，只轮询核实而不重复启动', async t => {
  const running = { ...interrupted, state: 'running', can_resume: false, message: '合成任务仍在运行' };
  const ui = await fixture(t, { holdStartup: true, importJobs: [completed, running] });
  const timers = new Map();
  let sequence = 0;
  t.mock.method(ui.window, 'setTimeout', (callback, delay) => {
    const id = ++sequence; timers.set(id, { callback, delay }); return id;
  });
  t.mock.method(ui.window, 'clearTimeout', id => { timers.delete(id); });
  ui.releaseStartup(restored); await ui.idle();
  assert.match(pane(ui).textContent, /正在导入.*合成任务仍在运行/);
  assert.equal(ui.one('progress').value, 2);
  assert.equal(timers.size, 1);
  assert.equal(ui.native.count('import_job_status'), 0);
  noAutomaticImport(ui);
  const [id, timer] = timers.entries().next().value;
  assert.equal(timer.delay, 600);
  timers.delete(id);
  ui.native.next('import_job_status', completed); await timer.callback();
  assert.equal(timers.size, 0);
  assert.match(pane(ui).textContent, /导入完成.*已处理 6 \/ 6 条消息/);
  noAutomaticImport(ui);
});

test('启动只有已完成或不可恢复的任务时正常读会话，不自动显示导入窗', async t => {
  const ui = await fixture(t, { restoreResponse: restored, importJobs: [completed,
    { ...interrupted, job_id: 'synthetic-failed', state: 'failed', can_resume: false },
  ] });
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  assert.equal(ui.document.querySelector('progress'), null);
  assert.deepEqual(ui.shownDialogs, []);
  noResumeControl(ui); noAutomaticImport(ui);
});

for (const [name, jobs] of [
  ['清单不是数组', { job: interrupted }],
  ['任务属于其他范围', [{ ...interrupted, scope: 'work' }]],
  ['任务标识为空', [{ ...interrupted, job_id: '' }]],
  ['任务状态未知', [{ ...interrupted, state: 'unknown' }]],
  ['恢复标记不是布尔值', [{ ...interrupted, can_resume: 'true' }]],
  ['进度计数为负数', [{ ...interrupted, events_processed: -1 }]],
  ['新增加重复与进度不符', [{ ...interrupted, events_added: 0 }]],
  ['未处理完却声称完成', [{ ...interrupted, state: 'completed', can_resume: false }]],
]) {
  test(`恢复读取${name}时拒绝继续且不默默宣告完成，已有资料仍可查看`, async t => {
    const ui = await fixture(t, { restoreResponse: restored, importJobs: jobs });
    assert.match(ui.one('#notice').textContent, /无法|无效|失败|损坏|重新|不完整/);
    assert.equal(ui.document.querySelector('progress'), null);
    assert.doesNotMatch(ui.one('#content').textContent, /导入完成/);
    assert.match(ui.one('#vault-badge').textContent, /合成资料库/);
    noResumeControl(ui); noAutomaticImport(ui);
    await ui.navigate('会话');
    assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
    noAutomaticImport(ui);
  });
}

test('未完成任务读取失败保留资料库与错误，不能变成完成或自动重试写入', async t => {
  const ui = await fixture(t, { restoreResponse: restored, configureNative(native) {
    native.fail('list_import_jobs', '合成导入状态文件损坏，请重新读取');
  } });
  assert.match(ui.one('#notice').textContent, /状态暂时无法核实|合成导入状态文件损坏/);
  assert.match(ui.one('#vault-badge').textContent, /合成资料库/);
  assert.equal(ui.native.count('list_import_jobs'), 1);
  noResumeControl(ui); noAutomaticImport(ui);
  await ui.navigate('会话');
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
});

test('启动等待导入清单时仍保持只读，回包后才显示可恢复任务', async t => {
  let releaseJobs;
  const ui = await fixture(t, { holdStartup: true, configureNative(native) { releaseJobs = native.hold('list_import_jobs'); } });
  ui.releaseStartup(restored); await setImmediate();
  assert.equal(ui.native.count('list_import_jobs'), 1);
  assert.equal(ui.one('#switch-vault').disabled, true);
  assert.equal(ui.document.querySelector('progress'), null);
  noAutomaticImport(ui);
  releaseJobs([interrupted]); await ui.idle();
  assert.equal(ui.one('progress').value, 2);
  ui.button('继续导入'); noAutomaticImport(ui);
});

for (const target of ['范围', '资料库']) {
  test(`恢复暂停任务后切换${target}，旧继续按钮不能写入新上下文`, async t => {
    const ui = await fixture(t, { restoreResponse: restored, importJobs: [interrupted] });
    const old = ui.button('继续导入');
    if (target === '范围') {
      ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
      assert.equal(ui.one('[aria-label="资料范围"]').value, 'work');
    } else {
      ui.native.next('choose_vault', { ...vault, session_id: 'synthetic-new-session', display_name: '另一合成资料库' });
      ui.one('#switch-vault').click(); await ui.click('打开已有资料库', ui.modal());
      assert.match(ui.one('#vault-badge').textContent, /另一合成资料库/);
    }
    await ui.navigate('添加资料'); await replay(ui, old);
    assert.equal(ui.document.querySelector('progress'), null);
    assert.equal(ui.native.count('resume_import_job'), 0);
    assert.equal(ui.native.count('start_import_job'), 0);
    noResumeControl(ui);
    ui.button('选择导出文件');
    ui.noWrites();
  });
}

test('恢复后的任务可先查看已保存原文，返回导入页仍需明确继续', async t => {
  const ui = await fixture(t, { restoreResponse: restored, importJobs: [interrupted] });
  ui.native.next('import_job_conversations', { job_id: interrupted.job_id, scope: 'personal', status: interrupted, conversations: [conversation], offset: 0, total: 1, next_offset: null });
  await ui.click('查看本批会话');
  assert.equal(ui.one('#content h1').textContent, '会话');
  assert.match(ui.one('.conversation-reader').textContent, /决定先核对来源/);
  await ui.navigate('添加资料');
  assert.equal(ui.one('progress').value, 2);
  ui.button('继续导入'); noAutomaticImport(ui);
});

test('范围设置保存挂起也先读完新范围，保存结束不重新绘制用户已打开的背景页', async t => {
  const ui = await fixture(t, { restoreResponse: restored });
  await ui.navigate('随身背景');
  const release = ui.native.hold('remember_workspace');
  const reads = ui.native.count('manage_memories');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  assert.equal(ui.native.count('manage_memories'), reads + 1, '界面就绪之前必须已读取新范围，不能等设置保存后才补加载');
  assert.equal(ui.native.matching('manage_memories').at(-1).payload.scope, 'work');
  await ui.click('选择背景');
  const currentPane = ui.one('#content').firstElementChild;
  const currentReads = ui.native.count('manage_memories');
  release(null); await ui.idle();
  assert.equal(ui.native.count('manage_memories'), currentReads, '迟到设置完成不能发起额外列表加载');
  assert.equal(currentPane.isConnected, true, '迟到设置完成不能替换刚打开的可点击页面节点');
  assert.equal(ui.one('[data-action="background-select"]').getAttribute('aria-pressed'), 'true');
  ui.noWrites();
});

test('范围设置迟到完成后不重新查询用户随后打开的搜索结果', async t => {
  const ui = await fixture(t, { restoreResponse: restored });
  const release = ui.native.hold('remember_workspace');
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  ui.fill('#query', '随后主动发起的查找'); await ui.click('查找');
  const reads = ui.native.count('search_records');
  const currentPane = ui.one('#content').firstElementChild;
  release(null); await ui.idle();
  assert.equal(ui.native.count('search_records'), reads, '保存位置不能重放一个新的检索');
  assert.equal(currentPane.isConnected, true);
  assert.equal(ui.one('#query').value, '随后主动发起的查找');
  ui.noWrites();
});

for (const fails of [false, true]) {
  test(`范围 A→B→A 的迟到设置${fails ? '失败' : '成功'}不重放读取，仍按顺序保存最后范围`, async t => {
    const ui = await fixture(t, { restoreResponse: restored });
    let settle;
    const pending = new Promise((resolve, reject) => { settle = () => fails ? reject(new Error('合成旧范围保存失败')) : resolve(null); });
    ui.native.next('remember_workspace', pending);
    ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
    ui.fill('[aria-label="资料范围"]', 'personal', 'change'); await ui.idle();
    const reads = ui.native.count('list_conversations');
    const currentPane = ui.one('#content').firstElementChild;
    assert.deepEqual(ui.native.matching('remember_workspace').map(call => call.payload.scope), ['work']);
    settle(); await ui.idle();
    assert.deepEqual(ui.native.matching('remember_workspace').map(call => call.payload.scope), ['work', 'personal']);
    assert.equal(ui.native.count('list_conversations'), reads);
    assert.equal(currentPane.isConnected, true);
    assert.equal(ui.one('[aria-label="资料范围"]').value, 'personal');
    assert.equal(ui.one('#notice').classList.contains('error'), false, '旧范围的失败不能污染新范围');
    ui.noWrites();
  });
}

test('当前范围的设置迟到失败只提示保存位置问题，不替换新搜索或正文', async t => {
  const ui = await fixture(t, { restoreResponse: restored });
  let rejectSave;
  ui.native.next('remember_workspace', new Promise((_, reject) => { rejectSave = reject; }));
  ui.fill('[aria-label="资料范围"]', 'work', 'change'); await ui.idle();
  ui.fill('#query', '保存设置期间继续查找'); await ui.click('查找');
  const reads = ui.native.count('search_records');
  const currentPane = ui.one('#content').firstElementChild;
  rejectSave(new Error('合成设置写入失败')); await ui.idle();
  assert.match(ui.one('#notice').textContent, /暂时无法记住这个位置/);
  assert.equal(ui.native.count('search_records'), reads);
  assert.equal(currentPane.isConnected, true);
  assert.equal(ui.one('#query').value, '保存设置期间继续查找');
  ui.noWrites();
});
