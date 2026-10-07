// 真实 Chromium 布局验收；原生服务使用显式合成边界，Rust任务另行验证。
import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { chromium } from '../../extension/node_modules/playwright/index.mjs';
import { workspaceAssets, captureBrowserEvidence, button, idle } from './workspace-browser-helpers.mjs';
let browser, assets;
before(async () => { assets = await workspaceAssets(); browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {}); });
after(async () => browser?.close());
const vault = { session_id: 'bulk-layout', display_name: '我的资料', root: '/synthetic/default', scopes: ['personal'], event_count: 0, memory_count: 0, health: { ok: true } };
const preview = { preview_id: 'bulk-preview', session_id: vault.session_id, scope: 'personal', format: 'auto', files: Array.from({ length: 32 }, (_, i) => ({ file_name: `第${i + 1}份合成DeepSeek历史导出.json`, byte_count: 4096 })), conversations: Array.from({ length: 120 }, (_, i) => ({ source_id: `synthetic-${i}`, event_count: 1 })), event_count: 120, redacted_event_count: 1, samples: [{ role: 'user', content: '我希望在别的AI中继续完成先前任务。' }], coverage: { deepseek: { branch_points: 1 }, notes: ['包含全部可见分支；不包含隐藏推理和附件原件。'] }, warning: '请核对数量后保存。' };
const running = { job_id: 'bulk-job', scope: 'personal', state: 'running', events_total: 120, events_processed: 0, events_added: 0, events_duplicates: 0, files_total: 32, conversations_total: 120, can_resume: false, message: '正在保存对话' };
async function setup(t, width, height) {
  const page = await browser.newPage({ viewport: { width, height }, locale: 'zh-CN' });
  const calls = [], errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.exposeFunction('__invoke', async (command, payload) => {
    calls.push({ command, payload });
    switch (command) {
      case 'restore_workspace': case 'remember_workspace': return null;
      case 'list_import_jobs': return [];
      case 'open_default_workspace': return vault;
      case 'vault_status': return vault;
      case 'cancel_previews': return null;
      case 'pick_import_files': return preview;
      case 'start_import_job': return running;
      case 'import_job_status': return { ...running, state: 'completed', events_processed: 120, events_added: 120, message: '已保存120条消息' };
      default: throw new Error(`未定义合成边界：${command}`);
    }
  });
  await page.addInitScript(() => { window.__TAURI__ = { core: { invoke: (command, payload) => window.__invoke(command, payload) } }; });
  await page.route('**/*', route => {
    const url = new URL(route.request().url()), body = url.origin === 'https://recallcard.test' && assets.get(url.pathname);
    return body ? route.fulfill({ status: 200, body, contentType: url.pathname.endsWith('.js') ? 'text/javascript; charset=utf-8' : url.pathname.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8' }) : route.abort();
  });
  t.after(async () => { if (!t.passed || errors.length) await captureBrowserEvidence(page, `bulk-import-${width}-${t.name}`); await page.close(); assert.deepEqual(errors, []); });
  await page.goto('https://recallcard.test/index.html'); await button(page, '开始使用'); await idle(page);
  return { page, calls };
}
async function reachable(control) {
  await control.scrollIntoViewIfNeeded();
  assert.equal(await control.evaluate(node => { const rect = node.getBoundingClientRect(), x = rect.x + rect.width / 2, y = rect.y + rect.height / 2; const hit = document.elementFromPoint(x, y); return rect.width > 0 && rect.height > 0 && y >= 0 && y <= innerHeight && (hit === node || node.contains(hit)); }), true, '主要操作中心必须在窗口内且无遮挡');
}
for (const [width, height] of [[1280, 800], [860, 700], [620, 600]]) {
  test(`多文件导入${width}px：首次开始无目录配置，单次批准与进度操作可达`, async t => {
    const { page, calls } = await setup(t, width, height);
    const choose = page.getByRole('button', { name: '选择导出文件', exact: true }); await reachable(choose); await choose.click(); await idle(page);
    assert.match(await page.locator('.import-job').textContent(), /32 个文件.*120 个有消息的会话.*120 条消息/);
    assert.equal(await page.locator('.import-job-details').evaluate(node => node.open), false);
    const approve = page.getByRole('button', { name: '导入全部 120 条消息', exact: true }); await reachable(approve);
    assert.equal(calls.some(call => call.command === 'start_import_job'), false);
    assert.equal(await page.locator('#modal').evaluate(node => node.open), false);
    await captureBrowserEvidence(page, `bulk-preview-${width}`); await approve.click(); await idle(page);
    await page.getByRole('heading', { name: '导入完成', exact: true }).waitFor();
    assert.equal(calls.filter(call => call.command === 'start_import_job').length, 1);
    assert.equal(await page.locator('progress').getAttribute('value'), '120');
    await reachable(page.getByRole('button', { name: '查看已保存会话', exact: true }));
    await reachable(page.getByRole('button', { name: '导入其他文件', exact: true }));
    await captureBrowserEvidence(page, `bulk-completed-${width}`);
  });
}
