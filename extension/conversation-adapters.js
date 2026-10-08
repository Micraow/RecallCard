/* Only visible DOM after the extension's explicit capture action. */
(() => {
  'use strict';
  const SKIP = 'script, style, template, noscript, button, [role="button"], textarea, input, select, [contenteditable="true"], [hidden], [aria-hidden="true"], .ds-think-content, [class*="ds-think"], [data-testid*="reasoning"], [data-message-author-role="system"], [data-message-author-role="tool"]';
  const BLOCK = new Set(['P', 'DIV', 'SECTION', 'ARTICLE', 'PRE', 'LI', 'UL', 'OL', 'BLOCKQUOTE', 'H1', 'H2', 'H3', 'H4', 'TABLE', 'TR']);
  const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u;
  const SELECTORS = { chatgpt: '[data-message-author-role="user"], [data-message-author-role="assistant"]', deepseek: '.ds-message' };
  function visible(node, doc) {
    if (!node?.isConnected || !node.getClientRects().length || node.closest('[hidden], [aria-hidden="true"]')) return false;
    for (let current = node; current; current = current.parentElement) {
      const style = doc.defaultView.getComputedStyle(current);
      if (style.display === 'none' || ['hidden', 'collapse'].includes(style.visibility) || style.opacity === '0') return false;
    }
    return true;
  }
  function plainText(root, doc) {
    let text = '';
    const walk = node => {
      if (node.nodeType === 3) { text += node.nodeValue || ''; return; }
      if (node.nodeType !== 1 || node.matches(SKIP) || !visible(node, doc)) return;
      if (node.tagName === 'BR') { text += '\n'; return; }
      if (node.tagName === 'PRE') {
        const code = node.querySelector('code');
        if (code && visible(code, doc)) {
          const raw = code.textContent.trim();
          try {
            const protocol = JSON.parse(raw)?.protocol;
            const label = protocol === 'recallcard.action/1' ? 'recallcard-action' : protocol === 'recallcard.final/1' ? 'recallcard-final' : null;
            if (label) { text += `\n\`\`\`${label}\n${raw}\n\`\`\`\n`; return; }
          } catch { /* Incomplete rendered JSON is not a complete request. */ }
        }
      }
      const block = BLOCK.has(node.tagName);
      if (block && text && !text.endsWith('\n')) text += '\n';
      for (const child of node.childNodes) walk(child);
      if (block && text && !text.endsWith('\n')) text += '\n';
    };
    walk(root);
    return text.replace(/\r\n?/gu, '\n').trim();
  }
  class ConversationAdapter {
    constructor(doc, location = doc.location) { this.doc = doc; this.location = location; }
    site() { return globalThis.RecallCardSites.forUrl(this.location.href); }
    selector() { return SELECTORS[this.site().id] || null; }
    nodes() {
      const selector = this.selector();
      if (!selector) return [];
      const nodes = [...this.doc.querySelectorAll(selector)].filter(node => visible(node, this.doc));
      return nodes.filter(node => !nodes.some(other => other !== node && other.contains(node)));
    }
    streaming() {
      return [...this.doc.querySelectorAll('[data-is-streaming="true"], [aria-busy="true"], button[aria-label="Stop generating"], button[aria-label="停止生成"], button[aria-label="停止回答"], .ds-markdown + .cursor, .cursor.streaming')].some(node => visible(node, this.doc));
    }
    completed(node) {
      if (!node || this.streaming() || node.getAttribute('data-is-streaming') === 'true' || node.getAttribute('aria-busy') === 'true') return false;
      // Silence is not completion. Only narrowly recognized per-turn receipts
      // are accepted; changed layouts use explicit pasted replies instead.
      if (node.getAttribute('data-is-streaming') === 'false') return true;
      const site = this.site();
      const scope = site.id === 'chatgpt' ? node.closest('[data-testid^="conversation-turn-"]') || node : node;
      const selector = site.id === 'chatgpt' ? 'button[data-testid="copy-turn-action-button"]' : '.ds-icon-button[aria-label="复制"], button[data-testid="copy-message"]';
      return [...scope.querySelectorAll(selector)].some(button => visible(button, this.doc));
    }
    read(captureId, capturedAt) {
      const site = this.site();
      if (!SELECTORS[site.id]) throw new Error(`${site.name} 暂未验证会话读取。仍可使用高级资料与草稿功能；请用网站官方导出或手动复制会话。`);
      if (this.streaming()) throw new Error('页面仍在生成回答，请等待完成后重新读取；没有保存未完成输出');
      const nodes = this.nodes();
      if (nodes.length > 5000) throw new Error('可见消息超过 5000 条，已停止；没有截断内容');
      const warnings = ['仅当前网页已加载且可见的普通文字；未自动滚动，不包含未挂载历史、隐藏内容、推理过程、图片或附件原件。', '没有可验证的消息时间时记为未知；捕获时间只代表本次读取时间。'];
      let unknownRoles = 0, skipped = 0;
      const usedIds = new Set();
      const messages = nodes.flatMap((node, index) => {
        let role = node.getAttribute('data-message-author-role') || node.getAttribute('data-role');
        if (!['user', 'assistant'].includes(role)) role = null;
        const markdown = site.id === 'deepseek' ? [...node.querySelectorAll('.ds-markdown')].filter(part => !part.closest('.ds-think-content, [class*="ds-think"]') && visible(part, this.doc)) : [];
        if (!role && markdown.length) role = 'assistant';
        // Hash class names and turn parity never establish a human role.
        const text = markdown.length && role === 'assistant' ? markdown.map(part => plainText(part, this.doc)).filter(Boolean).join('\n\n') : plainText(node, this.doc);
        if (!text) { skipped++; return []; }
        if (!role) unknownRoles++;
        let id = node.getAttribute('data-message-id');
        const metadata = {};
        if (!id || !/^[A-Za-z0-9_.:-]{1,192}$/u.test(id) || usedIds.has(id)) { id = `${captureId}:${index + 1}`; metadata.weaker_identity = true; }
        usedIds.add(id);
        const times = [...node.querySelectorAll('time[datetime]')].filter(time => visible(time, this.doc));
        const date = times.length === 1 ? times[0].getAttribute('datetime') : null;
        const occurred_at = date && ISO.test(date) && Number.isFinite(Date.parse(date)) ? date : null;
        return [{ id, role, text, occurred_at, ...(Object.keys(metadata).length ? { metadata } : {}) }];
      });
      if (skipped) warnings.push(`${skipped} 个消息容器没有可读取的普通文字，已跳过。`);
      if (unknownRoles) warnings.push(`${unknownRoles} 条消息没有可靠的角色标记，请在预览中逐条确认“我”或“AI”。`);
      if (messages.some(message => message.metadata?.weaker_identity)) warnings.push('部分消息没有稳定网站 ID，使用本次捕获编号与顺序；同一文件重导可去重，不承诺跨次捕获去重。');
      const path = new URL(site.route).pathname;
      const match = site.id === 'chatgpt' ? /\/c\/([A-Za-z0-9_-]+)$/u.exec(path) : /^\/a\/chat\/s\/([A-Za-z0-9_-]+)$/u.exec(path);
      const conversationId = match?.[1] || `capture:${captureId}`;
      if (!match) warnings.push('当前网址没有已验证的会话 ID，本次记录使用局部会话编号；无法认定它与其他捕获属于同一会话。');
      const snapshot = {
        schema: 'recallcard.conversation/1', capture_id: captureId, captured_at: capturedAt,
        title: [...(this.doc.title || '').replace(/[\u0000-\u001f\u007f]/gu, ' ').trim()].slice(0, 240).join('') || `${site.name} 会话`,
        source: { platform: site.id, conversation_id: conversationId, url: site.route },
        coverage: { extent: 'visible_only', complete: false, reason: '用户主动读取当前页面的可见文字片段；未验证全部历史是否已加载。', warnings },
        messages, metadata: { weaker_conversation_identity: !match, adapter: `${site.id}-visible-dom-v1` },
      };
      globalThis.RecallCardConversationFormat.checkSize(snapshot);
      return snapshot;
    }
  }
  globalThis.RecallCardConversationAdapter = ConversationAdapter;
})();
