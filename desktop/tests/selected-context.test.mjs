// 真实 Chromium 布局与交互；原生桥使用合成数据，Rust 服务另行验证。
import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { chromium } from '../../extension/node_modules/playwright/index.mjs';
import { workspaceAssets, captureBrowserEvidence, idle, button, openSearch } from './workspace-browser-helpers.mjs';
let browser, assets;
before(async () => { assets = await workspaceAssets(); browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {}); });
after(async () => browser?.close());
const vault = { session_id: 'selected-context-layout', root: '/synthetic/vault', display_name: '合成资料', scopes: ['personal'], event_count: 8, memory_count: 0, health: { ok: true } };
const rows = Array.from({ length: 8 }, (_, index) => ({
  ref: `event:selected-${index}`, kind: 'event', role: index % 2 ? 'assistant' : 'user',
  evidence: index % 2 ? 'AssistantSuggestion' : 'UserExplicit', state: 'captured',
  conversation_ref: `conversation-${index}`, conversation_title: `合成第${index + 1}个会话`,
  text: `合成资料${index + 1}：先完成实际使用任务，再核对原始出处。`,
  occurred_at: index ? null : '2026-10-07T08:00:00Z', platform: 'synthetic',
}));
function response(payload) {
  const selected = payload.references.map(reference => rows.find(row => row.ref === reference));
  assert.ok(selected.every(Boolean));
  const stable = 'recallcard.context/1\n固定说明与用户已选背景\n';
  const records = selected.map(row => ({ ...row, status: row.state, source_refs: [row.ref],
    sources: [{ ref: row.ref, role: row.role, conversation_ref: row.conversation_ref, conversation_title: row.conversation_title, occurred_at: row.occurred_at }],
    truncated: false, content_retained: true }));
  const value = { text: `${stable}${payload.goal}\n${records.map(row => `[${row.ref}]\n${row.text}`).join('\n')}`,
    stable_prefix: stable, background: { stable_text: '固定说明与用户已选背景\n', bootstrap_version: 'synthetic-stable', truncated: false, refs: [] },
    records, selected_refs: payload.references, selected_count: selected.length, included_count: selected.length,
    pending_refs: [], partial: false, truncated: false, scope: payload.scope, budget_tokens: payload.budgetTokens,
    budget_unit: 'conservative_utf8_bytes', estimated_tokens: 0 };
  for (;;) { const bytes = Buffer.byteLength(JSON.stringify(value)); if (bytes === value.estimated_tokens) break; value.estimated_tokens = bytes; }
  return value;
}
async function setup(t, width, height) {
  const page = await browser.newPage({ viewport: { width, height }, locale: 'zh-CN', timezoneId: 'UTC' });
  const calls = [], errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.exposeFunction('__invoke', async (command, payload) => {
    calls.push({ command, payload });
    switch (command) {
      case 'restore_workspace': case 'remember_workspace': case 'cancel_previews': case 'write_clipboard': return null;
      case 'list_import_jobs': return [];
      case 'choose_vault': case 'vault_status': return vault;
      case 'list_conversations': return { conversations: [], total: 0, next_offset: null };
      case 'search_records': return { results: rows, truncated: false };
      case 'prepare_selected_context': return response(payload);
      default: throw new Error(`未定义合成边界：${command}`);
    }
  });
  await page.addInitScript(() => { window.__TAURI__ = { core: { invoke: (command, payload) => window.__invoke(command, payload) } }; });
  await page.route('**/*', route => {
    const url = new URL(route.request().url()), body = url.origin === 'https://recallcard.test' && assets.get(url.pathname);
    return body ? route.fulfill({ status: 200, body, contentType: url.pathname.endsWith('.js') ? 'text/javascript; charset=utf-8' : url.pathname.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8' }) : route.abort();
  });
  t.after(async () => { if (!t.passed || errors.length) await captureBrowserEvidence(page, `selected-context-failure-${width}`); await page.close(); assert.deepEqual(errors, []); });
  await page.goto('https://recallcard.test/index.html'); await idle(page);
  await button(page, '打开已有资料库'); await idle(page); await openSearch(page, '核对原始出处');
  return { page, calls };
}
async function reachable(control) {
  await control.scrollIntoViewIfNeeded();
  assert.equal(await control.evaluate(node => {
    const rect = node.getBoundingClientRect(), x = rect.x + rect.width / 2, y = rect.y + rect.height / 2;
    const hit = document.elementFromPoint(x, y);
    return rect.width > 0 && rect.height > 0 && x >= 0 && x <= innerWidth && y >= 0 && y <= innerHeight && (hit === node || node.contains(hit));
  }), true, '关键操作中心必须在窗口内且无遮挡');
}
for (const [width, height] of [[1180, 820], [820, 620]]) {
  test(`跨会话交接${width}px：选择、核对与逐字复制在同一工作区可达`, async t => {
    const { page, calls } = await setup(t, width, height);
    await button(page, '带走这些资料'); await idle(page);
    assert.equal(await page.locator('#selection-context-panel .selected-context-record').count(), 3);
    await reachable(page.getByRole('button', { name: '复制交接内容', exact: true }));
    await captureBrowserEvidence(page, `selected-context-preview-${width}`);
    await button(page, '调整所选资料'); await idle(page);
    const first = page.getByRole('checkbox', { name: `带上资料：第 1 条，用户原话，合成第1个会话，${rows[0].text}`, exact: true });
    await reachable(first); await first.uncheck();
    const fourth = page.getByRole('checkbox', { name: `带上资料：第 4 条，AI 回复，合成第4个会话，${rows[3].text}`, exact: true });
    await reachable(fourth); await fourth.check();
    await button(page, '带走这些资料'); await idle(page);
    assert.deepEqual(calls.filter(call => call.command === 'prepare_selected_context').at(-1).payload.references, rows.slice(1, 4).map(row => row.ref));
    const preview = await page.getByLabel('交接内容预览', { exact: true }).inputValue();
    const copy = page.getByRole('button', { name: '复制交接内容', exact: true });
    await reachable(copy); await copy.click(); await idle(page);
    assert.deepEqual(calls.filter(call => call.command === 'write_clipboard').map(call => call.payload), [{ text: preview }]);
    assert.equal(calls.filter(call => call.command === 'prepare_selected_context').length, 3);
    assert.equal(await page.locator('#navigation button').count(), 2);
    await button(page, '返回搜索结果'); await idle(page);
    assert.equal(await page.locator('#selection-context-panel').count(), 0);
    assert.equal(await page.locator('#query').inputValue(), '核对原始出处');
    await reachable(page.locator('.results .result-card').first());
    await captureBrowserEvidence(page, `selected-context-return-${width}`);
  });
}
