import './site-adapters.js';
import './conversation-format.js';
const $ = id => document.getElementById(id);
const format = globalThis.RecallCardConversationFormat;
let tabId, state = null, busy = false, canReset = false, snapshot = null, approval = null;
let selected = new Set(), roles = new Map();
const supportedCapture = () => ['chatgpt', 'deepseek'].includes(state?.platform);
function status(text, error = false) { $('status').textContent = text; $('status').classList.toggle('error', error); }
function discardCapture() {
  snapshot = null; approval = null; selected = new Set(); roles = new Map();
  $('messages').replaceChildren(); $('handoff').value = ''; $('handoff-panel').hidden = true;
  $('save-status').textContent = '尚未保存到 RecallCard';
}
function setState(next) {
  if (state && (next?.nonce !== state.nonce || next?.session_ref !== state.session_ref)) discardCapture();
  if (approval && (next?.capture_confirmation_valid === false || (next?.connection && next.connection.connection_id !== approval.connection_id))) approval = null;
  state = next;
}
function selection() {
  return (snapshot?.messages || []).filter(message => selected.has(message.id)).map(message => ({ id: message.id, role: roles.get(message.id) }));
}
function captureFields() { return { capture_id: snapshot?.capture_id, snapshot_hash: snapshot?.metadata.snapshot_hash, selections: selection() }; }
function render() {
  $('site').textContent = state ? `${state.platform_name} · 当前页面` : '当前页面未连接';
  $('route').textContent = state?.route || '';
  $('binding').textContent = state ? `网站：${state.platform_name}\n当前对话：${state.route}` : '';
  $('preview').value = state?.preview?.text || '';
  $('privacy').textContent = `加入草稿后，${state?.platform_name || '当前网站'}即可读取这些资料。确认可以分享后，再在网页点击发送。`;
  $('connection-state').textContent = state?.connection ? (state.connection.capture_enabled ? `本机已连接 · 保存到 ${state.connection.vault_name || '资料库'} · ${state.connection.capture_scope || '已授权范围'}` : '本机已连接 · 尚未授权保存，请在桌面连接设置中开启') : '本机连接尚未确认 · 导出无需连接';
  $('capture-support').textContent = supportedCapture() ? '仅读取已加载且可见的普通文字，不自动滚动。DeepSeek 中未明确的角色需要你确认。' : '此网站尚未验证会话读取；可展开高级区使用草稿功能，或用网站官方导出。';
  for (const button of document.querySelectorAll('button')) button.disabled = busy || !state;
  $('reset').disabled = busy || !canReset;
  $('capture').disabled = busy || !state || !supportedCapture();
  for (const id of ['select-all', 'select-none', 'export-json', 'export-markdown', 'capture-preview', 'prepare-handoff']) $(id).disabled = busy || !state || !snapshot || (id !== 'select-all' && id !== 'select-none' && !selected.size);
  for (const node of $('messages').querySelectorAll('input, select')) node.disabled = busy || !state;
  $('insert').disabled = busy || state?.preview?.delivery !== 'prepared';
  $('remove').disabled = busy || state?.preview?.delivery !== 'draft';
  $('delivered').disabled = busy || !state?.preview || state.preview.delivery === 'user_confirmed_sent';
  $('capture-save').disabled = busy || !state || !approval;
  $('copy-handoff').disabled = busy || !state || !$('handoff').value;
  $('capture-panel').hidden = !snapshot;
  $('save-panel').hidden = !approval;
  if (snapshot) $('capture-summary').textContent = `已选择 ${selected.size} / ${snapshot.messages.length} 条`;
}
async function send(kind, extra = {}) {
  const response = await chrome.runtime.sendMessage({ kind, tabId, nonce: state?.nonce, session_ref: state?.session_ref, ...extra });
  if (!response?.ok) throw new Error(response?.error || '扩展未响应，请重新打开');
  return response.result;
}
async function operation(fn, { recoverState = false } = {}) {
  if (busy) return;
  busy = true; render();
  try { await fn(); }
  catch (error) {
    if (recoverState) { try { setState(await send('inspect')); } catch { discardCapture(); state = null; } }
    status(error.message, true);
  } finally { busy = false; render(); }
}
async function run(kind) {
  await operation(async () => {
    setState(await send(kind, kind === 'execute' ? { text: $('action').value } : {}));
    const messages = {
      inspect: supportedCapture() ? '可以读取当前可见会话。导出不需要本机连接。' : `已识别 ${state.platform_name}，目前支持高级草稿功能。每次切换对话后请重新连接，无法判断同网址同输入框的全部会话变化。`,
      reset: '已重新确认当前页面，请重新读取会话或准备使用说明。',
      connection: state.connection?.capture_enabled ? '本机已连接，可以预览并保存选中的消息。' : '本机已连接。保存尚未授权，请在桌面连接设置中开启；仍可直接导出。',
      bootstrap: '使用说明已准备，请检查后加入对话。', execute: '资料已找到，请检查预览。',
      insert: `资料已加入草稿。请检查后点击 ${state.platform_name} 网页的发送按钮。`,
      remove: '已移除加入的资料，你原有的草稿已保留。', delivered: '已记录为已发送。',
    };
    if (kind === 'reset') { discardCapture(); $('action').value = ''; }
    status(`${messages[kind]}${state.warnings?.length ? ` 请检查旧草稿：${state.warnings.join('；')}` : ''}`);
  }, { recoverState: true });
}
function changedSelection() {
  approval = null; $('handoff').value = ''; $('handoff-panel').hidden = true;
  $('save-status').textContent = '选择已改变，当前选择尚未保存'; render();
}
function showMessages() {
  $('messages').replaceChildren();
  for (const [index, message] of snapshot.messages.entries()) {
    const article = document.createElement('article'); article.className = 'message';
    const head = document.createElement('div'); head.className = 'message-head';
    const label = document.createElement('label');
    const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.checked = selected.has(message.id); checkbox.dataset.messageId = message.id;
    checkbox.addEventListener('change', () => { checkbox.checked ? selected.add(message.id) : selected.delete(message.id); changedSelection(); });
    const name = document.createElement('span'); name.textContent = `${index + 1}. ${message.role === 'user' ? '我' : message.role === 'assistant' ? 'AI' : '待确认角色'}`;
    label.append(checkbox, name); head.append(label);
    if (!message.role) {
      const chooser = document.createElement('select'); chooser.setAttribute('aria-label', `第 ${index + 1} 条消息的角色`);
      for (const [value, text] of [['', '选择角色'], ['user', '我'], ['assistant', 'AI']]) { const option = document.createElement('option'); option.value = value; option.textContent = text; chooser.append(option); }
      chooser.addEventListener('change', () => { roles.set(message.id, chooser.value); changedSelection(); }); head.append(chooser);
    }
    const time = document.createElement('time'); time.textContent = message.occurred_at || '原始时间未知'; head.append(time);
    const details = document.createElement('details'), summary = document.createElement('summary'), content = document.createElement('pre');
    summary.textContent = message.text.length > 120 ? message.text.slice(0, 120) + '…（展开全文）' : message.text;
    content.textContent = message.text; details.append(summary, content); article.append(head, details); $('messages').append(article);
  }
  $('capture-coverage').textContent = '这是可见片段，不能确认会话完整。预览文字未保存到资料库；原始时间未知时不会用捕获时间替代。';
  $('capture-details').textContent = `${snapshot.coverage.reason}\n\n${snapshot.coverage.warnings.join('\n\n')}\n\n捕获时间：${snapshot.captured_at}\n全文摘要：${snapshot.metadata.snapshot_hash}`;
}
async function selectedConversation() {
  if (!snapshot) throw new Error('请先读取当前可见会话');
  if (selection().some(message => !['user', 'assistant'].includes(message.role))) throw new Error('请逐条确认选中消息的“我 / AI”角色，或取消选择不确定的消息');
  return format.verify(await send('conversation', captureFields()));
}
async function download(kind) {
  const conversation = await selectedConversation();
  const isJson = kind === 'json';
  const text = isJson ? format.json(conversation) : format.markdown(conversation);
  const blob = new Blob([text], { type: isJson ? 'application/json;charset=utf-8' : 'text/markdown;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  try {
    const id = await chrome.downloads.download({ url, filename: `RecallCard-${conversation.source.platform}-${conversation.capture_id}.${isJson ? 'json' : 'md'}`, saveAs: true });
    if (!Number.isInteger(id)) throw new Error('浏览器没有接受下载，请重试；预览仍保留');
    const result = await chrome.downloads.search({ id });
    if (result[0]?.state === 'interrupted') throw new Error('下载已取消或失败；预览仍保留，可以重试');
    status(result[0]?.state === 'complete' ? '导出文件已下载完成。尚未写入 RecallCard 资料库。' : '已交给浏览器下载，请在下载列表确认完成。尚未写入 RecallCard 资料库。');
  } catch (error) { throw new Error(`导出未确认完成：${error.message}。预览仍保留`); }
  finally { setTimeout(() => URL.revokeObjectURL(url), 60000); }
}
for (const kind of ['bootstrap', 'reset', 'execute', 'insert', 'remove', 'delivered', 'connection']) $(kind).addEventListener('click', () => void run(kind));
$('capture').addEventListener('click', () => void operation(async () => {
  discardCapture(); snapshot = await send('capture');
  selected = new Set(snapshot.messages.map(message => message.id)); roles = new Map(snapshot.messages.map(message => [message.id, message.role]));
  showMessages(); status(`已读取 ${snapshot.messages.length} 条可见消息，请检查内容和角色后选择保存或导出。`);
}, { recoverState: true }));
$('select-all').addEventListener('click', () => { selected = new Set(snapshot.messages.map(message => message.id)); for (const node of $('messages').querySelectorAll('input')) node.checked = true; changedSelection(); });
$('select-none').addEventListener('click', () => { selected.clear(); for (const node of $('messages').querySelectorAll('input')) node.checked = false; changedSelection(); });
$('export-json').addEventListener('click', () => void operation(() => download('json')));
$('export-markdown').addEventListener('click', () => void operation(() => download('markdown')));
$('prepare-handoff').addEventListener('click', () => void operation(async () => {
  $('handoff').value = format.handoff(await selectedConversation()); $('handoff-panel').hidden = false;
  status('上下文已准备。复制到另一个 AI 后请检查内容，再由你发送。');
}));
$('copy-handoff').addEventListener('click', () => void operation(async () => {
  await selectedConversation();
  try { await navigator.clipboard.writeText($('handoff').value); status('上下文已复制。请到目标 AI 粘贴，检查后手动发送。'); }
  catch { $('handoff').focus(); $('handoff').select(); throw new Error('浏览器未允许剪贴板写入，已选中上下文，请手动复制'); }
}));
$('capture-preview').addEventListener('click', () => void operation(async () => {
  await selectedConversation(); approval = null;
  approval = await send('capture_preview', captureFields());
  $('save-summary').textContent = `目的资料库：${approval.vault_name}。保存范围：${approval.scope}。将写入 ${approval.event_count} 条消息，其中 ${approval.redacted_event_count || 0} 条经过脱敏。请检查以下本机返回的样本。${approval.samples_complete === false ? '这里只展示最多 8 条、每条最多 2048 字节的脱敏样本；保存会包含所有选中消息的完整脱敏正文。' : ''}`;
  $('save-samples').textContent = approval.samples.map(sample => typeof sample === 'string' ? sample : JSON.stringify(sample, null, 2)).join('\n\n');
  status('脱敏预览已就绪。确认这些内容适合保存后，点击“确认保存这些消息”。');
}));
$('capture-save').addEventListener('click', () => void operation(async () => {
  if (!approval) throw new Error('请先查看本机脱敏预览');
  const result = await send('capture_save', { ...captureFields(), approval_hash: approval.approval_hash });
  $('save-status').textContent = `已保存到 RecallCard：新增 ${result.events_added} 条，核对 ${result.events_seen} 条。`;
  approval = null; status('本机已确认保存。你仍可导出这份快照，或为另一个 AI 准备上下文。');
}, { recoverState: true }));
$('cancel-save').addEventListener('click', () => { approval = null; render(); status('已取消确认，未发起保存；会话预览仍保留。'); });
try {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tabs[0]?.id) throw new Error('请先打开受支持的对话标签页');
  globalThis.RecallCardSites.forUrl(tabs[0].url);
  tabId = tabs[0].id; canReset = true; await run('inspect');
} catch (error) { status(error.message, true); render(); }
// This freshness check reads identities only; it does not capture message text.
if (typeof setInterval !== 'undefined') setInterval(async () => {
  if (busy || !state || (!snapshot && !state.preview)) return;
  try { await send('check_state'); }
  catch { discardCapture(); state = null; status('当前会话、输入框或可见消息已变化。旧预览已废弃，请重新连接并读取。', true); render(); }
}, 1000);
