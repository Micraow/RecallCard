(() => {
  'use strict';
  if (window.top !== window) return;
  const adapter = new globalThis.RecallCardComposerAdapter(document);
  let route = location.origin + location.pathname.replace(/\/$/u, '');
  let token = crypto.randomUUID();
  let binding = null;
  let warnings = [];
  const checkNavigation = () => {
    const current = location.origin + location.pathname.replace(/\/$/u, '');
    if (current !== route) {
      warnings = adapter.reset();
      route = current; token = crypto.randomUUID(); binding = null;
    }
  };
  const bind = async () => {
    checkNavigation();
    const result = await chrome.runtime.sendMessage({ kind: 'bind', route, token });
    if (!result?.ok) throw new Error(result?.error || '无法绑定当前会话');
    binding = result.result;
    return { ...binding, inserted: adapter.status(), warnings };
  };
  chrome.runtime.onMessage.addListener((message, sender, respond) => {
    // The webpage has no listener/bridge. Only our own extension worker can
    // ask for composer operations; Chrome supplies sender identity.
    if (sender.id !== chrome.runtime.id || sender.tab || (sender.url && !sender.url.startsWith(chrome.runtime.getURL('')))) return false;
    const run = async () => {
      checkNavigation();
      if (message?.kind === 'describe') return bind();
      if (message?.kind === 'reset') { warnings = adapter.reset(); token = crypto.randomUUID(); binding = null; return bind(); }
      if (!binding || message.nonce !== binding.nonce || message.session_ref !== binding.session_ref || message.route !== route) throw new Error('页面或会话已变化；请重新打开扩展');
      if (message.kind === 'check') return { status: 'current' };
      if (message.kind === 'insert') {
        if (message.capsule?.nonce !== binding.nonce || message.capsule?.session_ref !== binding.session_ref) throw new Error('上下文来自其他会话');
        return adapter.insert(message.capsule);
      }
      if (message.kind === 'remove') return adapter.remove(message.id);
      if (message.kind === 'delivered') return adapter.confirmSent(message.id);
      throw new Error('不支持的输入框操作');
    };
    run().then((result) => respond({ ok: true, result }), (error) => respond({ ok: false, error: error.message }));
    return true;
  });
  // Only the URL is checked; no assistant messages, transcript, network
  // traffic, hidden state, cookies, or output tokens are observed.
  setInterval(checkNavigation, 500);
  window.addEventListener('popstate', checkNavigation);
  window.addEventListener('pagehide', () => { warnings = adapter.reset(); binding = null; token = crypto.randomUUID(); });
})();
