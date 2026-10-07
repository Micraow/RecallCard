(() => {
  'use strict';
  if (window.top !== window) return;
  const adapter = new globalThis.RecallCardComposerAdapter(document, location);
  const currentRoute = () => {
    try { return globalThis.RecallCardSites.forUrl(location.href).route; } catch { return null; }
  };
  const currentComposer = () => { try { return adapter.find(); } catch { return null; } };
  let route = currentRoute();
  let composer = currentComposer();
  let token = crypto.randomUUID();
  let binding = null;
  let manualConfirmed = false;
  let warnings = [];
  const checkNavigation = () => {
    const current = currentRoute();
    const editor = currentComposer();
    if (current !== route || editor !== composer) {
      warnings = adapter.reset();
      route = current; composer = editor; token = crypto.randomUUID(); binding = null; manualConfirmed = false;
    }
  };
  const bind = async (manual = false) => {
    checkNavigation();
    if (!route || !composer) throw new Error('当前网站、路径或输入框不可用；请返回受支持的对话并重新检查');
    if (globalThis.RecallCardSites.forUrl(location.href).id !== 'chatgpt' && !manualConfirmed && !manual) throw new Error('实验适配需要你确认当前对话：请先点击“重置 / 重新附上说明”；切换对话后也必须重置');
    if (manual) manualConfirmed = true;
    const selected = { route, token, composer };
    const result = await chrome.runtime.sendMessage({ kind: 'bind', route, token });
    checkNavigation();
    if (route !== selected.route || token !== selected.token || composer !== selected.composer) throw new Error('绑定期间页面或输入框已变化，请重新确认');
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
      if (message?.kind === 'reset') { warnings = adapter.reset(); token = crypto.randomUUID(); binding = null; manualConfirmed = false; return bind(true); }
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
  // Only URL and the selected composer identity are checked; no messages, transcript, network
  // traffic, hidden state, cookies, or output tokens are observed.
  setInterval(checkNavigation, 500);
  window.addEventListener('popstate', checkNavigation);
  window.addEventListener('pagehide', () => { warnings = adapter.reset(); binding = null; manualConfirmed = false; token = crypto.randomUUID(); });
})();
