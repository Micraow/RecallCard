import { displayDate, displayState, newBackgroundState, recordText } from './model.js';
import { invalidateMemoryContent } from './memory-management.js';

const evidenceLabels = { user_explicit: '用户明确表达', observed: '观察所得', assistant_suggestion: 'AI 建议，尚非用户事实' };
const roleLabels = { user: '用户原话', assistant: 'AI 回复', tool: '工具结果', system: '系统消息' };

export function backgroundReviewKey(state) {
  return JSON.stringify([state.epoch, state.vault?.session_id, state.scope, state.background.draft]);
}
export function sameBackground(a, b) {
  return Boolean(a && b && typeof a.stable_text === 'string' && a.stable_text.length
    && typeof a.bootstrap_version === 'string' && a.bootstrap_version.length
    && typeof a.copy_text === 'string' && a.copy_text.length
    && a.bootstrap_version === b.bootstrap_version && a.stable_text === b.stable_text && a.copy_text === b.copy_text);
}

export function createBackground(ui) {
  const { state, content, $, button, paragraph, heading, panel, hint, line, scopeSelect,
    invoke, run, render, showNotice, confirmDialog, navigate } = ui;
  const background = () => state.background;
  const args = () => ({ sessionId: state.vault.session_id, scope: state.scope });

  function clearReview() {
    background().review = null; background().reviewKey = '';
    document.querySelector('#background-review')?.remove();
  }
  function discard() {
    clearReview(); state.background = newBackgroundState();
    document.querySelector('#background-current')?.remove();
  }
  async function cancelServerPreviews() {
    clearReview();
    state.memory.review = null; state.memory.reviewKey = '';
    state.importPreview = null; state.importSelection = null; state.importSelectedIds = [];
    state.dreamPreview = null; state.notePreview = null;
    await invoke('cancel_previews', { sessionId: state.vault.session_id });
  }
  async function fetchPage(offset, current) {
    const result = await invoke(offset ? 'read_background_page' : 'read_background',
      { ...args(), ...(offset ? { offset } : {}) });
    if (!current()) return;
    if (state.memoryFilter === 'selected' && !offset) {
      let next = result.next_offset; let previous = 0;
      while (next != null && next > previous) {
        const page = await invoke('read_background_page', { ...args(), offset: next }); if (!current()) return;
        if (!sameBackground({ ...result.background, copy_text: result.copy_text }, { ...page.background, copy_text: page.copy_text })) throw new Error('资料已改变，请重新读取背景');
        result.candidates.push(...page.candidates || []); previous = next; next = page.next_offset;
      }
      result.candidates = result.candidates.filter(row => row.selected); result.next_offset = null;
    }
    Object.assign(background(), { snapshot: { ...result.background, copy_text: result.copy_text }, rows: result.candidates || [],
      total: result.total, selectedCount: result.selected_count, offset,
      nextOffset: result.next_offset, loaded: true, error: '' });
  }
  async function load(offset = 0) {
    await run('正在核对随身背景与可选记忆…', async current => {
      discard(); render();
      try { await cancelServerPreviews(); await fetchPage(offset, current); }
      catch (error) { if (current()) background().error = String(error?.message || error); throw error; }
    });
  }
  function open(row) { if (state.busy) return; if (background().draft && background().draft.id !== row.id) { const offset = background().offset; ui.discardBefore(async () => { await load(offset); const fresh = background().rows.find(item => item.id === row.id); if (fresh) await openNow(fresh); }); } else return openNow(row); }
  async function openNow(row) {
    await run('正在读取这条记忆与出处…', async current => {
      if (background().selected?.id !== row.id) ui.resetReaderView();
      if (background().draft?.id === row.id) {
        const key = backgroundReviewKey(state);
        try {
          const selected = await invoke('background_memory', { ...args(), id: row.id });
          if (!current() || key !== backgroundReviewKey(state)) return;
          if (selected.revision !== row.revision) throw new Error('记忆已有新版本，请刷新后重新查看');
          background().selected = selected; background().selectedRow = row; background().source = null; state.mobileDetail = true;
        } catch (error) { if (current()) { discard(); background().error = String(error?.message || error); } throw error; }
        return;
      }
      const offset = background().offset;
      discard();
      await cancelServerPreviews();
      try {
        await fetchPage(offset, current);
        if (!current()) return;
        const freshRow = background().rows.find(item => item.id === row.id);
        if (!freshRow) throw new Error('记忆列表已改变，请重新选择');
        const selected = await invoke('background_memory', { ...args(), id: row.id });
        if (!current()) return;
        if (selected.revision !== freshRow.revision) throw new Error('记忆已有新版本，请刷新后重新查看');
        background().selected = selected; background().selectedRow = freshRow; state.mobileDetail = true;
      } catch (error) { if (current()) { discard(); background().error = String(error?.message || error); } throw error; }
    });
  }
  async function openSource(memoryId, eventId) {
    await run('正在读取原始出处…', async current => {
      background().source = null;
      try {
        const source = await invoke('background_memory_source', { ...args(), memoryId, eventId });
        if (current() && (background().selected?.id === memoryId || background().review?.before.id === memoryId)) background().source = source;
      } catch (error) { if (current()) { discard(); background().error = String(error?.message || error); } throw error; }
    });
  }
  function change(row, include) {
    if (state.busy || (state.page !== 'memories' || state.memoryFilter === 'all') || !background().rows.includes(row)) return;
    if (background().draft && background().draft.id !== row.id) { const offset = background().offset; clearReview(); render(); ui.discardBefore(async () => { await load(offset); const fresh = background().rows.find(item => item.id === row.id); if (fresh) change(fresh, include); }); return; }
    if (background().draft?.id === row.id && include === row.selected) { load(background().offset); return; }
    if (include ? !row.can_include : !row.can_remove) { render(); return; }
    clearReview(); background().source = null;
    if (background().selected?.id !== row.id) background().selected = null;
    background().selectedRow = row;
    background().draft = { id: row.id, revision: row.revision, include };
    state.continuation = null; state.mobileDetail = true;
    render();
  }
  async function review() {
    await run('正在检查随身背景变更…', async current => {
      if (!background().draft) throw new Error('请先选择要改变的记忆');
      const key = backgroundReviewKey(state);
      const draft = { ...background().draft };
      await cancelServerPreviews();
      try {
        const result = await invoke('review_background_change', { ...args(), ...draft });
        if (current() && key === backgroundReviewKey(state)) {
          background().review = result; background().reviewKey = key;
          background().snapshot = { ...result.background_before, copy_text: result.copy_text_before };
        }
      } catch (error) { if (current()) { discard(); background().error = String(error?.message || error); } throw error; }
    });
  }
  function askConfirm(review, approveProtected) {
    if (background().review !== review || background().reviewKey !== backgroundReviewKey(state)) {
      clearReview(); showNotice('选择已改变，请重新检查随身背景', true); return;
    }
    if (review.requires_protected_approval && !approveProtected) {
      showNotice('这条记忆已受保护，请先勾选额外确认', true); return;
    }
    confirmDialog(review.include ? '确认每次接续带上' : '确认不再每次带上',
      `${review.include ? '将这条记忆加入随身背景并保护。' : '从随身背景移除这条记忆，保留记忆正文与现有保护，可随时重新选择。'}资料库：${state.vault.display_name}。资料或权限改变后，须重新检查。`,
      () => run('正在保存随身背景选择…', async current => {
        if (background().review !== review || background().reviewKey !== backgroundReviewKey(state)) {
          clearReview(); throw new Error('确认已失效，请重新检查随身背景');
        }
        const payload = { ...args(), previewId: review.preview_id, approveProtected };
        discard();
        let saved = false;
        try {
          await invoke('confirm_background_change', payload);
          saved = true;
          if (!current()) return;
          invalidateMemoryContent(state);
          await fetchPage(0, current);
          if (current()) showNotice(review.include ? '已加入随身背景并保护；下次接续按下方实际内容带上' : '已从随身背景移除；记忆与保护仍保留，可以重新选择');
        } catch (error) {
          const message = `${saved ? '选择已保存，但重新读取失败：' : ''}${String(error?.message || error)}`;
          if (current()) { discard(); background().error = message; }
          throw new Error(message);
        }
      }), '确认保存选择');
  }
  async function copy() {
    await run('正在重新核对实际随身背景…', async current => {
      const shown = background().snapshot;
      if ((state.page !== 'memories' || state.memoryFilter === 'all') || background().draft || background().review || !sameBackground(shown, shown)) {
        throw new Error('请先保存或取消选择，再重新检查实际背景');
      }
      // 读取失败、权限变化和版本变化都移除旧内容，不能再点旧复制回调。
      background().snapshot = null; render();
      let fresh;
      try { fresh = await invoke('read_background', args()); }
      catch (error) { if (current()) { discard(); background().error = String(error?.message || error); } throw error; }
      if (!current() || (state.page !== 'memories' || state.memoryFilter === 'all')) return;
      if (!sameBackground(shown, { ...fresh.background, copy_text: fresh.copy_text })) {
        discard(); background().error = '资料或权限已改变，请重新读取并检查背景';
        throw new Error(background().error);
      }
      background().snapshot = { ...fresh.background, copy_text: fresh.copy_text };
      await invoke('write_clipboard', { text: fresh.copy_text });
      if (current()) showNotice('已复制实际随身背景，检查适合分享后再粘贴到你选择的 AI');
    });
  }
  function textPreview(snapshot, label, copyText = snapshot?.copy_text) {
    const text = $('textarea', { rows: 9, readonly: true, 'aria-label': label });
    text.value = copyText || '';
    return $('div', { class: 'background-text' }, text,
      snapshot?.truncated ? hint('长度有限，本次只带上部分背景。下面的正文就是实际内容；已选择的记忆可能没有完整出现。', true) : null);
  }
  function currentPane() {
    const snapshot = background().snapshot;
    if (!snapshot) return null;
    const box = $('details', { id: 'background-current', class: 'background-current', open: Boolean(background().actualOpen) },
      $('summary', {}, `当前实际带上的背景 · ${background().selectedCount || 0} 条已选择`), paragraph('检查实际内容后，再复制到目标 AI。'),
      background().selectedCount === 0 ? hint('还没有选择随身记忆。当前只有使用说明与资料目录。') : null,
      textPreview(snapshot, '当前实际随身背景'));
    box.addEventListener('toggle', () => { if (box.isConnected) background().actualOpen = box.open; });
    if (background().draft || background().review) box.append(hint('选择尚未保存。请先检查并保存，或取消本次选择后再复制。'));
    else box.append($('div', { class: 'button-row' }, button('复制当前随身背景', copy, true),
      button('带上背景继续会话', () => navigate('conversations'))));
    return box;
  }
  function sources(memory) {
    const box = $('details', { class: 'source-details memory-sources', open: Boolean(background().source) }, $('summary', {}, `原始出处 · ${(memory.source_refs || []).length} 条`));
    for (const [index, eventId] of (memory.source_refs || []).entries()) box.append(button(`查看背景出处 ${index + 1}`, () => openSource(memory.id, eventId), false, 'small memory-source-link'));
    const source = background().source;
    if (source) box.append($('article', { class: 'sample background-source' },
      $('div', { class: 'result-meta' }, $('strong', {}, source.conversation_title || source.metadata?.conversation_title || source.source?.conversation_title || '原始会话'), $('span', { class: 'tag' }, roleLabels[source.role] || source.role)),
      $('div', { class: 'muted' }, `${source.platform || source.source?.platform || '来源未知'} · ${displayDate(source.occurred_at)}`),
      $('div', { class: 'body-text' }, recordText(source) || '此来源没有文本正文'), button('定位到这条消息', () => ui.locateEvent(`event:${source.id}`), false, 'small'),
      ui.technicalDetails(line('引用', source.id), line('会话引用', source.source?.conversation_id), line('保存时间', displayDate(source.captured_at)), source.source?.url ? line('出处地址', source.source.url) : null)));
    return box;
  }
  function detailsPane() {
    const selected = background().selected;
    if (!selected) return panel($('h3', {}, '选择记忆，随时核对出处'), paragraph('点“查看全文与出处”可阅读完整内容。勾选只准备本次变更，检查并确认后才保存。'));
    return panel($('h2', {}, '记忆全文'), $('div', { class: 'body-text background-full-text' }, selected.content),
      line('状态', displayState(selected.status)), line('证据性质', evidenceLabels[selected.evidence] || selected.evidence),
      line('保护', selected.protected ? '已保护' : '未保护'), line('记录时间', displayDate(selected.recorded_at)),
      line('所述事情的时间', displayDate(selected.observed_at)), selected.time_note ? line('时间说明', selected.time_note) : null,
      sources(selected));
  }
  function reviewPane() {
    const review = background().review;
    if (!review) return null;
    const box = $('section', { id: 'background-review', class: 'panel file-preview', 'aria-label': '随身背景变更预览' },
      $('h2', {}, '检查这次随身背景变更'), hint(review.warning),
      line('每次接续带上', review.include ? '本次加入' : '本次移除，随时可再加入'),
      line('保护设置', `${review.before.protected ? '已保护' : '未保护'} → ${review.after.protected ? '已保护' : '未保护'}`),
      line('证据性质', evidenceLabels[review.before.evidence] || review.before.evidence),
      $('div', { class: 'body-text' }, review.before.content), sources(review.before),
      $('h3', {}, '保存后的实际背景'), textPreview(review.background_after, '保存后的实际随身背景', review.copy_text_after),
      $('details', {}, $('summary', {}, '对照保存前的实际背景'), textPreview(review.background_before, '保存前的实际随身背景', review.copy_text_before)));
    const approval = $('input', { id: 'background-protected-approval', type: 'checkbox' });
    if (review.requires_protected_approval) box.append($('label', { class: 'check', for: 'background-protected-approval' }, approval,
      '这条记忆已受保护。我已检查实际背景与出处，明确同意本次变更。'));
    box.append($('div', { class: 'button-row' }, button('取消本次选择', () => load(background().offset)),
      button('保存随身背景选择', () => askConfirm(review, Boolean(approval.checked)), true)));
    return box;
  }
  function page() {
    content.append($('div', { class: 'workspace-heading' }, $('h1', {}, '记忆'), $('span', { class: 'muted' }, '每次接续带上')),
      ui.memoryToolbar());
    if (background().error) { content.append(hint(`随身背景需要重新核对：${background().error}`, true), button('重新读取随身背景', () => load())); return; }
    if (!background().loaded) { content.append(paragraph('正在读取随身背景…')); return; }
    content.append($('div', { class: 'section-heading' }, $('span', {}, `已选择 ${background().selectedCount ?? 0} 条 · ${state.memoryFilter === 'selected' ? '已选背景' : '选择背景'}`), $('div', { class: 'button-row' }, button('查看实际背景', () => { background().actualOpen = true; ui.resetReaderView(); state.mobileDetail = true; render(); }, false, 'small'), button('刷新随身背景', () => ui.discardBefore(() => load()), false, 'small'))));
    const list = $('section', { class: 'results background-list list-scroll', 'data-scroll': 'list', 'aria-label': '随身背景记忆列表' });
    const rows = state.memoryFilter === 'selected' ? background().rows.filter(row => row.selected || background().draft?.id === row.id) : background().rows;
    if (!rows.length) list.append($('div', { class: 'empty' }, $('h3', {}, state.memoryFilter === 'selected' ? '还没有已选背景' : '这个范围还没有记忆可选'), paragraph('从已有会话整理记忆，核对后选择每次带上。')));
    for (const row of rows) {
      const draft = background().draft?.id === row.id ? background().draft : null;
      const selected = draft ? draft.include : row.selected;
      const disabled = draft ? false : selected ? !row.can_remove : !row.can_include;
      const check = $('input', { type: 'checkbox', 'aria-label': `每次接续带上：${row.content}`, 'data-disabled': disabled ? 'true' : null });
      check.checked = Boolean(selected); check.addEventListener('change', () => change(row, check.checked));
      const card = $('article', { class: `result-card background-candidate ${background().selected?.id === row.id ? 'selected' : ''}`, 'data-memory-id': row.id },
        $('div', { class: 'result-meta' }, $('span', { class: 'tag' }, displayState(row.status)), $('span', { class: 'muted' }, row.protected ? '已保护' : '未保护')),
        paragraph(row.content), row.text_truncated ? $('span', { class: 'field-note' }, '这里只显示片段；请查看全文与出处。') : null, $('div', { class: 'candidate-actions' }, $('label', { class: 'check' }, check, '每次接续带上'), button('查看全文与出处', () => open(row), false, 'small')),
        row.reason ? $('span', { class: 'field-note' }, row.reason) : null,
        draft ? $('strong', { class: 'pending-change' }, draft.include ? '待保存：加入并保护' : '待保存：移除；保留记忆与保护') : null);
      if (row.selected && row.can_include) card.append(button('启用并保护这条背景', () => change(row, true), false, 'small'));
      list.append(card);
    }
    list.append($('div', { class: 'list-pagination' }, background().offset > 0 ? button('上一页记忆', () => ui.discardBefore(() => load(Math.max(0, background().offset - 30)))) : null,
      background().nextOffset != null ? button('下一页记忆', () => ui.discardBefore(() => load(background().nextOffset))) : null));
    const body = $('div', { class: 'reader-scroll', 'data-scroll': 'reader' });
    const actual = currentPane(); if (actual) body.append(actual);
    if (background().review) body.append(reviewPane());
    else {
      body.append(detailsPane());
      if (background().draft) body.append($('section', { class: 'pending-background' }, $('h3', {}, '本次选择尚未保存'), paragraph('检查实际文字与出处，确认后保存。'), $('div', { class: 'button-row' }, button('取消本次选择', () => load(background().offset)), button('检查随身背景变更', review, true))));
    }
    const pane = $('aside', { class: 'reader-pane background-detail' }, $('div', { class: 'reader-heading' }, ui.detailBack(), $('h2', {}, background().review ? '核对背景变更' : '背景与出处')), body);
    content.append($('div', { class: `results-layout background-layout workspace-split ${state.mobileDetail ? 'show-detail' : ''}` }, list, pane));
  }
  return { page, load, open, discard, clearReview };
}
