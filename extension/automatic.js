import { parseAction, validateAction, makeCapsule, newNonce, reserveRequest, PROTOCOL, byteLength, siteFor } from './protocol.js';
import './conversation-format.js';
import { relayState, setRelay, observeRelay } from './relay-state.js';
const format = globalThis.RecallCardConversationFormat;

// This code receives only the worker's freshly checked local grant. DOM/model
// content is never interpreted as setup, permission, client identity or sending.
export async function automaticCycle(broker, tabId, message) {
  let state = await broker.current(tabId, message);
  const relay = relayState(state);
  if (!relay.enabled || relay.paused) return { status: 'paused' };
  const checked = await broker.readConnection(tabId, message, state);
  state = checked.state;
  const grant = checked.connection.automation;
  const active = grant && grant.platform === siteFor(state.route).id && Number.isSafeInteger(grant.permission_revision);
  if (!active || !grant.capture && !grant.recall) {
    if (state.preview && state.automatic_revision) { await broker.discardStale(tabId, state, state.preview); state = await broker.current(tabId, message); }
    setRelay(state, 'blocked', '需要在桌面确认此网站的接入权限');
    await broker.api.save(tabId, state);
    return { status: checked.connection.state === 'revoked' ? 'revoked' : 'not_authorized', account_identity: 'unverified' };
  }
  if (state.automatic_revision && state.automatic_revision !== grant.permission_revision) {
    if (state.preview) await broker.discardStale(tabId, state, state.preview);
    state = await broker.current(tabId, message);
    state.auto_capture_hash = null;
  }
  state.automatic_revision = grant.permission_revision;
  await broker.api.save(tabId, state);
  const reply = await broker.api.content(tabId, { kind: 'automatic_snapshot', route: state.route, nonce: state.nonce, session_ref: state.session_ref, capture: !!grant.capture, recall: !!grant.recall, capsule_id: state.preview?.id || null, sent_message_id: state.relay_sent_message || null }, state.documentId);
  if (!reply?.ok) throw new Error(reply?.error || '自动读取尚未就绪');
  state = await broker.current(tabId, message);
  const snapshot = reply.result;
  if (snapshot.status !== 'stable') {
    if (snapshot.status === 'manual_fallback' && relayState(state).phase !== 'result_ready') {
      const pending = state.preview && ['prepared', 'draft', 'left_draft'].includes(state.preview.delivery);
      setRelay(state, pending ? 'awaiting_user_send' : state.preview ? 'waiting_reply' : 'blocked', pending ? '网页结构尚未可靠识别；请复制完整本轮资料后自行粘贴发送' : '网页结构尚未可靠识别；请粘贴完整回复或请求继续', { manual_fallback: true });
    }
    else if (!['awaiting_user_send', 'result_ready'].includes(relayState(state).phase)) setRelay(state, 'waiting_reply', '等待生成结束、完成标记与页面稳定');
    await broker.api.save(tabId, state);
    return { status: snapshot.status || 'waiting_for_stable_page', account_identity: 'unverified' };
  }
  const complete = observeRelay(state, snapshot.relay_observation);
  if (snapshot.completion_unknown && relayState(state).phase === 'waiting_reply') setRelay(state, 'waiting_reply', '尚未看到本轮完成凭据；网页改版时可粘贴完整请求或回答继续', { manual_fallback: true });
  await broker.api.save(tabId, state);
  let captured = 0;
  if (grant.capture && snapshot.conversation) {
    const conversation = await format.verify(snapshot.conversation);
    if (conversation.source.url !== state.route) throw new Error('自动捕获来源与当前页面不一致');
    const fingerprint = await format.hash({ source: conversation.source, messages: conversation.messages });
    if (state.auto_capture_hash !== fingerprint) {
      const chunks = []; let messages = [];
      for (const item of conversation.messages) {
        if (byteLength(JSON.stringify({ ...conversation, messages: [...messages, item] })) > 180 * 1024) {
          if (!messages.length) throw new Error('单条可见消息超过本机桥上限；未截断，请使用官方导入');
          chunks.push(messages); messages = [];
          if (byteLength(JSON.stringify({ ...conversation, messages: [item] })) > 180 * 1024) throw new Error('单条可见消息超过本机桥上限；未截断，请使用官方导入');
        }
        messages.push(item);
      }
      if (messages.length) chunks.push(messages);
      let previous = null;
      for (const batch of chunks) {
        state = await broker.current(tabId, message);
        const result = await broker.userNative(tabId, message, state, 'automatic_capture', { conversation: { ...conversation, messages: batch, metadata: { ...conversation.metadata, ...(previous ? { chunk_previous_message: { platform: conversation.source.platform, conversation_id: conversation.source.conversation_id, message_id: previous } } : {}) } }, permission_revision: grant.permission_revision });
        if (!Number.isInteger(result.events_added) || !Number.isInteger(result.events_seen)) throw new Error('未确认自动保存结果');
        captured += result.events_added;
        previous = batch.at(-1).id;
      }
      state = await broker.current(tabId, message);
      state.auto_capture_hash = fingerprint;
      state.last_capture_at = new Date(broker.api.now()).toISOString();
      state.auto_capture_note = snapshot.capture_note || null;
      await broker.api.save(tabId, state);
    }
  }
  if (complete) return { status: 'result_ready', captured, account_identity: 'unverified' };
  if (!grant.recall || !grant.provider_disclosure) return { status: 'capture_ready', captured, capture_note: snapshot.capture_note, account_identity: 'unverified' };
  if (state.preview?.delivery === 'draft') {
    if (snapshot.draft_ids?.includes(state.preview.id)) {
      setRelay(state, 'awaiting_user_send', '本轮资料已备好，请检查后自己点击网页发送');
      await broker.api.save(tabId, state);
      return { status: 'draft_ready', captured, account_identity: 'unverified' };
    }
    // A missing block is not evidence of human Send. Forget ownership without
    // claiming delivery and keep the bootstrap lifecycle receipt for this session.
    state.preview.delivery = 'left_draft';
    if (state.bootstrap?.id === state.preview.id) state.bootstrap.delivery = 'left_draft';
    setRelay(state, 'awaiting_user_send', '草稿已离开输入框，尚未确认发送；可手动确认或复制原资料', { manual_fallback: true });
    await broker.api.save(tabId, state);
  }
  let request;
  if (!state.bootstrap) request = validateAction({ protocol: PROTOCOL, request_id: `b_${newNonce()}`, nonce: state.nonce, session_ref: state.session_ref, action: 'bootstrap', arguments: {} }, state);
  else {
    for (const text of snapshot.requests || []) {
      try {
        const candidate = parseAction(text, state);
        if (!(state.used || []).includes(candidate.request_id) && candidate.action !== 'bootstrap') { request = candidate; break; }
      } catch { /* Partial, stale or forged model text never changes grants. */ }
    }
  }
  if (!request) return { status: relayState(state).phase === 'waiting_reply' ? 'waiting_reply' : 'ready', captured, account_identity: 'unverified' };
  return prepareAutomaticRequest(broker, tabId, message, state, request, grant, snapshot.composer_fingerprint, captured);
}

export async function prepareAutomaticRequest(broker, tabId, message, state, request, grant, composerFingerprint = null, captured = 0) {
  if (!grant?.recall || !grant.provider_disclosure || grant.platform !== siteFor(state.route).id || !Number.isSafeInteger(grant.permission_revision)) throw new Error('当前网站未获资料读取与提供给 AI 的授权');
  if (relayState(state).paused || !relayState(state).enabled) throw new Error('接力已暂停，请先恢复');
  state = reserveRequest(state, request, broker.api.now());
  setRelay(state, 'preparing', '正在按 AI 请求准备相关资料', { request_id: request.request_id, manual_fallback: false });
  await broker.api.save(tabId, state);
  const args = { action: request.action, arguments: request.arguments, permission_revision: grant.permission_revision };
  const result = await broker.userNative(tabId, message, state, 'authorized_read', args);
  state = await broker.current(tabId, message);
  // Revalidate immediately before disclosing in the website composer. Every
  // automatic read is grant-checked again by native, including revocation.
  const fresh = await broker.userNative(tabId, message, state, 'authorized_read', args);
  const stable = value => { if (request.action !== 'bootstrap') return value; const { coverage: _coverage, ...rest } = value; return rest; };
  const fingerprint = await format.hash(stable(result));
  if (fingerprint !== await format.hash(stable(fresh))) throw new Error('资料在准备期间变化，请等待下一次读取');
  const capsule = makeCapsule(request, fresh, state);
  // Persist the complete result before attempting the composer. A changed DOM or
  // user edit becomes a copy/paste fallback, never a lost result or replayed read.
  state.preview = { ...capsule, source_request: request, result_fingerprint: fingerprint, automatic_revision: grant.permission_revision };
  if (request.action === 'bootstrap') state.bootstrap = state.preview;
  state.last_read_at = new Date(broker.api.now()).toISOString();
  setRelay(state, 'awaiting_user_send', '本轮资料已准备，请检查后手动发送', { round: relayState(state).round + 1, request_id: request.request_id, manual_fallback: !composerFingerprint });
  await broker.api.save(tabId, state);
  const inserted = composerFingerprint ? await broker.api.content(tabId, { kind: 'insert', route: state.route, nonce: state.nonce, session_ref: state.session_ref, capsule, automatic: true, expected_composer_fingerprint: composerFingerprint }, state.documentId).catch(error => ({ ok: false, error: error.message })) : { ok: false, error: '没有已核验的输入框快照' };
  state = await broker.current(tabId, message);
  if (!inserted?.ok) {
    setRelay(state, 'awaiting_user_send', '输入框不可用或已被编辑；完整资料已保留，请复制后粘贴发送', { manual_fallback: true });
    await broker.api.save(tabId, state);
    return { status: 'manual_fallback', captured, account_identity: 'unverified' };
  }
  state.preview.delivery = 'draft';
  if (request.action === 'bootstrap') state.bootstrap = state.preview;
  await broker.api.save(tabId, state);
  return { status: 'draft_ready', captured, account_identity: 'unverified' };
}
