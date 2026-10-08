// 原创合成网页与本机桥，绝不连接真实账号、原始对话或私人剪贴板。
import { fileURLToPath } from 'node:url';
import { Broker } from '../../broker.js';
export const EXT = 'abcdefghijklmnopabcdefghijklmnop';
export async function relayHarness(browser, platform = 'chatgpt') {
  const origin = platform === 'deepseek' ? 'https://chat.deepseek.com' : 'https://chatgpt.com';
  const path = platform === 'deepseek' ? '/a/chat/s/relay-synthetic' : '/c/relay-synthetic';
  const page = await browser.newPage({ viewport: { width: 1080, height: 800 } });
  const composer = platform === 'deepseek' ? '<textarea class="ds-scroll-area" name="search">合成问题</textarea>' : '<textarea id="mobile-composer-prompt">合成问题</textarea>';
  await page.route('**/*', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html lang="zh-CN"><title>RecallCard 原创接力 fixture</title><body><main><h1>合成网页对话</h1><div id="turns"></div>' + composer + '<button id="website-send">网站发送（fixture）</button></main></body></html>' }));
  await page.goto(origin + path);
  let state = null, now = 10000, editDuringRead = false;
  const calls = [], writes = [];
  const grant = { capture: true, recall: true, provider_disclosure: true, permission_revision: 1, platform };
  const sender = () => ({ id: EXT, origin, url: page.url(), tab: { id: 1, url: page.url() }, frameId: 0, documentId: 'relay-document' });
  const broker = new Broker({ id: EXT, installationId: async () => '11111111-1111-4111-8111-111111111111', popupUrl: `chrome-extension://${EXT}/popup.html`, getTab: async () => ({ id: 1, active: true, url: page.url() }), load: async () => structuredClone(state), save: async (_, next) => { state = structuredClone(next); }, now: () => now += 2000,
    content: async (_, message) => { if (message.kind === 'insert') writes.push(message); return page.evaluate(({ message, EXT }) => new Promise(resolve => window.contentListener(message, { id: EXT, url: `chrome-extension://${EXT}/background.js` }, resolve)), { message, EXT }); },
    native: async (_, request) => {
      calls.push(request);
      if (request.action === 'connection') return { ok: true, result: { connection_id: 'c'.repeat(64), capture_enabled: true, automation: grant, state: 'reachable' } };
      if (request.action === 'automatic_capture') return { ok: true, result: { events_added: request.arguments.conversation.messages.length, events_seen: request.arguments.conversation.messages.length } };
      if (request.action === 'authorized_read') {
        if (editDuringRead) { editDuringRead = false; await page.locator('main textarea').fill('用户在等候中修改的草稿'); }
        const args = request.arguments;
        return { ok: true, result: args.action === 'bootstrap' ? { bootstrap_version: 'v1', stable_text: '仅合成偏好，回答给出依据。', refs: [] } : { status: args.arguments.cursor ? 'complete' : 'partial', results: [{ ref: 'event:evt_long_synthetic', text: args.arguments.cursor ? '第二段合成证据。' : '第一段合成证据。', text_range: { start: args.arguments.cursor ? 24 : 0, end: args.arguments.cursor ? 48 : 24, total: 48 }, snapshot: 'synthetic-v1' }], next_cursor: args.arguments.cursor ? null : 'synthetic-cursor-2', pending_refs: [] } };
      }
      throw new Error('意外本机动作 ' + request.action);
    },
  });
  await page.exposeFunction('relayWorker', message => broker.handle(message, sender()).then(result => ({ ok: true, result }), error => ({ ok: false, error: error.message })));
  await page.evaluate(EXT => {
    window.syntheticClock = 10000; Date.now = () => window.syntheticClock;
    window.setInterval = callback => { window.ticks ??= []; window.ticks.push(callback); return window.ticks.length; };
    // 测试专用：开放 shadow root 供 Playwright 的真实指针点击；产品仍为 closed。
    const attach = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function(init) { return attach.call(this, { ...init, mode: 'open' }); };
    window.clipboardWrites = []; window.clipboardReads = 0; window.websiteSends = 0;
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async text => window.clipboardWrites.push(text), readText: async () => { window.clipboardReads++; throw new Error('不允许读取真实剪贴板'); } } });
    document.querySelector('#website-send').addEventListener('click', () => window.websiteSends++);
    document.addEventListener('submit', () => window.websiteSends++);
    window.chrome = { runtime: { id: EXT, getURL: path => `chrome-extension://${EXT}/` + path, onMessage: { addListener: listener => window.contentListener = listener }, sendMessage: message => window.relayWorker(message) } };
  }, EXT);
  for (const name of ['site-adapters.js', 'composer-adapter.js', 'conversation-format.js', 'conversation-adapters.js', 'clipboard-adapter.js', 'relay-overlay.js', 'content.js']) await page.addScriptTag({ path: fileURLToPath(new URL('../../' + name, import.meta.url)) });
  const tick = async () => { await page.evaluate(() => new Promise(resolve => setTimeout(resolve, 0))); await page.evaluate(async () => { window.syntheticClock += 2000; await window.ticks.at(-1)(); }); };
  const settle = async () => { await tick(); await tick(); };
  const turn = async (role, id, text, { complete = true, rendered = false } = {}) => page.evaluate(({ platform, role, id, text, complete, rendered }) => {
    const node = document.createElement('div'); node.setAttribute('data-message-id', id);
    node.setAttribute(platform === 'deepseek' ? 'data-role' : 'data-message-author-role', role);
    if (platform === 'deepseek') node.className = 'ds-message';
    if (role === 'assistant' && complete) node.setAttribute('data-is-streaming', 'false');
    const body = platform === 'deepseek' && role === 'assistant' ? document.createElement('div') : node;
    if (body !== node) { body.className = 'ds-markdown'; node.append(body); }
    if (rendered) { const pre = document.createElement('pre'), code = document.createElement('code'); code.textContent = text; pre.append(code); body.append(pre); } else body.textContent = text;
    document.querySelector('#turns').append(node);
  }, { platform, role, id, text, complete, rendered });
  const sendDraft = async id => { const text = await page.locator('main textarea').inputValue(); await page.locator('main textarea').fill(''); await turn('user', id, text); return text; };
  const action = (id, args) => ({ protocol: 'recallcard.action/1', request_id: id, nonce: state.nonce, session_ref: state.session_ref, action: 'read', arguments: { refs: ['event:evt_long_synthetic'], budget_bytes: 4000, ...args } });
  return { page, calls, writes, tick, settle, turn, sendDraft, action, state: () => state, editDuringRead: () => { editDuringRead = true; } };
}
