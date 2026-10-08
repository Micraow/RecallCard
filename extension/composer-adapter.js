/* Standalone, dependency-free composer adapter. No conversation/output selectors. */
(() => {
  'use strict';
  class ComposerAdapter {
    constructor(doc, location = doc.location) { this.doc = doc; this.location = location; this.receipts = new Map(); }
    find() {
      const site = globalThis.RecallCardSites.forUrl(this.location?.href);
      const nodes = [...this.doc.querySelectorAll(site.selector)].filter((node) => {
        const style = this.doc.defaultView.getComputedStyle?.(node);
        return node.isConnected && node.getClientRects().length > 0 && style?.visibility !== 'hidden' && style?.visibility !== 'collapse' && style?.display !== 'none' && style?.opacity !== '0';
      });
      if (nodes.length !== 1) throw new Error('没有找到唯一可用的输入框；请从预览手动复制，扩展已安全停止');
      const node = nodes[0];
      if (node.disabled || node.readOnly || node.getAttribute('aria-disabled') === 'true' || node.getAttribute('aria-readonly') === 'true') throw new Error('输入框暂不可编辑');
      if (node.tagName !== 'TEXTAREA' && !(site.richText && node.isContentEditable && node.classList.contains('ProseMirror'))) throw new Error('输入框结构未受支持；请手动复制预览');
      return node;
    }
    text(node) { return node.tagName === 'TEXTAREA' ? node.value : node.textContent; }
    notify(node) {
      const EventClass = this.doc.defaultView.InputEvent || this.doc.defaultView.Event;
      node.dispatchEvent(new EventClass('input', { bubbles: true, inputType: 'insertText' }));
    }
    setTextarea(node, value) {
      const prototype = this.doc.defaultView.HTMLTextAreaElement?.prototype;
      const setter = prototype && Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
      if (setter) setter.call(node, value); else node.value = value;
    }
    prune() {
      for (const [id, receipt] of this.receipts) if (!receipt.node.isConnected) this.receipts.delete(id);
    }
    insert(capsule) {
      this.prune();
      if (this.receipts.has(capsule.id)) throw new Error('这个结果已经插入；不会重复追加');
      if (this.receipts.size) throw new Error('请先移除之前的上下文，或在手动发送后确认已发送');
      const node = this.find();
      if (!/^[A-Za-z0-9_-]{1,96}$/u.test(capsule.id) || !/^[a-f0-9]{48}$/u.test(capsule.nonce) || typeof capsule.text !== 'string' || capsule.text.length > 65536) throw new Error('上下文标识或大小无效');
      const before = this.text(node);
      if (before.includes('[RecallCard ') || before.includes('[/RecallCard ')) throw new Error('草稿中已有失去归属或已编辑的上下文，请先检查；不会重复追加');
      const marker = `${capsule.nonce}:${capsule.id}`;
      const block = `\n\n[RecallCard ${marker}]\n${capsule.text}\n[/RecallCard ${marker}]`;
      let insertedNode = null;
      if (node.tagName === 'TEXTAREA') this.setTextarea(node, before + block);
      else {
        insertedNode = this.doc.createElement('p');
        insertedNode.style.whiteSpace = 'pre-wrap';
        insertedNode.textContent = block;
        node.append(insertedNode);
      }
      // Save ownership before informing the editor, so even a re-render cannot
      // cause a second insertion to be mistaken for the first.
      this.receipts.set(capsule.id, { node, block, insertedNode, marker, markup: insertedNode?.innerHTML });
      this.notify(node);
      if (!this.text(node).includes(block)) throw new Error('网页编辑器改变了插入内容；请人工检查草稿，不会再次自动插入');
      return { status: 'draft', id: capsule.id };
    }
    remove(id) {
      const receipt = this.receipts.get(id);
      if (!receipt) throw new Error('没有可安全移除的已插入上下文');
      const { node, block, insertedNode, markup } = receipt;
      if (!node.isConnected) { this.receipts.delete(id); return { status: 'detached' }; }
      const text = this.text(node);
      const start = text.indexOf(block);
      if (start < 0 || text.indexOf(block, start + 1) >= 0) throw new Error('上下文已被编辑或复制；为保护草稿，请手动移除');
      if (node.tagName === 'TEXTAREA') this.setTextarea(node, text.slice(0, start) + text.slice(start + block.length));
      else if (insertedNode?.parentNode === node && insertedNode.textContent === block) {
        if (insertedNode.innerHTML !== markup) throw new Error('上下文的富文本结构已变化，请手动移除');
        insertedNode.remove();
      }
      else {
        // Preserve surrounding rich-text nodes. Do not replace innerHTML or
        // rebuild the composer when the editor has normalized our paragraph.
        const walker = this.doc.createTreeWalker(node, this.doc.defaultView.NodeFilter.SHOW_TEXT);
        let cursor = 0, first = null, last = null, current;
        while ((current = walker.nextNode())) {
          const end = cursor + current.textContent.length;
          if (!first && start >= cursor && start < end) first = [current, start - cursor];
          if (start + block.length > cursor && start + block.length <= end) { last = [current, start + block.length - cursor]; break; }
          cursor = end;
        }
        if (!first || !last) throw new Error('输入框结构已变化，请手动移除上下文');
        // Range 从同一个格式节点内开始/结束时，cloneContents 可能只有文字。
        // 同时检查边界祖先，避免把用户新增的粗体、链接或样式当成纯文本归一化。
        for (const boundary of [first[0], last[0]]) {
          let parent = boundary.parentNode;
          while (parent && parent !== node) {
            const attributes = Array.from(parent.attributes || []);
            const style = parent.getAttribute?.('style');
            if (!['P', 'DIV'].includes(parent.tagName) || attributes.some(attribute => attribute.name !== 'style') ||
                (style && !/^\s*white-space\s*:\s*pre-wrap\s*;?\s*$/iu.test(style))) {
              throw new Error('上下文的富文本结构已变化，请手动移除');
            }
            parent = parent.parentNode;
          }
          if (parent !== node) throw new Error('输入框结构已变化，请手动移除上下文');
        }
        const range = this.doc.createRange();
        range.setStart(...first); range.setEnd(...last);
        if (range.cloneContents().querySelector('*')) throw new Error('上下文中包含新增或重排的富文本节点，请手动移除');
        range.deleteContents();
      }
      this.receipts.delete(id);
      this.notify(node);
      return { status: 'removed', id };
    }
    confirmSent(id) {
      const receipt = this.receipts.get(id);
      if (receipt?.node.isConnected && (this.text(receipt.node).includes(`[RecallCard ${receipt.marker}]`) || this.text(receipt.node).includes(`[/RecallCard ${receipt.marker}]`))) throw new Error('上下文仍在草稿里，请先自己点击网页发送');
      this.receipts.delete(id);
      return { status: 'user_confirmed_sent', id };
    }
    reset() {
      this.prune();
      const warnings = [];
      for (const [id, receipt] of this.receipts) {
        const text = this.text(receipt.node);
        if (!text.includes(`[RecallCard ${receipt.marker}]`) && !text.includes(`[/RecallCard ${receipt.marker}]`)) {
          this.receipts.delete(id); // Gone is not evidence of delivery.
          continue;
        }
        try { this.remove(id); } catch (error) { warnings.push(error.message); }
      }
      return warnings;
    }
    status() { this.prune(); return [...this.receipts.keys()]; }
    draftIds() {
      this.prune();
      for (const [id, receipt] of this.receipts) {
        const text = this.text(receipt.node);
        if (!text.includes(`[RecallCard ${receipt.marker}]`) && !text.includes(`[/RecallCard ${receipt.marker}]`)) this.receipts.delete(id);
      }
      return [...this.receipts.keys()];
    }
  }
  globalThis.RecallCardComposerAdapter = ComposerAdapter;
})();
