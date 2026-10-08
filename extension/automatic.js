import { parseAction, validateAction, makeCapsule, newNonce, reserveRequest, PROTOCOL, byteLength, siteFor } from './protocol.js';
import './conversation-format.js';
const format = globalThis.RecallCardConversationFormat;

// This code receives only the worker's freshly checked local grant. DOM/model
// content is never interpreted as setup, permission, client identity or sending.
export async function automaticCycle(broker, tabId, message) {
  let state = await broker.current(tabId, message);
  const checked = await broker.readConnection(tabId, message, state);
  state = checked.state;
  const grant = checked.connection.automation;
  const active = grant && grant.platform === siteFor(state.route).id && Number.isSafeInteger(grant.permission_revision);
  if (!active || !grant.capture && !grant.recall) {
    if (state.preview?.delivery === 'draft' && state.automatic_revision) await broker.discardStale(tabId, state, state.preview);
    return { status: checked.connection.state === 'revoked' ? 'revoked' : 'not_authorized', account_identity: 'unverified' };
  }
  if (state.automatic_revision && state.automatic_revision !== grant.permission_revision) {
    if (state.preview) await broker.discardStale(tabId, state, state.preview);
    state = await broker.current(tabId, message);
    state.auto_capture_hash = null;
  }
  state.automatic_revision = grant.permission_revision;
  await broker.api.save(tabId, state);
  const reply = await broker.api.content(tabId, { kind: 'automatic_snapshot', route: state.route, nonce: state.nonce, session_ref: state.session_ref, capture: !!grant.capture, recall: !!grant.recall }, state.documentId);
  if (!reply?.ok) throw new Error(reply?.error || '自动读取尚未就绪');
  state = await broker.current(tabId, message);
  const snapshot = reply.result;
  if (snapshot.status !== 'stable') return { status: snapshot.status || 'waiting_for_stable_page', account_identity: 'unverified' };
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
  if (!grant.recall || !grant.provider_disclosure) return { status: 'capture_ready', captured, capture_note: snapshot.capture_note, account_identity: 'unverified' };
  if (state.preview?.delivery === 'draft') {
    if (snapshot.draft_ids?.includes(state.preview.id)) return { status: 'draft_ready', captured, account_identity: 'unverified' };
    // A missing block is not evidence of human Send. Forget ownership without
    // claiming delivery and keep the bootstrap lifecycle receipt for this session.
    state.preview.delivery = 'left_draft';
    if (state.bootstrap?.id === state.preview.id) state.bootstrap.delivery = 'left_draft';
    state.preview = null;
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
  if (!request) return { status: 'ready', captured, account_identity: 'unverified' };
  state = reserveRequest(state, request, broker.api.now());
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
  const inserted = await broker.api.content(tabId, { kind: 'insert', route: state.route, nonce: state.nonce, session_ref: state.session_ref, capsule }, state.documentId);
  if (!inserted?.ok) throw new Error(inserted?.error || '草稿没有确认插入');
  state = await broker.current(tabId, message);
  state.preview = { ...capsule, delivery: 'draft', source_request: request, result_fingerprint: fingerprint };
  if (request.action === 'bootstrap') state.bootstrap = state.preview;
  state.last_read_at = new Date(broker.api.now()).toISOString();
  await broker.api.save(tabId, state);
  return { status: 'draft_ready', captured, account_identity: 'unverified' };
}
