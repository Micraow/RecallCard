import { pages, shorten, displayDate, recordText, recordRef, newState, activateVault, resetScope, scopeOptions, displayState } from './model.js';
import { createMemoryManagement, invalidateMemoryContent } from './memory-management.js';
import { createBackground } from './background.js';
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
  state.viewport[key] = { ...state.viewport[key], reader: 0 };
  if (content.dataset.view === key) {
    for (const node of content.querySelectorAll('[data-scroll="reader"]')) node.scrollTop = 0;
  }
}
function discardBefore(action) {
  if (!state.memory.mode && !state.background.draft) { action(); return; }
  modal.replaceChildren($('h2', { id: 'modal-title' }, '还有未保存的更改'), paragraph('离开会放弃这次更改。继续编辑会保留当前输入。'), $('div', { class: 'button-row' }, button('继续编辑', () => modal.close()), button('放弃更改', () => { modal.close(); memoryManagement.discard(); background.discard(); action(); }, true)));
  modal.showModal();
}
async function enterPage(page) {
  captureView();
  if (['search', 'memories'].includes(page)) state.workspace = page;
  if (page === 'home') page = state.vault ? 'search' : 'home';
  state.page = page;
  if (!state.vault || !['search', 'memories'].includes(page)) render();
  content.focus({ preventScroll: true });
  if (!state.vault) return;
  if (page === 'search') await loadRecords(true);
  if (page === 'memories') {
    if (state.memoryFilter !== 'all') await background.load();
    else await memoryManagement.load(state.memory.offset, true);
  }
}
function navigate(page) {
  if (state.busy) return;
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
    state.workspace = 'memories'; state.page = 'memories'; state.mobileDetail = false;
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
  if (!current()) return;
  state.page = 'search';
  render();
  await loadRecords(true);
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
    memoryManagement.clearReview(); background.discard();
    state.dreamPreview = null; state.dreamEvidence = {}; state.notePreview = null;
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    if (next) next();
  });
}
function scopeSelect() {
  const node = $('select', { 'aria-label': '资料范围' });
  for (const scope of scopeOptions(state.vault, state.scope)) { const option = $('option', { value: scope }, scopeLabel(scope)); option.selected = scope === state.scope; node.append(option); }
  node.addEventListener('change', () => {
    const scope = node.value; node.value = state.scope;
    discardBefore(async () => {
      await cancelPreviews(() => resetScope(state, scope));
      // 设置保存可以排队，但不能延后资料读取，再在用户已开始下一步时重绘页面。
      // 串行保存与过期范围校验仍由 rememberWorkspace 负责；其完成后不再操作 UI。
      const remembering = rememberWorkspace();
      if (state.page === 'search') await loadRecords();
      else if (state.page === 'memories') await memoryManagement.load();
      await remembering;
    });
  });
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
    if (!preserve) { resetReaderView('search:'); state.viewport['search:'].list = 0; }
    state.selected = null; state.sources = [];
    state.results = []; state.searchLoaded = false; state.searchError = false; state.resultNote = '正在查找本地资料…'; render();
    const query = state.query.trim();
    let result;
    try { result = await invoke(query ? 'search_records' : 'browse_records', { ...args(), target: state.target, ...(query ? { query } : {}) }); }
    catch (error) { if (current()) { state.searchError = true; state.resultNote = '查找未完成'; } throw error; }
    if (!current()) return;
    state.results = result.results || []; state.searchLoaded = true; state.resultQuery = query;
    state.resultNote = result.truncated ? '结果较多，请缩小关键词范围' : `${state.results.length} 条可见资料`;
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
    technicalDetails(line('引用', ref), data.source?.url ? line('出处地址', data.source.url) : null));
}
function readingPane() {
  if (!state.selected) return $('aside', { class: 'reading-pane empty' }, $('h3', {}, '选择一条搜索结果'), paragraph('查看原话、相邻消息和出处。'));
  const item = state.selected; const reference = recordRef(item); const isMemory = reference.startsWith('memory:');
  const data = item.record || item.event || item.memory || item;
  const title = item.conversation_title || item.title || (isMemory ? '长期记忆' : '原始记录');
  const header = $('div', { class: 'reader-heading' }, detailBack(), $('div', { class: 'reader-title' }, $('h2', {}, title), $('span', { class: 'muted' }, `${item.platform || data.source?.platform || (isMemory ? '整理所得' : '来源未知')} · ${isMemory ? displayState(data.status || item.state) : roleLabel(item.role || data.role)}`)));
  const body = $('div', { class: 'reader-scroll', 'data-scroll': 'reader' },
    $('div', { class: 'body-text' }, recordText(data) || recordText(item) || '这条资料没有保存正文'),
    line('原始时间', displayDate(data.occurred_at || item.occurred_at)));
  if (item.text_truncated || item.truncated) body.append(hint('这里只显示部分内容，完整原文保存在资料库中。'));
  if (isMemory) {
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
  content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '搜索结果'), $('span', { class: 'muted' }, state.resultNote || '在当前范围内查找'), target, button('返回工作区', () => navigate(state.workspace), false, 'small')));
  const results = $('div', { class: 'results list-scroll', 'data-scroll': 'list', 'aria-label': '搜索结果' });
  if (!state.results.length) results.append(state.searchError
    ? $('div', { class: 'empty' }, $('h3', {}, '这次查找未完成'), paragraph('旧结果已移除。请重试，或修改上方查询。'), button('重试查找', () => loadRecords(false), true))
    : $('div', { class: 'empty' }, $('h3', {}, state.query ? '没有找到匹配资料' : '还没有可见资料'), paragraph('试试更短的原话，或检查顶部资料范围。')));
  for (const item of state.results) {
    const ref = recordRef(item); const isMemory = ref.startsWith('memory:');
    const source = item.source_summary?.sources?.[0];
    const resultButton = $('button', { class: `result-card ${recordRef(state.selected) === ref ? 'selected' : ''}`, onclick: () => readRecord(item), 'aria-pressed': String(recordRef(state.selected) === ref), 'data-reference': ref },
      $('strong', {}, item.conversation_title || item.title || (isMemory ? shorten(recordText(item), 52) : '原始记录')),
      $('span', { class: 'result-excerpt' }, shorten(recordText(item), 190)),
      $('span', { class: 'muted' }, `${isMemory ? '记忆' : roleLabel(item.role)} · ${item.platform || source?.platform || '来源未知'} · ${displayDate(item.occurred_at || item.valid_from)}`));
    results.append(resultButton);
  }
  content.append($('div', { class: `results-layout workspace-split ${state.mobileDetail && state.selected ? 'show-detail' : ''}` }, results, readingPane()));
}
async function refreshStatus() { state.vault = await invoke('vault_status', { sessionId: state.vault.session_id }); }
async function copyText(text) {
  try { await invoke('write_clipboard', { text }); showNotice(`已复制 · ${scopeLabel(state.scope)}，可粘贴到你选择的 AI`); }
  catch (error) { showNotice(typeof error === 'string' ? error : '系统剪贴板写入失败，请重试', true); }
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
  exportBox.append(button('搜索选择来源',()=>navigate('search')));
  for (const ref of refs) {
    const message = state.results.find(item => recordRef(item) === ref);
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
  content.append(heading('连接你的 AI 工具', '为支持 MCP 的客户端生成当前资料库的受限配置。生成后由你粘贴进客户端配置文件；应用不修改任何外部配置。'));
  content.append(hint(`连接使用顶部所选范围：${scopeLabel(state.scope)}`));
  const agent = panel($('h2', {}, '本地 Agent / MCP'),
    paragraph('配置包含固定的资料库路径与授权范围；Agent 只能读取该范围内的资料，并可通过 knowledge_write 保存你明示的事实。'),
    button('生成客户端配置', () => run('正在生成受限配置…', async current => {
      const value = await invoke('prepare_client_config', args());
      if (current()) state.clientConfig = JSON.stringify(value, null, 2);
    }), true));
  if (state.clientConfig) {
    const preview = $('textarea', { rows: 9, readonly: true, 'aria-label': 'MCP客户端配置' });
    preview.value = state.clientConfig;
    agent.append(preview,
      button('复制客户端配置', () => copyText(state.clientConfig)),
      hint('将配置合并进你选定客户端的 MCP 设置，保留原有服务。该客户端可能把读取的资料发送给其模型服务，请先确认范围。'),
      paragraph('添加后让客户端先调用 bootstrap 读取随身背景，再用 search 验证一句原话能被找到。'));
  }
  content.append(agent, panel($('h2', {}, '资料库状态'),
    line('当前资料库', state.vault.display_name),
    line('已保存记录 / 长期记忆', `${state.vault.event_count} / ${state.vault.memory_count}`),
    line('完整性检查', state.vault.health?.ok ? '通过' : '需要检查'),
    button('重新检查', () => run('正在检查资料库…', refreshStatus), false, 'small')));
}

function render() {
  if (!state.busy && !state.skipCapture) captureView();
  document.querySelector('#navigation').replaceChildren(...pages.map(([id, title, icon]) => $('button', { class: state.workspace === id ? 'active' : '', 'aria-current': state.workspace === id ? 'page' : null, 'data-workspace': id, onclick: () => navigate(id) }, $('span', { class: 'symbol', 'aria-hidden': 'true' }, icon), title)));
  document.querySelector('#location').textContent = ({ home: '开始使用', search: '搜索', memories: '记忆', dream: '整理记忆', connect: '连接与设置' })[state.page] || '搜索';
  document.querySelector('#vault-badge').textContent = state.vault ? shorten(state.vault.display_name, 35) : '尚未打开资料库';
  document.querySelector('#switch-vault').textContent = state.vault ? '切换资料库' : '打开资料库';
  document.querySelector('#query').value = state.query;
  document.querySelector('#query').setAttribute('data-disabled', String(!state.vault));
  document.querySelector('#global-scope').replaceChildren(...(state.vault ? [scopeSelect()] : []));
  content.replaceChildren();
  content.dataset.view = `${state.page}:${state.page === 'memories' ? state.memoryFilter : ''}`;
  content.className = ['search', 'memories'].includes(state.page) && state.vault ? 'workspace-content' : 'form-content';
  if (state.page !== 'home' && !state.vault) needsVault();
  else ({ home, search: searchPage, dream: dreamPage, memories: () => state.memoryFilter === 'all' ? memoryManagement.page() : background.page(), connect: connectPage })[state.page]();
  restoreView(); setBusy();
}
document.querySelector('#switch-vault').addEventListener('click', () => discardBefore(vaultChooser));
document.querySelector('#connect-button').addEventListener('click', () => navigate('connect'));
document.querySelector('#query').addEventListener('input', event => { if (state.busy) { event.target.value = state.query; return; } state.query = event.target.value; });
document.querySelector('#global-search').addEventListener('submit', event => { event.preventDefault(); if (!state.vault || state.busy) return; discardBefore(async () => { captureView(); state.page = 'search'; state.mobileDetail = false; render(); await loadRecords(); }); });
document.querySelector('.brand').addEventListener('click', event => { event.preventDefault(); navigate(state.vault ? 'search' : 'home'); });
document.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key === 'k' && !state.busy) { event.preventDefault(); document.querySelector('#query')?.focus(); }
  if (['ArrowDown', 'ArrowUp'].includes(event.key) && event.target.matches('button.result-card')) {
    const siblings = [...(event.target.closest('.results') || event.target.parentElement).querySelectorAll('button.result-card')]; const index = siblings.indexOf(event.target);
    event.preventDefault(); siblings[Math.max(0, Math.min(siblings.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus();
  }
});
const sharedUI = { state, content, $, button, paragraph, heading, panel, hint, line, scopeSelect, invoke, run, render, showNotice, confirmDialog, navigate, memoryToolbar, detailBack, technicalDetails, actionButton, showBackground, discardBefore, resetReaderView, readRecord };
const memoryManagement = createMemoryManagement(sharedUI);
const background = createBackground(sharedUI);
render();
restoreWorkspace();
