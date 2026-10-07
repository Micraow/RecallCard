import './site-adapters.js';
const $ = (id) => document.getElementById(id);
let tabId, state = null, busy = false, canReset = false;
function status(text, error = false) { $('status').textContent = text; $('status').classList.toggle('error', error); }
function render() {
  $('binding').textContent = state ? `网站：${state.platform_name}\n当前对话：${state.route}` : '';
  $('preview').value = state?.preview?.text || '';
  $('privacy').textContent = `加入草稿后，${state?.platform_name || '当前网站'}即可读取这些资料。确认可以分享后，再在网页点击发送。`;
  for (const button of document.querySelectorAll('button')) button.disabled = busy || !state;
  $('reset').disabled = busy || !canReset;
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
      inspect: state.platform !== 'chatgpt' ? `已识别 ${state.platform_name}。还不能自动判断是否换了对话，切换后请点“重新连接当前对话”。${state.warnings?.join('；') || ''}` : state.warnings?.length ? `已切换对话，请检查旧草稿：${state.warnings.join('；')}` : (state.preview ? '已恢复资料预览。手动复制前请重新查找，确认内容仍然有效。' : '已识别当前对话。点击“准备使用说明”开始。'),
      reset: state.warnings?.length ? `已重新连接，请检查旧草稿：${state.warnings.join('；')}` : '已重新连接当前对话，请重新准备使用说明。',
      bootstrap: '使用说明已准备，请检查后加入对话。',
      execute: '资料已找到，请检查预览。',
      insert: `资料已加入草稿。请检查后点击 ${state.platform_name} 网页的发送按钮。`,
      remove: '已移除加入的资料，你原有的草稿已保留。',
      delivered: '已记录为已发送。',
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
  if (!tabs[0]?.id) throw new Error('请先打开受支持的对话标签页');
  globalThis.RecallCardSites.forUrl(tabs[0].url);
  tabId = tabs[0].id; canReset = true;
  await run('inspect');
} catch (error) { status(error.message, true); render(); }
