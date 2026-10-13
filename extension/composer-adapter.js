/* Standalone, dependency-free composer adapter. No conversation/output selectors. */
(() => {
  'use strict';
  class ComposerAdapter {
    constructor(doc, location = doc.location) { this.doc = doc; this.location = location; this.receipts = new Map(); this.verifying = false; this.verificationFailed = false; this.verificationWriteStarted = false; }
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
      const originalNodes = node.tagName === 'TEXTAREA' ? [] : Array.from(node.childNodes || node.children).map(child => ({ node: child, markup: child.outerHTML ?? child.textContent }));
      if (before.includes('[RecallCard ') || before.includes('[/RecallCard ')) throw new Error('草稿中已有失去归属或已编辑的上下文，请先检查；不会重复追加');
      const marker = `${capsule.nonce}:${capsule.id}`;
      const block = `\n\n[RecallCard ${marker}]\n${capsule.text}\n[/RecallCard ${marker}]`;
      let insertedNode = null;
      if (this.verifying) this.verificationWriteStarted = true;
      if (node.tagName === 'TEXTAREA') this.setTextarea(node, before + block);
      else {
        insertedNode = this.doc.createElement('p');
        insertedNode.style.whiteSpace = 'pre-wrap';
        insertedNode.textContent = block;
        node.append(insertedNode);
      }
      // Save ownership before informing the editor, so even a re-render cannot
      // cause a second insertion to be mistaken for the first.
      this.receipts.set(capsule.id, { node, block, before, expected: before + block, originalNodes, renderedExpected: node.tagName === 'TEXTAREA' ? null : node.innerText, insertedNode, marker, markup: insertedNode?.innerHTML });
      this.notify(node);
      if (!this.text(node).includes(block)) throw new Error('网页编辑器改变了插入内容；请人工检查草稿，不会再次自动插入');
      return { status: 'draft', id: capsule.id };
    }
    unobscured(node) {
      const view = this.doc.defaultView;
      if (this.doc.visibilityState === 'hidden' || typeof this.doc.elementFromPoint !== 'function' || typeof node.getBoundingClientRect !== 'function') throw new Error('无法核验当前可见输入框；请手动复制预览');
      const r = node.getBoundingClientRect();
      const left = Math.max(0, r.left), right = Math.min(view.innerWidth, r.right);
      const top = Math.max(0, r.top), bottom = Math.min(view.innerHeight, r.bottom);
      if (!(right > left && bottom > top)) throw new Error('输入框不在可见区域；请手动复制预览');
      const hit = this.doc.elementFromPoint((left + right) / 2, (top + bottom) / 2);
      if (hit !== node && !node.contains?.(hit)) throw new Error('输入框被其他内容遮挡；请手动复制预览');
    }
    editorFlush() {
      const view = this.doc.defaultView;
      return new Promise((resolve, reject) => {
        if (typeof view.requestAnimationFrame !== 'function' || typeof view.setTimeout !== 'function') { reject(new Error('网页编辑器无法完成绘制核验；请手动复制')); return; }
        let done = false, first = null, second = null, quiet = null;
        const finish = error => {
          if (done) return;
          done = true; view.clearTimeout(timeout); view.clearTimeout(quiet);
          if (first !== null) view.cancelAnimationFrame?.(first);
          if (second !== null) view.cancelAnimationFrame?.(second);
          if (error) reject(error); else resolve();
        };
        const timeout = view.setTimeout(() => finish(new Error('网页编辑器核验超时；请检查草稿并手动复制')), 1500);
        first = view.requestAnimationFrame(() => { second = view.requestAnimationFrame(() => {
          quiet = view.setTimeout(() => finish(), 50);
        }); });
      });
    }
    // 只确认完整规范化文本；不使用前缀/关键词命中，也不回滚或再次覆盖用户草稿。
    async insertVerified(capsule, isCurrent = () => true) {
      if (this.verifying) throw new Error('正在核验上一次填框，不会重复追加');
      if (this.verificationFailed) throw new Error('此前填框无法确认；请人工检查草稿并手动复制，刷新页面前不会再自动写入');
      if (!isCurrent()) throw new Error('当前会话已变化，请手动复制预览');
      const node = this.find();
      this.unobscured(node);
      const url = this.location?.href;
      let receipt = null;
      this.verifying = true;
      this.verificationWriteStarted = false;
      try {
        this.insert(capsule);
        receipt = this.receipts.get(capsule.id);
        await this.editorFlush();
        if (!isCurrent() || this.location?.href !== url || !node.isConnected || this.find() !== node) throw new Error('填框期间会话或输入框已变化；请人工检查草稿并手动复制');
        this.unobscured(node);
        const normalize = text => text.replace(/\r\n?/gu, '\n');
        if (!receipt || normalize(this.text(node)) !== normalize(receipt.expected)) throw new Error('完整草稿读回不一致；可能被编辑器截断或改写，请人工检查，不会重试覆盖');
        if (node.tagName !== 'TEXTAREA') {
          if (typeof receipt.renderedExpected !== 'string' || normalize(node.innerText) !== normalize(receipt.renderedExpected)) throw new Error('富文本可见全文或换行已变化；请人工检查草稿并手动复制');
          const children = Array.from(node.childNodes || node.children);
          if (receipt.originalNodes.some((original, index) => children[index] !== original.node || (original.node.outerHTML ?? original.node.textContent) !== original.markup)) throw new Error('原有富文本草稿结构已变化；请人工检查，不会覆盖还原');
        }
        return { status: 'draft', id: capsule.id, full_text_verified: true, existing_draft_preserved: true };
      } catch (error) {
        // 写入后结果不明，不让 SPA 重建/自动重试绕过失败。只有页面重新加载才重置。
        if (this.verificationWriteStarted) this.verificationFailed = true;
        throw error;
      } finally { this.verifying = false; }
    }
    // content 的最终会话检查也受同一保护；检查导致的 reset 不能撤回草稿。
    confirmVerifiedContext(isCurrent) {
      if (this.verifying) throw new Error('其他填框仍在核验，不能确认旧结果');
      this.verifying = true;
      try {
        if (!isCurrent()) throw new Error('核验期间会话已变化；请人工检查草稿，不会重复填框');
      } catch (error) { this.verificationFailed = true; throw error; }
      finally { this.verifying = false; }
    }
    remove(id) {
      if (this.verifying) throw new Error('仍在核验填框，不会并发修改草稿；请稍后人工检查');
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
      if (this.verifying) throw new Error('仍在核验草稿，不能提前确认发送');
      const receipt = this.receipts.get(id);
      if (receipt?.node.isConnected && (this.text(receipt.node).includes(`[RecallCard ${receipt.marker}]`) || this.text(receipt.node).includes(`[/RecallCard ${receipt.marker}]`))) throw new Error('上下文仍在草稿里，请先自己点击网页发送');
      this.receipts.delete(id);
      return { status: 'user_confirmed_sent', id };
    }
    reset() {
      if (this.verifying || this.verificationFailed) return ['填框结果尚未可靠确认，已停止自动修改；请人工检查草稿并手动复制'];
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
