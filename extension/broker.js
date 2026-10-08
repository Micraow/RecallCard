import { automaticCycle, prepareAutomaticRequest } from './automatic.js';
import { parseAction, validateAction, routeFor, siteFor, newNonce, reserveRequest, makeCapsule, byteLength, fail, PROTOCOL } from './protocol.js';
import './conversation-format.js';
import { relayState, setRelay, capsuleText, finalReceipt } from './relay-state.js';
const conversationFormat = globalThis.RecallCardConversationFormat;
export const HOST = 'com.recallcard.host';
export function verifyPopup(sender, extensionId, popupUrl) {
  if (sender.id !== extensionId || sender.tab || sender.url !== popupUrl) fail('仅允许扩展自己的弹窗发起操作');
}
export function verifyContent(sender, extensionId, liveUrl, requestedRoute) {
  if (sender.id !== extensionId || !Number.isInteger(sender.tab?.id) || sender.frameId !== 0 || !sender.documentId) fail('拒绝未知扩展、子框架或缺少文档身份的消息');
  const route = routeFor(liveUrl);
  if (sender.origin !== siteFor(liveUrl).origin) fail('消息来源与当前网站不匹配');
  if (routeFor(sender.url) !== route || routeFor(sender.tab.url) !== route || requestedRoute !== route) fail('页面地址或标签身份已变化');
  return route;
}
export function bindSession(previous, { route, token, documentId, installationId = null }) {
  if (typeof token !== 'string' || !/^[a-f0-9-]{36}$/u.test(token)) fail('文档会话标识无效');
  const platform = siteFor(route);
  if (previous?.route === route && previous.token === token && previous.documentId === documentId && (previous.installation_id || null) === installationId) return previous;
  return { route, token, documentId, installation_id: installationId, nonce: newNonce(), session_ref: `${platform.id}:${token}`, used: [], preview: null, bootstrap: null, last_request_at: 0 };
}
export function publicState(state) {
  const preview = state.preview && Object.fromEntries(Object.entries(state.preview).filter(([key]) => !['source_request', 'result_fingerprint'].includes(key)));
  return { installation_id: state.installation_id || null, extension_id: state.extension_id || null, route: state.route, platform: siteFor(state.route).id, platform_name: siteFor(state.route).name, capabilities: siteFor(state.route).capabilities, nonce: state.nonce, session_ref: state.session_ref, preview, connection: state.connection || null, capture_confirmation_valid: !!state.capture_approval, last_capture_at: state.last_capture_at || null, last_read_at: state.last_read_at || null, auto_capture_note: state.auto_capture_note || null, automatic_status: state.automatic_status || null, automatic_error: state.automatic_error || null, relay: relayState(state) };
}
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
  return value;
}
export async function resultFingerprint(action, result) {
  let selected = result;
  if (action === 'bootstrap') {
    if (!result || typeof result.bootstrap_version !== 'string' || !result.bootstrap_version) fail('Bootstrap 缺少可核验的版本，不能使用缓存');
    // Coverage is a dynamic observation; the stable snapshot includes the
    // version, text, refs and all other fields that will be disclosed.
    const { coverage: _coverage, ...stable } = result;
    selected = stable;
  }
  const bytes = new TextEncoder().encode(JSON.stringify(canonical(selected)));
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (n) => n.toString(16).padStart(2, '0')).join('');
}
export class Broker {
  constructor(api) { this.api = api; this.locks = new Map(); }
  async serial(tabId, operation) {
    const previous = this.locks.get(tabId) || Promise.resolve();
    const next = previous.catch(() => {}).then(operation);
    this.locks.set(tabId, next);
    try { return await next; } finally { if (this.locks.get(tabId) === next) this.locks.delete(tabId); }
  }
  async current(tabId, message) {
    if (!Number.isInteger(tabId)) fail('标签页编号无效');
    const tab = await this.api.getTab(tabId);
    const state = await this.api.load(tabId);
    if (this.api.installationId && (!state?.installation_id || state.installation_id !== await this.api.installationId())) fail('安装绑定已变化，请重新连接；旧请求不会继续');
    if (!state || routeFor(tab.url) !== state.route || message.nonce !== state.nonce || message.session_ref !== state.session_ref) fail('会话已过期，请重新打开扩展');
    if (!tab.active) fail('请回到目标对话标签页再操作');
    const check = await this.api.content(tabId, { kind: 'check', route: state.route, nonce: state.nonce, session_ref: state.session_ref }, state.documentId);
    if (!check?.ok) fail('页面文档或会话已变化，请重新打开扩展');
    return state;
  }
  async selectedConversation(tabId, message, state, checkOnly = false) {
    const reply = await this.api.content(tabId, { kind: checkOnly ? 'capture_check' : 'capture_select', route: state.route, nonce: state.nonce, session_ref: state.session_ref, capture_id: message.capture_id, snapshot_hash: message.snapshot_hash, selections: message.selections }, state.documentId);
    if (!reply?.ok) fail(reply?.error || '会话快照无法核验，请重新读取');
    await this.current(tabId, message);
    return checkOnly ? reply.result : conversationFormat.verify(reply.result);
  }
  async userNative(tabId, message, state, action, args) {
    // This method is only reached from authenticated popup operations. These
    // actions are intentionally absent from validateAction / model requests.
    const request = { protocol: PROTOCOL, request_id: `u_${newNonce()}`, nonce: state.nonce, session_ref: state.session_ref, action, arguments: args };
    if (byteLength(JSON.stringify(request)) > 200 * 1024) fail('此会话较大，请导出 JSON 后在桌面导入；本机直存上限约 200 KiB，没有截断内容');
    const response = await this.api.native(HOST, { ...request, ...(state.installation_id ? { installation_id: state.installation_id } : {}) });
    await this.current(tabId, message);
    if (!response || byteLength(JSON.stringify(response)) > 900 * 1024 || typeof response.ok !== 'boolean') fail('本机桥响应无效');
    if (!response.ok) fail(`本机操作未完成：${typeof response.error === 'string' ? response.error.slice(0, 400) : '未知错误'}`);
    if (!Object.hasOwn(response, 'result')) fail('本机响应缺少 result');
    return response.result;
  }
  async readConnection(tabId, message, state) {
    state.connection = null;
    await this.api.save(tabId, state);
    let connection;
    try {
      connection = await this.userNative(tabId, message, state, 'connection', {});
      if (!connection || typeof connection.capture_enabled !== 'boolean' || !/^[a-f0-9]{64}$/u.test(connection.connection_id || '')) fail('本机桥缺少资料库身份校验，请升级；旧版本只能使用只读资料功能和文件导出');
    } catch (error) {
      state = await this.current(tabId, message);
      state.capture_approval = null;
      await this.api.save(tabId, state);
      throw error;
    }
    state = await this.current(tabId, message);
    if (state.capture_approval && (state.capture_approval.connection_id !== connection.connection_id || !connection.capture_enabled)) state.capture_approval = null;
    state.connection = connection;
    await this.api.save(tabId, state);
    return { state, connection };
  }
  async readNative(tabId, message, state, request, verification = false) {
    const now = this.api.now();
    if (verification && state.last_request_at && now - state.last_request_at < 1000) {
      await new Promise((resolve) => setTimeout(resolve, 1000 - (now - state.last_request_at)));
      state = await this.current(tabId, message);
    }
    state = reserveRequest(state, request, this.api.now());
    await this.api.save(tabId, state); // Reserve before I/O, including failed checks.
    const response = await this.api.native(HOST, { ...request, ...(state.installation_id ? { installation_id: state.installation_id } : {}) });
    if (!response || byteLength(JSON.stringify(response)) > 900 * 1024 || typeof response.ok !== 'boolean') fail('本机桥返回了无效或过大的响应');
    state = await this.current(tabId, message); // Navigation/reset invalidates late results.
    if (!response.ok) fail(`本机读取失败：${typeof response.error === 'string' ? response.error.slice(0, 400) : '未知错误'}`);
    if (!Object.hasOwn(response, 'result')) fail('本机响应缺少 result');
    return { state, result: response.result };
  }
  async discardStale(tabId, session, capsule) {
    let state = await this.api.load(tabId);
    if (!state || state.nonce !== session.nonce || state.session_ref !== session.session_ref) return '';
    let warning = '';
    if (capsule.delivery === 'draft') {
      try {
        const removed = await this.api.content(tabId, { kind: 'remove', route: state.route, nonce: state.nonce, session_ref: state.session_ref, id: capsule.id }, state.documentId);
        if (!removed?.ok) warning = '旧上下文可能仍在网页草稿里，请先人工删除；';
      } catch { warning = '旧上下文可能仍在网页草稿里，请先人工删除；'; }
    }
    // Re-read after the awaited DOM operation. Never resurrect a reset session.
    state = await this.api.load(tabId);
    if (!state || state.nonce !== session.nonce || state.session_ref !== session.session_ref) return warning;
    state.preview = null;
    state.bootstrap = null;
    state.relay_sent_message = null;
    setRelay(state, 'blocked', '旧资料或权限已失效，请重新准备', { request_id: null, manual_fallback: true });
    await this.api.save(tabId, state);
    return warning;
  }
  async revalidate(tabId, message, state, capsule) {
    try {
      if (!capsule.source_request || !capsule.result_fingerprint) fail('旧预览没有新鲜度凭据');
      const source = validateAction(capsule.source_request, state);
      const request = validateAction({ ...source, request_id: `v_${newNonce()}`, nonce: state.nonce, session_ref: state.session_ref }, state);
      const checked = await this.readNative(tabId, message, state, request, true);
      if (await resultFingerprint(request.action, checked.result) !== capsule.result_fingerprint) fail('本机资料、Bootstrap 版本或授权范围已经变化');
      return await this.current(tabId, message);
    } catch (error) {
      const warning = await this.discardStale(tabId, state, capsule);
      fail(`${error.message}；旧预览已废弃。${warning}请重新请求并检查资料，扩展没有发送任何内容`);
    }
  }
  async relayCommand(tabId, message) {
    let state = await this.current(tabId, message);
    const command = message.command;
    if (command === 'inspect') return publicState(state);
    if (['pause', 'resume', 'disable', 'enable'].includes(command)) {
      const old = relayState(state);
      const paused = ['pause', 'disable'].includes(command);
      state.relay = { ...old, enabled: command === 'disable' ? false : command === 'enable' ? true : old.enabled, paused };
      setRelay(state, paused ? 'paused' : state.preview ? 'awaiting_user_send' : 'idle', paused ? '接力已暂停，恢复后会重新核对当前会话与权限' : '接力已恢复，正在重新核对当前会话');
      await this.api.save(tabId, state);
      return publicState(state);
    }
    if (relayState(state).paused || !relayState(state).enabled) fail('接力已暂停，请先恢复');
    if (command === 'delivered') {
      if (!state.preview) fail('没有待确认的本轮资料');
      const reply = await this.api.content(tabId, { kind: 'delivered', route: state.route, nonce: state.nonce, session_ref: state.session_ref, id: state.preview.id }, state.documentId);
      if (!reply?.ok) fail(reply?.error || '请先自己点击网页发送');
      state = await this.current(tabId, message);
      state.preview.delivery = 'user_confirmed_sent';
      if (state.bootstrap?.id === state.preview.id) state.bootstrap.delivery = 'user_confirmed_sent';
      setRelay(state, 'waiting_reply', '已按你的确认记录发送，等待完整回复');
      await this.api.save(tabId, state);
      return publicState(state);
    }
    if (!['copy', 'execute', 'bootstrap'].includes(command)) fail('不支持的接力操作');
    const checked = await this.readConnection(tabId, message, state);
    state = checked.state;
    const grant = checked.connection.automation;
    if (!grant?.recall || !grant.provider_disclosure || grant.platform !== siteFor(state.route).id || !Number.isSafeInteger(grant.permission_revision)) fail('请在桌面确认此网站的读取和提供资料权限');
    if (command === 'bootstrap') {
      if (state.bootstrap || state.preview) fail('本会话已有使用说明或本轮资料，请继续当前轮；重新开始需显式重置');
      const request = validateAction({ protocol: PROTOCOL, request_id: `b_${newNonce()}`, nonce: state.nonce, session_ref: state.session_ref, action: 'bootstrap', arguments: {} }, state);
      await prepareAutomaticRequest(this, tabId, message, state, request, grant);
      return publicState(await this.current(tabId, message));
    }
    if (command === 'copy') {
      const capsule = state.preview;
      if (!capsule || capsule.automatic_revision !== grant.permission_revision) fail('本轮资料或权限已变化，请重新准备');
      const result = await this.userNative(tabId, message, state, 'authorized_read', { action: capsule.source_request.action, arguments: capsule.source_request.arguments, permission_revision: grant.permission_revision });
      if (await resultFingerprint(capsule.action, result) !== capsule.result_fingerprint) {
        await this.discardStale(tabId, state, capsule);
        fail('资料已变化，旧副本已废弃，请重新准备');
      }
      return { text: capsuleText(capsule), request_id: capsule.id };
    }
    if (state.preview && ['observed_sent', 'user_confirmed_sent'].includes(state.preview.delivery) && finalReceipt(message.text, state, state.preview.id)) {
      setRelay(state, 'result_ready', '已收到你粘贴的本轮完整回答标记，可回到对话阅读');
      await this.api.save(tabId, state);
      return publicState(state);
    }
    if (state.preview?.delivery === 'draft') fail('本轮资料仍在草稿，请先手动发送');
    const request = parseAction(message.text, state);
    if (request.action === 'bootstrap' && state.bootstrap) fail('本会话已有使用说明；如需重新开始请显式重置会话');
    await prepareAutomaticRequest(this, tabId, message, state, request, grant);
    return publicState(await this.current(tabId, message));
  }
  async handle(message, sender) {
    if (message?.kind === 'relay_control') {
      const tab = await this.api.getTab(sender.tab?.id);
      verifyContent(sender, this.api.id, tab.url, message.route);
      return this.serial(tab.id, async () => {
        const state = await this.current(tab.id, message);
        if (state.documentId !== sender.documentId) fail('接力操作来自过期文档');
        return this.relayCommand(tab.id, message);
      });
    }
    if (message?.kind === 'automatic_tick') {
      const tab = await this.api.getTab(sender.tab?.id);
      verifyContent(sender, this.api.id, tab.url, message.route);
      return this.serial(tab.id, async () => {
        const state = await this.current(tab.id, message);
        if (state.documentId !== sender.documentId) fail('自动请求来自过期文档');
        try {
          const result = await automaticCycle(this, tab.id, message);
          const current = await this.current(tab.id, message);
          current.automatic_status = result.status; current.automatic_error = null;
          current.auto_capture_note = result.capture_note || current.auto_capture_note || null;
          await this.api.save(tab.id, current);
          return result;
        } catch (error) {
          const current = await this.api.load(tab.id);
          if (current?.nonce === message.nonce) {
            current.automatic_status = 'failed';
            setRelay(current, 'blocked', error.message, { manual_fallback: true });
            current.automatic_error = '自动接入未完成；打开连接检查，确认授权、身份与页面是否稳定';
            await this.api.save(tab.id, current);
          }
          throw error;
        }
      });
    }
    if (message?.kind === 'bind') {
      const tab = await this.api.getTab(sender.tab?.id);
      const route = verifyContent(sender, this.api.id, tab.url, message.route);
      return this.serial(tab.id, async () => {
        const state = bindSession(await this.api.load(tab.id), { route, token: message.token, documentId: sender.documentId, installationId: this.api.installationId ? await this.api.installationId() : null });
        state.extension_id = this.api.id;
        await this.api.save(tab.id, state);
        const { preview: _preview, ...binding } = publicState(state);
        return binding;
      });
    }
    verifyPopup(sender, this.api.id, this.api.popupUrl);
    if (!['inspect', 'reset', 'bootstrap', 'execute', 'insert', 'remove', 'delivered', 'capture', 'capture_check', 'check_state', 'conversation', 'connection', 'pair', 'capture_preview', 'capture_save'].includes(message?.kind)) fail('不支持的扩展操作');
    const tabId = message.tabId;
    if (!Number.isInteger(tabId)) fail('标签页编号无效');
    const tab = await this.api.getTab(tabId);
    routeFor(tab.url);
    if (!tab.active) fail('请回到目标对话标签页再操作');
    if (['inspect', 'reset'].includes(message.kind)) {
      const response = await this.api.content(tabId, { kind: message.kind === 'inspect' ? 'describe' : 'reset' });
      if (!response?.ok) fail(response?.error || '页面扩展未就绪，请刷新当前受支持的对话页面');
      const state = await this.current(tabId, response.result);
      return { ...publicState(state), inserted: response.result.inserted, warnings: response.result.warnings };
    }
    return this.serial(tabId, async () => {
      let state = await this.current(tabId, message);
      if (message.kind === 'check_state') return { status: 'current' };
      if (message.kind === 'pair') {
        const pending = await this.userNative(tabId, message, state, 'request_pairing', {});
        if (pending?.status !== 'pending' || pending.data_access !== false) fail('连接请求没有确认进入待批准状态');
        return pending;
      }
      if (message.kind === 'connection') {
        return publicState((await this.readConnection(tabId, message, state)).state);
      }
      if (message.kind === 'capture') {
        const reply = await this.api.content(tabId, { kind: 'capture', route: state.route, nonce: state.nonce, session_ref: state.session_ref }, state.documentId);
        if (!reply?.ok) fail(reply?.error || '无法读取可见会话');
        await this.current(tabId, message);
        conversationFormat.checkSize(reply.result);
        state.capture_approval = null;
        await this.api.save(tabId, state);
        return reply.result;
      }
      if (['capture_check', 'conversation', 'capture_preview', 'capture_save'].includes(message.kind)) {
        const conversation = await this.selectedConversation(tabId, message, state, message.kind === 'capture_check');
        if (message.kind === 'capture_check' || message.kind === 'conversation') return conversation;
        if (message.kind === 'capture_preview') {
          state.capture_approval = null;
          await this.api.save(tabId, state);
          const preview = await this.userNative(tabId, message, state, 'capture_preview', { conversation });
          if (!preview || typeof preview.approval_hash !== 'string' || !preview.approval_hash || !Number.isInteger(preview.event_count) || preview.event_count < 1 || !Array.isArray(preview.samples) || !/^[a-f0-9]{64}$/u.test(preview.connection_id || '') || typeof preview.vault_name !== 'string' || !preview.vault_name || typeof preview.scope !== 'string' || !preview.scope) fail('本机保存预览缺少资料库身份或目的范围，请升级本机桥；旧版本只能只读或导出');
          await this.selectedConversation(tabId, message, state);
          state = await this.current(tabId, message);
          state.capture_approval = { capture_id: conversation.capture_id, snapshot_hash: conversation.metadata.snapshot_hash, approval_hash: preview.approval_hash, connection_id: preview.connection_id };
          await this.api.save(tabId, state);
          return preview;
        }
        const approval = state.capture_approval;
        if (!approval || approval.capture_id !== conversation.capture_id || approval.snapshot_hash !== conversation.metadata.snapshot_hash || approval.approval_hash !== message.approval_hash) fail('保存预览已变化，请重新查看脱敏预览后确认');
        const checkedConnection = await this.readConnection(tabId, message, state);
        state = checkedConnection.state;
        if (!checkedConnection.connection.capture_enabled || checkedConnection.connection.connection_id !== approval.connection_id || !state.capture_approval) fail('保存目的资料库或授权范围已变化，旧确认已清除。请重新查看目的资料库和脱敏预览，再明确确认保存');
        await this.selectedConversation(tabId, message, state);
        try {
          const result = await this.userNative(tabId, message, state, 'capture_save', { conversation, approval_hash: approval.approval_hash });
          if (!result || !Number.isInteger(result.events_added) || !Number.isInteger(result.events_seen)) fail('本机保存结果无效');
          return result;
        } catch (error) { fail(`${error.message}。未确认保存结果；请在桌面检查，重试同一快照会去重`); }
      }
      if (['insert', 'remove', 'delivered'].includes(message.kind)) {
        if (!state.preview) fail('请先准备并检查上下文');
        if (message.kind === 'insert' && state.preview.delivery !== 'prepared') fail('结果已插入或已确认发送，不会重复追加');
        if (message.kind === 'insert') state = await this.revalidate(tabId, message, state, state.preview);
        const reply = await this.api.content(tabId, { kind: message.kind, route: state.route, nonce: state.nonce, session_ref: state.session_ref, ...(message.kind === 'insert' ? { capsule: publicState(state).preview } : {}), id: state.preview.id }, state.documentId);
        if (!reply?.ok) fail(reply?.error || '页面输入框未响应');
        state = await this.current(tabId, message);
        state.preview.delivery = message.kind === 'insert' ? 'draft' : message.kind === 'remove' ? 'prepared' : 'user_confirmed_sent';
        if (state.bootstrap?.id === state.preview.id) state.bootstrap.delivery = state.preview.delivery;
        await this.api.save(tabId, state);
        return publicState(state);
      }
      if (message.kind === 'bootstrap' && state.bootstrap) {
        if (state.preview?.delivery === 'draft' && state.preview.id !== state.bootstrap.id) fail('请先移除当前草稿中的上下文，或确认它已手动发送');
        state = await this.revalidate(tabId, message, state, state.bootstrap);
        state.preview = state.bootstrap;
        await this.api.save(tabId, state);
        return publicState(state);
      }
      if (state.preview?.delivery === 'draft') fail('请先移除当前草稿中的上下文，或确认它已手动发送');
      const request = message.kind === 'execute' ? parseAction(message.text, state) : validateAction({ protocol: PROTOCOL, request_id: `b_${newNonce()}`, nonce: state.nonce, session_ref: state.session_ref, action: 'bootstrap', arguments: {} }, state);
      if (request.action === 'bootstrap' && state.bootstrap) fail('本会话已有固定 Bootstrap；请用准备 Bootstrap 查看，更新需显式重置会话');
      const checked = await this.readNative(tabId, message, state, request);
      const fingerprint = await resultFingerprint(request.action, checked.result);
      state = await this.current(tabId, message);
      state.preview = { ...makeCapsule(request, checked.result, state), source_request: request, result_fingerprint: fingerprint };
      if (request.action === 'bootstrap') state.bootstrap = state.preview;
      await this.api.save(tabId, state);
      return publicState(state);
    });
  }
}
