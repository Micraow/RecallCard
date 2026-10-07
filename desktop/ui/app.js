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
const heading = (title, subtitle, eyebrow = 'RECALLCARD / LOCAL FIRST') => $('div', {}, $('div', { class: 'eyebrow' }, eyebrow), $('h1', {}, title), paragraph(subtitle));
const panel = (...children) => $('section', { class: 'panel' }, ...children);
const hint = (text, warning = false) => $('div', { class: `hint ${warning ? 'warning' : ''}` }, text);
const line = (title, value) => $('div', { class: 'info-line' }, $('span', { class: 'muted' }, title), $('span', { class: 'value' }, String(value ?? '—')));
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
  modal.replaceChildren($('h2', { id: 'modal-title' }, state.vault ? '切换资料库' : '开始自己的资料库'), paragraph('选择一个本机文件夹保存记忆。已有 CLI 资料库可以直接打开，导入和整理都在本机完成。'), hint('新建时请选择空文件夹。取消文件选择不会改变当前资料库。'), $('div', { class: 'button-row' }, button('取消', () => modal.close()), button('打开已有资料库', () => chooseVault(false)), button('创建新资料库', () => chooseVault(true), true)));
  modal.showModal();
}
function confirmDialog(title, text, action, label = '确认继续') {
  modal.replaceChildren($('h2', { id: 'modal-title' }, title), paragraph(text), $('div', { class: 'button-row' }, button('取消', () => modal.close()), button(label, () => { modal.close(); action(); }, true)));
  modal.showModal();
}
async function cancelPreviews(next) {
  await run('正在取消待确认操作…', async () => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.dreamPreview = null;
    if (next) next();
  });
}
function scopeSelect() {
  const node = $('select', { 'aria-label': '资料范围' });
  for (const scope of scopeOptions(state.vault, state.scope)) { const option = $('option', { value: scope }, scope); option.selected = scope === state.scope; node.append(option); }
  node.addEventListener('change', async () => { const scope = node.value; await cancelPreviews(() => resetScope(state, scope)); if (state.page === 'search') loadRecords(); });
  return node;
}
function needsVault() {
  content.append(heading('先打开一个资料库', '选择保存记忆的文件夹，即可导入、检索和审阅资料。'), $('div', { class: 'empty' }, $('div', { class: 'empty-icon' }, '▧'), $('h2', {}, '你的资料，保存在你自己的文件夹'), paragraph('从已有资料库继续，或创建一个新的本地空间。'), button('选择资料库', vaultChooser, true)));
}
function home() {
  const art = $('div', { class: 'hero-art', 'aria-hidden': 'true' }, $('div', { class: 'memory-card back' }), $('div', { class: 'memory-card front' }, $('div', { class: 'card-mark' }, '✧'), $('div', { class: 'card-line' }), $('div', { class: 'card-line' }), $('div', { class: 'card-line short' })));
  content.append($('div', { class: 'hero' }, $('div', { class: 'hero-copy' }, $('div', { class: 'eyebrow' }, 'A PLACE FOR WHAT MATTERS'), $('h1', {}, '把散落的想法，找回来。'), paragraph(state.vault ? `正在使用 ${state.vault.display_name}。把对话变成可查找、可追溯的个人记忆。` : '把 AI 对话和重要决定，收进一个属于自己的资料库。'), $('div', { class: 'button-row' }, button(state.vault ? '查找我的资料' : '创建新资料库', () => state.vault ? navigate('search') : chooseVault(true), true), button(state.vault ? '导入新资料' : '打开已有资料库', () => state.vault ? navigate('import') : chooseVault(false)))), art));
  if (state.vault) {
    content.append($('div', { class: 'grid-three' }, panel($('span', { class: 'muted' }, '原始记录'), $('div', { class: 'stat' }, String(state.vault.event_count)), paragraph('完整保留出处，重复导入自动去重')), panel($('span', { class: 'muted' }, '长期记忆'), $('div', { class: 'stat' }, String(state.vault.memory_count)), paragraph('经过审阅的决定、偏好与背景')), panel($('span', { class: 'muted' }, '资料库状态'), $('div', { class: 'stat' }, state.vault.health?.ok ? '正常' : '待检查'), paragraph('本地文件 · 中文与英文检索'))));
  }
  content.append($('div', { class: 'section-heading' }, $('h2', {}, state.vault ? '接下来，你可以…' : '三个简单的步骤'), $('span', {}, '离线也能开始')));
  const features = [
    ['↥', '导入已有对话', '支持 ChatGPT 官方导出、Claude Code 日志和标准 JSONL。先预览，再写入。', 'import', ''],
    ['⌕', '找到当时的上下文', '输入关键词，查看原始记录、长期记忆，以及每条记忆背后的出处。', 'search', 'purple'],
    ['✧', '把重要的事留下来', '选择来源、导出整理包，逐条审阅 Dream 结果后，再保存为长期记忆。', 'dream', 'sand'],
  ];
  content.append($('div', { class: 'grid-three' }, features.map(([icon, title, description, page, color]) => panel($('div', { class: `step-icon ${color}` }, icon), $('h3', {}, title), paragraph(description), $('button', { class: 'text-link', onclick: () => state.vault ? navigate(page) : vaultChooser() }, '开始使用 →')))));
  content.append($('div', { class: 'section-heading' }, $('h2', {}, '始终由你决定'), $('span', {}, 'PRIVATE BY DEFAULT')), hint('资料不会自动发送到模型。浏览器中的最终发送、长期记忆的发布，都由你主动确认。'));
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
    if (!first) { if (result.truncated || result.pending_refs?.length) { state.selected = { ...item, truncated: true }; state.sources = []; showNotice('完整记录超过桌面读取上限，显示搜索片段；原文保留在 Vault 中'); return; } throw new Error('记录不可见或已更新，请重新查找'); }
    state.selected = { ...item, text_truncated: false, ...first, truncated: Boolean(result.truncated || first.truncated), ref: reference };
    state.sources = [];
    if (source || reference.startsWith('memory:')) {
      const sources = await invoke('read_sources', { ...args(), reference });
      if (current()) { state.sources = sources.results || []; if (sources.truncated) showNotice('出处较长，本次未能显示全部；完整证据保留在 Vault 原文件中'); }
    }
  });
}
function readingPane() {
  if (!state.selected) return $('aside', { class: 'panel reading-pane empty' }, $('div', { class: 'empty-icon' }, '▤'), $('h3', {}, '从左侧选择一条资料'), paragraph('查看完整内容、时间和原始出处。'));
  const item = state.selected;
  const reference = recordRef(item);
  const pane = $('aside', { class: 'panel reading-pane' }, $('div', { class: 'result-meta' }, $('h2', {}, reference.startsWith('memory:') ? '长期记忆' : '原始记录'), $('span', { class: 'tag' }, state.scope)), $('div', { class: 'ref' }, reference));
  const data = item.record || item.event || item.memory || item;
  const body = recordText(data) || recordText(item);
  pane.append($('div', { class: 'body-text' }, body || '本记录不包含可显示的正文'), line('记录时间', displayDate(data.recorded_at || data.captured_at || data.occurred_at || item.occurred_at)), line('状态', displayState(data.status || data.state || item.state)));
  if (item.text_truncated || item.truncated) pane.append(hint('正文较长，此处显示的是有界片段。完整记录保留在 Vault 原文件中。'));
  pane.append($('hr', { class: 'divider' }), $('h3', {}, '加入本次整理'));
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
  if (!state.results.length) { content.append($('div', { class: 'empty' }, $('div', { class: 'empty-icon' }, '⌕'), $('h3', {}, state.query ? '暂时没有找到匹配资料' : '这里还没有可见资料'), paragraph(state.query ? '试试更短的关键词，或检查当前资料范围。' : '先导入一份对话，之后可以按关键词查找。'), button('导入资料', () => navigate('import')))); return; }
  content.append($('div', { class: 'section-heading' }, $('span', {}, state.resultNote || `${state.results.length} 条资料`), $('span', {}, `当前范围：${state.scope}`)));
  const results = $('div', { class: 'results' });
  for (const item of state.results) {
    const ref = recordRef(item);
    results.append($('button', { class: `result-card ${recordRef(state.selected) === ref ? 'selected' : ''}`, onclick: () => readRecord(item) }, $('div', { class: 'result-meta' }, $('span', { class: `tag ${ref.startsWith('memory:') ? 'purple' : ''}` }, ref.startsWith('memory:') ? '长期记忆' : '原始记录'), $('span', { class: 'muted' }, displayDate(item.occurred_at || item.valid_from))), paragraph(recordText(item)), $('div', { class: 'ref' }, shorten(ref, 55))));
  }
  content.append($('div', { class: 'results-layout' }, results, readingPane()));
}
async function refreshStatus() { state.vault = await invoke('vault_status', { sessionId: state.vault.session_id }); }
function importPage() {
  content.append(heading('把已有对话带进来', '只读取你主动选择的文件。预览确认后，才会写入当前资料库。'));
  const format = $('select', { id: 'import-format', 'aria-label': '导入格式' }, $('option', { value: 'chatgpt-export' }, 'ChatGPT 官方导出 JSON'), $('option', { value: 'claude-code' }, 'Claude Code 对话 JSONL'), $('option', { value: 'manual-jsonl' }, 'RecallCard 标准 JSONL'));
  format.value = state.importFormat || 'chatgpt-export';
  format.addEventListener('change', () => { const next = format.value; cancelPreviews(() => { state.importFormat = next; }); });
  const scope = $('input', { id: 'import-scope', value: state.scope, placeholder: 'personal', 'aria-label': '导入范围' });
  scope.addEventListener('change', () => { const next = scope.value.trim(); cancelPreviews(() => resetScope(state, next)); });
  const choose = () => run('正在读取导入预览…', async current => {
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
    state.importPreview = null; state.dreamPreview = null;
    const result = await invoke('pick_import', { ...args(), format: state.importFormat || 'chatgpt-export' });
    if (current() && result) state.importPreview = result;
  });
  content.append(panel($('div', { class: 'form-stack' }, $('div', {}, $('label', { for: 'import-format' }, '文件格式'), format, $('div', { class: 'field-note' }, 'ChatGPT 请先解压官方导出 ZIP，再选择 conversations.json')), $('div', {}, $('label', { for: 'import-scope' }, '存入范围'), scope, $('div', { class: 'field-note' }, '例如 personal 或 work；检索与整理按范围隔离')), hint('单个文件最多 16 MiB、5000 条事件。导入不会自动创建长期记忆；重复导入相同消息不会重复保存。'), button('选择文件并预览', choose, true))));
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
function dreamPage() {
  content.append(heading('让重要的记忆沉淀下来', 'Dream 是一次有出处的整理。导出来源包、取得整理结果，再由你逐条审阅和发布。'));
  content.append($('div', { class: 'toolbar' }, $('div', {}, $('label', {}, '本次整理范围'), scopeSelect())), hint('此页面不会调用模型或发送资料。可以把导出的来源包交给你自己选择的工作流，生成符合 Dream 格式的 JSON 后再导入。'));
  const refs = state.selectedRefs;
  const exportBox = panel($('h2', {}, '1. 准备来源'), paragraph(refs.length ? `已从“查找与阅读”选中 ${refs.length} 条资料。` : '先在“查找与阅读”中选择原始记录，可一并选择需要更新的记忆。'));
  for (const ref of refs) exportBox.append($('div', { class: 'info-line' }, $('span', { class: 'ref' }, ref), button('移除', () => { state.selectedRefs = refs.filter(r => r !== ref); render(); }, false, 'small quiet')));
  exportBox.append($('div', { class: 'button-row' }, button('去选择资料', () => navigate('search')), ...(refs.some(r => r.startsWith('event:')) ? [button('导出本次来源包', () => run('正在保存来源包…', async () => { const saved = await invoke('export_dream', { ...args(), sourceRefs: refs.filter(r => r.startsWith('event:')), memoryRefs: refs.filter(r => r.startsWith('memory:')) }); if (saved) showNotice(`来源包已保存：${saved}`); }), true)] : [])));
  const reviewBox = panel($('h2', {}, '2. 审阅整理结果'), paragraph('选择该资料库导出的同一个 job 对应的 Dream 结果文件。系统会核对来源摘要、版本与范围。'), $('div', { class: 'button-row' }, button('选择结果并审阅', () => run('正在核对整理结果…', async current => { await invoke('cancel_previews', { sessionId: state.vault.session_id }); state.importPreview = null; state.dreamPreview = null; const result = await invoke('pick_dream', args()); if (result && current()) state.dreamPreview = result; }), true)));
  content.append($('div', { class: 'grid-two file-preview' }, exportBox, reviewBox));
  const preview = state.dreamPreview;
  if (!preview) return;
  const review = preview.review;
  const box = $('section', { class: 'panel file-preview' }, $('h2', {}, '逐条检查本次变更'), paragraph(preview.file_name), line('作业编号', review.job_id));
  for (const diagnostic of review.diagnostics || []) box.append(hint(diagnostic, true));
  if (review.already_applied) box.append(hint('这份结果已经发布，不会重复写入。'));
  const names = { add: '新增记忆', update: '更新记忆', supersede: '替代旧记忆', noop: '保持原样', conflict: '存在冲突' };
  const changes = $('div', { class: 'changes' });
  for (const change of review.changes || []) changes.append($('div', { class: 'change' }, $('span', { class: 'tag' }, names[change.operation] || change.operation), change.before ? $('div', { class: 'before' }, `之前\n${recordText(change.before)}`) : null, change.after ? $('div', { class: 'after' }, `之后\n${recordText(change.after)}`) : null, $('div', { class: 'ref' }, (change.after?.source_refs || []).join(' · '))));
  box.append(changes);
  const approval = $('input', { type: 'checkbox', id: 'protected-approval' });
  if (review.requires_protected_approval) box.append($('label', { class: 'check', for: 'protected-approval' }, approval, '本次会修改受保护记忆。我已逐条审阅并明确同意这些变更。'));
  box.append($('div', { class: 'button-row' }, button('取消审阅', () => cancelPreviews()), ...(review.can_apply && !review.already_applied ? [button('确认发布本次变更', () => {
    const protectedApproved = approval.checked;
    if (review.requires_protected_approval && !protectedApproved) { showNotice('请先勾选受保护记忆的额外确认', true); return; }
    confirmDialog('发布经过审阅的记忆', `将本次 ${review.changes.length} 项变更写入 ${state.vault.display_name}。来源或版本发生变化时会停止，不会覆盖新内容。`, () => run('正在发布记忆…', async () => { const receipt = await invoke('apply_dream', { sessionId: state.vault.session_id, previewId: preview.preview_id, approveProtected: protectedApproved }); state.dreamPreview = null; state.results = []; await refreshStatus(); showNotice(`已保存 ${receipt.changes.length} 条记忆变更，发布凭证保存在本机`); }), '确认发布');
  }, true)] : [])));
  content.append(box);
}
function connectPage() {
  content.append(heading('连接与状态', '资料库可以同时服务桌面界面、浏览器扩展与本地 Agent。桌面应用本身已可独立导入和查找。'));
  content.append($('div', { class: 'grid-two file-preview' }, panel($('h2', {}, '资料库健康状态'), line('资料库', state.vault.display_name), line('目录', state.vault.root), line('原始记录 / 长期记忆', `${state.vault.event_count} / ${state.vault.memory_count}`), line('完整性检查', state.vault.health?.ok ? '通过' : '请检查资料库'), line('云端 API', '未启用'), button('重新检查', () => run('正在检查资料库…', async () => { await refreshStatus(); showNotice('资料库检查完成'); }), false, 'small')), panel($('h2', {}, '与 CLI 配合使用'), paragraph('这个桌面应用直接读取同一个 Vault。原来的 recallcard 命令行、只读 MCP 和 Agent Hook 仍可使用。'), hint('首版桌面界面暂不管理 Git 同步、云 API 配额或后台守护进程。高级配置仍通过 CLI 完成。'))));
  const extension = panel($('h2', {}, '浏览器扩展：按需连接'), paragraph('1. 在浏览器允许安装的环境中，加载独立扩展 ZIP 解压后的目录，并复制扩展 ID。'), paragraph('2. 使用随 CLI 运行包提供的 recallcard 程序生成 Native Messaging 配置，按输出指引人工注册。'), paragraph('3. 在扩展中选择资料，再点“注入输入框”。最终发送由你点击。'), hint('桌面应用不会安装扩展或更改浏览器策略。当前页面未检测浏览器连接，不能据此判断扩展已连接。', true), $('div', { class: 'toolbar' }, scopeSelect()), $('div', { class: 'code-block' }, nativeInstructions(state.vault.root, state.scope)), paragraph('完整中文步骤：github.com/Micraow/RecallCard → docs/quickstart-linux-v0.3.md'));
  extension.classList.add('file-preview'); content.append(extension);
}
function render() {
  document.querySelector('#navigation').replaceChildren(...pages.map(([id, title, icon]) => $('button', { class: state.page === id ? 'active' : '', 'aria-current': state.page === id ? 'page' : null, onclick: () => navigate(id) }, $('span', { class: 'symbol', 'aria-hidden': 'true' }, icon), title)));
  document.querySelector('#location').textContent = pages.find(p => p[0] === state.page)?.[1] || '概览';
  document.querySelector('#vault-badge').textContent = state.vault ? shorten(state.vault.display_name, 23) : '尚未打开资料库';
  document.querySelector('#switch-vault').textContent = state.vault ? '切换资料库' : '打开资料库';
  content.replaceChildren();
  if (state.page !== 'home' && !state.vault) needsVault();
  else ({ home, search: searchPage, import: importPage, dream: dreamPage, connect: connectPage })[state.page]();
  setBusy();
}
document.querySelector('#switch-vault').addEventListener('click', vaultChooser);
document.querySelector('.brand').addEventListener('click', event => { event.preventDefault(); navigate('home'); });
document.addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key === 'k' && !state.busy) { event.preventDefault(); navigate('search'); document.querySelector('#query')?.focus(); } });
render();
