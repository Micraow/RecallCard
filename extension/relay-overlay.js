/* A small extension-owned closed shadow panel, not a webpage message bridge. */
(() => {
  'use strict';
  const labels = { idle: '等待连接', preparing: '准备相关资料', awaiting_user_send: '待你发送', waiting_reply: '等待完整回复', result_ready: '回答已就绪', paused: '已暂停', blocked: '需要检查' };
  class RelayOverlay {
    constructor(doc, action) {
      this.doc = doc; this.action = action; this.state = null; this.busy = false;
      const host = doc.createElement('aside'); host.id = 'recallcard-relay';
      host.setAttribute('aria-label', 'RecallCard 接力');
      const root = host.attachShadow({ mode: 'closed' });
      const create = (tag, text, parent = root) => { const node = doc.createElement(tag); if (text) node.textContent = text; parent.append(node); return node; };
      const style = create('style'); style.textContent = `:host{position:fixed!important;right:16px!important;bottom:20px!important;z-index:2147483646!important;color:#172633!important;font:14px/1.5 system-ui,sans-serif!important}*{box-sizing:border-box}button,textarea{font:inherit}button{cursor:pointer;border:1px solid #c9d6db;border-radius:9px;padding:7px 10px;background:#fff;color:#123b48}button:disabled{opacity:.5;cursor:default}button:focus-visible,textarea:focus-visible{outline:3px solid #29a7b3;outline-offset:2px}.pill{background:#153d48;color:white;box-shadow:0 4px 18px #0003}.panel{width:min(340px,calc(100vw - 32px));max-height:70vh;overflow:auto;background:#fffdf8;border:1px solid #ccd6d6;box-shadow:0 8px 30px #0003;border-radius:14px;padding:14px;margin-bottom:8px}.row{display:flex;gap:7px;flex-wrap:wrap;margin-top:10px}.muted{font-size:12px;color:#53666f}p{margin:7px 0}textarea{width:100%;min-height:80px;border:1px solid #bccace;border-radius:8px;padding:8px;background:white;color:#172633;resize:vertical}[hidden]{display:none!important}strong{font-size:16px}summary{cursor:pointer;margin:9px 0}@media(prefers-color-scheme:dark){.panel{background:#16232a;color:#eaf2f3;border-color:#48606b}.muted{color:#afc3ca}button,textarea{background:#223741;color:#edf7f8;border-color:#48606b}}`;
      this.panel = create('section'); this.panel.className = 'panel';
      this.title = create('strong', 'RecallCard · 等待连接', this.panel);
      this.detail = create('p', '一次连接后，按你的问题准备资料。最终发送由你完成。', this.panel); this.detail.setAttribute('role', 'status'); this.detail.setAttribute('aria-live', 'polite');
      this.round = create('p', '当前会话 · 未开始', this.panel); this.round.className = 'muted';
      const row = create('div', '', this.panel); row.className = 'row';
      const button = (text, handler, parent = row) => {
        const node = create('button', text, parent); node.type = 'button';
        node.addEventListener('click', event => { if (event.isTrusted) void handler(event); }); return node;
      };
      this.bootstrapButton = button('准备使用说明', () => this.run(() => action('bootstrap')));
      this.textStartButton = button('开始简洁接力', () => this.run(async () => { this.startingText = true; this.cancelButton.hidden = false; try { await action('text_start'); } finally { this.startingText = false; } }));
      create('p', '简洁接力：开局背景 → 按需检索 → 正式回答提醒。仅移除未改过的旧资料块，其余草稿保留。', this.panel).className = 'muted';
      this.copyButton = button('复制本轮资料', event => this.run(async () => {
        const result = await action('copy'); this.full.value = result.text; this.full.hidden = false;
        try {
          await globalThis.RecallCardClipboard.copy(result.text, { doc, navigator: doc.defaultView.navigator, userGesture: event.isTrusted });
          this.notice('完整资料已复制。粘贴后请自己点击网页发送。');
        } catch { this.full.focus(); this.full.select(); this.notice('剪贴板不可用；已选中完整资料，请手动复制。'); }
      }));
      this.sentButton = button('我已发送', () => this.run(() => action('delivered')));
      this.pauseButton = button('暂停', () => { const command = !this.startingText && (this.state?.text_relay?.phase === 'paused' || !this.state?.text_relay && this.state?.relay?.paused) ? 'resume' : 'pause'; if (this.state?.text_relay || this.startingText) void action(this.startingText ? 'text_' + command : command).catch(error => this.error(error.message)); else void this.run(() => action(command)); });
      this.retryButton = button('重试本轮读取', () => this.run(() => action('retry')));
      this.cancelButton = button('结束本次简洁接力', () => { void action('text_cancel').catch(error => this.error(error.message)); });
      const preview = create('details', '', this.panel); create('summary', '本轮资料预览', preview);
      this.packetPreview = create('textarea', '', preview); this.packetPreview.readOnly = true; this.packetPreview.setAttribute('aria-label', '简洁接力本轮资料预览'); this.previewSection = preview;
      this.finalOutput = create('textarea', '', this.panel); this.finalOutput.readOnly = true; this.finalOutput.hidden = true; this.finalOutput.setAttribute('aria-label', 'AI 正式回答，内容仍需核对');
      this.full = create('textarea', '', this.panel); this.full.readOnly = true; this.full.hidden = true; this.full.setAttribute('aria-label', '完整本轮资料，可手动复制');
      const fallback = create('details', '', this.panel); create('summary', '网页改版或手动接力', fallback);
      create('p', '复制 AI 的完整请求并粘贴在这里。不会后台读取剪贴板，也不会申请持续监听权限。', fallback).className = 'muted';
      this.input = create('textarea', '', fallback); this.input.setAttribute('aria-label', '粘贴完整 AI 请求或回答'); this.input.placeholder = '粘贴 AI 整条回复；简洁接力以 RECALLCARD/2 开头';
      this.executeButton = button('处理已粘贴内容', () => this.run(async () => { const text = this.input.value; await action('execute', { text }); this.input.value = ''; }), fallback);
      const help = create('details', '', this.panel); create('summary', '能力与隐私边界', help);
      create('p', 'ChatGPT / DeepSeek：按已授权范围读取完成的可见文字；简洁接力由你复制和发送，本轮不并行自动采集对话。旧模式资料只放入未变化的输入框。无法确定完成或 DOM 变化时请手动复制粘贴。Qwen（仅 chat.qwen.ai）/ Z.ai：实验性输入框与手动接力；Qwen Studio 站点未确认。真实账号与页面改版尚未验收。', help).className = 'muted';
      create('p', '剪贴板：仅你点击时写入；读取只通过你主动粘贴。网站内隐藏历史、附件和推理过程不读取。账号身份未核验。', help).className = 'muted';
      const controls = create('div', '', this.panel); controls.className = 'row';
      this.disableButton = button('关闭接力', () => this.run(async () => { await action('disable'); this.panel.hidden = true; }), controls);
      this.pill = button('RecallCard · 接力', () => this.run(async () => {
        if (this.state?.relay?.enabled === false) await action('enable');
        this.panel.hidden = !this.panel.hidden;
      }), root); this.pill.className = 'pill';
      this.retryButton.hidden = true; this.cancelButton.hidden = true; this.previewSection.hidden = true;
      doc.body.append(host); this.host = host;
    }
    notice(text) { this.detail.textContent = text; }
    error(text) { this.notice(text); }
    reset() { this.state = null; this.full.value = ''; this.full.hidden = true; this.input.value = ''; this.packetPreview.value = ''; this.finalOutput.value = ''; this.finalOutput.hidden = true; this.title.textContent = 'RecallCard · 会话已变化'; this.notice('正在重新确认当前对话；旧资料不会重放。'); }
    update(state) {
      if (this.state?.nonce && this.state.nonce !== state.nonce) this.reset();
      this.state = state;
      const textRelay = state.text_relay;
      const relay = textRelay ? { ...textRelay, request_id: textRelay.packet?.id, paused: textRelay.phase === 'paused', enabled: true } : state.relay || { phase: 'idle', round: 0 };
      this.textStartButton.hidden = !!textRelay;
      this.bootstrapButton.hidden = !!textRelay;
      this.retryButton.hidden = !textRelay?.canRetry;
      this.cancelButton.hidden = !textRelay && !this.startingText || !!textRelay?.cancelled || !!textRelay?.completed;
      this.previewSection.hidden = !textRelay?.packet;
      this.packetPreview.value = textRelay?.packet?.text || '';
      this.finalOutput.hidden = !textRelay?.final;
      this.finalOutput.value = textRelay?.final?.body || '';
      if (textRelay?.completed) { this.full.hidden = true; this.full.value = ''; }
      this.disableButton.hidden = !!textRelay;
      this.title.textContent = `RecallCard · ${labels[relay.phase] || '等待连接'}`;
      this.pill.textContent = relay.enabled === false ? 'RecallCard · 开启接力' : `RecallCard · ${labels[relay.phase] || '接力'}`;
      this.round.textContent = `${state.platform_name || '当前网站'} · 第 ${relay.round || 0} 轮 · 发送由你完成` + (textRelay ? ` · 读取 ${textRelay.budget?.reads || 0}/${textRelay.limits?.reads || 0} · 资料预算 ${textRelay.budget?.disclosureBytes || 0} 字节` : '');
      this.notice((relay.detail || '等待确认本机连接与授权') + (textRelay?.cancelled || textRelay?.completed ? '。如需重新开始，请在扩展弹窗重新连接当前对话。' : ''));
      this.pauseButton.textContent = relay.paused ? '恢复' : '暂停';
      this.pauseButton.disabled = !!textRelay?.cancelled || !!textRelay?.completed;
      this.bootstrapButton.disabled = !!relay.request_id || relay.paused;
      this.copyButton.disabled = !relay.request_id || relay.paused || !!textRelay?.completed || !!textRelay?.cancelled;
      this.sentButton.disabled = relay.phase !== 'awaiting_user_send';
      this.executeButton.disabled = relay.paused || !!textRelay?.completed || !!textRelay?.cancelled;
      if (relay.enabled === false) this.panel.hidden = true;
      if (relay.request_id !== this.lastRequest) { this.full.value = ''; this.full.hidden = true; this.lastRequest = relay.request_id; }
    }
    async run(work) {
      if (this.busy) return;
      this.busy = true;
      try { await work(); } catch (error) { this.error(error.message); }
      finally { this.busy = false; }
    }
  }
  globalThis.RecallCardRelayOverlay = RelayOverlay;
})();
