import { displayDate, displayState, newBackgroundState, newMemoryState, recordText, shorten } from './model.js';

const evidenceLabels = { user_explicit: '用户明确表达', observed: '观察所得', assistant_suggestion: 'AI 建议，尚非用户事实' };
const roleLabels = { user: '用户原话', assistant: 'AI 回复', tool: '工具结果', system: '系统消息' };
const operationLabels = { edit: '修改记忆', forget: '遗忘记忆', restore: '撤销遗忘规则' };

export function memoryLabels(text) {
  return [...new Set(String(text).split('\n').map(label => label.trim()).filter(Boolean))];
}
// 成员标记只由随身背景流程维护；普通标签编辑不能新增或删除它。
export function editableMemoryLabels(labels = []) { return labels.filter(label => label !== 'bootstrap'); }
export function preservedMemoryLabels(text, labels = []) {
  return [...memoryLabels(text).filter(label => label !== 'bootstrap'), ...labels.filter(label => label === 'bootstrap')];
}
export function canEditMemory(memory, row) {
  return Boolean(memory && row && !row.hidden && ['active', 'tentative'].includes(memory.status));
}
export function memoryReviewKey(state) {
  const memory = state.memory;
  return JSON.stringify([state.epoch, state.vault?.session_id, state.scope, memory.selected?.id,
    memory.selected?.revision, memory.mode, memory.draft, memory.reason]);
}
// 变更后立即废弃可能包含旧内容的阅读、交接、整理任务和来源选择。
export function invalidateMemoryContent(state) {
  if (state.conversation?.session_ref) state.resumeConversation = { session_ref: state.conversation.session_ref, goal: state.continuationGoal || '', offset: state.conversationOffset || 0 };
  state.results = []; state.resultNote = ''; state.selected = null; state.sources = [];
  state.selectedRefs = []; state.dreamPreview = null; state.dreamEvidence = {}; state.dreamTask = null; state.dreamResultText = '';
  state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.notePreview = null;
  state.conversations = []; state.conversation = null; state.conversationRows = [];
  state.conversationOffset = 0; state.conversationListOffset = 0; state.conversationListNext = null;
  state.continuation = null; state.continuationGoal = '';
  state.background = newBackgroundState(); state.continuationOpen = false; state.importBatch = null; state.searchLoaded = false;
  const includeHidden = state.memory.includeHidden;
  state.memory = { ...newMemoryState(), includeHidden };
}

export function createMemoryManagement(ui) {
  const { state, content, $, button, paragraph, heading, panel, hint, line, scopeSelect,
    invoke, run, render, showNotice, confirmDialog } = ui;
  const args = () => ({ sessionId: state.vault.session_id, scope: state.scope });
  const memory = () => state.memory;

  function clearReview() {
    memory().review = null; memory().reviewKey = '';
    document.querySelector('#memory-review')?.remove();
  }
  function discard() {
    clearReview(); memory().mode = ''; memory().draft = null; memory().reason = '';
  }
  async function cancelServerPreviews() {
    clearReview();
    state.importPreview = null; state.importSelection = null; state.importSelectedIds = [];
    state.dreamPreview = null; state.notePreview = null;
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
  }
  function clearSelection() {
    discard(); memory().selected = null; memory().selectedRow = null;
    memory().source = null; memory().sourceId = '';
  }
  async function fetchPage(offset, current) {
    const result = await invoke('manage_memories', { ...args(), includeHidden: memory().includeHidden, offset });
    if (!current()) return;
    Object.assign(memory(), { rows: result.memories || [], total: result.total, offset,
      nextOffset: result.next_offset, loaded: true, error: '' });
  }
  async function load(offset = 0, preserve = false) {
    await run('正在读取记忆列表…', async current => {
      const selectedId = state.pendingMemoryId || (preserve ? memory().selected?.id : null); state.pendingMemoryId = null;
      clearSelection(); memory().rows = []; memory().loaded = false; memory().error = ''; render();
      try {
        await cancelServerPreviews(); await fetchPage(offset, current);
        const row = memory().rows.find(item => item.id === selectedId);
        if (row) { const selected = await invoke('managed_memory', { ...args(), id: row.id }); if (current()) { memory().selected = selected; memory().selectedRow = row; } }
      } catch (error) { if (current()) { clearSelection(); memory().error = String(error?.message || error); } throw error; }
    });
  }
  function open(row) { if (state.busy) return; ui.discardBefore(() => openNow(row)); }
  async function openNow(row) {
    await run('正在读取完整记忆…', async current => {
      if (memory().selected?.id !== row.id) ui.resetReaderView();
      clearSelection(); render();
      await cancelServerPreviews();
      const selected = await invoke('managed_memory', { ...args(), id: row.id });
      if (!current()) return;
      memory().selected = selected; memory().selectedRow = row; state.mobileDetail = true;
    });
  }
  async function openSource(eventId) {
    await run('正在读取原始出处…', async current => {
      memory().source = null; memory().sourceId = ''; render();
      try { const source = await invoke('managed_memory_source', { ...args(), memoryId: memory().selected.id, eventId }); if (current()) { memory().source = source; memory().sourceId = eventId; } } catch (error) { if (current()) clearSelection(); throw error; }
    });
  }
  function begin(mode) {
    discard(); memory().mode = mode;
    if (mode === 'edit') {
      const selected = memory().selected;
      memory().draft = { content: selected.content, protected: selected.protected, labels: editableMemoryLabels(selected.labels).join('\n') };
    }
    render();
  }
  async function cancel() {
    await run('正在取消记忆操作…', async () => { discard(); await cancelServerPreviews(); });
  }
  async function review() {
    await run('正在核对记忆变更与影响范围…', async current => {
      await cancelServerPreviews();
      const key = memoryReviewKey(state);
      const selected = memory().selected;
      const draft = memory().draft;
      const result = memory().mode === 'edit'
        ? await invoke('review_memory_edit', { ...args(), id: selected.id, revision: selected.revision,
          edit: { content: draft.content, protected: draft.protected, labels: preservedMemoryLabels(draft.labels, selected.labels) } })
        : await invoke('review_memory_visibility', { ...args(), id: selected.id,
          restore: memory().mode === 'restore', reason: memory().reason });
      if (current() && key === memoryReviewKey(state)) {
        memory().review = result; memory().reviewKey = key;
      }
    });
  }
  function askConfirm(review, approveProtected) {
    if (memory().review !== review || memory().reviewKey !== memoryReviewKey(state)) {
      clearReview(); showNotice('内容已改变，请重新检查变更', true); return;
    }
    if (review.requires_protected_approval && !approveProtected) {
      showNotice('受影响记忆中有受保护内容，请先勾选额外确认', true); return;
    }
    const action = operationLabels[review.operation];
    const text = `将${action}，影响 ${review.affected_memories} 条记忆和 ${review.affected_events} 条原始记录。资料库：${state.vault.display_name}。来源、版本或遗忘规则已改变时会停止，请重新审阅。`;
    confirmDialog(`确认${action}`, text, () => run('正在保存记忆变更…', async current => {
      if (memory().review !== review || memory().reviewKey !== memoryReviewKey(state)) {
        clearReview(); throw new Error('确认已失效，请重新检查变更');
      }
      clearReview(); // 包括保存失败：旧的批准和预览不能再次使用。
      const result = await invoke('confirm_memory_change', { sessionId: state.vault.session_id,
        previewId: review.preview_id, approveProtected });
      if (!current()) return;
      invalidateMemoryContent(state);
      const message = review.operation === 'edit' ? '记忆修改已保存，原始出处保持原样'
        : review.operation === 'forget' ? '遗忘规则已保存，受影响内容已退出检索和交接'
          : result.hidden ? '这条遗忘规则已撤销；内容仍受其他遗忘规则影响，尚未回到检索'
            : !['active', 'tentative'].includes(result.status) ? '这条遗忘规则已撤销；记忆仍为已替代或撤回状态，不会作为有效记忆检索'
              : '这条遗忘规则已撤销，记忆已恢复可见';
      showNotice(message);
      try {
        const vault = await invoke('vault_status', { sessionId: state.vault.session_id });
        if (!current()) return;
        state.vault = vault;
        await fetchPage(0, current);
      } catch (error) {
        if (current()) { memory().error = String(error?.message || error); showNotice(`${message}；刷新列表失败，请重试`, true); }
      }
    }), '确认执行');
  }

  function sourcePane() {
    const selected = memory().selected;
    const box = $('details', { class: 'source-details memory-sources', open: Boolean(memory().source) }, $('summary', {}, `原始出处 · ${(selected.source_refs || []).length} 条`));
    for (const [index, eventId] of (selected.source_refs || []).entries()) box.append(button(`查看出处 ${index + 1}`, () => openSource(eventId), false, 'small memory-source-link'));
    const source = memory().source;
    if (source) box.append($('article', { class: 'sample memory-source' },
      $('div', { class: 'result-meta' }, $('strong', {}, source.conversation_title || source.metadata?.conversation_title || source.source?.conversation_title || '原始会话'), $('span', { class: 'tag' }, roleLabels[source.role] || source.role)),
      $('div', { class: 'muted' }, `${source.platform || source.source?.platform || '来源未知'} · ${displayDate(source.occurred_at)}`),
      $('div', { class: 'body-text' }, recordText(source) || '此来源没有文本正文'),
      !memory().selectedRow.hidden ? button('定位到这条消息', () => ui.locateEvent(`event:${source.id}`), false, 'small') : null,
      ui.technicalDetails(line('引用', source.id), line('原始会话', source.source?.conversation_id), line('原始消息', source.source?.message_id), line('保存时间', displayDate(source.captured_at)), source.source?.url ? line('出处地址', source.source.url) : null)));
    return box;
  }
  function detailsPane() {
    const selected = memory().selected;
    if (!selected) return $('aside', { class: 'memory-detail empty' }, $('h3', {}, '选择一条记忆'), paragraph('核对原话与出处，纠正内容或选择每次接续带上的背景。'));
    const row = memory().selectedRow;
    const head = $('div', { class: 'reader-heading' }, ui.detailBack(), $('div', { class: 'reader-title' }, $('h2', {}, '记忆详情'), $('span', { class: 'muted' }, evidenceLabels[selected.evidence] || selected.evidence)));
    if (!row.hidden && selected.source_refs?.length) head.append(ui.actionButton('带到另一个AI', () => ui.locateEvent(`event:${selected.source_refs[0]}`, true), 'open-continuation', true));
    const body = $('div', { class: 'reader-scroll', 'data-scroll': 'reader' }, $('div', { class: 'body-text memory-full-text' }, selected.content),
      $('div', { class: 'memory-status' }, $('span', { class: 'tag' }, `${row.hidden ? '已隐藏 · ' : ''}${displayState(selected.status)}`), $('span', { class: 'muted' }, selected.protected ? '已保护' : '未保护')),
      sourcePane(),
      $('div', { class: 'background-membership' }, line('每次接续带上', (selected.labels || []).includes('bootstrap') ? (selected.protected ? '已选择' : '已选择，尚需启用保护') : '未选择'), !row.hidden ? button('设置每次接续带上', () => ui.showBackground(false, selected.id), false, 'small') : null));
    if (selected.evidence === 'assistant_suggestion') body.append(hint('来自 AI 建议，尚非用户已确认事实。修改正文不会改变证据性质。', true));
    if (row.hidden) body.append(hint(row.can_restore ? '此内容已隐藏，仅供你主动检查。可以撤销这条记忆自己的遗忘规则。' : '此内容受其他记忆或来源的遗忘规则影响，不能在这里恢复。', true));
    if (!['active', 'tentative'].includes(selected.status)) body.append(hint('已替代或撤回的记忆不能直接编辑；撤销遗忘也不会改变该状态。'));
    if (!memory().mode) body.append($('div', { class: 'button-row' }, canEditMemory(selected, row) ? button('修改正文、标签与保护', () => begin('edit')) : null,
      row.can_restore ? button('撤销这条遗忘规则', () => begin('restore')) : button('设置遗忘规则', () => begin('forget'), false, 'danger')));
    else body.append(editor());
    const preview = reviewPane(); if (preview) body.append(preview);
    body.append(ui.technicalDetails(line('引用', selected.id), line('版本', selected.revision), line('标签', editableMemoryLabels(selected.labels).join('、') || '无'), line('记录时间', displayDate(selected.recorded_at)), line('更新时间', displayDate(selected.updated_at)), line('所述事情的时间', displayDate(selected.observed_at)), selected.time_note ? line('时间说明', selected.time_note) : null, selected.valid_from ? line('有效起始时间', displayDate(selected.valid_from)) : null, selected.valid_to ? line('有效截止时间', displayDate(selected.valid_to)) : null));
    return $('aside', { class: 'memory-detail reader-pane' }, head, body);
  }
  function editor() {
    const mode = memory().mode;
    const box = $('section', { class: 'memory-editor' }, $('h3', {}, operationLabels[mode]));
    if (mode === 'edit') {
      const draft = memory().draft;
      const text = $('textarea', { id: 'memory-content', rows: 7, maxlength: 65536, 'aria-label': '记忆正文' });
      text.value = draft.content;
      text.addEventListener('input', () => { draft.content = text.value; clearReview(); });
      const labels = $('textarea', { id: 'memory-labels', rows: 3, 'aria-label': '记忆标签', placeholder: '每行一个标签' });
      labels.value = draft.labels;
      labels.addEventListener('input', () => { draft.labels = labels.value; clearReview(); });
      const protect = $('input', { type: 'checkbox', id: 'memory-protected' });
      protect.checked = Boolean(draft.protected);
      protect.addEventListener('change', () => { draft.protected = protect.checked; clearReview(); });
      box.append($('label', { for: 'memory-content' }, '记忆正文'), text,
        $('label', { for: 'memory-labels' }, '记忆标签（每行一个）'), labels,
        $('label', { for: 'memory-protected', class: 'check' }, protect, '保护这条记忆，后续变更需要额外确认'),
        hint('这里只修改正文、标签与保护设置。是否每次接续带上，请在记忆的背景选择中设置。证据性质、原始出处、原始时间和事实状态保持原样。'));
    } else if (mode === 'forget') {
      const reason = $('textarea', { id: 'memory-forget-reason', rows: 3, maxlength: 4096, 'aria-label': '遗忘原因', placeholder: '说明这次为什么不再使用这些内容' });
      reason.value = memory().reason;
      reason.addEventListener('input', () => { memory().reason = reason.value; clearReview(); });
      box.append(hint('将隐藏这条记忆、它的原始来源和依赖相同来源的其他记忆。资料文件保留，可以撤销规则。', true),
        $('label', { for: 'memory-forget-reason' }, '遗忘原因'), reason);
    } else box.append(hint('只撤销这条记忆自己的遗忘规则；其他遗忘规则和记忆的撤回、替代状态仍然生效。', true));
    box.append($('div', { class: 'button-row' }, button('取消操作', cancel), button('检查变更与影响', review, true)));
    return box;
  }
  function reviewPane() {
    const review = memory().review;
    if (!review) return null;
    const box = $('section', { id: 'memory-review', class: 'panel file-preview', 'aria-label': '记忆变更预览' },
      $('h2', {}, '确认变更与影响范围'), hint(review.warning, review.operation !== 'edit'),
      line('操作', operationLabels[review.operation]), line('受影响记忆', review.affected_memories),
      line('受影响原始记录', review.affected_events));
    const change = $('div', { class: 'change' }, $('div', { class: 'before' }, `修改前\n${review.before.content}`));
    if (review.after) change.append($('div', { class: 'after' }, `修改后\n${review.after.content}`),
      line('保护设置', `${review.before.protected ? '已保护' : '未保护'} → ${review.after.protected ? '已保护' : '未保护'}`),
      line('原有标签', editableMemoryLabels(review.before.labels).join('、') || '无'),
      line('修改后标签', editableMemoryLabels(review.after.labels).join('、') || '无'));
    else change.append($('div', { class: 'after' }, review.operation === 'forget'
      ? '执行后：这些内容退出检索、交接和后续整理；保留资料文件'
      : '执行后：撤销这条规则；仍受其他规则影响的内容可能继续隐藏'));
    box.append(change);
    const approval = $('input', { id: 'memory-protected-approval', type: 'checkbox' });
    if (review.requires_protected_approval) box.append($('label', { for: 'memory-protected-approval', class: 'check' }, approval,
      '受影响记忆中有受保护内容。我已检查影响范围，同意本次变更。'));
    box.append($('div', { class: 'button-row' }, button('取消本次变更', cancel),
      button('继续确认', () => askConfirm(review, Boolean(approval.checked)), true)));
    return box;
  }
  function page() {
    content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '记忆'), $('span', { class: 'muted' }, `${memory().total || 0} 条记忆`)));
    content.append(ui.memoryToolbar());
    const hidden = $('input', { type: 'checkbox', id: 'include-hidden-memories' });
    hidden.checked = memory().includeHidden;
    hidden.addEventListener('change', () => { const next = hidden.checked; hidden.checked = memory().includeHidden; ui.discardBefore(() => { memory().includeHidden = next; load(0); }); });
    content.append($('div', { class: 'toolbar memory-options' },
      $('label', { class: 'check', for: 'include-hidden-memories' }, hidden, '显示隐藏与失效记忆'),
      button('刷新记忆列表', () => ui.discardBefore(() => load(0)), false, 'small')));
    if (memory().includeHidden) content.append(hint('此列表包含平时不会提供给 AI 的内容。已隐藏内容只供你在管理页主动检查。', true));
    if (memory().error) { content.append(hint(`记忆列表读取失败：${memory().error}`, true), button('重试读取记忆', () => load(0))); return; }
    if (!memory().loaded) { content.append(paragraph(state.busy ? '正在读取记忆…' : '正在准备记忆列表…')); return; }
    if (!memory().rows.length) {
      content.append($('div', { class: 'empty' }, $('h3', {}, '当前范围没有可显示的长期记忆'),
        paragraph(memory().includeHidden ? '可以切换资料范围，或在整理记忆页保存新的记忆。' : '可以在整理记忆页保存记忆，或主动勾选查看已隐藏和已失效的内容。'))); return;
    }
    content.append($('div', { class: 'section-heading' },
      $('span', {}, `共 ${memory().total} 条 · 当前 ${memory().offset + 1}–${memory().offset + memory().rows.length} 条 · 每页最多 30 条`)));
    const list = $('section', { class: 'results memory-list list-scroll', 'data-scroll': 'list', 'aria-label': '记忆列表' });
    for (const row of memory().rows) {
      list.append($('button', { class: `result-card ${memory().selected?.id === row.id ? 'selected' : ''}`, 'data-memory-id': row.id, 'aria-pressed': String(memory().selected?.id === row.id), onclick: () => open(row) },
        $('div', { class: 'result-meta' }, $('span', { class: 'tag' }, `${row.hidden ? '已隐藏 · ' : ''}${displayState(row.status)}`),
          $('span', { class: 'tag purple' }, row.protected ? '已保护' : '未保护')),
        paragraph(shorten(row.content, 130)), $('div', { class: 'muted' }, displayDate(row.updated_at))));
    }
    list.append($('div', { class: 'button-row' },
      memory().offset > 0 ? button('上一页', () => ui.discardBefore(() => load(Math.max(0, memory().offset - 30)))) : null,
      memory().nextOffset != null ? button('下一页', () => ui.discardBefore(() => load(memory().nextOffset))) : null));
    content.append($('div', { class: `results-layout memory-layout workspace-split ${state.mobileDetail && memory().selected ? 'show-detail' : ''}` }, list, detailsPane()));

  }
  return { page, load, discard, clearReview };
}
