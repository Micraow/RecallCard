// 合成 Vault 上的真实 Chromium DOM 回归；不读取真实对话或连接外部 AI。
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from '../../extension/node_modules/playwright/index.mjs';

const vault = { session_id: 'synthetic-memory-session', display_name: '合成记忆库',
  root: '/synthetic/vault', scopes: ['personal', 'work'], event_count: 2, memory_count: 1, health: { ok: true } };
const original = { id: 'mem_synthetic', revision: 3, content: '完整记忆正文。列表只显示片段。',
  scope: 'personal', status: 'tentative', protected: true, labels: ['长期偏好'],
  evidence: 'assistant_suggestion', source_refs: ['evt_synthetic'],
  recorded_at: '2026-10-06T08:00:00Z', updated_at: '2026-10-06T09:00:00Z',
  observed_at: null, time_note: '原文没有日期' };
const source = { id: 'evt_synthetic', scope: 'personal', role: 'assistant',
  captured_at: '2026-10-06T08:00:00Z', occurred_at: null,
  content: '完整原始 AI 建议，不能当作用户亲口确认的事实。',
  source: { platform: 'synthetic', conversation_id: 'conversation', message_id: 'message' } };
let browser;
const assets = new Map();
before(async () => {
  for (const file of ['index.html', 'app.js', 'model.js', 'memory-management.js', 'styles.css']) {
    assets.set(`/${file}`, await readFile(new URL(`../ui/${file}`, import.meta.url)));
  }
  browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {});
});
after(async () => { await browser?.close(); });

async function fixture(t, overrides = {}) {
  const page = await browser.newPage({ locale: 'zh-CN', timezoneId: 'UTC' });
  page.setDefaultTimeout(5000);
  const data = { memory: { ...structuredClone(original), ...overrides }, hidden: false, canRestore: false,
    stillHiddenAfterRestore: false, pending: null, listTotal: 1 };
  const calls = []; const errors = []; const queues = new Map();
  page.on('pageerror', error => errors.push(error.message));
  t.after(async () => { await page.close(); assert.deepEqual(errors, [], '页面没有未处理异常'); });
  const defaults = (command, payload) => {
    const row = { ...data.memory, content: '列表记忆片段', hidden: data.hidden, can_restore: data.canRestore };
    switch (command) {
      case 'choose_vault': case 'vault_status': return vault;
      case 'cancel_previews': data.pending = null; return null;
      case 'manage_memories': return { memories: payload.scope === 'work' || (!payload.includeHidden && (data.hidden || !['active', 'tentative'].includes(row.status))) ? [] : [row], total: data.listTotal, next_offset: payload.offset + 30 < data.listTotal ? payload.offset + 30 : null };
      case 'managed_memory': return data.memory;
      case 'managed_memory_source': return source;
      case 'review_memory_edit':
        data.pending = { preview_id: 'memory-edit-preview', operation: 'edit', before: data.memory,
          after: { ...data.memory, ...payload.edit }, affected_events: 0, affected_memories: 1,
          requires_protected_approval: data.memory.protected, warning: '保存新的版本，证据性质不自动升级。' };
        return data.pending;
      case 'review_memory_visibility':
        data.pending = { preview_id: 'memory-visibility-preview', operation: payload.restore ? 'restore' : 'forget',
          before: data.memory, after: null, affected_events: 2, affected_memories: 3,
          requires_protected_approval: true, warning: '这会影响共用来源的其他受保护记忆。' };
        return data.pending;
      case 'confirm_memory_change': {
        const pending = data.pending;
        if (!pending) throw new Error('预览已失效');
        if (pending.requires_protected_approval && !payload.approveProtected) throw new Error('缺少受保护确认');
        if (pending.operation === 'edit') data.memory = { ...pending.after, revision: data.memory.revision + 1 };
        else if (pending.operation === 'forget') { data.hidden = true; data.canRestore = true; }
        else { data.hidden = data.stillHiddenAfterRestore; data.canRestore = false; }
        data.pending = null; return { hidden: data.hidden, status: data.memory.status };
      }
      case 'browse_records': case 'search_records': return { results: data.hidden ? [] : [{ ref: `memory:${data.memory.id}@${data.memory.revision}`, text: data.memory.content }], truncated: false };
      case 'read_record': return { results: [{ ref: payload.reference, record: data.memory }] };
      case 'read_sources': return { results: [{ events: [source] }] };
      case 'list_conversations': return { conversations: [{ session_ref: 'conversation', title: '合成会话', message_count: 1 }] };
      case 'conversation_messages': return { messages: [{ ref: 'event:evt_synthetic', role: 'assistant', text: source.content }], total: 1, next_offset: null };
      case 'prepare_continuation': return { text: '需要清空的旧交接正文', message_count: 1, available_messages: 1 };
      default: throw new Error(`未定义的测试命令：${command}`);
    }
  };
  const native = {
    data, calls, count: command => calls.filter(call => call.command === command).length,
    matching: command => calls.filter(call => call.command === command),
    next(command, action) { const queue = queues.get(command) || []; queue.push(action); queues.set(command, queue); },
    fail(command, message) { this.next(command, () => { throw new Error(message); }); },
    hold(command) { let release; const promise = new Promise(resolve => { release = resolve; }); this.next(command, () => promise); return release; },
  };
  await page.exposeFunction('__invoke', async (command, payload) => {
    calls.push({ command, payload });
    const queued = queues.get(command)?.shift();
    return structuredClone(await (queued ? queued(payload) : defaults(command, payload)));
  });
  await page.addInitScript(() => { window.__TAURI__ = { core: { invoke: (command, payload) => window.__invoke(command, payload) } }; });
  await page.route('**/*', async route => {
    const url = new URL(route.request().url());
    const body = url.origin === 'https://recallcard.test' && assets.get(url.pathname);
    if (!body) return route.abort();
    await route.fulfill({ status: 200, body, contentType: url.pathname.endsWith('.js') ? 'text/javascript; charset=utf-8'
      : url.pathname.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8' });
  });
  await page.goto('https://recallcard.test/index.html');
  await button(page, '打开已有资料库'); await idle(page);
  await navigate(page, '记忆管理');
  return { page, native };
}
async function idle(page) { await page.waitForFunction(() => document.querySelector('#operation').textContent === '准备就绪'); }
async function button(page, name) { await page.getByRole('button', { name, exact: true }).click(); }
async function navigate(page, name) { await page.locator('#navigation').getByRole('button', { name, exact: true }).click(); await idle(page); }
async function select(page) { await page.locator('.memory-list .result-card').first().click(); await idle(page); }
async function edit(page) { await select(page); await button(page, '修改正文、标签与保护'); await page.locator('#memory-content').fill('纠正后的记忆正文'); }
async function review(page) { await button(page, '检查变更与影响'); await idle(page); }
async function approve(page) { await page.locator('#memory-protected-approval').check(); await button(page, '继续确认'); }
async function commit(page) { await page.locator('#modal').getByRole('button', { name: '确认执行', exact: true }).click(); await idle(page); }

test('默认只读取可见记忆，分页30条，详情和来源读取全文、角色与未知时间', async t => {
  const { page, native } = await fixture(t);
  assert.deepEqual(native.matching('manage_memories')[0].payload, { sessionId: vault.session_id, scope: 'personal', includeHidden: false, offset: 0 });
  assert.equal(await page.locator('#include-hidden-memories').isChecked(), false);
  assert.equal(native.count('managed_memory_source'), 0);
  await select(page);
  assert.equal(await page.locator('.memory-full-text').textContent(), original.content);
  assert.match(await page.locator('.memory-detail').textContent(), /AI 建议，尚非用户事实/);
  assert.match(await page.locator('.memory-detail').textContent(), /时间未知/);
  await button(page, '查看出处 1'); await idle(page);
  assert.equal(await page.locator('.memory-source .body-text').textContent(), source.content);
  assert.match(await page.locator('.memory-source').textContent(), /AI 回复/);
  assert.deepEqual(native.matching('managed_memory_source')[0].payload, { sessionId: vault.session_id, scope: 'personal', memoryId: original.id, eventId: source.id });
  native.data.listTotal = 61; await button(page, '刷新记忆列表'); await idle(page);
  await button(page, '下一页'); await idle(page);
  assert.equal(native.matching('manage_memories').at(-1).payload.offset, 30);
  await button(page, '上一页'); await idle(page);
  assert.equal(native.matching('manage_memories').at(-1).payload.offset, 0);
  assert.equal(native.count('confirm_memory_change'), 0);
});

test('纠正与取消保护仅发送可编辑字段，保护批准默认关闭，最终确认前不写入', async t => {
  const { page, native } = await fixture(t);
  await edit(page); await page.locator('#memory-labels').fill('新标签\n共同主题'); await page.locator('#memory-protected').uncheck(); await review(page);
  assert.deepEqual(native.matching('review_memory_edit')[0].payload, {
    sessionId: vault.session_id, scope: 'personal', id: original.id, revision: 3,
    edit: { content: '纠正后的记忆正文', protected: false, labels: ['新标签', '共同主题'] },
  });
  assert.match(await page.locator('#memory-review .before').textContent(), /完整记忆正文/);
  assert.match(await page.locator('#memory-review .after').textContent(), /纠正后的记忆正文/);
  assert.equal(await page.locator('#memory-protected-approval').isChecked(), false);
  await button(page, '继续确认');
  assert.equal(await page.locator('#modal').evaluate(node => node.open), false);
  assert.equal(native.count('confirm_memory_change'), 0);
  await approve(page); await page.locator('#modal').getByRole('button', { name: '取消', exact: true }).click();
  assert.equal(native.count('confirm_memory_change'), 0);
  await button(page, '继续确认'); await commit(page);
  assert.deepEqual(native.matching('confirm_memory_change')[0].payload, { sessionId: vault.session_id, previewId: 'memory-edit-preview', approveProtected: true });
  assert.equal(native.data.memory.evidence, 'assistant_suggestion'); assert.equal(native.data.memory.status, 'tentative');
  assert.deepEqual(native.data.memory.source_refs, original.source_refs);
  assert.equal(native.data.memory.recorded_at, original.recorded_at);
  assert.equal(native.data.memory.protected, false); assert.equal(native.data.memory.revision, 4);
  assert.equal(await page.locator('#memory-review').count(), 0);
});

test('正文、标签、保护和遗忘原因的输入变化均撤销旧预览与批准', async t => {
  const { page, native } = await fixture(t);
  await edit(page);
  for (const change of [() => page.locator('#memory-content').fill('另一个更正'),
    () => page.locator('#memory-labels').fill('另一个标签'), () => page.locator('#memory-protected').uncheck()]) {
    await review(page); await page.locator('#memory-protected-approval').check(); await change();
    assert.equal(await page.locator('#memory-review').count(), 0);
    assert.equal(native.count('confirm_memory_change'), 0);
  }
  await button(page, '取消操作'); await idle(page);
  await button(page, '设置遗忘规则'); await page.locator('#memory-forget-reason').fill('已过期'); await review(page);
  await page.locator('#memory-protected-approval').check(); await page.locator('#memory-forget-reason').fill('需要重新判断');
  assert.equal(await page.locator('#memory-review').count(), 0);
  await review(page); assert.equal(await page.locator('#memory-protected-approval').isChecked(), false);
  await button(page, '取消本次变更'); await idle(page);
  assert.equal(native.count('confirm_memory_change'), 0);
});

test('遗忘影响共享出处与其他受保护记忆，恢复仍受其他规则影响时明确说明', async t => {
  const { page, native } = await fixture(t, { protected: false });
  await select(page); await button(page, '设置遗忘规则'); await page.locator('#memory-forget-reason').fill('我不再希望使用这些内容'); await review(page);
  assert.match(await page.locator('#memory-review').textContent(), /受影响记忆3/);
  assert.match(await page.locator('#memory-review').textContent(), /受影响原始记录2/);
  assert.equal(await page.locator('#memory-protected-approval').isChecked(), false);
  await button(page, '继续确认'); assert.equal(native.count('confirm_memory_change'), 0);
  await approve(page); await commit(page);
  assert.equal(await page.locator('.memory-list .result-card').count(), 0);
  await page.locator('#include-hidden-memories').check(); await idle(page); await select(page);
  assert.equal(await page.getByRole('button', { name: '修改正文、标签与保护', exact: true }).count(), 0);
  await button(page, '查看出处 1'); await idle(page);
  assert.equal(await page.locator('.memory-source .body-text').textContent(), source.content);
  await button(page, '撤销这条遗忘规则'); await review(page);
  native.data.stillHiddenAfterRestore = true;
  await approve(page); await commit(page);
  assert.match(await page.locator('#notice').textContent(), /仍受其他遗忘规则影响/);
  assert.equal(native.matching('review_memory_visibility').at(-1).payload.restore, true);
  await select(page);
  assert.equal(await page.getByRole('button', { name: '撤销这条遗忘规则', exact: true }).count(), 0);
  assert.match(await page.locator('.memory-detail').textContent(), /当前隐藏来自其他记忆或来源/);
});

test('保存期间禁用重复确认和范围切换；快照冲突撤销预览，允许重新审查', async t => {
  const { page, native } = await fixture(t);
  await edit(page); await review(page); await approve(page);
  const release = native.hold('confirm_memory_change');
  await page.locator('#modal').getByRole('button', { name: '确认执行', exact: true }).evaluate(node => { node.click(); node.click(); });
  await page.waitForFunction(() => document.querySelector('#operation').textContent.includes('正在保存记忆变更'));
  assert.equal(native.count('confirm_memory_change'), 1);
  assert.equal(await page.locator('#switch-vault').isDisabled(), true);
  assert.equal(await page.getByRole('combobox', { name: '资料范围' }).isDisabled(), true);
  release({ hidden: false, status: 'tentative' }); await idle(page);
  await edit(page); await review(page); await approve(page);
  native.fail('confirm_memory_change', '资料、来源或遗忘规则已改变，请重新审阅影响'); await commit(page);
  assert.match(await page.locator('#notice').textContent(), /已改变/);
  assert.equal(await page.locator('#memory-review').count(), 0);
  assert.equal(await page.locator('#memory-content').inputValue(), '纠正后的记忆正文');
  await review(page); assert.equal(await page.locator('#memory-protected-approval').isChecked(), false);
});

test('范围与资料库切换废弃全文、来源、草稿、隐藏筛选和预览，取消不会写入', async t => {
  const { page, native } = await fixture(t);
  await page.locator('#include-hidden-memories').check(); await idle(page);
  await edit(page); await review(page); await page.locator('#memory-protected-approval').check();
  await page.getByRole('combobox', { name: '资料范围' }).selectOption('work'); await idle(page);
  assert.equal(await page.locator('#memory-review, #memory-content, .memory-full-text').count(), 0);
  assert.equal(await page.locator('#include-hidden-memories').isChecked(), false);
  assert.equal(native.matching('manage_memories').at(-1).payload.scope, 'work');
  await page.getByRole('combobox', { name: '资料范围' }).selectOption('personal'); await idle(page);
  await edit(page); await review(page);
  await button(page, '切换资料库');
  native.next('choose_vault', () => ({ ...vault, session_id: 'second-session', display_name: '另一个合成库' }));
  await page.locator('#modal').getByRole('button', { name: '打开已有资料库', exact: true }).click(); await idle(page);
  await navigate(page, '记忆管理');
  assert.equal(await page.locator('#memory-review, #memory-content, .memory-full-text').count(), 0);
  assert.equal(native.matching('manage_memories').at(-1).payload.sessionId, 'second-session');
  assert.equal(native.count('confirm_memory_change'), 0);
});

test('已替代记忆只在主动筛选后出现且不能编辑，读取失败可重试', async t => {
  const { page, native } = await fixture(t, { status: 'superseded' });
  assert.equal(await page.locator('.memory-list .result-card').count(), 0);
  native.fail('manage_memories', '合成读取故障');
  await page.locator('#include-hidden-memories').check(); await idle(page);
  assert.match(await page.locator('#content').textContent(), /记忆列表读取失败/);
  await button(page, '重试读取记忆'); await idle(page); await select(page);
  assert.equal(await page.getByRole('button', { name: '修改正文、标签与保护', exact: true }).count(), 0);
  assert.match(await page.locator('.memory-detail').textContent(), /已替代或撤回的记忆不能直接编辑/);
  assert.equal(native.count('confirm_memory_change'), 0);
});

test('遗忘后旧检索、来源选择与交接正文被丢弃，往返页面不会恢复旧预览', async t => {
  const { page, native } = await fixture(t);
  await navigate(page, '查找与阅读');
  await page.locator('.result-card').click(); await idle(page);
  await button(page, '选择这条资料'); await idle(page);
  await navigate(page, '会话与接续'); await page.getByRole('button', { name: /合成会话/ }).click(); await idle(page);
  await button(page, '准备交接内容'); await idle(page);
  assert.equal(await page.getByRole('textbox', { name: '交接内容预览' }).count(), 1);
  await navigate(page, '记忆管理'); await select(page); await button(page, '设置遗忘规则');
  await page.locator('#memory-forget-reason').fill('不再使用'); await review(page); await approve(page); await commit(page);
  await navigate(page, '查找与阅读'); assert.equal(await page.locator('.result-card, .reading-pane .body-text').count(), 0);
  await navigate(page, '整理记忆'); assert.equal(await page.getByRole('button', { name: '移除', exact: true }).count(), 0);
  await navigate(page, '会话与接续'); assert.equal(await page.getByRole('textbox', { name: '交接内容预览' }).count(), 0);
  await navigate(page, '记忆管理'); assert.equal(await page.locator('#memory-review').count(), 0);
  assert.equal(native.count('confirm_memory_change'), 1);
});

test('恶意标记保持纯文本，离开管理页撤销尚未保存的审阅', async t => {
  const hostile = '<img src=x onerror="window.__bad=1"><script>window.__bad=2</script>';
  const { page, native } = await fixture(t, { content: hostile, labels: [hostile] });
  await select(page); assert.equal(await page.locator('.memory-full-text').textContent(), hostile);
  await button(page, '修改正文、标签与保护'); await page.locator('#memory-content').fill(`${hostile}更新`); await review(page);
  assert.equal(await page.locator('#content img, #content script, #content [onerror]').count(), 0);
  assert.equal(await page.evaluate(() => window.__bad), undefined);
  await navigate(page, '概览'); await navigate(page, '记忆管理');
  assert.equal(await page.locator('#memory-review').count(), 0);
  assert.equal(native.count('confirm_memory_change'), 0);
});
