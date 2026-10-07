// 合成数据上的真实 Chromium DOM 回归；拦截全部资源，不访问外网或真实 Vault。
// 运行前在 extension 执行 npm ci 和 npx playwright install chromium。
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from '../../extension/node_modules/playwright/index.mjs';

const vault = {
  session_id: 'synthetic-session', root: '/synthetic/资料库', display_name: '合成资料库',
  scopes: ['personal', 'work'], event_count: 2, memory_count: 1, health: { ok: true },
};
const eventRef = 'event:evt_synthetic';
const memoryRef = 'memory:mem_synthetic@1';
// 与 Rust serde(flatten) 的实际输出一致，正文、时间和状态不在 data 内。
const event = {
  schema_version: 1, id: 'evt_synthetic', scope: 'personal', role: 'user',
  captured_at: '2026-10-06T08:00:00Z', occurred_at: '2026-10-06T07:00:00Z',
  content: '原始事件完整正文，检索片段不包含这一句。', parts: [],
  source: { platform: 'synthetic', conversation_id: 'conversation', message_id: 'message' },
};
const memory = {
  schema_version: 1, id: 'mem_synthetic', revision: 1, status: 'active',
  recorded_at: '2026-10-06T09:00:00Z', updated_at: '2026-10-06T09:00:00Z',
  scope: 'personal', content: '长期记忆完整正文，只存在于 read 返回值中。',
  source_refs: [eventRef], protected: true, authority: 'user', evidence: 'user_explicit',
};
const importPreview = {
  preview_id: 'synthetic-import', session_id: vault.session_id,
  file_name: 'conversations.json', format: 'chatgpt-export', scope: 'personal',
  byte_count: 2048, event_count: 2, redacted_event_count: 0, truncated: false,
  samples: [{ role: 'user', content: '这是待导入的合成对话。' }], warning: '预览尚未写入。',
};
const dreamPreview = {
  preview_id: 'synthetic-dream', session_id: vault.session_id,
  file_name: 'dream-result.json', scope: 'personal',
  review: {
    job_id: 'synthetic-job', already_applied: false, can_apply: true,
    requires_protected_approval: true, diagnostics: [],
    changes: [{ operation: 'update', before: memory, after: {
      ...memory, revision: 2, content: '经过审阅的新记忆正文。',
    } }],
  },
};
const dreamTask = {
  job_id: 'synthetic-job', input_hash: 'a'.repeat(64), scope: 'personal',
  source_count: 1, memory_count: 0, byte_count: 125,
  text: '合成完整任务：保持用户原话与助手建议的区别。\nrecallcard.dream-job/1\n原始事件完整正文。',
  sources: [{ ref: eventRef, role: 'user', text: event.content, occurred_at: null }],
};
const inlineResult = JSON.stringify({ schema: 'recallcard.dream-result/1', job_id: 'synthetic-job', input_hash: 'a'.repeat(64), proposals: [{ operation: 'noop', scope: 'personal' }] });

let browser;
const assets = new Map();
before(async () => {
  for (const name of ['index.html', 'app.js', 'model.js', 'memory-management.js', 'styles.css']) {
    assets.set(`/${name}`, await readFile(new URL(`../ui/${name}`, import.meta.url)));
  }
  browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH
    ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {});
});
after(async () => { await browser?.close(); });

async function fixture(t) {
  const page = await browser.newPage({ locale: 'zh-CN', timezoneId: 'UTC' });
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  t.after(async () => {
    await page.close();
    assert.deepEqual(errors, [], '页面不应产生未处理的 JavaScript 异常');
  });
  const calls = [];
  const queues = new Map();
  const defaultResponse = (command, payload) => {
    switch (command) {
      case 'choose_vault': case 'vault_status': return vault;
      case 'cancel_previews': return null;
      case 'preview_note': return { preview_id: 'synthetic-note', session_id: vault.session_id, scope: payload.scope, content: payload.content, redacted: false };
      case 'confirm_note': return { ref: eventRef, event };
      case 'browse_records': case 'search_records': return {
        results: payload.scope === 'personal' ? [
          { ref: eventRef, text: '事件检索片段', occurred_at: event.occurred_at },
          { ref: memoryRef, text: '记忆检索片段' },
        ] : [], truncated: false,
      };
      case 'read_record': return { results: [{ ref: payload.reference,
        record: payload.reference === eventRef ? event : memory }], truncated: false };
      case 'read_sources': return { results: [{ ref: memoryRef, events: [event] }], truncated: false };
      case 'list_conversations': return { conversations: [{ session_ref:'synthetic-conversation',title:'从 DeepSeek 继续',platform:'deepseek',message_count:2,captured_at:event.captured_at }] };
      case 'conversation_messages': return { messages:[{ref:eventRef,role:'user',text:'决定用 Rust，助手的建议还没有确认。',occurred_at:null}],total:1,next_offset:null };
      case 'prepare_continuation': return {text:'recallcard.context/1\n用户原话：决定用 Rust。',message_count:1,available_messages:1};
      case 'prepare_client_config': return {mcpServers:{recallcard:{command:'/synthetic/recallcard',args:['mcp','--scope',payload.scope]}}};
      case 'install_browser_connection': return {registered:true,capture_enabled:payload.allowCapture,note:'请回到扩展检查连接'};
      case 'pick_import': return importPreview;
      case 'confirm_import': return { events_added: 2, events_seen: 2 };
      case 'pick_dream': return dreamPreview;
      case 'prepare_dream_task': return dreamTask;
      case 'review_dream_text': return { ...dreamPreview, file_name: '粘贴的整理结果' };
      case 'write_clipboard': return null;
      case 'apply_dream': return { changes: [{ id: memory.id, revision: 2 }] };
      case 'export_dream': return '/synthetic/dream-job.json';
      default: throw new Error(`测试没有定义原生命令：${command}`);
    }
  };
  const native = {
    calls,
    count: command => calls.filter(call => call.command === command).length,
    matching: command => calls.filter(call => call.command === command),
    next(command, result) {
      const queue = queues.get(command) || [];
      queue.push(() => result);
      queues.set(command, queue);
    },
    failNext(command, message) {
      const queue = queues.get(command) || [];
      queue.push(() => { throw new Error(message); });
      queues.set(command, queue);
    },
    holdNext(command) {
      let release;
      const promise = new Promise(resolve => { release = resolve; });
      this.next(command, promise);
      return release;
    },
  };
  await page.exposeFunction('__syntheticInvoke', async (command, payload) => {
    calls.push({ command, payload });
    const queued = queues.get(command)?.shift();
    return structuredClone(await (queued ? queued() : defaultResponse(command, payload)));
  });
  await page.addInitScript(() => {
    window.__TAURI__ = { core: { invoke: (command, payload) => window.__syntheticInvoke(command, payload) } };
  });
  await page.route('**/*', async route => {
    const url = new URL(route.request().url());
    const body = url.origin === 'https://recallcard.test' && assets.get(url.pathname);
    if (!body) return route.abort();
    const contentType = url.pathname.endsWith('.js') ? 'text/javascript; charset=utf-8'
      : url.pathname.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8';
    await route.fulfill({ status: 200, body, contentType });
  });
  await page.goto('https://recallcard.test/index.html');
  await page.getByRole('heading', { name: '换一个 AI，也能接着聊。' }).waitFor();
  return { page, native };
}

async function clickButton(page, name) {
  if (name === '选择文件并预览' && !await page.locator('#file-import-details').evaluate(node => node.open)) await page.locator('#file-import-details > summary').click();
  if (['选择结果并审阅', '导出本次来源包'].includes(name) && !await page.locator('#dream-file-options').evaluate(node => node.open)) await page.locator('#dream-file-options > summary').click();
  return page.getByRole('button', { name, exact: true }).click();
}
async function idle(page) {
  await page.waitForFunction(() => document.querySelector('#operation').textContent === '准备就绪');
}
async function openVault(page) {
  await clickButton(page, '打开已有资料库');
  await idle(page);
  assert.equal(await page.locator('#vault-badge').textContent(), vault.display_name);
}
async function navigate(page, name) {
  await page.locator('#navigation').getByRole('button', { name, exact: true }).click();
  await idle(page);
}
async function previewImport(page) {
  await navigate(page, '添加资料');
  await clickButton(page, '选择文件并预览');
  await page.getByRole('heading', { name: '确认导入', exact: true }).waitFor();
  await idle(page);
}
async function previewDream(page) {
  await navigate(page, '整理记忆');
  await clickButton(page, '选择结果并审阅');
  await page.getByRole('heading', { name: '逐条检查本次变更' }).waitFor();
  await idle(page);
}
async function selectRecord(page, reference) {
  await page.locator('.result-card').filter({ hasText: reference }).click();
  await idle(page);
}
async function generateDreamTask(page) {
  await navigate(page, '查找与阅读');
  await selectRecord(page, eventRef);
  await clickButton(page, '选择这条资料');
  await idle(page);
  await navigate(page, '整理记忆');
  await clickButton(page, '生成完整整理任务');
  await idle(page);
}
async function reviewInlineDream(page, text = inlineResult) {
  await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).fill(text);
  await clickButton(page, '检查并预览结果');
  await idle(page);
}
async function assertNoWrite(native) {
  assert.equal(native.count('confirm_import'), 0);
  assert.equal(native.count('apply_dream'), 0);
  assert.equal(native.count('confirm_note'), 0);
}

test('首次打开和切换资料库时取消文件选择，不创建或丢失当前 Vault', async t => {
  const { page, native } = await fixture(t);
  native.next('choose_vault', null);
  await clickButton(page, '创建新资料库');
  await idle(page);
  assert.equal(await page.locator('#vault-badge').textContent(), '尚未打开资料库');
  assert.deepEqual(native.matching('choose_vault')[0].payload, { create: true });
  await openVault(page);
  await previewImport(page);
  await clickButton(page, '切换资料库');
  assert.equal(await page.locator('#modal').evaluate(node => node.open), true);
  native.next('choose_vault', null);
  await page.locator('#modal').getByRole('button', { name: '打开已有资料库', exact: true }).click();
  await idle(page);
  assert.equal(await page.locator('#modal').evaluate(node => node.open), false);
  assert.equal(await page.locator('#vault-badge').textContent(), vault.display_name);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 1);
  await navigate(page, '查找与阅读');
  assert.deepEqual(native.matching('browse_records').at(-1).payload,
    { sessionId: vault.session_id, scope: 'personal', target: 'all' });
  await assertNoWrite(native);
});

test('导入预览与取消确认不写入，显式确认只提交一次且使用原预览编号', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await previewImport(page);
  assert.deepEqual(native.matching('pick_import')[0].payload,
    { sessionId: vault.session_id, scope: 'personal', format: 'chatgpt-export' });
  await assertNoWrite(native);
  assert.match(await page.locator('.file-preview').textContent(), /尚未写入/);
  await clickButton(page, '确认导入 2 条记录');
  await assertNoWrite(native);
  await page.locator('#modal').getByRole('button', { name: '取消', exact: true }).click();
  await assertNoWrite(native);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 1);
  await clickButton(page, '确认导入 2 条记录');
  const release = native.holdNext('confirm_import');
  await page.locator('#modal').getByRole('button', { name: '确认导入', exact: true }).evaluate(button => {
    button.click(); button.click();
  });
  await page.waitForFunction(() => document.querySelector('#operation').textContent.includes('正在导入'));
  assert.equal(native.count('confirm_import'), 1);
  assert.deepEqual(native.matching('confirm_import')[0].payload,
    { sessionId: vault.session_id, previewId: importPreview.preview_id });
  assert.equal(await page.locator('#switch-vault').isDisabled(), true);
  release({ events_added: 2, events_seen: 2 });
  await idle(page);
  assert.equal(native.count('confirm_import'), 1);
  assert.equal(native.count('vault_status'), 1);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 0);
  assert.match(await page.locator('#notice').textContent(), /导入完成：新增 2 条/);
});

test('取消导入预览同时废弃原生待确认操作，往返页面不恢复旧预览', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await previewImport(page);
  const cancellationCount = native.count('cancel_previews');
  await clickButton(page, '取消这次导入');
  await idle(page);
  assert.equal(native.count('cancel_previews'), cancellationCount + 1);
  assert.deepEqual(native.matching('cancel_previews').at(-1),
    { command: 'cancel_previews', payload: { sessionId: vault.session_id } });
  await navigate(page, '概览');
  await navigate(page, '添加资料');
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 0);
  native.next('pick_import', null);
  await clickButton(page, '选择文件并预览');
  await idle(page);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 0);
  await assertNoWrite(native);
});

test('阅读展示扁平化 Event/Memory 完整正文、记忆状态时间与 events 出处', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '查找与阅读');
  await selectRecord(page, eventRef);
  assert.equal(await page.locator('.reading-pane .body-text').textContent(), event.content);
  assert.equal(native.count('read_sources'), 0);
  const chunkedEvent = { ...event, content: '', parts: [{ text: '第一段原文' }, { text: '第二段原文' }] };
  native.next('read_record', { results: [{ ref: eventRef, record: chunkedEvent }] });
  await selectRecord(page, eventRef);
  assert.equal(await page.locator('.reading-pane .body-text').textContent(), '第一段原文\n第二段原文');
  await selectRecord(page, memoryRef);
  const pane = page.locator('.reading-pane');
  assert.equal(await pane.locator('.body-text').textContent(), memory.content);
  assert.equal(await pane.locator('.info-line').filter({ hasText: '状态' }).locator('.value').textContent(), '有效');
  assert.match(await pane.locator('.info-line').filter({ hasText: '记录时间' }).textContent(), /2026/);
  assert.match(await pane.locator('.sample').textContent(), new RegExp(event.content));
  assert.equal(await pane.locator('.sample .ref').textContent(), eventRef);
  assert.deepEqual(native.matching('read_sources')[0].payload,
    { sessionId: vault.session_id, scope: 'personal', reference: memoryRef });
});

test('Dream 展示扁平化前后内容，受保护记忆需额外勾选和最终确认', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await previewDream(page);
  assert.equal(await page.locator('.change .before').textContent(), `之前\n${memory.content}`);
  assert.equal(await page.locator('.change .after').textContent(), '之后\n经过审阅的新记忆正文。');
  assert.equal(await page.locator('.change .ref').textContent(), eventRef);
  await clickButton(page, '保存这些记忆');
  assert.equal(await page.locator('#modal').evaluate(node => node.open), false);
  assert.match(await page.locator('#notice').textContent(), /请先勾选受保护记忆/);
  await assertNoWrite(native);
  await page.locator('#protected-approval').check();
  await clickButton(page, '保存这些记忆');
  await assertNoWrite(native);
  await page.locator('#modal').getByRole('button', { name: '取消', exact: true }).click();
  await assertNoWrite(native);
  await clickButton(page, '保存这些记忆');
  await page.locator('#modal').getByRole('button', { name: '确认保存', exact: true }).click();
  await idle(page);
  assert.deepEqual(native.matching('apply_dream'), [{ command: 'apply_dream', payload: {
    sessionId: vault.session_id, previewId: dreamPreview.preview_id, approveProtected: true,
  } }]);
  assert.equal(await page.locator('.changes').count(), 0);
  assert.match(await page.locator('#notice').textContent(), /已保存 1 条记忆变更/);
});

test('取消 Dream 审阅废弃原生预览，冲突或已发布结果不能再次发布', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await previewDream(page);
  const cancellationCount = native.count('cancel_previews');
  await clickButton(page, '取消审阅');
  await idle(page);
  assert.equal(native.count('cancel_previews'), cancellationCount + 1);
  assert.deepEqual(native.matching('cancel_previews').at(-1).payload, { sessionId: vault.session_id });
  assert.equal(await page.locator('.changes').count(), 0);
  await navigate(page, '概览');
  await navigate(page, '整理记忆');
  assert.equal(await page.locator('.changes').count(), 0);
  for (const review of [
    { ...dreamPreview.review, can_apply: false, diagnostics: ['来源版本已经变化'] },
    { ...dreamPreview.review, already_applied: true },
  ]) {
    native.next('pick_dream', { ...dreamPreview, review });
    await clickButton(page, '选择结果并审阅');
    await idle(page);
    assert.equal(await page.getByRole('button', { name: '保存这些记忆', exact: true }).count(), 0);
  }
  await assertNoWrite(native);
});

test('导入文件名、正文、Dream 诊断与变更中的恶意标记保持纯文本', async t => {
  const { page, native } = await fixture(t);
  const hostile = '<img src=x onerror="window.__injected=1"><script>window.__injected=2</script>';
  await openVault(page);
  native.next('pick_import', { ...importPreview, file_name: hostile, warning: hostile,
    samples: [{ role: 'user', content: hostile }] });
  await previewImport(page);
  assert.equal(await page.locator('.file-name').textContent(), hostile);
  assert.equal(await page.locator('.sample p').textContent(), hostile);
  native.next('pick_dream', { ...dreamPreview, file_name: hostile, review: {
    ...dreamPreview.review, diagnostics: [hostile],
    changes: [{ operation: 'update', before: { ...memory, content: hostile }, after: { ...memory, content: hostile } }],
  } });
  await previewDream(page);
  assert.equal(await page.locator('.change .before').textContent(), `之前\n${hostile}`);
  assert.equal(await page.locator('.change .after').textContent(), `之后\n${hostile}`);
  assert.equal(await page.locator('#content img, #content script, #content [onerror]').count(), 0);
  assert.equal(await page.evaluate(() => window.__injected), undefined);
  await assertNoWrite(native);
});

test('耗时文件选择禁用重复操作、切换资料库和快捷键，结束后恢复', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '添加资料');
  if (!await page.locator('#file-import-details').evaluate(node => node.open)) await page.locator('#file-import-details > summary').click();
  const release = native.holdNext('pick_import');
  await page.getByRole('button', { name: '选择文件并预览', exact: true }).evaluate(button => {
    button.click(); button.click();
  });
  await page.waitForFunction(() => document.querySelector('#operation').textContent.includes('正在读取导入预览'));
  assert.equal(native.count('pick_import'), 1);
  assert.equal(await page.locator('button:not([disabled]), input:not([disabled]), select:not([disabled])').count(), 0);
  await page.keyboard.press('Control+k');
  assert.equal(await page.locator('#location').textContent(), '添加资料');
  release(importPreview);
  await idle(page);
  assert.equal(await page.locator('#switch-vault').isEnabled(), true);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 1);
  await assertNoWrite(native);
});

test('切换范围清除选中来源、阅读结果与两类预览，并以新范围查找', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '查找与阅读');
  await selectRecord(page, eventRef);
  await clickButton(page, '选择这条资料');
  await previewImport(page);
  await previewDream(page);
  await page.locator('#dream-file-options > summary').click();
  assert.equal(await page.getByRole('button', { name: '导出本次来源包', exact: true }).count(), 1);
  await page.getByRole('combobox', { name: '资料范围', exact: true }).selectOption('work');
  await idle(page);
  assert.deepEqual(native.matching('cancel_previews').at(-1).payload, { sessionId: vault.session_id });
  assert.equal(await page.locator('.changes').count(), 0);
  assert.equal(await page.getByRole('button', { name: '导出本次来源包', exact: true }).count(), 0);
  assert.equal(await page.getByRole('button', { name: '移除', exact: true }).count(), 0);
  await navigate(page, '添加资料');
  assert.equal(await page.locator('#import-scope').inputValue(), 'work');
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 0);
  await navigate(page, '查找与阅读');
  assert.deepEqual(native.matching('browse_records').at(-1).payload,
    { sessionId: vault.session_id, scope: 'work', target: 'all' });
  assert.equal(await page.locator('.result-card, .reading-pane .body-text').count(), 0);
  await assertNoWrite(native);
});

test('原生操作失败后解除忙碌状态并保留当前 Vault，可重新预览', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '添加资料');
  native.failNext('pick_import', '文件已改变，请重新选择');
  await clickButton(page, '选择文件并预览');
  await idle(page);
  assert.match(await page.locator('#notice').textContent(), /文件已改变，请重新选择/);
  assert.equal(await page.locator('#switch-vault').isEnabled(), true);
  assert.equal(await page.locator('#vault-badge').textContent(), vault.display_name);
  await previewImport(page);
  assert.equal(native.count('pick_import'), 2);
  await assertNoWrite(native);
});

test('完整记录或出处超出读取预算时显示片段和明确提示，不伪称完整内容', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '查找与阅读');
  native.next('read_record', { results: [], truncated: true, pending_refs: [eventRef] });
  await selectRecord(page, eventRef);
  assert.equal(await page.locator('.reading-pane .body-text').textContent(), '事件检索片段');
  assert.match(await page.locator('.reading-pane .hint').textContent(), /部分内容/);
  assert.match(await page.locator('#notice').textContent(), /记录较长，先显示部分内容/);
  native.next('read_sources', { results: [], truncated: true, pending_refs: [memoryRef] });
  await selectRecord(page, memoryRef);
  assert.equal(await page.locator('.reading-pane .body-text').textContent(), memory.content);
  assert.match(await page.locator('#notice').textContent(), /出处较长，完整内容保存在资料库文件中/);
  assert.equal(await page.locator('.reading-pane .sample').count(), 0);
});

test('切换到无效 Vault 后清除失效会话和预览，不能继续提交旧资料库', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await previewImport(page);
  await previewDream(page);
  await clickButton(page, '切换资料库');
  native.failNext('choose_vault', '无法打开资料库，请选择有效的 RecallCard Vault');
  await page.locator('#modal').getByRole('button', { name: '打开已有资料库', exact: true }).click();
  await idle(page);
  assert.equal(await page.locator('#vault-badge').textContent(), '尚未打开资料库');
  assert.match(await page.locator('#notice').textContent(), /无法打开资料库/);
  await navigate(page, '添加资料');
  assert.equal(await page.getByRole('heading', { name: '打开资料库', exact: true }).count(), 1);
  assert.equal(await page.getByRole('button', { name: '确认导入 2 条记录', exact: true }).count(), 0);
  await navigate(page, '整理记忆');
  assert.equal(await page.locator('.changes').count(), 0);
  assert.equal(await page.getByRole('button', { name: '保存这些记忆', exact: true }).count(), 0);
  await assertNoWrite(native);
});


test('默认入口直接粘贴文字，预览后确认保存并进入查找，无需选择文件', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await navigate(page, '添加资料');
  assert.equal(await page.locator('#file-import-details').evaluate(node => node.open), false);
  await page.locator('#note-content').fill('合成首条记录：周末整理书单');
  await clickButton(page, '预览并保存'); await idle(page);
  assert.equal(await page.locator('.note-preview').textContent(), '合成首条记录：周末整理书单');
  assert.equal(native.count('confirm_note'), 0); assert.equal(native.count('pick_import'), 0);
  await clickButton(page, '返回修改'); await idle(page);
  assert.equal(await page.locator('#note-content').inputValue(), '合成首条记录：周末整理书单');
  await page.locator('#note-content').fill('合成首条记录：周日整理书单');
  await clickButton(page, '预览并保存'); await idle(page);
  await clickButton(page, '确认保存记录'); await idle(page);
  assert.deepEqual(native.matching('confirm_note')[0].payload, { sessionId: vault.session_id, previewId: 'synthetic-note' });
  assert.equal(await page.locator('#location').textContent(), '查找与阅读');
  assert.equal(native.count('confirm_note'), 1); assert.equal(native.count('pick_import'), 0);
});

test('来源式导入保留选择，会话可预览并为另一个AI准备交接',async t=>{
  const {page,native}=await fixture(t);await openVault(page);await navigate(page,'添加资料');
  await page.getByRole('heading',{name:'这次从哪里带入对话？'}).waitFor();
  await page.getByRole('heading',{name:'浏览器扩展',exact:true}).locator('..').getByRole('button',{name:'选择这类文件'}).click();
  assert.equal(await page.locator('#import-format').inputValue(),'recallcard-conversation');
  await clickButton(page,'选择文件并预览');await idle(page);assert.equal(native.matching('pick_import').at(-1).payload.format,'recallcard-conversation');
  await navigate(page,'会话与接续');await page.getByRole('button',{name:/从 DeepSeek 继续/}).click();await idle(page);
  assert.match(await page.locator('.conversation-message').textContent(),/时间未知/);
  await page.getByRole('textbox',{name:'接下来要做什么'}).fill('先实现已经确认的部分');await clickButton(page,'准备交接内容');await idle(page);
  assert.match(await page.getByRole('textbox',{name:'交接内容预览'}).inputValue(),/recallcard.context\/1/);
  assert.equal(native.matching('prepare_continuation').at(-1).payload.goal,'先实现已经确认的部分');
  await assertNoWrite(native);
});
test('浏览器写入许可默认关闭，配置不冒充真实连接成功',async t=>{
  const {page,native}=await fixture(t);await openVault(page);await navigate(page,'连接与状态');
  assert.equal(await page.locator('#allow-browser-capture').isChecked(),false);
  await page.getByRole('textbox',{name:'RecallCard扩展ID'}).fill('abcdefghijklmnopabcdefghijklmnop');
  await clickButton(page,'连接此浏览器');await idle(page);assert.equal(native.matching('install_browser_connection')[0].payload.allowCapture,false);
  assert.match(await page.locator('#content').textContent(),/请回到扩展检查连接/);
  await clickButton(page,'生成客户端配置');await idle(page);assert.match(await page.getByRole('textbox',{name:'MCP客户端配置'}).inputValue(),/personal/);
});

test('整理任务由当前来源生成，复制前重新校验，再写入原生剪贴板', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await generateDreamTask(page);
  assert.equal(await page.locator('#dream-file-options').evaluate(node => node.open), false);
  assert.equal(await page.locator('textarea[aria-label="完整整理任务"]').inputValue(), dreamTask.text);
  assert.match(await page.locator('#content').textContent(), /用户原话/);
  assert.match(await page.locator('#content').textContent(), /时间未知/);
  assert.deepEqual(native.matching('prepare_dream_task')[0].payload, {
    sessionId: vault.session_id, scope: 'personal', sourceRefs: [eventRef], memoryRefs: [],
  });
  assert.equal(native.count('write_clipboard'), 0);
  const release = native.holdNext('prepare_dream_task');
  await clickButton(page, '复制整理任务');
  await page.waitForFunction(() => document.querySelector('#operation').textContent.includes('正在重新核对任务'));
  assert.equal(native.count('write_clipboard'), 0);
  assert.equal(await page.locator('#switch-vault').isDisabled(), true);
  release(dreamTask);
  await idle(page);
  assert.equal(native.count('prepare_dream_task'), 2);
  assert.deepEqual(native.matching('write_clipboard')[0].payload, { text: dreamTask.text });
  assert.ok(native.calls.findLastIndex(call => call.command === 'prepare_dream_task') < native.calls.findIndex(call => call.command === 'write_clipboard'));
  await assertNoWrite(native);
});

test('来源或旧记忆变化以及重新核对失败都阻止复制并撤销旧审查', async t => {
  for (const fails of [false, true]) {
    const { page, native } = await fixture(t);
    await openVault(page);
    await generateDreamTask(page);
    await reviewInlineDream(page);
    if (fails) native.failNext('prepare_dream_task', '来源已撤回，请重新选择');
    else native.next('prepare_dream_task', { ...dreamTask, text: '来源已经改变的新任务' });
    await clickButton(page, '复制整理任务');
    await idle(page);
    assert.equal(native.count('write_clipboard'), 0);
    assert.equal(await page.getByRole('button', { name: '复制整理任务', exact: true }).count(), 0);
    assert.equal(await page.locator('.changes').count(), 0);
    assert.match(await page.locator('#notice').textContent(), fails ? /来源已撤回/ : /来源或记忆已经改变/);
    assert.equal(await page.locator('#switch-vault').isEnabled(), true);
    await assertNoWrite(native);
  }
});

test('阅读区增减来源立即废弃任务、粘贴正文与审查，包括原生取消失败', async t => {
  for (const fails of [false, true]) {
    const { page, native } = await fixture(t);
    await openVault(page);
    await generateDreamTask(page);
    await reviewInlineDream(page);
    await navigate(page, '查找与阅读');
    await selectRecord(page, memoryRef);
    const cancellations = native.count('cancel_previews');
    if (fails) native.failNext('cancel_previews', '无法取消，请重新审查');
    await clickButton(page, '选择这条资料');
    await idle(page);
    assert.equal(native.count('cancel_previews'), cancellations + 1);
    await navigate(page, '整理记忆');
    assert.equal(await page.getByRole('button', { name: '复制整理任务', exact: true }).count(), 0);
    assert.equal(await page.locator('.changes').count(), 0);
    assert.equal(await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).inputValue(), '');
    assert.equal(await page.getByRole('button', { name: '移除', exact: true }).count(), fails ? 1 : 2);
    await assertNoWrite(native);
  }
});

test('粘贴完整结果先审查，返回修改和取消确认不发布，受保护批准仅用于本次确认', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await generateDreamTask(page);
  const fenced = `\`\`\`json\n${inlineResult}\n\`\`\``;
  await reviewInlineDream(page, fenced);
  assert.deepEqual(native.matching('review_dream_text')[0].payload, {
    sessionId: vault.session_id, scope: 'personal', text: fenced,
  });
  assert.equal(native.count('pick_dream'), 0);
  await page.locator('#protected-approval').check();
  await clickButton(page, '返回修改结果');
  await idle(page);
  assert.equal(await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).inputValue(), fenced);
  assert.equal(await page.locator('.changes').count(), 0);
  await reviewInlineDream(page, fenced);
  assert.equal(await page.locator('#protected-approval').isChecked(), false);
  await clickButton(page, '保存这些记忆');
  assert.equal(await page.locator('#modal').evaluate(node => node.open), false);
  await page.locator('#protected-approval').check();
  await clickButton(page, '保存这些记忆');
  await page.locator('#modal').getByRole('button', { name: '取消', exact: true }).click();
  await assertNoWrite(native);
  await clickButton(page, '保存这些记忆');
  const release = native.holdNext('apply_dream');
  await page.locator('#modal').getByRole('button', { name: '确认保存', exact: true }).evaluate(button => { button.click(); button.click(); });
  await page.waitForFunction(() => document.querySelector('#operation').textContent.includes('正在保存记忆'));
  assert.equal(native.count('apply_dream'), 1);
  assert.deepEqual(native.matching('apply_dream')[0].payload, {
    sessionId: vault.session_id, previewId: dreamPreview.preview_id, approveProtected: true,
  });
  release({ changes: [{ id: memory.id, revision: 2 }] });
  await idle(page);
  assert.match(await page.locator('#notice').textContent(), /已保存 1 条记忆变更/);
  assert.equal(await page.locator('.changes').count(), 0);
});

test('半截结果报错保留输入供修正，文件选择取消不恢复旧审查', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await generateDreamTask(page);
  const partial = inlineResult.slice(0, -1);
  native.failNext('review_dream_text', '结果不完整，请复制完整 JSON 或完整代码块');
  await reviewInlineDream(page, partial);
  assert.match(await page.locator('#notice').textContent(), /结果不完整/);
  assert.equal(await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).inputValue(), partial);
  assert.equal(await page.locator('.changes').count(), 0);
  await reviewInlineDream(page);
  native.next('pick_dream', null);
  await clickButton(page, '选择结果并审阅');
  await idle(page);
  assert.equal(await page.locator('.changes').count(), 0);
  assert.equal(await page.getByRole('button', { name: '保存这些记忆', exact: true }).count(), 0);
  await assertNoWrite(native);
});

test('任务生成与剪贴板失败可以重试，报错不显示复制成功', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await generateDreamTask(page);
  native.failNext('write_clipboard', '合成系统剪贴板不可用');
  await clickButton(page, '复制整理任务');
  await idle(page);
  assert.match(await page.locator('#notice').textContent(), /剪贴板/);
  assert.doesNotMatch(await page.locator('#notice').textContent(), /已复制/);
  assert.equal(await page.getByRole('button', { name: '复制整理任务', exact: true }).isEnabled(), true);
  native.failNext('prepare_dream_task', '任务超过限制，请减少来源');
  await clickButton(page, '生成完整整理任务');
  await idle(page);
  assert.equal(await page.getByRole('button', { name: '复制整理任务', exact: true }).count(), 0);
  assert.match(await page.locator('#notice').textContent(), /任务超过限制/);
  await clickButton(page, '生成完整整理任务');
  await idle(page);
  assert.equal(await page.getByRole('button', { name: '复制整理任务', exact: true }).isEnabled(), true);
  await assertNoWrite(native);
});

test('范围和资料库切换清除完整任务、结果输入和确认权', async t => {
  const { page, native } = await fixture(t);
  await openVault(page);
  await generateDreamTask(page);
  await reviewInlineDream(page);
  await page.getByRole('combobox', { name: '资料范围', exact: true }).selectOption('work');
  await idle(page);
  assert.equal(await page.locator('textarea[aria-label="完整整理任务"], .changes').count(), 0);
  assert.equal(await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).inputValue(), '');
  await page.getByRole('combobox', { name: '资料范围', exact: true }).selectOption('personal');
  await idle(page);
  await generateDreamTask(page);
  await reviewInlineDream(page);
  await clickButton(page, '切换资料库');
  native.next('choose_vault', { ...vault, session_id: 'second-session', display_name: '第二个合成资料库' });
  await page.locator('#modal').getByRole('button', { name: '打开已有资料库', exact: true }).click();
  await idle(page);
  await navigate(page, '整理记忆');
  assert.equal(await page.locator('textarea[aria-label="完整整理任务"], .changes').count(), 0);
  assert.equal(await page.getByRole('textbox', { name: 'AI整理结果', exact: true }).inputValue(), '');
  assert.equal(await page.getByRole('button', { name: '移除', exact: true }).count(), 0);
  await assertNoWrite(native);
});

test('交接预览后资料撤回或改变，复制前重新核对并废弃旧正文',async t=>{
  const {page,native}=await fixture(t);await openVault(page);await navigate(page,'会话与接续');await page.getByRole('button',{name:/从 DeepSeek 继续/}).click();await idle(page);
  await clickButton(page,'准备交接内容');await idle(page);native.next('prepare_continuation',{text:'权限改变后的新内容',message_count:0,available_messages:0});
  await clickButton(page,'复制交接内容');await idle(page);assert.match(await page.locator('#notice').textContent(),/资料或权限已改变/);
  assert.equal(await page.getByRole('textbox',{name:'交接内容预览'}).count(),0);
});
