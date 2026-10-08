import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { relayHarness } from './fixtures/relay-harness.mjs';
let browser;
before(async () => { browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {}); });
after(async () => { await browser?.close(); });
for (const platform of ['chatgpt', 'deepseek']) {
  test(`${platform} 完整接力：bootstrap→read partial→cursor续读→当前轮回答就绪，零自动发送`, async () => {
    const h = await relayHarness(browser, platform);
    try {
      await h.settle(); assert.equal(h.state().relay.phase, 'awaiting_user_send'); assert.equal(h.state().relay.round, 1);
      await h.sendDraft('u1'); await h.turn('assistant', 'a1', JSON.stringify(h.action('r_page1', {})), { rendered: true }); await h.settle();
      assert.equal(h.state().preview.id, 'r_page1'); assert.equal(h.state().relay.round, 2);
      const first = await h.sendDraft('u2'); assert.match(first, /synthetic-cursor-2/); assert.match(first, /text_range/);
      await h.turn('assistant', 'a2', JSON.stringify(h.action('r_page2', { cursor: 'synthetic-cursor-2' })), { rendered: true }); await h.settle();
      assert.equal(h.state().relay.round, 3); const second = await h.sendDraft('u3'); assert.match(second, /第二段合成证据/);
      const final = { protocol: 'recallcard.final/1', nonce: h.state().nonce, session_ref: h.state().session_ref, after_request_id: 'r_page2' };
      await h.turn('assistant', 'a3', JSON.stringify(final), { rendered: true }); await h.settle();
      assert.equal(h.state().relay.phase, 'result_ready'); assert.match(await h.page.locator('#recallcard-relay strong').innerText(), /回答已就绪/);
      assert.equal(h.calls.filter(r => r.action === 'authorized_read' && r.arguments.arguments.cursor === 'synthetic-cursor-2').length, 2);
      const reads = h.calls.filter(r => r.action === 'authorized_read').length; await h.settle(); assert.equal(h.calls.filter(r => r.action === 'authorized_read').length, reads);
      assert.equal(await h.page.evaluate(() => window.websiteSends), 0); assert.equal(await h.page.evaluate(() => window.clipboardReads), 0);
    } finally { await h.page.close(); }
  });
}
test('已闭合请求块仍无完成凭据时不执行；只观察输入变化不读取剪贴板', async () => {
  const h = await relayHarness(browser);
  try {
    await h.settle(); await h.sendDraft('u1'); const reads = h.calls.filter(r => r.action === 'authorized_read').length;
    await h.turn('assistant', 'streaming', JSON.stringify(h.action('r_wait', {})), { rendered: true, complete: false }); await h.settle(); await h.settle();
    assert.equal(h.calls.filter(r => r.action === 'authorized_read').length, reads); assert.equal(h.state().relay.phase, 'waiting_reply');
    await h.page.locator('[data-message-id="streaming"]').evaluate(node => node.setAttribute('data-is-streaming', 'false')); await h.settle();
    assert.equal(h.state().preview.id, 'r_wait'); assert.equal(await h.page.evaluate(() => window.clipboardReads), 0);
  } finally { await h.page.close(); }
});
test('用户异步编辑后保留全文，显式复制；暂停/恢复/开关不重放同轮', async () => {
  const h = await relayHarness(browser);
  try {
    h.editDuringRead(); await h.settle(); assert.equal(await h.page.locator('main textarea').inputValue(), '用户在等候中修改的草稿');
    assert.equal(h.state().preview.delivery, 'prepared'); assert.equal(h.state().relay.manual_fallback, true);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '复制本轮资料', exact: true }).click();
    await h.page.waitForFunction(() => window.clipboardWrites.length === 1);
    assert.match((await h.page.evaluate(() => window.clipboardWrites))[0], /仅合成偏好/);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '暂停', exact: true }).click();
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('接力已暂停'));
    const count = h.calls.length; await h.settle(); assert.equal(h.calls.length, count); const id = h.state().preview.id;
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '恢复', exact: true }).click(); await h.settle(); assert.equal(h.state().preview.id, id); assert.equal(h.state().relay.round, 1);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '关闭接力', exact: true }).click(); await h.settle(); assert.equal(h.state().relay.enabled, false);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: 'RecallCard · 开启接力', exact: true }).click(); await h.settle(); assert.equal(h.state().relay.enabled, true); assert.equal(h.state().preview.id, id);
    assert.equal(await h.page.locator('main textarea').inputValue(), '用户在等候中修改的草稿'); assert.equal(await h.page.evaluate(() => window.websiteSends), 0);
  } finally { await h.page.close(); }
});
test('输入框改版时手动粘贴完整请求可续读，不重复request_id、不自动填写未知节点', async () => {
  const h = await relayHarness(browser);
  try {
    await h.page.locator('main textarea').evaluate(node => node.id = 'unknown-editor');
    await h.settle(); assert.equal(h.state().relay.manual_fallback, true);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '准备使用说明', exact: true }).click();
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('输入框不可用或已被编辑'));
    assert.equal(h.state().preview.action, 'bootstrap'); assert.equal(h.state().preview.delivery, 'prepared');
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '我已发送', exact: true }).click();
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('已按你的确认记录发送'));
    await h.page.locator('#recallcard-relay').getByText('网页改版或手动接力', { exact: true }).click();
    const input = h.page.locator('#recallcard-relay').getByRole('textbox', { name: '粘贴完整 AI 请求或回答' });
    const text = '```recallcard-action\n' + JSON.stringify(h.action('r_manual', {})) + '\n```'; await input.fill(text);
    await h.page.locator('#recallcard-relay').getByRole('button', { name: '处理已粘贴内容' }).click();
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('输入框不可用或已被编辑'));
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('第 2 轮'));
    assert.equal(h.state().preview.id, 'r_manual');
    await input.fill(text); await h.page.locator('#recallcard-relay').getByRole('button', { name: '处理已粘贴内容' }).click();
    await h.page.waitForFunction(() => document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('已处理或正在处理'));
    assert.equal(h.writes.length, 0); assert.equal(await h.page.locator('main textarea').inputValue(), '合成问题');
  } finally { await h.page.close(); }
});
