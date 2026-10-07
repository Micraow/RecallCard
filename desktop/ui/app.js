import { pages, shorten, displayDate, recordText, recordRef, newState, activateVault, resetScope, scopeOptions, nativeInstructions, displayState, importSelectionStats, importCoverageLines } from './model.js';
import { createMemoryManagement, invalidateMemoryContent } from './memory-management.js';
import { createBackground } from './background.js';
import { createImportTasks } from './import-tasks.js';
import { createContextSelection } from './context-selection.js';
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
  captureView(); state.skipCapture = true;
  state.busy = true;
  const epoch = state.epoch;
  setBusy();
  document.querySelector('#operation').textContent = label;
  try { await action(() => epoch === state.epoch); }
  catch (error) { showNotice(typeof error === 'string' ? error : error.message || '操作未完成，请重试', true); }
  finally { state.busy = false; render(); state.skipCapture = false; }
}
function setBusy() {
  document.querySelectorAll('button, input, select, textarea').forEach(n => { n.disabled = state.busy || n.getAttribute('data-disabled') === 'true'; });
  if (importTasks.active()) document.querySelectorAll('#navigation button, #global-search input, #global-search button, #global-scope select, #switch-vault, #connect-button').forEach(node => { node.disabled = true; });
  document.querySelector('#operation').className = state.busy ? 'busy-mark' : '';
  if (!state.busy) document.querySelector('#operation').textContent = '准备就绪';
}
const roleLabel = role => ({ user: '用户原话', assistant: 'AI 回复', tool: '工具结果', system: '系统消息' })[role] || '角色未知';
function actionButton(text, action, name, primary = false) { const node = button(text, action, primary); node.dataset.action = name; return node; }
function captureView() {
  const key = content.dataset.view;
  if (!key) return;
  state.viewport[key] = { ...state.viewport[key], ...Object.fromEntries([...content.querySelectorAll('[data-scroll]')].map(node => [node.dataset.scroll, node.scrollTop])) };
}
function restoreView() {
  const saved = state.viewport[content.dataset.view] || {};
  for (const node of content.querySelectorAll('[data-scroll]')) node.scrollTop = saved[node.dataset.scroll] || 0;
}
function resetReaderView(key = content.dataset.view) {
  state.viewport[key] = { ...state.viewport[key], reader: 0, continuation: 0 };
  if (content.dataset.view === key) {
    for (const node of content.querySelectorAll('[data-scroll="reader"], [data-scroll="continuation"]')) node.scrollTop = 0;
  }
}
function discardBefore(action) {
  if (!state.memory.mode && !state.background.draft) { action(); return; }
  modal.replaceChildren($('h2', { id: 'modal-title' }, '还有未保存的更改'), paragraph('离开会放弃这次更改。继续编辑会保留当前输入。'), $('div', { class: 'button-row' }, button('继续编辑', () => modal.close()), button('放弃更改', () => { modal.close(); memoryManagement.discard(); background.discard(); action(); }, true)));
  modal.showModal();
}
async function enterPage(page) {
  captureView();
  if (['conversations', 'memories'].includes(page)) state.workspace = page;
  if (page === 'home') page = state.vault ? 'conversations' : 'home';
  state.page = page;
  if (page !== 'search' && state.contextSelection) { state.contextSelection.open = false; contextSelection.invalidate(); }
  if (!['conversations', 'search'].includes(page)) state.continuationOpen = false;
  if (!state.vault || !['search', 'conversations', 'memories'].includes(page)) render();
  content.focus({ preventScroll: true });
  if (!state.vault) return;
  if (page === 'search') await loadRecords(true);
  if (page === 'conversations') await loadConversations(state.conversationListOffset || 0, true);
  if (page === 'memories') {
    if (state.memoryFilter !== 'all') await background.load();
    else await memoryManagement.load(state.memory.offset, true);
  }
}
function navigate(page) {
  if (state.busy) return;
  if (page !== 'import' && importTasks.active()) { showNotice('请先暂停导入，再查看其他资料'); return; }
  if (page === 'background') { showBackground(false); return; }
  discardBefore(async () => {
    if (state.vault && state.page !== page) {
      await cancelPreviews();
      memoryManagement.discard(); background.discard();
    }
    await enterPage(page);
  });
}
function showBackground(selectedOnly = false, selectedId = null) {
  if (state.busy) return;
  discardBefore(async () => {
    captureView(); memoryManagement.discard(); background.discard();
    state.memoryFilter = selectedOnly ? 'selected' : 'background';
    state.workspace = 'memories'; state.page = 'memories'; state.mobileDetail = false; state.continuationOpen = false;
    render(); await background.load();
    const row = state.background.rows.find(item => item.id === selectedId); if (row) await background.open(row);
  });
}
function memoryToolbar() {
  const toolbar = $('div', { class: 'toolbar memory-tabs' },
    actionButton('全部记忆', () => { discardBefore(() => { state.memoryFilter = 'all'; enterPage('memories'); }); }, 'memory-all'),
    actionButton('已选背景', () => showBackground(true), 'background-selected'),
    actionButton('选择背景', () => showBackground(false), 'background-select'),
    button('整理记忆', () => navigate('dream'), false, 'small quiet'));
  for (const [index, node] of [...toolbar.querySelectorAll('[data-action]')].entries()) { const active = state.memoryFilter === ['all', 'selected', 'background'][index]; node.classList.toggle('filter-active', active); node.setAttribute('aria-pressed', String(active)); }
  return toolbar;
}
function detailBack(action) { return actionButton('返回列表', () => { state.mobileDetail = false; if (action) action(); else render(); }, 'back-to-list'); }
function technicalDetails(...items) { return $('details', { class: 'technical-details' }, $('summary', {}, '详细信息'), ...items); }
function locateEvent(reference, continuation = false) { if (!state.busy) discardBefore(() => locateEventNow(reference, continuation)); }
async function locateEventNow(reference, continuation = false) {
  await run('正在定位原始会话…', async current => {
    const previous = state.page;
    const location = await invoke('event_location', { ...args(), reference });
    if (!current()) return;
    const ref = location.conversation_ref || location.session_ref;
    const result = await invoke('conversation_messages', { ...args(), conversationRef: ref, offset: location.offset || 0 });
    if (!current()) return;
    captureView(); state.readerReturn = previous;
    resetReaderView('conversations:');
    const preservedGoal = state.conversation?.session_ref === ref ? state.continuationGoal : ''; state.resumeConversation = null;
    state.conversation = { session_ref: result.session_ref || ref, title: result.title || location.conversation_title || location.title || '原始会话', platform: result.platform || location.platform, message_count: result.total ?? location.total };
    state.conversationRows = result.messages || []; state.conversationOffset = location.offset || 0;
    state.conversationNext = result.next_offset; state.conversationOrderKnown = result.order_known; rememberBranches(result, ref);
    state.focusedEvent = reference; state.continuation = null; state.continuationGoal = preservedGoal; state.continuationOpen = continuation;
    state.page = 'conversations'; state.workspace = 'conversations'; state.mobileDetail = true;
  });
}
function workspaceCurrent() {
  const epoch = state.epoch, session = state.vault?.session_id, scope = state.scope;
  return () => state.epoch === epoch && state.vault?.session_id === session && state.scope === scope;
}
let rememberQueue = Promise.resolve();
async function rememberWorkspace(current = workspaceCurrent()) {
  if (!state.vault || !current()) return;
  const payload = args();
  const operation = rememberQueue.then(async () => {
    if (!current()) return;
    try { await invoke('remember_workspace', payload); }
    catch (error) { if (current()) showNotice('资料已打开，但暂时无法记住这个位置。下次仍可手动打开。', true); }
  });
  // 串行保存并跳过已经过期的设置，避免较早范围的迟到写入覆盖新选择。
  rememberQueue = operation.catch(() => {});
  await operation;
}
async function openWorkspaceContents(current = workspaceCurrent()) {
  try {
    const jobs = await invoke('list_import_jobs', args());
    if (!current()) return;
    if (importTasks.restore(jobs)) { state.page = 'import'; return; }
  } catch (error) {
    if (!current()) return;
    showNotice('资料已打开，但上次导入的状态暂时无法核实。可在添加资料中重新检查。', true);
  }
  if (!current()) return;
  if (!state.vault.event_count) { state.page = 'import'; return; }
  state.page = 'conversations';
  const listed = await invoke('list_conversations', { ...args(), offset: 0 });
  if (!current()) return;
  state.conversations = listed.conversations || []; state.conversationTotal = listed.total; state.conversationListNext = listed.next_offset;
  if (state.conversations[0]) await readConversation(state.conversations[0], 0, current);
}
async function restoreWorkspace() {
  if (!window.__TAURI__?.core?.invoke) return;
  await run('正在打开上次的资料…', async initialCurrent => {
    const restored = await invoke('restore_workspace', {});
    if (!initialCurrent() || !restored) return;
    if (!restored.vault || typeof restored.vault.session_id !== 'string' || !restored.vault.session_id || typeof restored.scope !== 'string' || !restored.scope.trim() || restored.scope.length > 128 || /[\u0000-\u001f]/.test(restored.scope)) throw new Error('上次的位置无法核实，请重新选择资料库');
    activateVault(state, restored.vault); resetScope(state, restored.scope);
    await openWorkspaceContents(workspaceCurrent());
  });
}
async function chooseVault(create) {
  modal.close();
  await run(create ? '正在创建资料库…' : '正在打开资料库…', async initialCurrent => {
    let vault;
    try { vault = await invoke('choose_vault', { create }); }
    catch (error) {
      if (initialCurrent()) { activateVault(state, { scopes: [] }); state.vault = null; }
      throw error;
    }
    if (!vault || !initialCurrent()) return;
    activateVault(state, vault); const current = workspaceCurrent(); showNotice(`已打开 ${vault.display_name}`);
    await rememberWorkspace(current);
    if (current()) await openWorkspaceContents(current);
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
    memoryManagement.clearReview(); background.discard(); importTasks.clearPreview();
    state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.dreamPreview = null; state.dreamEvidence = {}; state.notePreview = null;
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    if (next) next();
  });
}
function scopeSelect() {
  const node = $('select', { 'aria-label': '资料范围' });
  for (const scope of scopeOptions(state.vault, state.scope)) { const option = $('option', { value: scope }, scopeLabel(scope)); option.selected = scope === state.scope; node.append(option); }
  node.addEventListener('change', () => { const scope = node.value; node.value = state.scope; discardBefore(async () => { await cancelPreviews(() => resetScope(state, scope)); await rememberWorkspace(); if (state.page === 'search') loadRecords(); else if (state.page === 'conversations') loadConversations(); else if (state.page === 'memories') memoryManagement.load(); }); });
  return node;
}
function needsVault() {
  content.append(heading('打开资料库', '选择保存记忆的文件夹，即可导入、检索和审阅资料。'), $('div', { class: 'empty' }, $('div', { class: 'empty-icon' }, '▧'), $('h2', {}, '选择保存资料的位置'), paragraph('从已有资料库继续，或创建一个新的本地空间。'), button('选择资料库', vaultChooser, true)));
}
function home() {
  content.append($('section', { class: 'onboarding' }, $('h1', {}, '把以前的对话接着用'), paragraph('导入历史，找到原话，换一个 AI 继续。'), button('开始使用', () => run('正在准备本机保存位置…', async initialCurrent => {
    const vault = await invoke('open_default_workspace', {}); if (!initialCurrent()) return;
    activateVault(state, vault); const current = workspaceCurrent();
    await rememberWorkspace(current);
    if (current()) await openWorkspaceContents(current);
  }), true), paragraph('默认保存在本机，不需要账号或 API。'), $('div', { class: 'button-row' }, button('打开已有资料库', () => chooseVault(false)), button('创建新资料库', () => chooseVault(true)))));
}
async function readRecordValue(item, current) {
  const reference = recordRef(item);
  const result = await invoke('read_record', { ...args(), reference });
  if (!current()) return;
  const first = result.results?.[0];
  if (!first) { if (result.truncated || result.pending_refs?.length) { state.selected = { ...item, truncated: true, sources_loaded: false }; state.sources = []; showNotice('记录较长，先显示部分内容。完整原文保存在资料库文件中'); return; } throw new Error('这条资料已不可见，请重新查找'); }
  let records = [], sourcesTruncated = false;
  if (reference.startsWith('memory:')) {
    const sources = await invoke('read_sources', { ...args(), reference });
    if (!current()) return;
    records = sources.results || []; sourcesTruncated = Boolean(sources.truncated);
    if (sources.truncated) showNotice('出处较长，完整内容保存在资料库文件中');
  }
  state.selected = { ...item, ...first, text_truncated: false, truncated: Boolean(result.truncated || first.truncated), ref: reference, sources_loaded: true, sources_truncated: sourcesTruncated };
  state.sources = records;
}
async function loadRecords(preserve = false) {
  await run('正在查找本地资料…', async current => {
    const selected = preserve ? state.selected : null;
    if (!preserve) { resetReaderView('search:'); state.viewport['search:'].list = 0; contextSelection.clear(); }
    else contextSelection.invalidate();
    state.selected = null; state.sources = []; state.continuation = null;
    state.results = []; state.searchLoaded = false; state.searchError = false; state.resultNote = '正在查找本地资料…'; render();
    const query = state.query.trim();
    let result;
    try { result = await invoke(query ? 'search_records' : 'browse_records', { ...args(), target: state.target, ...(query ? { query } : {}) }); }
    catch (error) { if (current()) { state.searchError = true; state.resultNote = '查找未完成'; } throw error; }
    if (!current()) return;
    state.results = result.results || []; state.searchLoaded = true; state.resultQuery = query;
    state.resultNote = result.truncated ? '结果较多，请缩小关键词范围' : `${state.results.length} 条可见资料`;
    if (preserve) contextSelection.refreshResults();
    const freshSelected = selected && state.results.find(item => recordRef(item) === recordRef(selected));
    if (freshSelected) await readRecordValue(freshSelected, current);
  });
}
async function readRecord(item) {
  await run('正在读取记录与出处…', async current => {
    if (recordRef(state.selected) !== recordRef(item)) resetReaderView('search:');
    state.selected = null; state.sources = []; render();
    await readRecordValue(item, current);
    if (current()) state.mobileDetail = true;
  });
}
function sourceRecord(event) {
  const data = event.event || event.record || event;
  const ref = recordRef(event) || (data.id ? `event:${data.id}` : '');
  return $('article', { class: 'source-record' },
    $('div', { class: 'result-meta' }, $('strong', {}, event.conversation_title || data.conversation_title || data.metadata?.conversation_title || data.source?.conversation_title || '原始出处'), $('span', { class: 'tag' }, roleLabel(data.role || event.role))),
    $('div', { class: 'muted' }, `${event.platform || data.source?.platform || '来源未知'} · ${displayDate(data.occurred_at || event.occurred_at)}`),
    $('div', { class: 'body-text' }, recordText(data) || recordText(data.data) || '此来源没有文本正文'),
    ref ? button('定位到这条消息', () => locateEvent(ref), false, 'small') : null,
    technicalDetails(line('引用', ref), data.source?.url ? line('出处地址', data.source.url) : null));
}
function readingPane() {
  if (!state.selected) return $('aside', { class: 'reading-pane empty' }, $('h3', {}, '选择一条搜索结果'), paragraph('查看原话、相邻消息和出处。'));
  const item = state.selected; const reference = recordRef(item); const isMemory = reference.startsWith('memory:');
  const data = item.record || item.event || item.memory || item;
  const title = item.conversation_title || item.title || (isMemory ? '长期记忆' : '原始记录');
  const header = $('div', { class: 'reader-heading' }, detailBack(), $('div', { class: 'reader-title' }, $('h2', {}, title), $('span', { class: 'muted' }, `${item.platform || data.source?.platform || (isMemory ? '整理所得' : '来源未知')} · ${isMemory ? displayState(data.status || item.state) : roleLabel(item.role || data.role)}`)));
  if (!isMemory) header.append(actionButton('带到另一个AI', () => locateEvent(reference, true), 'open-continuation', true));
  const body = $('div', { class: 'reader-scroll', 'data-scroll': 'reader' },
    $('div', { class: 'body-text' }, recordText(data) || recordText(item) || '这条资料没有保存正文'),
    line('原始时间', displayDate(data.occurred_at || item.occurred_at)));
  if (item.text_truncated || item.truncated) body.append(hint('这里只显示部分内容，完整原文保存在资料库中。'));
  if (!isMemory) body.append(button('查看相邻消息', () => locateEvent(reference), false, 'small'));
  else {
    if (item.sources_loaded === false) body.append(hint('这条记忆的出处尚未读取，不能据此判断有无出处。'), button('重新读取资料与出处', () => readRecord(item), false, 'small'));
    else {
      const sourceCount = state.sources.reduce((n, source) => n + (source.events || source.sources || source.records || [source]).length, 0);
      const sourceLabel = item.sources_truncated && !sourceCount ? '原始出处尚未读出' : `${item.sources_truncated ? '已读原始出处' : '原始出处'} · ${sourceCount} 条`;
      const sources = $('details', { class: 'source-details' }, $('summary', {}, sourceLabel));
      if (item.sources_truncated) sources.append(hint('部分出处尚未读出，这不是完整的来源数量。'));
      for (const source of state.sources) for (const event of source.events || source.sources || source.records || [source]) sources.append(sourceRecord(event));
      body.append(sources);
    }
    body.append(button('管理这条记忆', () => { state.pendingMemoryId = data.id || reference.slice(7).split('@')[0]; state.memoryFilter = 'all'; navigate('memories'); }, false, 'small'));
  }
  const chosen = state.selectedRefs.includes(reference);
  body.append($('details', { class: 'organize-details' }, $('summary', {}, '用于整理记忆'), button(chosen ? '已选择 · 点击移除' : '选择这条资料', () => changeDreamSources(chosen ? state.selectedRefs.filter(r => r !== reference) : [...state.selectedRefs, reference]), false, 'small'), state.selectedRefs.length ? button(`前往整理（${state.selectedRefs.length} 条）`, () => navigate('dream'), false, 'small') : null), technicalDetails(line('引用', reference), line('记录时间', displayDate(data.recorded_at || data.captured_at)), line('状态', displayState(data.status || data.state || item.state))));
  return $('aside', { class: 'reading-pane reader-pane' }, header, body);
}
function searchPage() {
  const target = $('select', { 'aria-label': '资料类型' }, $('option', { value: 'all' }, '所有资料'), $('option', { value: 'events' }, '原始记录'), $('option', { value: 'memories' }, '长期记忆'));
  target.value = state.target; target.addEventListener('change', () => { state.target = target.value; loadRecords(); });
  content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '搜索结果'), $('span', { class: 'muted' }, state.resultNote || '在当前范围内查找'), target, state.results.length ? button('带走这些资料', () => contextSelection.open(), true) : null, button('返回工作区', () => navigate(state.workspace), false, 'small')));
  const results = $('div', { class: 'results list-scroll', 'data-scroll': 'list', 'aria-label': '搜索结果' });
  if (!state.results.length) results.append(state.searchError
    ? $('div', { class: 'empty' }, $('h3', {}, '这次查找未完成'), paragraph('旧结果已移除。请重试，或修改上方查询。'), button('重试查找', () => loadRecords(Boolean(state.contextSelection)), true))
    : $('div', { class: 'empty' }, $('h3', {}, state.query ? '没有找到匹配资料' : '还没有可见资料'), paragraph('试试更短的原话，或检查顶部资料范围。')));
  for (const item of state.results) {
    const ref = recordRef(item); const isMemory = ref.startsWith('memory:');
    const source = item.source_summary?.sources?.[0];
    const resultButton = $('button', { class: `result-card ${recordRef(state.selected) === ref ? 'selected' : ''}`, onclick: () => { if (state.contextSelection) state.contextSelection.open = false; contextSelection.invalidate(); readRecord(item); }, 'aria-pressed': String(recordRef(state.selected) === ref), 'data-reference': ref },
      $('strong', {}, item.conversation_title || item.title || (isMemory ? shorten(recordText(item), 52) : '原始记录')),
      $('span', { class: 'result-excerpt' }, shorten(recordText(item), 190)),
      $('span', { class: 'muted' }, `${isMemory ? '记忆' : roleLabel(item.role)} · ${item.platform || source?.platform || '来源未知'} · ${displayDate(item.occurred_at || item.valid_from)}`));
    const check = contextSelection.checkbox(item);
    results.append(check ? $('div', { class: `context-result-row ${check.querySelector('input').checked ? 'included' : ''}` }, check, resultButton) : resultButton);
  }
  content.append($('div', { class: `results-layout workspace-split ${state.mobileDetail && (state.selected || contextSelection.isOpen()) ? 'show-detail' : ''}` }, results, contextSelection.isOpen() ? contextSelection.pane() : readingPane()));
}
async function refreshStatus() { state.vault = await invoke('vault_status', { sessionId: state.vault.session_id }); }
function discardChangedImport(error) {
  if (/文件已改变|清单已失效|资料库会话已失效/.test(String(error?.message || error))) {
    state.importSelection = null; state.importSelectedIds = []; state.importPreview = null;
  }
}
function importPage() {
  content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '导入会话'), button('返回会话', () => navigate('conversations'), false, 'small')));
  if (!state.importPreview && !state.importSelection && !state.notePreview) content.append(importTasks.pane());
  if (state.importJobs?.current || state.importJobs?.preview) return;
  const noteSection = $('details', { id: 'note-import-details', class: 'panel note-import', open: Boolean(state.noteOpen || state.notePreview) }, $('summary', {}, '写一条新笔记'));
  noteSection.addEventListener('toggle', () => { if (noteSection.isConnected) state.noteOpen = noteSection.open; });
  const text = $('textarea', { id: 'note-content', rows: 5, maxlength: 65536, placeholder: '例如：我希望项目说明优先使用中文。这里写你自己的新补充，不粘贴多角色聊天。', 'aria-label': '资料正文' }, state.noteText || '');
  text.addEventListener('input', () => { state.noteText = text.value; });
  const previewNote = () => run('正在准备预览…', async current => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.dreamPreview = null; state.dreamEvidence = {}; state.notePreview = null;
    const preview = await invoke('preview_note', { ...args(), content: state.noteText || '' });
    if (current()) state.notePreview = preview;
  });
  if (!state.notePreview) {
    noteSection.append(panel($('h2', {}, '或者，写下你自己的补充说明'), paragraph('这里保存的是你本人的新笔记。完整聊天请使用上面的来源导入，以保留用户/AI角色和原始时间。'), text, $('div', { class: 'button-row' }, button('预览并保存', previewNote, true))));
  } else {
    const note = state.notePreview;
    noteSection.append(panel($('h2', {}, '确认保存的内容'), $('div', { class: 'body-text note-preview' }, note.content), note.redacted ? hint('检测到可能的敏感字段，已在预览中遮蔽。') : null,
      $('div', { class: 'button-row' }, button('返回修改', () => cancelPreviews()), button('确认保存记录', () => run('正在保存记录…', async () => {
        await invoke('confirm_note', { sessionId: state.vault.session_id, previewId: note.preview_id });
        background.discard(); state.continuation = null;
        state.notePreview = null; state.noteText = ''; state.results = [];
        await refreshStatus(); showNotice('记录已保存，可以开始查找'); state.query = ''; state.page = 'search'; state.workspace = 'conversations';
        const listed = await invoke('browse_records', { ...args(), target: state.target }); state.results = listed.results || []; state.selected = null;
      }), true))));
  }
  const fileDetails = $('details', { id: 'file-import-details', class: 'panel import-file-options', open: Boolean(state.fileImportOpen) }, $('summary', {}, '高级：按会话选择或指定格式'));
  fileDetails.addEventListener('toggle', () => { if (fileDetails.isConnected) state.fileImportOpen = fileDetails.open; });
  content.append(fileDetails);
  if (!state.importSelection && !state.importPreview) content.append(noteSection);
  const format = $('select', { id: 'import-format', 'aria-label': '导入格式' }, $('option', { value: 'auto' }, '自动识别支持的会话文件'), $('option', { value: 'recallcard-conversation' }, 'RecallCard 扩展导出的会话 JSON'), $('option', { value: 'chatgpt-export' }, 'ChatGPT 备份 ZIP / 会话 JSON'), $('option', { value: 'deepseek-export' }, 'DeepSeek 官方历史导出'), $('option', { value: 'claude-code' }, 'Claude Code 对话 JSONL'), $('option', { value: 'manual-jsonl' }, 'RecallCard 标准 JSONL'));
  format.value = state.importFormat || 'auto';
  format.addEventListener('change', () => { const next = format.value; cancelPreviews(() => { state.importFormat = next; }); });
  const scope = $('input', { id: 'import-scope', value: state.scope, placeholder: 'personal', 'aria-label': '导入范围' });
  scope.addEventListener('change', () => { const next = scope.value.trim(); cancelPreviews(() => resetScope(state, next)); });
  const choose = () => run('正在读取导入预览…', async current => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.dreamPreview = null; state.dreamEvidence = {}; state.notePreview = null;
    const result = await invoke('pick_import', { ...args(), format: state.importFormat || 'auto' });
    if (current() && result) { state.fileImportOpen = false; if (result.selection_id) { state.importSelection = result; state.importSelectedIds = []; } else state.importPreview = result; }
  });
  fileDetails.append($('div', { class: 'form-stack file-options-fields' }, $('div', {}, $('label', { for: 'import-format' }, '文件格式'), format, $('div', { class: 'field-note' }, 'ZIP 无需解压：先查看会话列表、勾选本批会话，再审查消息样本')), $('details', {}, $('summary', {}, '高级：资料分类'), $('div', {}, $('label', { for: 'import-scope' }, '分类编号'), scope, $('div', { class: 'field-note' }, '默认 personal；已有资料可沿用原分类编号'))), paragraph('ZIP 最多 64 MiB；单个 JSON / JSONL 最多 16 MiB。官方导出每批最多 100000 条消息，相同消息自动去重。'), button('选择文件并预览', choose, true)));
  const selection = state.importSelection;
  if (selection && !state.importPreview) {
    const stats = importSelectionStats(selection, state.importSelectedIds);
    const selectionBox = $('section', { class: 'panel file-preview archive-selection' }, $('h2', {}, '选择本批要导入的会话'),
      $('div', { class: 'file-name' }, selection.file_name),
      line('备份可导入范围', `${selection.coverage.conversations_available} 个会话 / ${selection.coverage.events_available} 条消息`),
      hint('尚未写入资料库。官方导出每批最多 100000 条消息；超出时请分批选择。超限会明确停止，不会截断消息。'));
    const coverage = $('details', {}, $('summary', {}, '文件覆盖范围与跳过内容'));
    for (const detail of importCoverageLines(selection.coverage)) coverage.append(paragraph(detail));
    selectionBox.append(coverage);
    const setSelection = ids => { state.importSelectedIds = ids; render(); };
    selectionBox.append($('div', { class: 'button-row' },
      button('选择全部可导入会话', () => setSelection(selection.conversations.filter(c => c.event_count > 0).map(c => c.selection_key || c.source_id)), false, 'small'),
      button('清空选择', () => setSelection([]), false, 'small')));
    for (const conversation of selection.conversations) {
      const check = $('input', { type: 'checkbox', 'aria-label': `选择会话：${conversation.title || conversation.source_id}`, 'data-disabled': conversation.event_count === 0 ? 'true' : null });
      check.checked = state.importSelectedIds.includes(conversation.selection_key || conversation.source_id);
      check.addEventListener('change', () => setSelection(check.checked ? [...state.importSelectedIds, conversation.selection_key || conversation.source_id] : state.importSelectedIds.filter(id => id !== (conversation.selection_key || conversation.source_id))));
      selectionBox.append($('label', { class: 'check sample' }, check, $('span', {}, $('strong', {}, conversation.title || '未命名会话'),
        paragraph(`${conversation.event_count} 条消息 · 用户 ${conversation.user_messages} · 助手 ${conversation.assistant_messages} · 工具 ${conversation.tool_messages}`),
        $('span', { class: 'ref' }, conversation.source_id))));
    }
    selectionBox.append(line('本批已选', `${stats.conversations} 个会话 / ${stats.events} 条消息`));
    if (stats.events > 100000) selectionBox.append(hint('本批超过 100000 条，请取消部分会话后预览。当前没有写入任何消息。', true));
    const previewSelected = button('预览所选会话', () => {
      if (!stats.valid) return;
      return run('正在核对备份并生成所选会话预览…', async current => {
        state.importPreview = null;
        try {
          const result = await invoke('preview_import_selection', { sessionId: state.vault.session_id, selectionId: selection.selection_id, sourceIds: [...state.importSelectedIds] });
          if (current()) { state.importPreview = result; state.fileImportOpen = false; }
        } catch (error) { discardChangedImport(error); throw error; }
      });
    }, true);
    previewSelected.setAttribute('data-disabled', String(!stats.valid));
    selectionBox.append($('div', { class: 'button-row' }, button('取消这次导入', () => cancelPreviews()), previewSelected));
    content.append(selectionBox);
  }
  const preview = state.importPreview;
  if (!preview) return;
  const box = $('section', { class: 'panel file-preview' }, $('div', { class: 'section-heading' }, $('h2', {}, '确认导入'), $('span', { class: 'badge' }, '尚未写入')), $('div', { class: 'file-name' }, preview.file_name), line('解析到的记录', `${preview.event_count} 条`), line('写入资料库', state.vault.display_name), line('写入范围', preview.scope), line('文件大小', `${(preview.byte_count / 1024).toFixed(1)} KiB`));
  if (preview.redacted_event_count) box.append(hint(`${preview.redacted_event_count} 条记录包含已遮蔽字段，请检查下方预览。`));
  box.append(paragraph(preview.warning));
  if (preview.conversations?.length) box.append(line('本批会话', preview.conversations.map(c => c.title || c.source_id).join('、')));
  const coverage = $('details', {}, $('summary', {}, selection ? '原文件覆盖范围（包含未选会话）' : '导入覆盖范围'));
  for (const detail of importCoverageLines(preview.coverage)) coverage.append(paragraph(detail));
  if (preview.coverage) box.append(coverage);
  for (const sample of preview.samples || []) {
    const sourceTitle = preview.conversations?.find(c => c.source_id === sample.source?.conversation_id)?.title || sample.source?.conversation_id;
    box.append($('div', { class: 'sample' }, $('span', { class: 'tag' }, ({ user: '用户', assistant: '助手', tool: '工具', system: '系统' })[sample.role] || sample.role),
      paragraph(`${sample.source?.platform || '来源未知'} · 原始时间：${displayDate(sample.occurred_at)}`),
      sourceTitle ? paragraph(`来源会话：${sourceTitle}`) : null, paragraph(sample.content)));
  }
  if (preview.truncated) box.append(paragraph('预览仅展示部分内容，导入确认后会处理全部已解析记录。'));
  const importEpoch = state.epoch;
  const importSession = state.vault.session_id;
  const importScope = state.scope;
  const confirmChosenImport = () => {
    // 当前预览上的明确按钮就是写入动作；失效或已移除的按钮不能重新提交。
    if (state.busy || !confirmImport.isConnected || state.page !== 'import' || state.importPreview !== preview
      || state.epoch !== importEpoch || state.vault?.session_id !== importSession || state.scope !== importScope) return;
    return run('正在导入，完成前请勿关闭应用…', async () => {
      state.importPreview = null;
      try {
        const result = await invoke('confirm_import', { sessionId: importSession, previewId: preview.preview_id });
        state.importSelectedIds = []; state.importSelection = null; state.results = []; background.discard(); state.continuation = null; await refreshStatus();
        const batch = result.conversations || [];
        state.importBatch = batch.length ? { conversations: batch, added: result.events_added, duplicates: result.events_duplicates ?? Math.max(0, result.events_seen - result.events_added) } : null;
        const listed = await invoke('list_conversations', { ...args(), offset: 0 });
        state.conversations = listed.conversations || []; state.conversationTotal = listed.total; state.conversationListNext = listed.next_offset; state.conversationListOffset = 0;
        state.viewport['conversations:'] = { list: 0, reader: 0, continuation: 0 };
        state.conversation = null; state.conversationRows = []; state.page = 'conversations'; state.workspace = 'conversations'; state.mobileDetail = false;
        const first = batch[0] || state.conversations[0]; if (first) await readConversation(first, 0, () => true);
        showNotice(`导入完成：新增 ${result.events_added} 条，重复 ${result.events_duplicates ?? Math.max(0, result.events_seen - result.events_added)} 条`);
      } catch (error) { discardChangedImport(error); throw error; }
    });
  };
  const confirmImport = button(`确认导入 ${preview.event_count} 条记录`, confirmChosenImport, true);
  box.append(paragraph('确认后保存以上记录，已存在的相同记录会跳过。'), $('div', { class: 'button-row' }, button('取消这次导入', () => cancelPreviews()), confirmImport));
  if (selection) box.append(button('返回会话选择', () => run('正在返回会话选择…', async () => {
    state.importPreview = null;
    try { await invoke('return_import_selection', { sessionId: state.vault.session_id, selectionId: selection.selection_id }); } catch (error) { discardChangedImport(error); throw error; }
  })));
  content.append(box);
}
function rememberBranches(result, conversationRef) {
  if (state.branchConversationRef !== conversationRef) { state.continuationBranchRef = ''; state.continuationBranchLabel = ''; }
  state.branchConversationRef = conversationRef;
  state.conversationBranches = result.branch_summary || null;
  if (!state.conversationBranches?.selection_required) { state.continuationBranchRef = ''; state.continuationBranchLabel = ''; }
}
function selectBranch(row) {
  state.continuationBranchRef = row.branch_ref || row.ref; state.continuationBranchLabel = shorten(row.text, 80);
  state.continuation = null; state.continuationOpen = true; state.mobileDetail = true; render();
}
function continuationArgs() {
  return { ...args(), conversationRef: state.conversation.session_ref, goal: state.continuationGoal || '', ...(state.continuationBranchRef ? { branchRef: state.continuationBranchRef } : {}) };
}
async function readConversation(item, offset, current) {
  const result = await invoke('conversation_messages', { ...args(), conversationRef: item.session_ref, offset });
  if (!current()) return;
  if (state.conversation?.session_ref !== item.session_ref) { state.continuationGoal = ''; state.continuation = null; }
  state.conversation = { ...item, ...(result.session_ref ? { session_ref: result.session_ref } : {}), ...(result.title ? { title: result.title } : {}), ...(result.platform ? { platform: result.platform } : {}), ...(result.total != null ? { message_count: result.total } : {}) }; state.conversationRows = result.messages || [];
  rememberBranches(result, item.session_ref); state.conversationOrderKnown = result.order_known; state.conversationOffset = offset; state.conversationNext = result.next_offset;
}
async function loadConversations(offset = 0, preserve = false) {
  offset = Number.isSafeInteger(offset) ? offset : 0;
  if (state.importBatch && !state.importBatch.jobId) offset = 0;
  await run('正在读取已保存的会话…', async current => {
    if (!preserve) resetReaderView('conversations:');
    const selected = preserve ? state.conversation : null;
    const resume = state.resumeConversation;
    const goal = selected ? state.continuationGoal : resume?.goal || ''; const selectedOffset = selected ? state.conversationOffset : resume?.offset || 0;
    state.conversation = null; state.conversationRows = []; state.continuation = null; render();
    const batchJob = state.importBatch?.jobId;
    if (batchJob) { state.importBatch.conversations = []; state.importBatch.error = ''; state.conversations = []; render(); }
    let result;
    try { result = batchJob ? await importTasks.readResults(batchJob, offset) : await invoke('list_conversations', { ...args(), offset }); }
    catch (error) { if (current() && batchJob && batchJob === state.importBatch?.jobId) state.importBatch.error = '这次读取未完成，重新读取后再选择会话。'; throw error; }
    if (!current()) return;
    state.conversationListOffset = offset; state.conversationListNext = result.next_offset;
    state.conversationTotal = result.total; state.conversations = result.conversations || []; state.conversationNote = result.note;
    if (batchJob) {
      Object.assign(state.importBatch, { conversations: state.conversations, total: result.total, added: result.status.events_added, duplicates: result.status.events_duplicates, note: result.note, error: '' });
    } else if (state.importBatch) {
      const refs = new Set(state.importBatch.conversations.map(item => item.session_ref));
      const accessible = [...state.conversations];
      let next = result.next_offset; let previous = offset;
      while (next != null && next > previous && accessible.filter(item => refs.has(item.session_ref)).length < refs.size) {
        const page = await invoke('list_conversations', { ...args(), offset: next }); if (!current()) return;
        accessible.push(...page.conversations || []); previous = next; next = page.next_offset;
      }
      const freshByRef = new Map(accessible.map(item => [item.session_ref, item]));
      state.importBatch.conversations = state.importBatch.conversations.map(item => freshByRef.get(item.session_ref)).filter(Boolean);
    }
    const currentSelected = batchJob ? state.conversations.find(item => item.session_ref === selected?.session_ref) : selected;
    const active = currentSelected || (!batchJob && resume ? state.conversations.find(item => item.session_ref === resume.session_ref) || { session_ref: resume.session_ref, title: '当前会话', platform: '' } : null) || state.importBatch?.conversations[0] || state.conversations[0];
    if (active) {
      try { await readConversation(active, currentSelected || (!batchJob && resume) ? selectedOffset : 0, current); if ((currentSelected || (!batchJob && resume)) && current()) state.continuationGoal = goal; state.resumeConversation = null; }
      catch (error) { state.conversation = null; state.conversationRows = []; state.continuationGoal = ''; state.continuationOpen = false; state.resumeConversation = null; throw error; }
    }
  });
}
async function openConversation(item, offset = 0) {
  await run('正在打开会话…', async current => {
    const previous = state.conversation?.session_ref; state.resumeConversation = null;
    if (previous !== item.session_ref || state.conversationOffset !== offset) resetReaderView('conversations:');
    const goal = previous === item.session_ref ? state.continuationGoal : '';
    state.conversation = null; state.conversationRows = []; state.continuation = null; render();
    await readConversation(item, offset, current);
    if (current()) { state.continuationGoal = goal; state.mobileDetail = true; state.focusedEvent = ''; }
  });
}
async function copyText(text) {
  try { await invoke('write_clipboard', { text }); showNotice(`已复制 · ${scopeLabel(state.scope)}，可粘贴到你选择的 AI`); }
  catch (error) { showNotice(typeof error === 'string' ? error : '系统剪贴板写入失败，请重试', true); }
}
function continuationPane() {
  const goal = $('textarea', { rows: 3, placeholder: '接下来希望 AI 帮你做什么？', 'aria-label': '接下来要做什么' });
  goal.value = state.continuationGoal || '';
  goal.addEventListener('input', () => { state.continuationGoal = goal.value; state.continuation = null; document.querySelector('.copy-continuation')?.setAttribute('disabled', ''); document.querySelector('#continuation-preview')?.remove(); });
  const prepare = () => { if (state.conversationBranches?.selection_required && !state.continuationBranchRef) return; return run('正在准备有来源的交接内容…', async current => {
    state.continuation = null; render();
    const result = await invoke('prepare_continuation', continuationArgs());
    if (current()) state.continuation = result;
  }); };
  const body = $('div', { class: 'continuation-scroll', 'data-scroll': 'continuation' },
    $('label', {}, '下一步目标'), goal,
    $('div', { class: 'handoff-selection' }, $('span', { class: 'muted' }, '本次会话'), $('strong', {}, state.conversation.title), paragraph(`${state.conversation.message_count ?? state.conversationRows.length} 条已保存消息 · ${scopeLabel(state.scope)}`)),
    $('details', { class: 'handoff-settings' }, $('summary', {}, '背景与覆盖范围'), paragraph('带上当前范围中已选择的稳定背景，以及本次会话的有界消息。网站未保存的历史不在其中。'), button('设置随身背景', () => showBackground(false), false, 'small')));
  const branches = state.conversationBranches;
  if (branches?.selection_required) {
    const choices = new Map((branches.branches || []).map(row => [row.branch_ref, row]));
    for (const row of state.conversationRows) if (row.branch?.is_branch_end) choices.set(row.ref, { ...row, branch_ref: row.ref });
    if (state.continuationBranchRef && !choices.has(state.continuationBranchRef)) choices.set(state.continuationBranchRef, { branch_ref: state.continuationBranchRef, text: state.continuationBranchLabel });
    const select = $('select', { id: 'continuation-branch', 'aria-label': '选择要继续的分支' }, $('option', { value: '' }, '选择一条分支的末尾消息'));
    for (const row of choices.values()) select.append($('option', { value: row.branch_ref }, `${roleLabel(row.role)} · ${shorten(row.text, 80)}`));
    select.value = state.continuationBranchRef || '';
    select.addEventListener('change', () => { const row = choices.get(select.value); if (row) selectBranch(row); else { state.continuationBranchRef = ''; state.continuationBranchLabel = ''; state.continuation = null; render(); } });
    body.append($('section', { class: 'handoff-branches' }, $('label', { for: 'continuation-branch' }, `这段对话有 ${branches.branch_count} 条分支`), paragraph('选择想接着聊的那条，只带上它经过的消息，其他回答仍保存在原会话里。'), select,
      branches.branches_truncated ? paragraph('这里只列出部分末尾。返回阅读、翻到相应消息，点击“从此分支继续”可选择其他分支。') : null));
  }
  if (state.continuation) {
    const shown = state.continuation;
    const preview = $('textarea', { rows: 12, readonly: true, 'aria-label': '交接内容预览' }); preview.value = shown.text;
    body.append($('section', { id: 'continuation-preview' }, hint(`已带上 ${shown.message_count} / ${shown.available_messages} 条消息`), shown.truncated ? hint('部分背景或消息因长度限制未带上。展开的正文就是实际复制内容。', true) : null, shown.has_gaps ? hint('所选分支存在省略或当前不可访问的消息，交接中已标明缺口。', true) : null,
      $('details', { class: 'continuation-text', open: true }, $('summary', {}, '检查完整交接内容'), preview)));
  } else body.append(paragraph('生成后检查实际文字，再复制到目标 AI 发送。'));
  const copy = button('复制交接内容', () => run('正在重新核对交接内容…', async current => {
    const shown = state.continuation; state.continuation = null; render();
    const fresh = await invoke('prepare_continuation', continuationArgs());
    if (!current()) return;
    if (!shown || fresh.text !== shown.text) throw new Error('资料或权限已改变，请重新生成并检查交接预览');
    state.continuation = fresh; await copyText(shown.text);
  }), true, 'copy-continuation');
  copy.setAttribute('data-disabled', String(!state.continuation));
  const prepareButton = button('准备交接内容', prepare, !state.continuation); prepareButton.setAttribute('data-disabled', String(Boolean(branches?.selection_required && !state.continuationBranchRef)));
  return $('aside', { id: 'continuation-panel', class: 'continuation-pane', 'aria-label': '带到另一个AI' },
    $('div', { class: 'reader-heading' }, $('h2', {}, '带到另一个AI'), actionButton('返回阅读', () => { state.continuationOpen = false; render(); }, 'close-continuation')),
    body, $('div', { class: 'continuation-actions' }, prepareButton, copy));
}
function conversationPage() {
  content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '会话'), $('span', { class: 'muted' }, `${state.conversationTotal ?? state.conversations.length} 个会话`),
    state.readerReturn ? button('返回之前的阅读', () => { const page = state.readerReturn; state.readerReturn = ''; navigate(page); }, false, 'small') : null,
    button(state.importBatch?.jobId ? '重新读取本批会话' : '刷新已保存会话', () => { if (!state.importBatch?.jobId) state.importBatch = null; loadConversations(state.importBatch?.jobId ? state.conversationListOffset : 0, true); }, false, 'small')));
  if (state.importBatch) content.append($('div', { class: 'import-batch-summary' }, $('strong', {}, `本批导入 · ${state.importBatch.total ?? state.importBatch.conversations.length} 个会话`), $('span', {}, `新增 ${state.importBatch.added} 条 · 重复 ${state.importBatch.duplicates} 条`), button('查看全部会话', () => { state.importBatch = null; loadConversations(0, true); }, false, 'small')));
  if (state.importBatch?.jobId) content.append(state.importBatch.error ? hint(state.importBatch.error, true) : $('details', { class: 'coverage-details' }, $('summary', {}, '本批会话范围'), paragraph(state.importBatch.note || '正在核对本批已保存、当前可查看的会话…')));
  const items = state.importBatch?.conversations || state.conversations;
  const list = $('div', { class: 'conversation-list list-scroll', 'data-scroll': 'list', 'aria-label': '会话列表' });
  if (!items.length) list.append(state.importBatch?.jobId
    ? $('div', { class: 'empty' }, $('h2', {}, state.importBatch.error ? '本批会话尚未读出' : '本页没有可查看的本批会话'), paragraph('已处理数量包含重复记录；被遗忘、替换或当前不可访问的消息不会显示。'), button('返回导入记录', () => navigate('import')))
    : $('div', { class: 'empty' }, $('h2', {}, '还没有保存的会话'), paragraph('导入文件，或从浏览器扩展保存一段对话。'), button('选择对话来源', () => navigate('import'), true)));
  for (const item of items) list.append($('button', { class: `result-card ${state.conversation?.session_ref === item.session_ref ? 'selected' : ''}`, 'data-conversation-ref': item.session_ref, 'aria-pressed': String(state.conversation?.session_ref === item.session_ref), onclick: () => openConversation(item) },
    $('strong', {}, item.title), $('span', { class: 'muted' }, `${item.platform} · ${item.message_count} 条消息`), $('span', { class: 'muted row-date' }, displayDate(item.captured_at))));
  if (!state.importBatch || state.importBatch.jobId) list.append($('div', { class: 'list-pagination' }, state.conversationListOffset ? button('回到第一页', () => loadConversations(0), false, 'small') : null, state.conversationListNext != null ? button('更多会话', () => loadConversations(state.conversationListNext), false, 'small') : null));
  const header = $('div', { class: 'reader-heading' }, detailBack(), $('div', { class: 'reader-title' }, $('h2', {}, state.conversation?.title || '选择一段会话'), state.conversation ? $('span', { class: 'muted' }, `${state.conversation.platform || '来源未知'} · ${scopeLabel(state.scope)}`) : null));
  const body = $('div', { class: 'reader-scroll', 'data-scroll': 'reader' });
  if (state.conversation) {
    header.append(actionButton('带到另一个AI', () => { state.continuationOpen = true; state.mobileDetail = true; render(); }, 'open-continuation', true));
    body.append($('details', { class: 'coverage-details' }, $('summary', {}, '消息覆盖范围'), paragraph(state.conversationOrderKnown ? '保留已保存片段中的消息顺序；未加载或未保存的历史不在其中。' : '部分片段没有可核实的先后关系，其余按保存顺序显示，不代表原始时间顺序。')));
    if (state.conversationBranches?.selection_required) body.append(hint(`已保留 ${state.conversationBranches.branch_count} 条分支。下面保留各段原文，带走使用时选择一条分支，避免混合不同回答。`));
    for (const row of state.conversationRows || []) body.append($('article', { class: `conversation-message ${row.ref === state.focusedEvent ? 'located-message' : ''}`, 'data-reference': row.ref },
      $('div', { class: 'result-meta' }, $('strong', { class: 'message-role' }, roleLabel(row.role)), $('span', { class: 'muted' }, displayDate(row.occurred_at))),
      $('div', { class: 'body-text' }, row.text), row.text_truncated ? hint('本条是节选，可用顶部搜索查看原话。') : null, row.branch?.gap_before ? hint('这条消息之前有省略或当前不可访问的内容。') : null, state.conversationBranches?.selection_required && row.branch?.is_branch_end ? button('从此分支继续', () => selectBranch(row), false, 'small branch-end') : null, technicalDetails(line('引用', row.ref))));
    body.append($('div', { class: 'button-row' }, state.conversationOffset ? button('回到开头', () => openConversation(state.conversation, 0), false, 'small') : null, state.conversationNext != null ? button('后续消息', () => openConversation(state.conversation, state.conversationNext), false, 'small') : null));
    body.append(button(`整理当前这页的 ${state.conversationRows.length} 条消息`, () => changeDreamSources(state.conversationRows.map(row => row.ref), () => { state.page = 'dream'; }), false, 'small'));
  } else body.append($('div', { class: 'empty' }, $('h3', {}, '从列表选择会话'), paragraph('查看原话，或带到另一个 AI 继续。')));
  const reader = state.continuationOpen && state.conversation ? continuationPane() : $('section', { class: 'reader-pane conversation-reader' }, header, body);
  content.append($('div', { class: `conversation-layout workspace-split ${state.mobileDetail && state.conversation ? 'show-detail' : ''}` }, list, reader));
}
function changeDreamSources(refs, next) {
  if (state.busy) return;
  // 来源改变即隐藏旧任务与审查；即使原生取消失败，也不能继续提交旧预览。
  state.dreamTask = null; state.dreamPreview = null; state.dreamEvidence = {}; state.dreamResultText = '';
  render();
  return cancelPreviews(() => { state.selectedRefs = refs; if (next) next(); });
}
function dreamPage() {
  content.append(heading('把对话整理成长期记忆', '选好来源，复制完整任务给你常用的 AI，把结果贴回来核对即可。'));
  content.append($('div', { class:'toolbar' }, button('返回阅读', () => navigate(state.workspace), false, 'small')));
  const refs=state.selectedRefs;
  const taskArgs=()=>({...args(),sourceRefs:state.selectedRefs.filter(r=>r.startsWith('event:')),memoryRefs:state.selectedRefs.filter(r=>r.startsWith('memory:'))});
  const exportBox=panel($('h2',{},'1. 选好要整理的对话'),paragraph(refs.length?`已选 ${refs.length} 条来源或旧记忆。也可回到阅读区增减。`:'先打开一段会话，或搜索原话后选择来源。没有来源时不会凭空生成记忆。'));
  exportBox.append(button('选择会话',()=>navigate('conversations')),button('查找其他来源',()=>navigate('search')));
  for (const ref of refs) {
    const message = state.conversationRows.find(item => item.ref === ref) || state.results.find(item => recordRef(item) === ref);
    const label = message ? shorten(recordText(message), 160) : ref.startsWith('memory:') ? '已选记忆，生成任务时重新核验' : '已选原始消息，生成任务时重新核验';
    exportBox.append($('div', { class: 'source-selection-row' }, $('div', {}, $('strong', {}, message?.role ? roleLabel(message.role) : ref.startsWith('memory:') ? '已有记忆' : '原始消息'), paragraph(label), technicalDetails(line('引用', ref))), button('移除', () => changeDreamSources(state.selectedRefs.filter(item => item !== ref)), false, 'small quiet')));
  }

  if(refs.some(r=>r.startsWith('event:')))exportBox.append(button('生成完整整理任务',()=>run('正在准备完整任务与来源预览…',async current=>{state.dreamPreview=null;state.dreamTask=null;state.dreamResultText='';await invoke('cancel_previews',{sessionId:state.vault.session_id});const task=await invoke('prepare_dream_task',taskArgs());if(current())state.dreamTask=task;}),true));
  if(state.dreamTask){
    const task=state.dreamTask;
    exportBox.append(hint(`本次包含 ${task.source_count} 条原始消息、${task.memory_count} 条旧记忆。任务已带好规则和输出格式，无需你编写 JSON。`));
    for(const item of task.sources||[])exportBox.append($('div',{class:'sample'},$('span',{class:'tag'},item.role==='user'?'用户原话':item.role==='assistant'?'AI回复':'工具/其他'),paragraph(item.text),$('span',{class:'muted'},displayDate(item.occurred_at)),item.truncated?hint('这张来源卡片只显示部分正文；展开下面的完整任务检查全部原文。'):null));
    const text=$('textarea',{rows:9,readonly:true,'aria-label':'完整整理任务'});text.value=task.text;
    exportBox.append($('details',{},$('summary',{},'查看完整任务与原文'),text),paragraph('复制后粘贴到你选择的 AI。先检查是否适合向该服务分享这些资料，再由你发送。'),button('复制整理任务',()=>run('正在重新核对任务…',async current=>{const shown=state.dreamTask;state.dreamTask=null;state.dreamPreview=null;await invoke('cancel_previews',{sessionId:state.vault.session_id});const fresh=await invoke('prepare_dream_task',taskArgs());if(!current())return;if(!shown||fresh.text!==shown.text){state.dreamResultText='';throw new Error('来源或记忆已经改变，请重新生成任务并检查');}state.dreamTask=fresh;await copyText(fresh.text);}),true));
  }
  const reviewBox=panel($('h2',{},'2. 把 AI 的整理结果贴回来'),paragraph('完整复制 AI 返回的 JSON 或整个 JSON 代码块。软件会检查来源、时间、旧记忆版本和保护状态，然后展示每项变更。'));
  if(!state.dreamPreview){const resultText=$('textarea',{rows:10,maxlength:1048576,'aria-label':'AI整理结果',placeholder:'粘贴 AI 根据上面的完整任务返回的结果…'});resultText.value=state.dreamResultText||'';resultText.addEventListener('input',()=>{state.dreamResultText=resultText.value;});reviewBox.append(resultText,button('检查并预览结果',()=>run('正在检查整理结果与原始证据…',async current=>{await invoke('cancel_previews',{sessionId:state.vault.session_id});const result=await invoke('review_dream_text',{...args(),text:state.dreamResultText||''});if(current()){state.dreamPreview=result;state.dreamEvidence={};}}),true));}
  else reviewBox.append(hint('结果已检查。请在下面逐条对照原文，再决定是否保存。'),button('返回修改结果',()=>cancelPreviews()));
  const files=$('details',{id:'dream-file-options'},$('summary',{},'已有任务或结果文件'),paragraph('保留文件方式，便于在不同工作流间传递。'),button('选择结果并审阅',()=>run('正在读取结果文件…',async current=>{state.dreamPreview=null;await invoke('cancel_previews',{sessionId:state.vault.session_id});const result=await invoke('pick_dream',args());if(result&&current()){state.dreamPreview=result;state.dreamEvidence={};}})));
  if(refs.some(r=>r.startsWith('event:')))files.append(button('导出本次来源包',()=>run('正在保存来源包…',async()=>{const saved=await invoke('export_dream',taskArgs());if(saved)showNotice(`来源包已保存：${saved}`);})));
  reviewBox.append(files);
  content.append($('div',{class:'grid-two file-preview'},exportBox,reviewBox));
  const preview = state.dreamPreview;
  if (!preview) return;
  const review = preview.review;
  const box = $('section', { class: 'panel file-preview' }, $('h2', {}, '逐条检查本次变更'), paragraph(preview.file_name), technicalDetails(line('整理任务', review.job_id)));
  for (const diagnostic of review.diagnostics || []) box.append(hint(diagnostic, true));
  if (review.already_applied) box.append(hint('这份结果已经保存。'));
  const names = { add: '新增记忆', update: '更新记忆', supersede: '替代旧记忆', noop: '保持原样', conflict: '存在冲突' };
  const changes = $('div', { class: 'changes' });
  for (const [index, change] of (review.changes || []).entries()) {
    const sourceRefs = change.after?.source_refs || change.before?.source_refs || [];
    const item = $('div', { class: 'change' }, $('span', { class: 'tag' }, names[change.operation] || change.operation), change.before ? $('div', { class: 'before' }, `之前\n${recordText(change.before)}`) : null, change.after ? $('div', { class: 'after' }, `之后\n${recordText(change.after)}`) : null);
    if (sourceRefs.length) {
      const evidence = $('details', { class: 'source-details dream-evidence', open: Boolean(state.dreamEvidence?.[index]) }, $('summary', {}, `原始出处 · ${sourceRefs.length} 条`),
        button('读取来源原话', () => run('正在核对整理结果的原话…', async current => {
          state.dreamEvidence = {};
          const records = [];
          for (const ref of sourceRefs) {
            const result = await invoke('read_record', { ...args(), reference: ref.startsWith('event:') ? ref : `event:${ref}` });
            if (!current()) return;
            if (!result.results?.length) throw new Error('这项原始出处已不可见或超出读取范围，请重新审阅');
            records.push(...result.results);
          }
          if (current() && state.dreamPreview === preview) state.dreamEvidence[index] = records;
        }), false, 'small'));
      for (const source of state.dreamEvidence?.[index] || []) evidence.append(sourceRecord(source));
      item.append(evidence);
    }
    changes.append(item);
  }
  box.append(changes);
  const approval = $('input', { type: 'checkbox', id: 'protected-approval' });
  if (review.requires_protected_approval) box.append($('label', { class: 'check', for: 'protected-approval' }, approval, '本次会修改受保护记忆。我已逐条审阅并明确同意这些变更。'));
  box.append($('div', { class: 'button-row' }, button('取消审阅', () => cancelPreviews()), ...(review.can_apply && !review.already_applied ? [button('保存这些记忆', () => {
    const protectedApproved = approval.checked;
    if (review.requires_protected_approval && !protectedApproved) { showNotice('请先勾选受保护记忆的额外确认', true); return; }
    confirmDialog('保存整理结果', `将本次 ${review.changes.length} 项变更写入 ${state.vault.display_name}。来源或版本发生变化时会停止，不会覆盖新内容。`, () => run('正在保存记忆…', async () => { const receipt = await invoke('apply_dream', { sessionId: state.vault.session_id, previewId: preview.preview_id, approveProtected: protectedApproved }); invalidateMemoryContent(state); await refreshStatus(); showNotice(`已保存 ${receipt.changes.length} 条记忆变更`); }), '确认保存');
  }, true)] : [])));
  content.append(box);
}
function connectPage() {
  content.append(heading('连接你的 AI 工具', '先选资料范围，再连接浏览器或本地 Agent。连接成功后，已保存的对话可在同一资料库中被查到。'));
  content.append(hint(`连接使用顶部所选范围：${scopeLabel(state.scope)}`));
  const extension=panel($('h2',{},'浏览器扩展'),paragraph('扩展可在网页上预览、保存和导出对话；没有本机连接时仍能导出文件。连接后可直接保存到当前资料库。'));
  const id=$('input',{placeholder:'粘贴浏览器扩展页显示的32位ID','aria-label':'RecallCard扩展ID',value:state.extensionId||''});id.addEventListener('input',()=>state.extensionId=id.value.trim());
  const browser=$('select',{'aria-label':'浏览器'},$('option',{value:'chromium'},'Chromium'),$('option',{value:'chrome'},'Google Chrome'),$('option',{value:'brave'},'Brave'));browser.value=state.browser||'chromium';browser.addEventListener('change',()=>state.browser=browser.value);
  const capture=$('input',{type:'checkbox',id:'allow-browser-capture'});capture.checked=Boolean(state.allowBrowserCapture);capture.addEventListener('change',()=>state.allowBrowserCapture=capture.checked);
  extension.append(paragraph('1. 正常安装 RecallCard 扩展，在浏览器扩展管理页复制它的 ID'),$('label',{},'2. 选择浏览器并填写扩展 ID'),browser,id,$('label',{class:'check',for:'allow-browser-capture'},capture,'同时允许扩展把我预览确认的对话保存到此范围'),button('连接此浏览器',()=>run('正在配置本机连接…',async current=>{const result=await invoke('install_browser_connection',{...args(),extensionId:state.extensionId||'',browser:state.browser||'chromium',allowCapture:Boolean(state.allowBrowserCapture)});if(current()&&result){state.connectionResult=result;showNotice('本机连接已注册，请到扩展点击“检查连接”');}}),true));
  if(state.connectionResult)extension.append(hint(state.connectionResult.note),line('保存对话',state.connectionResult.capture_enabled?'已允许':'未允许'),paragraph('3. 回到扩展点击“检查连接”，看到资料库名称后，再预览并保存当前会话'));
  extension.append(paragraph('若浏览器提示策略禁止安装，应用不会改变该策略。仍可使用导出文件和桌面导入。'));
  const agent=panel($('h2',{},'本地 Agent / MCP'),paragraph('为支持 MCP 的客户端生成当前资料库的只读配置。配置完成后，客户端可按需查找原话和出处。'),button('生成客户端配置',()=>run('正在生成受限读取配置…',async current=>{const value=await invoke('prepare_client_config',args());if(current())state.clientConfig=JSON.stringify(value,null,2);}),true));
  if(state.clientConfig){const preview=$('textarea',{rows:9,readonly:true,'aria-label':'MCP客户端配置'});preview.value=state.clientConfig;agent.append(preview,button('复制客户端配置',()=>copyText(state.clientConfig)),hint('将配置加入你选定客户端的 MCP 设置，保留原有服务。该客户端可能把读取的资料发送给其模型服务，请先确认范围。'),paragraph('添加后先让客户端读取随身背景，再搜索刚保存对话中的一句原话。这里生成配置不等于宿主已经连接。'));}
  content.append($('div',{class:'grid-two file-preview'},extension,agent),panel($('h2',{},'资料库状态'),line('当前资料库',state.vault.display_name),line('已保存记录 / 长期记忆',`${state.vault.event_count} / ${state.vault.memory_count}`),line('完整性检查',state.vault.health?.ok?'通过':'需要检查'),button('重新检查',()=>run('正在检查资料库…',refreshStatus),false,'small')));
}
function render() {
  if (!state.busy && !state.skipCapture) captureView();
  document.querySelector('#navigation').replaceChildren(...pages.map(([id, title, icon]) => $('button', { class: state.workspace === id ? 'active' : '', 'aria-current': state.workspace === id ? 'page' : null, 'data-workspace': id, onclick: () => navigate(id) }, $('span', { class: 'symbol', 'aria-hidden': 'true' }, icon), title)));
  document.querySelector('#location').textContent = ({ home: '开始使用', conversations: '会话', search: '搜索', memories: '记忆', import: '导入会话', dream: '整理记忆', connect: '连接与设置' })[state.page] || '会话';
  document.querySelector('#vault-badge').textContent = state.vault ? shorten(state.vault.display_name, 35) : '尚未打开资料库';
  document.querySelector('#switch-vault').textContent = state.vault ? '切换资料库' : '打开资料库';
  document.querySelector('#query').value = state.query;
  document.querySelector('#query').setAttribute('data-disabled', String(!state.vault));
  document.querySelector('#global-scope').replaceChildren(...(state.vault ? [scopeSelect()] : []));
  content.replaceChildren();
  content.dataset.view = `${state.page}:${state.page === 'memories' ? state.memoryFilter : ''}`;
  content.className = ['conversations', 'search', 'memories'].includes(state.page) && state.vault ? 'workspace-content' : 'form-content';
  if (state.page !== 'home' && !state.vault) needsVault();
  else ({ home, conversations: conversationPage, search: searchPage, import: importPage, dream: dreamPage, memories: () => state.memoryFilter === 'all' ? memoryManagement.page() : background.page(), connect: connectPage })[state.page]();
  restoreView(); setBusy();
}
document.querySelector('#switch-vault').addEventListener('click', () => discardBefore(vaultChooser));
document.querySelector('#import-button').addEventListener('click', () => state.vault ? navigate('import') : vaultChooser());
document.querySelector('#connect-button').addEventListener('click', () => navigate('connect'));
document.querySelector('#query').addEventListener('input', event => { if (state.busy) { event.target.value = state.query; return; } state.query = event.target.value; });
document.querySelector('#global-search').addEventListener('submit', event => { event.preventDefault(); if (!state.vault || state.busy) return; discardBefore(async () => { captureView(); state.page = 'search'; state.mobileDetail = false; render(); await loadRecords(); }); });
document.querySelector('.brand').addEventListener('click', event => { event.preventDefault(); navigate(state.vault ? 'conversations' : 'home'); });
document.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key === 'k' && !state.busy) { event.preventDefault(); document.querySelector('#query')?.focus(); }
  if (event.key === 'Escape' && contextSelection.isOpen() && !modal.open) contextSelection.close();
  if (event.key === 'Escape' && state.continuationOpen && !modal.open) { state.continuationOpen = false; render(); }
  if (['ArrowDown', 'ArrowUp'].includes(event.key) && event.target.matches('button.result-card')) {
    const siblings = [...(event.target.closest('.results') || event.target.parentElement).querySelectorAll('button.result-card')]; const index = siblings.indexOf(event.target);
    event.preventDefault(); siblings[Math.max(0, Math.min(siblings.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus();
  }
});
const sharedUI = { state, content, $, button, paragraph, heading, panel, hint, line, scopeSelect, invoke, run, render, showNotice, confirmDialog, navigate, memoryToolbar, detailBack, technicalDetails, actionButton, locateEvent, showBackground, discardBefore, resetReaderView, readRecord };
const memoryManagement = createMemoryManagement(sharedUI);
const background = createBackground(sharedUI);
const contextSelection = createContextSelection(sharedUI);
const importTasks = createImportTasks({ ...sharedUI, viewImported: job => {
  if (state.importBatch?.jobId !== job.job_id) {
    state.importBatch = { jobId: job.job_id, conversations: [], added: job.events_added, duplicates: job.events_duplicates };
    state.conversation = null; state.conversationRows = []; state.continuation = null; state.continuationGoal = ''; state.resumeConversation = null;
    state.conversationListOffset = 0; state.conversationListNext = null; state.viewport['conversations:'] = {};
  }
  navigate('conversations');
} });
render();
restoreWorkspace();
