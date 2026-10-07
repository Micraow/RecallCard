/* Fixed first-party origins and composer-only selectors; no transcript access. */
(() => {
  'use strict';
  const uuid = '[A-Fa-f0-9]{8}-[A-Fa-f0-9]{4}-[A-Fa-f0-9]{4}-[A-Fa-f0-9]{4}-[A-Fa-f0-9]{12}';
  const sites = Object.freeze([
    Object.freeze({ id: 'chatgpt', name: 'ChatGPT', origin: 'https://chatgpt.com', paths: /^(?:\/|\/c\/[A-Za-z0-9_-]+|\/g\/[A-Za-z0-9_-]+(?:\/c\/[A-Za-z0-9_-]+)?)\/?$/u, selector: '#prompt-textarea, textarea#mobile-composer-prompt', richText: true }),
    Object.freeze({ id: 'qwen', name: 'Qwen', origin: 'https://chat.qwen.ai', paths: new RegExp(`^(?:/|/c/${uuid})/?$`, 'u'), selector: 'textarea.message-input-textarea', richText: false }),
    Object.freeze({ id: 'zai', name: 'Z.ai', origin: 'https://chat.z.ai', paths: new RegExp(`^(?:/|/c/${uuid})/?$`, 'u'), selector: 'textarea#chat-input', richText: false }),
  ]);
  function forUrl(url) {
    let parsed;
    try { parsed = new URL(url); } catch { throw new Error('页面地址无效'); }
    const site = sites.find(candidate => candidate.origin === parsed.origin);
    if (!site || parsed.username || parsed.password || !site.paths.test(parsed.pathname) || parsed.pathname.includes('//') || (site.id !== 'chatgpt' && (parsed.search || parsed.hash))) {
      throw new Error('此网站或路径尚未支持；仅允许已列出的 ChatGPT、Qwen、Z.ai 对话页面');
    }
    return Object.freeze({ id: site.id, name: site.name, origin: site.origin, selector: site.selector, richText: site.richText, route: parsed.origin + parsed.pathname.replace(/\/$/u, '') });
  }
  globalThis.RecallCardSites = Object.freeze({ forUrl });
})();
