(() => {
  'use strict';
  if (window.top !== window) return;
  const adapter = new globalThis.RecallCardComposerAdapter(document, location);
  const conversation = globalThis.RecallCardConversationAdapter ? new globalThis.RecallCardConversationAdapter(document, location) : null;
  const format = globalThis.RecallCardConversationFormat;
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
  let capture = null;
  let messageNodes = [];
  try { messageNodes = conversation?.nodes() || []; } catch { /* unsupported path */ }
  let changedMessages = false;
  const invalidate = () => {
    warnings = adapter.reset();
    token = crypto.randomUUID(); binding = null; manualConfirmed = false; capture = null;
  };
  const checkNavigation = () => {
    const current = currentRoute();
    const editor = currentComposer();
    let nodes = [];
    try { nodes = conversation?.nodes() || []; } catch { /* unsupported navigation */ }
    if (current !== route || editor !== composer || changedMessages || nodes.length !== messageNodes.length || nodes.some((node, index) => node !== messageNodes[index])) {
      invalidate();
      route = current; composer = editor;
    }
    messageNodes = nodes; changedMessages = false;
  };
  const bind = async (manual = false) => {
    checkNavigation();
    if (!route) throw new Error('当前网站或路径不可用；请返回受支持的对话并重新检查');
    if (!['chatgpt', 'deepseek'].includes(globalThis.RecallCardSites.forUrl(location.href).id) && !manualConfirmed && !manual) throw new Error('实验适配需要你确认当前对话：请先点击“重新连接当前对话”重置；切换对话后也必须重置');
    if (manual) manualConfirmed = true;
    const selected = { route, token, composer };
    const result = await chrome.runtime.sendMessage({ kind: 'bind', route, token });
    checkNavigation();
    if (route !== selected.route || token !== selected.token || composer !== selected.composer) throw new Error('绑定期间页面或输入框已变化，请重新确认');
    if (!result?.ok) throw new Error(result?.error || '无法绑定当前会话');
    binding = result.result;
    return { ...binding, inserted: adapter.status(), warnings, composer_available: !!composer };
  };
  chrome.runtime.onMessage.addListener((message, sender, respond) => {
    // The webpage has no listener/bridge. Only our own extension worker can
    // ask for composer operations; Chrome supplies sender identity.
    if (sender.id !== chrome.runtime.id || sender.tab || (sender.url && !sender.url.startsWith(chrome.runtime.getURL('')))) return false;
    const run = async () => {
      checkNavigation();
      if (message?.kind === 'describe') return bind();
      if (message?.kind === 'reset') { invalidate(); return bind(true); }
      if (!binding || message.nonce !== binding.nonce || message.session_ref !== binding.session_ref || message.route !== route) throw new Error('页面或会话已变化；请重新打开扩展');
      if (message.kind === 'check') return { status: 'current' };
      if (message.kind === 'capture') {
        if (!conversation) throw new Error('请重新加载扩展与当前网页');
        capture = null;
        const selectedToken = token;
        const raw = conversation.read(crypto.randomUUID(), new Date().toISOString());
        const rawHash = await format.hash(raw);
        const snapshot = await format.seal(raw);
        checkNavigation();
        if (token !== selectedToken) throw new Error('读取期间会话已变化，请等待页面稳定后重新确认');
        capture = { raw, rawHash, snapshot };
        return snapshot;
      }
      if (message.kind === 'capture_select' || message.kind === 'capture_check') {
        if (!capture || capture.snapshot.capture_id !== message.capture_id || capture.snapshot.metadata.snapshot_hash !== message.snapshot_hash) throw new Error('会话预览已过期，请重新读取并确认');
        const receipt = capture, selectedToken = token;
        const current = conversation.read(receipt.raw.capture_id, receipt.raw.captured_at);
        if (await format.hash(current) !== receipt.rawHash) { invalidate(); throw new Error('可见消息内容或身份已变化，旧预览已废弃，请重新读取并确认'); }
        const result = message.kind === 'capture_select' ? await format.select(receipt.snapshot, message.selections) : { status: 'current' };
        checkNavigation();
        if (token !== selectedToken || capture !== receipt) throw new Error('核验期间会话已变化，请重新读取并确认');
        return result;
      }
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
  // Observe identity/mutation only. Message text is read solely after capture,
  // and for revalidation of that same user-selected snapshot.
  if (typeof MutationObserver !== 'undefined') new MutationObserver(records => {
    let selector;
    try { selector = conversation?.selector(); } catch { return; }
    if (selector && records.some(record => {
      const node = record.target.nodeType === 1 ? record.target : record.target.parentElement;
      return node?.closest(selector);
    })) changedMessages = true;
  }).observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true });
  setInterval(checkNavigation, 500);
  window.addEventListener('popstate', checkNavigation);
  window.addEventListener('pagehide', invalidate);
})();
