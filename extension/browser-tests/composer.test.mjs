// 合成页面上的真实 Chromium DOM 回归；不访问任何厂商页面。
import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
let browser;
before(async () => { browser = await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH ? { executablePath: process.env.RECALLCARD_CHROMIUM_PATH } : {}); });
after(async () => { await browser?.close(); });
async function pageFor(html) {
  const page = await browser.newPage();
  await page.route('**/*', route => route.abort());
  await page.setContent('<!doctype html><meta charset="utf-8"><main>' + html + '</main>');
  await page.addScriptTag({ path: fileURLToPath(new URL('../site-adapters.js', import.meta.url)) });
  await page.addScriptTag({ path: fileURLToPath(new URL('../composer-adapter.js', import.meta.url)) });
  return page;
}
const platforms = [
  ['ChatGPT', 'https://chatgpt.com/', '<textarea id="prompt-textarea"></textarea>'],
  ['ChatGPT mobile', 'https://chatgpt.com/', '<textarea id="mobile-composer-prompt"></textarea>'],
  ['Qwen', 'https://chat.qwen.ai/', '<textarea class="message-input-textarea"></textarea>'],
  ['Z.ai', 'https://chat.z.ai/', '<textarea id="chat-input"></textarea>'],
];
for (const [name, url, html] of platforms) test(`${name}：原生 textarea 追加/撤销保留用户编辑，绝不发送`, async () => {
  const page = await pageFor(html);
  try {
    const result = await page.evaluate(url => {
      const events = [];
      for (const type of ['click', 'keydown', 'submit', 'input']) document.addEventListener(type, () => events.push(type));
      const adapter = new RecallCardComposerAdapter(document, { href: url });
      const node = adapter.find(); node.value = '已有草稿🙂\n第二行';
      const original = node.value;
      adapter.insert({ id: 'engine', nonce: 'a'.repeat(48), text: '合成上下文' });
      const appended = node.value.startsWith(original) && node.value.includes('合成上下文');
      let repeated = false; try { adapter.insert({ id: 'engine', nonce: 'a'.repeat(48), text: '合成上下文' }); } catch { repeated = true; }
      node.value = '新增前文\n' + node.value + '\n新增后文';
      adapter.remove('engine');
      return { appended, repeated, text: node.value, expected: '新增前文\n' + original + '\n新增后文', events };
    }, url);
    assert.equal(result.appended, true); assert.equal(result.repeated, true);
    assert.equal(result.text, result.expected); assert.deepEqual(result.events, ['input', 'input']);
  } finally { await page.close(); }
});
test('真实富文本节点身份与前后编辑在撤销后保留', async () => {
  const page = await pageFor('<div id="prompt-textarea" class="ProseMirror" contenteditable="true"><p><strong>原有粗体</strong></p></div>');
  try {
    const result = await page.evaluate(() => {
      const adapter = new RecallCardComposerAdapter(document, { href: 'https://chatgpt.com/' });
      const node = adapter.find(); const original = node.firstChild;
      adapter.insert({ id: 'rich', nonce: 'b'.repeat(48), text: '合成上下文' });
      const extra = document.createElement('p'); extra.textContent = '新增用户草稿'; node.append(extra);
      adapter.remove('rich');
      return { same: node.firstChild === original && node.lastChild === extra, html: node.innerHTML };
    });
    assert.equal(result.same, true); assert.equal(result.html, '<p><strong>原有粗体</strong></p><p>新增用户草稿</p>');
  } finally { await page.close(); }
});
test('重建注入节点并新增格式后拒绝撤销，不删除用户改动', async () => {
  const page = await pageFor('<div id="prompt-textarea" class="ProseMirror" contenteditable="true"><p>原有草稿</p></div>');
  try {
    const result = await page.evaluate(() => {
      const adapter = new RecallCardComposerAdapter(document, { href: 'https://chatgpt.com/' });
      const node = adapter.find();
      adapter.insert({ id: 'changed', nonce: 'c'.repeat(48), text: '合成上下文' });
      const injected = node.lastChild;
      const replacement = document.createElement('strong'); replacement.textContent = injected.textContent;
      node.replaceChild(replacement, injected);
      const before = node.innerHTML; let rejected = false;
      try { adapter.remove('changed'); } catch { rejected = true; }
      return { rejected, before, after: node.innerHTML };
    });
    assert.equal(result.rejected, true); assert.equal(result.after, result.before);
  } finally { await page.close(); }
});
test('真实CSS隐藏和只读检查不会误写其它编辑器', async () => {
  const page = await pageFor('<textarea id="mobile-composer-prompt" style="display:none"></textarea><textarea id="prompt-textarea" readonly>保留</textarea>');
  try {
    const result = await page.evaluate(() => {
      const adapter = new RecallCardComposerAdapter(document, { href: 'https://chatgpt.com/' });
      let rejected = false; try { adapter.insert({ id: 'readonly', nonce: 'd'.repeat(48), text: '合成' }); } catch { rejected = true; }
      return { rejected, values: [...document.querySelectorAll('textarea')].map(n => n.value) };
    });
    assert.equal(result.rejected, true); assert.deepEqual(result.values, ['', '保留']);
  } finally { await page.close(); }
});
test('无格式段落归一化后仍能精确移除自己的纯文本', async () => {
  const page = await pageFor('<div id="prompt-textarea" class="ProseMirror" contenteditable="true"><p>原有草稿</p></div>');
  try {
    const result = await page.evaluate(() => {
      const adapter = new RecallCardComposerAdapter(document, { href: 'https://chatgpt.com/' });
      const node = adapter.find(); const original = node.firstChild;
      adapter.insert({ id: 'normalized', nonce: 'e'.repeat(48), text: '合成上下文' });
      const injected = node.lastChild;
      const normalized = document.createElement('p'); normalized.textContent = injected.textContent;
      node.replaceChild(normalized, injected);
      const removed = adapter.remove('normalized');
      return { removed: removed.status, text: node.textContent, same: node.firstChild === original };
    });
    assert.equal(result.removed, 'removed'); assert.equal(result.text, '原有草稿'); assert.equal(result.same, true);
  } finally { await page.close(); }
});
