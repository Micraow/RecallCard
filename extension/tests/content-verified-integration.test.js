// 实际 content 与 ComposerAdapter 的合成集成测试；不读取网站或真实对话。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
import { setup } from './fixtures/composer.mjs';

const contentSource = await readFile(new URL('../content.js', import.meta.url), 'utf8');
for (const timing of ['核验等待中', '核验成功后最终确认前']) {
  test(`真实 content 与 Adapter：${timing}同 URL 会话变化后不回写、不再次填框`, async () => {
    const { node, doc } = setup();
    node.value = '必须保留的原草稿';
    node.getBoundingClientRect = () => ({ left: 0, top: 0, right: 600, bottom: 120 });
    doc.elementFromPoint = () => node;
    doc.visibilityState = 'visible';
    Object.assign(doc.defaultView, {
      innerWidth: 800, innerHeight: 600, setTimeout, clearTimeout,
      requestAnimationFrame: fn => setTimeout(fn, 0), cancelAnimationFrame: clearTimeout,
    });
    const messageNode = id => ({ getAttribute: key => key === 'data-message-id' ? id : key === 'data-message-author-role' ? 'user' : '' });
    let messages = [messageNode('first')], listener, adapter;
    const changeConversation = () => { messages = [messageNode('other-conversation')]; };
    const location = { origin: 'https://chatgpt.com', pathname: '/c/unchanged', get href() { return this.origin + this.pathname; } };
    const window = { addEventListener() {} }; window.top = window;
    class ActualAdapter extends globalThis.RecallCardComposerAdapter {
      constructor(document, loc) { super(document, loc); adapter = this; }
      async insertVerified(...args) {
        const result = await super.insertVerified(...args);
        if (timing === '核验成功后最终确认前') changeConversation();
        return result;
      }
    }
    class ConversationAdapter { nodes() { return messages; } }
    const extensionId = 'abcdefghijklmnopabcdefghijklmnop';
    const chrome = { runtime: {
      id: extensionId,
      getURL: path => `chrome-extension://${extensionId}/${path}`,
      onMessage: { addListener: fn => { listener = fn; } },
      sendMessage: async message => ({ ok: true, result: {
        route: message.route, nonce: 'a'.repeat(48), session_ref: `chatgpt:${message.token}`,
      } }),
    } };
    vm.runInNewContext(contentSource, {
      window, document: doc, location, crypto: webcrypto, chrome,
      RecallCardSites: globalThis.RecallCardSites,
      RecallCardComposerAdapter: ActualAdapter,
      RecallCardConversationAdapter: ConversationAdapter,
      setInterval() {}, // 不依赖轮询；最终核验必须主动检查当前会话身份。
    });
    const send = message => new Promise(resolve => listener(message, {
      id: extensionId, url: chrome.runtime.getURL('background.js'),
    }, resolve));
    const first = (await send({ kind: 'describe' })).result;
    node.dispatchEvent = event => {
      node.events.push(event.type);
      if (timing === '核验等待中' && node.events.length === 1) setTimeout(changeConversation, 0);
    };
    const pending = send({ kind: 'insert', ...first, capsule: { id: 'first', ...first, text: '原创合成资料' } });
    const written = node.value;
    const result = await pending;
    assert.equal(result.ok, false);
    assert.match(result.error, /会话/);
    assert.equal(adapter.verificationFailed, true);
    assert.equal(node.value, written);
    assert.deepEqual(node.events, ['input']);
    const rebound = (await send({ kind: 'describe' })).result;
    const retry = await send({ kind: 'insert', ...rebound, capsule: { id: 'retry', ...rebound, text: '不得再次追加' } });
    assert.equal(retry.ok, false);
    assert.match(retry.error, /不会再自动写入/);
    assert.equal(node.value, written);
    assert.deepEqual(node.events, ['input']);
  });
}
