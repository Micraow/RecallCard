import { strictJson, isRecord } from './protocol.js';
export const PHASES = Object.freeze(['idle', 'preparing', 'awaiting_user_send', 'waiting_reply', 'result_ready', 'paused', 'blocked']);
export function relayState(state) {
  return { enabled: true, paused: false, phase: 'idle', round: 0, request_id: null, detail: '等待本机连接与授权', manual_fallback: false, result_ready: false, ...(state.relay || {}) };
}
export function setRelay(state, phase, detail, extra = {}) {
  if (!PHASES.includes(phase)) throw new Error('接力状态无效');
  state.relay = { ...relayState(state), phase, detail, result_ready: phase === 'result_ready', ...extra };
  return state.relay;
}
export function capsuleText(capsule) {
  return `[RecallCard ${capsule.nonce}:${capsule.id}]\n${capsule.text}\n[/RecallCard ${capsule.nonce}:${capsule.id}]`;
}
export function finalReceipt(text, state, requestId) {
  if (typeof text !== 'string' || text.includes('recallcard-action')) return false;
  const match = /```recallcard-final\n([^]*?)\n```\s*$/u.exec(text.replace(/\r\n/gu, '\n'));
  if (!match || text.slice(0, match.index).includes('```recallcard-final')) return false;
  try {
    const data = strictJson(match[1]);
    return isRecord(data) && Object.keys(data).length === 4 && data.protocol === 'recallcard.final/1' && data.nonce === state.nonce && data.session_ref === state.session_ref && data.after_request_id === requestId;
  } catch { return false; }
}
// A visible, stable user turn must contain the exact nonce-bound capsule marker.
// Emptying the composer, copying, or losing its DOM node is never Send evidence.
export function observeRelay(state, observation) {
  if (!observation || !state.preview) return false;
  const preview = state.preview;
  const sent = observation.sent?.find(item => item.id === preview.id);
  if (sent) {
    preview.delivery = 'observed_sent';
    if (state.bootstrap?.id === preview.id) state.bootstrap.delivery = 'observed_sent';
    state.relay_sent_message = sent.message_id;
    setRelay(state, 'waiting_reply', '已看到你发送本轮资料，等待 AI 完整回复');
  }
  const last = observation.last;
  if (['observed_sent', 'user_confirmed_sent'].includes(preview.delivery) && last?.role === 'assistant' && last.complete && last.after_sent && finalReceipt(last.text, state, preview.id)) {
    setRelay(state, 'result_ready', 'AI 已标记本轮回答完成，可回到对话阅读；内容仍需你判断');
    return true;
  }
  return false;
}
