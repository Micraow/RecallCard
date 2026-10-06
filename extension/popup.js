const $ = (id) => document.getElementById(id);
let tabId, state = null, busy = false;
function status(text, error = false) { $('status').textContent = text; $('status').classList.toggle('error', error); }
function render() {
  $('binding').textContent = state ? `session_ref: ${state.session_ref}\nnonce: ${state.nonce}\n${state.route}` : '';
  $('preview').value = state?.preview?.text || '';
  for (const button of document.querySelectorAll('button')) button.disabled = busy || !state;
  $('insert').disabled = busy || state?.preview?.delivery !== 'prepared';
  $('remove').disabled = busy || state?.preview?.delivery !== 'draft';
  $('delivered').disabled = busy || !state?.preview || state.preview.delivery === 'user_confirmed_sent';
}
async function send(kind) {
  const response = await chrome.runtime.sendMessage({ kind, tabId, nonce: state?.nonce, session_ref: state?.session_ref, ...(kind === 'execute' ? { text: $('action').value } : {}) });
  if (!response?.ok) throw new Error(response?.error || '扩展未响应，请重新打开');
  return response.result;
}
async function run(kind) {
  if (busy) return;
  busy = true; render();
  try {
    state = await send(kind);
    const messages = {
      inspect: state.warnings?.length ? `已切换会话；旧草稿需要人工检查：${state.warnings.join('；')}` : (state.preview ? '已恢复暂存预览；插入前将重新核验本机资料。手动复制前请重新请求并检查。' : '已绑定当前会话。资料只会在你点击后准备。'),
      reset: state.warnings?.length ? `已重置；请人工检查旧草稿：${state.warnings.join('；')}` : '已重置会话。旧请求已失效，请重新准备 Bootstrap。',
      bootstrap: 'Bootstrap 已准备。请检查资料和来源，再决定是否插入。',
      execute: '本机读取完成。请检查预览；尚未插入或发送。',
      insert: '已追加到可见草稿。请检查后自己点击 ChatGPT 的发送按钮。',
      remove: '只移除了未被改动的 RecallCard 上下文，保留你的其他草稿。',
      delivered: '已记录你的手动发送确认；扩展没有自动检测模型是否收到。',
    };
    if (kind === 'reset') $('action').value = '';
    status(messages[kind]);
  } catch (error) {
    // A failed freshness check may have revoked the stored preview. Do not
    // keep its stale text or enabled insertion button in the open popup.
    try { state = await send('inspect'); } catch { state = null; }
    status(error.message, true);
  }
  finally { busy = false; render(); }
}
for (const kind of ['bootstrap', 'reset', 'execute', 'insert', 'remove', 'delivered']) $(kind).addEventListener('click', () => void run(kind));
try {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tabs[0]?.id) throw new Error('请先打开 ChatGPT 对话标签页');
  tabId = tabs[0].id;
  await run('inspect');
} catch (error) { status(error.message, true); render(); }
