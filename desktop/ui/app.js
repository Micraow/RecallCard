import { pages, shorten, displayDate, recordText, recordRef, newState, activateVault, resetScope, scopeOptions, nativeInstructions, displayState } from './model.js';
const state = newState();
const content = document.querySelector('#content');
const modal = document.querySelector('#modal');
const notice = document.querySelector('#notice');
const $ = (tag, attrs = {}, ...children) => {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === 'class') node.className = value;
    else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
    else if (key === 'text') node.textContent = value;
    else if (value != null && value !== false) node.setAttribute(key, value === true ? '' : value);
  }
  for (const child of children.flat()) if (child != null) node.append(typeof child === 'string' ? document.createTextNode(child) : child);
  return node;
};
const button = (text, action, primary = false, extra = '') => $('button', { class: `button ${primary ? 'primary' : ''} ${extra}`, type: 'button', onclick: action }, text);
const paragraph = text => $('p', {}, text);
const heading = (title, subtitle, eyebrow = 'RecallCard') => $('div', {}, $('div', { class: 'eyebrow' }, eyebrow), $('h1', {}, title), paragraph(subtitle));
const panel = (...children) => $('section', { class: 'panel' }, ...children);
const hint = (text, warning = false) => $('div', { class: `hint ${warning ? 'warning' : ''}` }, text);
const line = (title, value) => $('div', { class: 'info-line' }, $('span', { class: 'muted' }, title), $('span', { class: 'value' }, String(value ?? '—')));
const scopeLabel = scope => ({ personal: '个人资料', work: '工作资料' })[scope] || scope;
const args = () => ({ sessionId: state.vault.session_id, scope: state.scope });
function showNotice(message, error = false) {
  notice.className = `notice ${error ? 'error' : ''}`;
  notice.replaceChildren($('button', { 'aria-label': '关闭提示', onclick: () => { notice.hidden = true; } }, '×'), document.createTextNode(String(message)));
  notice.hidden = false;
}
async function invoke(command, payload = {}) {
  if (!window.__TAURI__?.core?.invoke) throw new Error('请从 RecallCard 桌面应用打开此界面');
  return window.__TAURI__.core.invoke(command, payload);
}
async function run(label, action) {
  if (state.busy) return;
  state.busy = true;
  const epoch = state.epoch;
  setBusy();
  document.querySelector('#operation').textContent = label;
  try { await action(() => epoch === state.epoch); }
  catch (error) { showNotice(typeof error === 'string' ? error : error.message || '操作未完成，请重试', true); }
  finally { state.busy = false; render(); }
}
function setBusy() {
  document.querySelectorAll('button, input, select, textarea').forEach(n => { n.disabled = state.busy; });
  document.querySelector('#operation').className = state.busy ? 'busy-mark' : '';
  if (!state.busy) document.querySelector('#operation').textContent = '准备就绪';
}
function navigate(page) {
  if (state.busy) return;
  state.page = page;
  render();
  content.focus({ preventScroll: true });
  if (page === 'search' && state.vault && !state.results.length) loadRecords();
  if (page === 'conversations' && state.vault) loadConversations();
}
async function chooseVault(create) {
  modal.close();
  await run(create ? '正在创建资料库…' : '正在打开资料库…', async () => {
    try {
      const vault = await invoke('choose_vault', { create });
      if (vault) { activateVault(state, vault); showNotice(`已打开 ${vault.display_name}`); }
    } catch (error) {
      // 核心在一次失败的切换后会失效旧会话，页面也立即清除旧预览。
      activateVault(state, { scopes: [] }); state.vault = null; throw error;
    }
  });
}
function vaultChooser() {
  if (state.busy) return;
  modal.replaceChildren($('h2', { id: 'modal-title' }, state.vault ? '切换资料库' : '开始自己的资料库'), paragraph('选择已有资料文件夹，或创建一个新资料库。'), hint('新建时请选择一个空文件夹。'), $('div', { class: 'button-row' }, button('取消', () => modal.close()), button('打开已有资料库', () => chooseVault(false)), button('创建新资料库', () => chooseVault(true), true)));
  modal.showModal();
}
function confirmDialog(title, text, action, label = '确认继续') {
  modal.replaceChildren($('h2', { id: 'modal-title' }, title), paragraph(text), $('div', { class: 'button-row' }, button('取消', () => modal.close()), button(label, () => { modal.close(); action(); }, true)));
  modal.showModal();
}
async function cancelPreviews(next) {
  await run('正在取消待确认操作…', async () => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.dreamPreview = null; state.notePreview = null;
    if (next) next();
  });
}
function scopeSelect() {
  const node = $('select', { 'aria-label': '资料范围' });
  for (const scope of scopeOptions(state.vault, state.scope)) { const option = $('option', { value: scope }, scopeLabel(scope)); option.selected = scope === state.scope; node.append(option); }
  node.addEventListener('change', async () => { const scope = node.value; await cancelPreviews(() => resetScope(state, scope)); if (state.page === 'search') loadRecords(); else if(state.page === 'conversations') loadConversations(); });
  return node;
}
function needsVault() {
  content.append(heading('打开资料库', '选择保存记忆的文件夹，即可导入、检索和审阅资料。'), $('div', { class: 'empty' }, $('div', { class: 'empty-icon' }, '▧'), $('h2', {}, '选择保存资料的位置'), paragraph('从已有资料库继续，或创建一个新的本地空间。'), button('选择资料库', vaultChooser, true)));
}
function home() {
  const art = $('div', { class: 'hero-art', 'aria-hidden': 'true' }, $('div', { class: 'memory-card back' }), $('div', { class: 'memory-card front' }, $('div', { class: 'card-mark' }, '✧'), $('div', { class: 'card-line' }), $('div', { class: 'card-line' }), $('div', { class: 'card-line short' })));
  content.append($('div', { class: 'hero' }, $('div', { class: 'hero-copy' }, $('div', { class: 'eyebrow' }, '给重要的事，一个归处'), $('h1', {}, '换一个 AI，也能接着聊。'), paragraph(state.vault ? `正在使用 ${state.vault.display_name}。从这里添加资料，查找原文和出处。` : '把不同 AI 的对话保存在自己的资料库，带上原话和背景继续任务。'), $('div', { class: 'button-row' }, button(state.vault ? (state.vault.event_count ? '继续已有会话' : '接入第一份对话') : '创建新资料库', () => state.vault ? navigate(state.vault.event_count ? 'conversations' : 'import') : chooseVault(true), true), button(state.vault ? '添加资料' : '打开已有资料库', () => state.vault ? navigate('import') : chooseVault(false)))), art));
  if (state.vault) {
    content.append($('div', { class: 'grid-three' }, panel($('span', { class: 'muted' }, '原始记录'), $('div', { class: 'stat' }, String(state.vault.event_count)), paragraph('保存的对话、想法和原始资料')), panel($('span', { class: 'muted' }, '长期记忆'), $('div', { class: 'stat' }, String(state.vault.memory_count)), paragraph('已整理的决定、偏好与背景')), panel($('span', { class: 'muted' }, '资料库状态'), $('div', { class: 'stat' }, state.vault.health?.ok ? '正常' : '待检查'), paragraph('随时查找和整理已有资料'))));
  }
  content.append($('div', { class: 'section-heading' }, $('h2', {}, state.vault ? '接下来，你可以…' : '三个简单的步骤'), $('span', {}, '离线也能开始')));
  const features = [
    ['↥', '接入你的对话', '用浏览器扩展保存 DeepSeek / ChatGPT 对话，或选择已有导出文件。', 'import', ''],
    ['⇄', '在另一个 AI 继续', '选择一段已保存的对话，带上原话、背景和接下来的目标。', 'conversations', 'purple'],
    ['✧', '把重要的事留下来', '导入整理结果，审阅后保存重要记忆。', 'dream', 'sand'],
  ];
  content.append($('div', { class: 'grid-three' }, features.map(([icon, title, description, page, color]) => panel($('div', { class: `step-icon ${color}` }, icon), $('h3', {}, title), paragraph(description), $('button', { class: 'text-link', onclick: () => state.vault ? navigate(page) : vaultChooser() }, ({import:'导入对话 →',search:'查找资料 →',conversations:'继续会话 →',dream:'整理记忆 →'})[page])))));
}
async function loadRecords() {
  await run('正在查找本地资料…', async current => {
    const query = state.query.trim();
    const result = await invoke(query ? 'search_records' : 'browse_records', { ...args(), target: state.target, ...(query ? { query } : {}) });
    if (!current()) return;
    state.results = result.results || [];
    state.selected = null; state.sources = [];
    state.resultNote = result.truncated ? '只显示部分结果；使用更具体的关键词缩小范围' : `${state.results.length} 条可见资料`;
  });
}
async function readRecord(item, source = false) {
  await run('正在读取记录与出处…', async current => {
    const reference = recordRef(item);
    const result = await invoke('read_record', { ...args(), reference });
    if (!current()) return;
    const first = result.results?.[0];
    if (!first) { if (result.truncated || result.pending_refs?.length) { state.selected = { ...item, truncated: true }; state.sources = []; showNotice('记录较长，先显示部分内容。完整原文保存在资料库文件中'); return; } throw new Error('这条资料已更新，请重新查找'); }
    state.selected = { ...item, text_truncated: false, ...first, truncated: Boolean(result.truncated || first.truncated), ref: reference };
    state.sources = [];
    if (source || reference.startsWith('memory:')) {
      const sources = await invoke('read_sources', { ...args(), reference });
      if (current()) { state.sources = sources.results || []; if (sources.truncated) showNotice('出处较长，完整内容保存在资料库文件中'); }
    }
  });
}
function readingPane() {
  if (!state.selected) return $('aside', { class: 'panel reading-pane empty' }, $('div', { class: 'empty-icon' }, '▤'), $('h3', {}, '从左侧选择一条资料'), paragraph('查看完整内容、时间和原始出处。'));
  const item = state.selected;
  const reference = recordRef(item);
  const pane = $('aside', { class: 'panel reading-pane' }, $('div', { class: 'result-meta' }, $('h2', {}, reference.startsWith('memory:') ? '长期记忆' : '原始记录'), $('span', { class: 'tag' }, scopeLabel(state.scope))), $('div', { class: 'ref' }, reference));
  const data = item.record || item.event || item.memory || item;
  const body = recordText(data) || recordText(item);
  pane.append($('div', { class: 'body-text' }, body || '这条资料没有保存正文'), line('记录时间', displayDate(data.recorded_at || data.captured_at || data.occurred_at || item.occurred_at)), line('状态', displayState(data.status || data.state || item.state)));
  if (item.text_truncated || item.truncated) pane.append(hint('此处只显示部分内容。完整记录保存在资料库文件中。'));
  pane.append($('hr', { class: 'divider' }), $('h3', {}, '整理这条资料'));
  const chosen = state.selectedRefs.includes(reference);
  pane.append(button(chosen ? '已选择 · 点击移除' : '选择这条资料', () => { state.selectedRefs = chosen ? state.selectedRefs.filter(r => r !== reference) : [...state.selectedRefs, reference]; render(); }, false, 'small'));
  if (state.selectedRefs.length) pane.append($('button', { class: 'text-link', onclick: () => navigate('dream') }, `前往整理（${state.selectedRefs.length} 条） →`));
  if (state.sources.length) {
    pane.append($('hr', { class: 'divider' }), $('h3', {}, '原始出处'));
    for (const source of state.sources) {
      const nested = source.events || source.sources || source.records || [source];
      for (const event of nested) pane.append($('div', { class: 'sample' }, $('div', { class: 'ref' }, recordRef(event) || (event.id ? `event:${event.id}` : '')), recordText(event) || recordText(event.event?.data) || recordText(event.data) || '出处元数据已校验'));
    }
  }
  return pane;
}
function searchPage() {
  content.append(heading('找回当时的想法', '同时检索原始对话和已整理的记忆。每条结果都保留出处。'));
  const query = $('input', { id: 'query', type: 'search', placeholder: '搜索一个决定、偏好或话题…', value: state.query, 'aria-label': '搜索关键词' });
  query.addEventListener('input', () => { state.query = query.value; });
  const target = $('select', { 'aria-label': '资料类型' }, $('option', { value: 'all' }, '所有资料'), $('option', { value: 'events' }, '原始记录'), $('option', { value: 'memories' }, '长期记忆'));
  target.value = state.target; target.addEventListener('change', () => { state.target = target.value; loadRecords(); });
  const form = $('form', { class: 'toolbar', onsubmit: event => { event.preventDefault(); loadRecords(); } }, $('div', { class: 'grow search-box' }, $('span', { class: 'search-icon' }, '⌕'), query), scopeSelect(), target, $('button', { class: 'button primary', type: 'submit' }, '查找'));
  content.append(form);
  if (!state.results.length) { content.append($('div', { class: 'empty' }, $('div', { class: 'empty-icon' }, '⌕'), $('h3', {}, state.query ? '暂时没有找到匹配资料' : '这里还没有可见资料'), paragraph(state.query ? '试试更短的关键词，或检查当前资料范围。' : '先保存一段文字，或导入已有对话。'), button('导入资料', () => navigate('import')))); return; }
  content.append($('div', { class: 'section-heading' }, $('span', {}, state.resultNote || `${state.results.length} 条资料`), $('span', {}, `当前分类：${scopeLabel(state.scope)}`)));
  const results = $('div', { class: 'results' });
  for (const item of state.results) {
    const ref = recordRef(item);
    results.append($('button', { class: `result-card ${recordRef(state.selected) === ref ? 'selected' : ''}`, onclick: () => readRecord(item) }, $('div', { class: 'result-meta' }, $('span', { class: `tag ${ref.startsWith('memory:') ? 'purple' : ''}` }, ref.startsWith('memory:') ? '长期记忆' : '原始记录'), $('span', { class: 'muted' }, displayDate(item.occurred_at || item.valid_from))), paragraph(recordText(item)), $('div', { class: 'ref' }, shorten(ref, 55))));
  }
  content.append($('div', { class: 'results-layout' }, results, readingPane()));
}
async function refreshStatus() { state.vault = await invoke('vault_status', { sessionId: state.vault.session_id }); }
function importPage() {
  const sourceChoices = panel($('h2', {}, '这次从哪里带入对话？'), paragraph('选择来源后，检查消息角色和覆盖范围，再保存到当前资料库。'), $('div', { class: 'grid-three' },
    ...[['recallcard-conversation', '浏览器扩展', 'DeepSeek、ChatGPT 等：在扩展中保存到本机，或导出 JSON 后选择文件'], ['chatgpt-export', 'ChatGPT 官方导出', '选择解压后的 conversations.json；只导入当前分支'], ['claude-code', 'Claude Code 对话', '选择你主动提供的会话 JSONL 文件；不扫描其他项目']].map(([format,title,help]) => panel($('h3',{},title),paragraph(help),button('选择这类文件',()=>{state.importFormat=format;state.fileImportOpen=true;render();document.querySelector('#file-import-details')?.scrollIntoView({block:'center'});},false,'small')))));
  content.append(heading('添加资料', '从对话来源开始，保留原话、角色和出处；个人补充可单独记为新笔记。'), sourceChoices);
  const text = $('textarea', { id: 'note-content', rows: 5, maxlength: 65536, placeholder: '例如：我希望项目说明优先使用中文。这里写你自己的新补充，不粘贴多角色聊天。', 'aria-label': '资料正文' }, state.noteText || '');
  text.addEventListener('input', () => { state.noteText = text.value; });
  const previewNote = () => run('正在准备预览…', async current => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.dreamPreview = null; state.notePreview = null;
    const preview = await invoke('preview_note', { ...args(), content: state.noteText || '' });
    if (current()) state.notePreview = preview;
  });
  if (!state.notePreview) {
    content.append(panel($('h2', {}, '或者，写下你自己的补充说明'), paragraph('这里保存的是你本人的新笔记。完整聊天请使用上面的来源导入，以保留用户/AI角色和原始时间。'), text, $('div', { class: 'button-row' }, button('预览并保存', previewNote, true))));
  } else {
    const note = state.notePreview;
    content.append(panel($('h2', {}, '确认保存的内容'), $('div', { class: 'body-text note-preview' }, note.content), note.redacted ? hint('检测到可能的敏感字段，已在预览中遮蔽。') : null,
      $('div', { class: 'button-row' }, button('返回修改', () => cancelPreviews()), button('确认保存记录', () => run('正在保存记录…', async () => {
        await invoke('confirm_note', { sessionId: state.vault.session_id, previewId: note.preview_id });
        state.notePreview = null; state.noteText = ''; state.results = [];
        await refreshStatus(); showNotice('记录已保存，可以开始查找'); state.query = ''; state.page = 'search';
        const listed = await invoke('browse_records', { ...args(), target: state.target }); state.results = listed.results || []; state.selected = null;
      }), true))));
  }
  const fileDetails = $('details', { id: 'file-import-details', class: 'panel import-file-options', open: Boolean(state.fileImportOpen || state.importPreview) }, $('summary', {}, '或者，导入对话文件'));
  fileDetails.addEventListener('toggle', () => { state.fileImportOpen = fileDetails.open; });
  content.append(fileDetails);
  const format = $('select', { id: 'import-format', 'aria-label': '导入格式' }, $('option', { value: 'auto' }, '自动识别支持的会话文件'), $('option', { value: 'recallcard-conversation' }, 'RecallCard 扩展导出的会话 JSON'), $('option', { value: 'chatgpt-export' }, 'ChatGPT 官方导出 JSON'), $('option', { value: 'claude-code' }, 'Claude Code 对话 JSONL'), $('option', { value: 'manual-jsonl' }, 'RecallCard 标准 JSONL'));
  format.value = state.importFormat || 'chatgpt-export';
  format.addEventListener('change', () => { const next = format.value; cancelPreviews(() => { state.importFormat = next; }); });
  const scope = $('input', { id: 'import-scope', value: state.scope, placeholder: 'personal', 'aria-label': '导入范围' });
  scope.addEventListener('change', () => { const next = scope.value.trim(); cancelPreviews(() => resetScope(state, next)); });
  const choose = () => run('正在读取导入预览…', async current => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.dreamPreview = null; state.notePreview = null;
    const result = await invoke('pick_import', { ...args(), format: state.importFormat || 'chatgpt-export' });
    if (current() && result) state.importPreview = result;
  });
  fileDetails.append($('div', { class: 'form-stack file-options-fields' }, $('div', {}, $('label', { for: 'import-format' }, '文件格式'), format, $('div', { class: 'field-note' }, 'ChatGPT 请先解压官方导出 ZIP，再选择 conversations.json')), $('details', {}, $('summary', {}, '高级：资料分类'), $('div', {}, $('label', { for: 'import-scope' }, '分类编号'), scope, $('div', { class: 'field-note' }, '默认 personal；已有资料可沿用原分类编号'))), paragraph('支持解压后的对话文件，单个文件最多 16 MiB。相同消息会自动去重。'), button('选择文件并预览', choose, true)));
  const preview = state.importPreview;
  if (!preview) return;
  const box = $('section', { class: 'panel file-preview' }, $('div', { class: 'section-heading' }, $('h2', {}, '确认导入'), $('span', { class: 'badge' }, '尚未写入')), $('div', { class: 'file-name' }, preview.file_name), line('解析到的记录', `${preview.event_count} 条`), line('写入范围', preview.scope), line('文件大小', `${(preview.byte_count / 1024).toFixed(1)} KiB`));
  if (preview.redacted_event_count) box.append(hint(`${preview.redacted_event_count} 条记录包含已遮蔽字段，请检查下方预览。`));
  box.append(paragraph(preview.warning));
  for (const sample of preview.samples || []) box.append($('div', { class: 'sample' }, $('span', { class: 'tag' }, sample.role === 'user' ? '用户' : sample.role === 'assistant' ? '助手' : sample.role), paragraph(sample.content)));
  if (preview.truncated) box.append(paragraph('预览仅展示部分内容，导入确认后会处理全部已解析记录。'));
  box.append($('div', { class: 'button-row' }, button('取消这次导入', () => cancelPreviews()), button(`确认导入 ${preview.event_count} 条记录`, () => confirmDialog('确认写入资料库', `将 ${preview.file_name} 中的 ${preview.event_count} 条记录导入 ${state.vault.display_name} / ${preview.scope}。已存在的相同记录会跳过。`, () => run('正在导入，完成前请勿关闭应用…', async () => { const result = await invoke('confirm_import', { sessionId: state.vault.session_id, previewId: preview.preview_id }); state.importPreview = null; state.results = []; await refreshStatus(); showNotice(`导入完成：新增 ${result.events_added} 条，处理 ${result.events_seen} 条`); }), '确认导入'), true)));
  content.append(box);
}
async function loadConversations(offset = 0) {
  offset = Number.isSafeInteger(offset) ? offset : 0;
  await run('正在读取已保存的会话…', async current => {
    const result = await invoke('list_conversations', { ...args(), offset });
    if (current()) { state.conversationListOffset=offset;state.conversationListNext=result.next_offset;state.conversationTotal=result.total;state.conversations = result.conversations || []; state.conversationNote = result.note; state.conversation=null;state.conversationRows=[];state.continuation=null; }
  });
}
async function openConversation(item, offset = 0) {
  await run('正在打开会话…', async current => {
    const result = await invoke('conversation_messages', { ...args(), conversationRef: item.session_ref, offset });
    if (current()) { if(state.conversation?.session_ref!==item.session_ref) state.continuationGoal=''; state.conversation = item; state.conversationRows = result.messages || []; state.conversationOrderKnown=result.order_known;state.conversationOffset = offset; state.conversationNext = result.next_offset; state.continuation = null; }
  });
}
function copyText(text) {
  const field = $('textarea', { readonly: true, 'aria-label': '复制内容' }); field.value = text;
  document.body.append(field); field.select();
  try {
    if (!document.execCommand('copy')) throw new Error('copy failed');
    showNotice('已复制，可以粘贴到你选择的客户端');
  } catch { showNotice('无法自动复制，请在预览框中全选后按 Ctrl+C', true); }
  field.remove();
}
function conversationPage() {
  content.append(heading('会话与接续', '按来源找回一段对话，把原话和接下来的目标带到另一个 AI。'));
  content.append($('div', {class:'toolbar'},scopeSelect(),button('刷新已保存会话',()=>loadConversations(0),false,'small'),button('接入对话',()=>navigate('import'),false,'small')));
  if (!state.conversations.length) { content.append(panel($('h2',{},'还没有保存的会话'),paragraph('在浏览器扩展中预览并保存当前对话，或者导入对话文件。未保存的网站历史不会自动出现在这里。'),button('选择对话来源',()=>navigate('import'),true))); return; }
  const list=$('div',{class:'conversation-list'});
  list.append(paragraph(`共 ${state.conversationTotal ?? state.conversations.length} 个可访问会话`));
  if(state.conversationListOffset)list.append(button('回到第一页',()=>loadConversations(0),false,'small'));
  if(state.conversationListNext!=null)list.append(button('更多会话',()=>loadConversations(state.conversationListNext),false,'small'));
  for(const item of state.conversations) list.append($('button',{class:`result-card ${state.conversation?.session_ref===item.session_ref?'selected':''}`,onclick:()=>openConversation(item)},$('strong',{},item.title),$('span',{class:'muted'},`${item.platform} · ${item.message_count} 条已保存消息`),$('span',{class:'muted'},`最近保存 ${displayDate(item.captured_at)}`)));
  const pane=panel($('h2',{},state.conversation?.title||'选择要继续的对话'));
  if(state.conversation){
    pane.append(hint(state.conversationOrderKnown?'以下保留已捕获片段中的消息顺序；网站未加载或未保存的历史不在其中。':'部分片段缺少可核实的先后关系；保留已知关系，其余按保存顺序显示，不代表原始时间顺序。'));
    for(const row of state.conversationRows||[]) pane.append($('article',{class:'conversation-message'},$('div',{class:'result-meta'},$('span',{class:'tag'},row.role==='user'?'我':row.role==='assistant'?'AI':'工具/其他'),$('span',{class:'muted'},displayDate(row.occurred_at))),$('div',{class:'body-text'},row.text),row.text_truncated?hint('本条较长，这里是节选。请在“查找与阅读”按原话检索查看。'):null,$('div',{class:'ref'},row.ref)));
    pane.append($('div',{class:'button-row'},...(state.conversationOffset?[button('回到开头',()=>openConversation(state.conversation,0),false,'small')]:[]),...(state.conversationNext!=null?[button('后续消息',()=>openConversation(state.conversation,state.conversationNext),false,'small')]:[])));
    const goal=$('textarea',{rows:2,placeholder:'例如：接着实现上次确定的方案，先检查还缺什么','aria-label':'接下来要做什么'});goal.value=state.continuationGoal||'';goal.addEventListener('input',()=>{state.continuationGoal=goal.value;state.continuation=null;document.querySelector('.copy-continuation')?.setAttribute('disabled','');});
    pane.append($('hr',{class:'divider'}),$('h2',{},'在另一个 AI 继续'),paragraph('写下下一步，程序会准备稳定背景、来源和选定会话中的消息。你检查后复制，再到目标客户端发送。'),goal,button('准备交接内容',()=>run('正在准备有来源的交接内容…',async current=>{const result=await invoke('prepare_continuation',{...args(),conversationRef:state.conversation.session_ref,goal:state.continuationGoal||''});if(current())state.continuation=result;}),true));
    if(state.continuation){const preview=$('textarea',{rows:12,readonly:true,'aria-label':'交接内容预览'});preview.value=state.continuation.text;pane.append(preview,hint(`已带上 ${state.continuation.message_count} / ${state.continuation.available_messages} 条消息。复制和发送前请检查是否适合分享给目标服务。`),button('复制交接内容',()=>run('正在重新核对交接内容…',async current=>{const shown=state.continuation;const fresh=await invoke('prepare_continuation',{...args(),conversationRef:state.conversation.session_ref,goal:state.continuationGoal||''});if(!current())return;if(!shown||fresh.text!==shown.text){state.continuation=null;throw new Error('资料或权限已改变，请重新生成并检查交接预览');}copyText(shown.text);}),true,'copy-continuation'));}
  } else pane.append(paragraph('先从左侧选择一段已保存的对话。'));
  content.append($('div',{class:'conversation-layout'},list,pane));
}
function dreamPage() {
  content.append(heading('整理记忆', '选好资料，导出整理包，再导入结果逐条审阅。'));
  content.append($('div', { class: 'toolbar' }, $('div', {}, $('label', {}, '本次整理范围'), scopeSelect())), hint('请使用其他工具整理导出的资料，并将结果保存为 RecallCard 整理结果文件（JSON）。'));
  const refs = state.selectedRefs;
  const exportBox = panel($('h2', {}, '1. 准备来源'), paragraph(refs.length ? `已从“查找与阅读”选中 ${refs.length} 条资料。` : '先在“查找与阅读”中选择原始记录，可一并选择需要更新的记忆。'));
  for (const ref of refs) exportBox.append($('div', { class: 'info-line' }, $('span', { class: 'ref' }, ref), button('移除', () => { state.selectedRefs = refs.filter(r => r !== ref); render(); }, false, 'small quiet')));
  exportBox.append($('div', { class: 'button-row' }, button('去选择资料', () => navigate('search')), ...(refs.some(r => r.startsWith('event:')) ? [button('导出本次来源包', () => run('正在保存来源包…', async () => { const saved = await invoke('export_dream', { ...args(), sourceRefs: refs.filter(r => r.startsWith('event:')), memoryRefs: refs.filter(r => r.startsWith('memory:')) }); if (saved) showNotice(`来源包已保存：${saved}`); }), true)] : [])));
  const reviewBox = panel($('h2', {}, '2. 审阅整理结果'), paragraph('选择与来源包配套的整理结果文件。'), $('div', { class: 'button-row' }, button('选择结果并审阅', () => run('正在核对整理结果…', async current => { await invoke('cancel_previews', { sessionId: state.vault.session_id }); state.importPreview = null; state.dreamPreview = null; state.notePreview = null; const result = await invoke('pick_dream', args()); if (result && current()) state.dreamPreview = result; }), true)));
  content.append($('div', { class: 'grid-two file-preview' }, exportBox, reviewBox));
  const preview = state.dreamPreview;
  if (!preview) return;
  const review = preview.review;
  const box = $('section', { class: 'panel file-preview' }, $('h2', {}, '逐条检查本次变更'), paragraph(preview.file_name), line('整理任务', review.job_id));
  for (const diagnostic of review.diagnostics || []) box.append(hint(diagnostic, true));
  if (review.already_applied) box.append(hint('这份结果已经保存。'));
  const names = { add: '新增记忆', update: '更新记忆', supersede: '替代旧记忆', noop: '保持原样', conflict: '存在冲突' };
  const changes = $('div', { class: 'changes' });
  for (const change of review.changes || []) changes.append($('div', { class: 'change' }, $('span', { class: 'tag' }, names[change.operation] || change.operation), change.before ? $('div', { class: 'before' }, `之前\n${recordText(change.before)}`) : null, change.after ? $('div', { class: 'after' }, `之后\n${recordText(change.after)}`) : null, $('div', { class: 'ref' }, (change.after?.source_refs || []).join(' · '))));
  box.append(changes);
  const approval = $('input', { type: 'checkbox', id: 'protected-approval' });
  if (review.requires_protected_approval) box.append($('label', { class: 'check', for: 'protected-approval' }, approval, '本次会修改受保护记忆。我已逐条审阅并明确同意这些变更。'));
  box.append($('div', { class: 'button-row' }, button('取消审阅', () => cancelPreviews()), ...(review.can_apply && !review.already_applied ? [button('保存这些记忆', () => {
    const protectedApproved = approval.checked;
    if (review.requires_protected_approval && !protectedApproved) { showNotice('请先勾选受保护记忆的额外确认', true); return; }
    confirmDialog('保存整理结果', `将本次 ${review.changes.length} 项变更写入 ${state.vault.display_name}。来源或版本发生变化时会停止，不会覆盖新内容。`, () => run('正在保存记忆…', async () => { const receipt = await invoke('apply_dream', { sessionId: state.vault.session_id, previewId: preview.preview_id, approveProtected: protectedApproved }); state.dreamPreview = null; state.results = []; await refreshStatus(); showNotice(`已保存 ${receipt.changes.length} 条记忆变更`); }), '确认保存');
  }, true)] : [])));
  content.append(box);
}
function connectPage() {
  content.append(heading('连接你的 AI 工具', '先选资料范围，再连接浏览器或本地 Agent。连接成功后，已保存的对话可在同一资料库中被查到。'));
  content.append($('div',{class:'toolbar'},$('label',{},'允许此客户端使用的资料范围'),scopeSelect()));
  const extension=panel($('h2',{},'浏览器扩展'),paragraph('扩展可在网页上预览、保存和导出对话；没有本机连接时仍能导出文件。连接后可直接保存到当前资料库。'));
  const id=$('input',{placeholder:'粘贴浏览器扩展页显示的32位ID','aria-label':'RecallCard扩展ID',value:state.extensionId||''});id.addEventListener('input',()=>state.extensionId=id.value.trim());
  const browser=$('select',{'aria-label':'浏览器'},$('option',{value:'chromium'},'Chromium'),$('option',{value:'chrome'},'Google Chrome'),$('option',{value:'brave'},'Brave'));browser.value=state.browser||'chromium';browser.addEventListener('change',()=>state.browser=browser.value);
  const capture=$('input',{type:'checkbox',id:'allow-browser-capture'});capture.checked=Boolean(state.allowBrowserCapture);capture.addEventListener('change',()=>state.allowBrowserCapture=capture.checked);
  extension.append(paragraph('1. 正常安装 RecallCard 扩展，在浏览器扩展管理页复制它的 ID'),$('label',{},'2. 选择浏览器并填写扩展 ID'),browser,id,$('label',{class:'check',for:'allow-browser-capture'},capture,'同时允许扩展把我预览确认的对话保存到此范围'),button('连接此浏览器',()=>run('正在配置本机连接…',async current=>{const result=await invoke('install_browser_connection',{...args(),extensionId:state.extensionId||'',browser:state.browser||'chromium',allowCapture:Boolean(state.allowBrowserCapture)});if(current()&&result){state.connectionResult=result;showNotice('本机连接已注册，请到扩展点击“检查连接”');}}),true));
  if(state.connectionResult)extension.append(hint(state.connectionResult.note),line('保存对话',state.connectionResult.capture_enabled?'已允许':'未允许'),paragraph('3. 回到扩展点击“检查连接”，看到资料库名称后，再预览并保存当前会话'));
  extension.append(paragraph('若浏览器提示策略禁止安装，应用不会改变该策略。仍可使用导出文件和桌面导入。'));
  const agent=panel($('h2',{},'本地 Agent / MCP'),paragraph('为支持 MCP 的客户端生成当前资料库的只读配置。配置完成后，客户端可按需查找原话和出处。'),button('生成客户端配置',()=>run('正在生成受限读取配置…',async current=>{const value=await invoke('prepare_client_config',args());if(current())state.clientConfig=JSON.stringify(value,null,2);}),true));
  if(state.clientConfig){const preview=$('textarea',{rows:9,readonly:true,'aria-label':'MCP客户端配置'});preview.value=state.clientConfig;agent.append(preview,button('复制客户端配置',()=>copyText(state.clientConfig)),hint('将配置加入你选定客户端的 MCP 设置，保留原有服务。该客户端可能把读取的资料发送给其模型服务，请先确认范围。'),paragraph('添加后先让客户端调用 bootstrap，再搜索刚保存对话中的一句原话。这里生成配置不等于宿主已经连接。'));}
  content.append($('div',{class:'grid-two file-preview'},extension,agent),panel($('h2',{},'资料库状态'),line('当前资料库',state.vault.display_name),line('已保存记录 / 长期记忆',`${state.vault.event_count} / ${state.vault.memory_count}`),line('完整性检查',state.vault.health?.ok?'通过':'需要检查'),button('重新检查',()=>run('正在检查资料库…',refreshStatus),false,'small')));
}
function render() {
  document.querySelector('#navigation').replaceChildren(...pages.map(([id, title, icon]) => $('button', { class: state.page === id ? 'active' : '', 'aria-current': state.page === id ? 'page' : null, onclick: () => navigate(id) }, $('span', { class: 'symbol', 'aria-hidden': 'true' }, icon), title)));
  document.querySelector('#location').textContent = pages.find(p => p[0] === state.page)?.[1] || '概览';
  document.querySelector('#vault-badge').textContent = state.vault ? shorten(state.vault.display_name, 23) : '尚未打开资料库';
  document.querySelector('#switch-vault').textContent = state.vault ? '切换资料库' : '打开资料库';
  content.replaceChildren();
  if (state.page !== 'home' && !state.vault) needsVault();
  else ({ home, conversations: conversationPage, search: searchPage, import: importPage, dream: dreamPage, connect: connectPage })[state.page]();
  setBusy();
}
document.querySelector('#switch-vault').addEventListener('click', vaultChooser);
document.querySelector('.brand').addEventListener('click', event => { event.preventDefault(); navigate('home'); });
document.addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key === 'k' && !state.busy) { event.preventDefault(); navigate('search'); document.querySelector('#query')?.focus(); } });
render();
