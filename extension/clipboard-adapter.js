/* No background clipboard reads, permission requests, or global paste listener. */
(() => {
  'use strict';
  globalThis.RecallCardClipboard = Object.freeze({
    capability(doc, navigator) {
      return { write: doc.visibilityState !== 'hidden' && typeof navigator?.clipboard?.writeText === 'function', read: 'paste_only', monitoring: false, permission_requested: false };
    },
    async copy(text, { doc, navigator, userGesture }) {
      if (!userGesture || doc.visibilityState === 'hidden' || !doc.hasFocus()) throw new Error('请回到当前标签，点击“复制本轮资料”');
      if (typeof text !== 'string' || !text || new TextEncoder().encode(text).length > 66 * 1024) throw new Error('没有可复制的完整本轮资料');
      if (typeof navigator?.clipboard?.writeText !== 'function') throw new Error('浏览器没有提供剪贴板写入；请选择完整资料手动复制');
      await navigator.clipboard.writeText(text);
      return { status: 'copied', read: false, monitoring: false };
    },
  });
})();
