import { displayDate, displayState, recordRef, recordText, shorten } from './model.js';

export function createContextSelection(ui) {
  const { state, $, button, paragraph, hint, invoke, run, render, showNotice } = ui;
  const budget = 32768;
  const refValid = value => typeof value === 'string' && value.length <= 256 && /^(event:[A-Za-z0-9_-]+|memory:[A-Za-z0-9_-]+@[1-9][0-9]*)$/.test(value);
  const role = value => ({ user: '用户原话', assistant: 'AI 回复', tool: '工具结果', system: '系统来源' })[value] || '角色未知';
  const evidence = value => ({ UserExplicit: '用户明确表达', user_explicit: '用户明确表达', Observed: '观察所得', observed: '观察所得', AssistantSuggestion: 'AI 建议，尚非用户事实', assistant_suggestion: 'AI 建议，尚非用户事实' })[value] || '证据性质未分类';
  const data = () => state.contextSelection ||= { open: false, candidates: [], references: [], query: '', key: '', goal: '', preview: null, version: 0 };
  const title = row => row.conversation_title || row.title || shorten(recordText(row), 52) || '未命名资料';
  const available = () => state.results.filter(row => refValid(recordRef(row)));
  const same = (selected, version, epoch, session, scope) => state.contextSelection === selected && selected.version === version && state.epoch === epoch && state.vault?.session_id === session && state.scope === scope && state.page === 'search' && selected.open;
  function invalidate() { if (state.contextSelection) { state.contextSelection.preview = null; state.contextSelection.version++; } }
  function clear() { invalidate(); state.contextSelection = null; }
  function refreshResults() {
    const selected = state.contextSelection;
    if (!selected) return;
    invalidate(); selected.open = false;
    if (selected.query !== (state.resultQuery || '')) { clear(); return; }
    const rows = available(), previousCount = selected.references.length;
    selected.candidates = rows; selected.key = rows.map(recordRef).join('\n');
    selected.references = rows.map(recordRef).filter(reference => selected.references.includes(reference));
    if (selected.references.length !== previousCount) showNotice('部分所选资料已不在当前结果中，请检查剩余选择后继续');
  }
  function close() { if (state.busy) return; invalidate(); data().open = false; state.mobileDetail = false; render(); }
  const request = selected => ({ sessionId: state.vault.session_id, scope: state.scope, references: [...selected.references], goal: selected.goal, budgetTokens: budget });

  function validate(result, payload) {
    const invalid = () => { throw new Error('交接内容与所选资料不一致，请重新准备'); };
    if (!result || result.scope !== payload.scope || typeof result.text !== 'string' || !result.text || typeof result.stable_prefix !== 'string' || !result.stable_prefix || !result.text.startsWith(result.stable_prefix) || !result.background || typeof result.background.stable_text !== 'string' || typeof result.background.bootstrap_version !== 'string' || !result.background.bootstrap_version || !result.stable_prefix.endsWith(result.background.stable_text) || !Array.isArray(result.records) || !Array.isArray(result.pending_refs) || !Array.isArray(result.selected_refs) || JSON.stringify(result.selected_refs) !== JSON.stringify(payload.references) || result.selected_count !== payload.references.length || result.included_count !== result.records.length || typeof result.truncated !== 'boolean' || typeof result.partial !== 'boolean' || result.budget_tokens !== budget || result.budget_unit !== 'conservative_utf8_bytes' || !Number.isSafeInteger(result.estimated_tokens) || result.estimated_tokens < 1 || result.estimated_tokens > budget || new Blob([JSON.stringify(result)]).size > budget) invalid();
    const included = result.records.map(record => record.ref), pending = result.pending_refs;
    if (new Set([...included, ...pending]).size !== payload.references.length || [...included, ...pending].some(reference => !payload.references.includes(reference)) || JSON.stringify(included) !== JSON.stringify(payload.references.filter(reference => included.includes(reference))) || JSON.stringify(pending) !== JSON.stringify(payload.references.filter(reference => !included.includes(reference))) || (pending.length && (!result.truncated || !result.partial))) invalid();
    for (const record of result.records) {
      if (!refValid(record.ref) || typeof record.text !== 'string' || !['event', 'memory'].includes(record.kind) || !record.ref.startsWith(`${record.kind}:`) || typeof record.truncated !== 'boolean' || !Array.isArray(record.source_refs) || !Array.isArray(record.sources) || (record.kind === 'memory' ? record.role !== null : !['user', 'assistant', 'tool', 'system'].includes(record.role))) invalid();
    }
    return result;
  }
  async function prepare(control = null) {
    const selected = data();
    if (state.busy || !selected.open || state.page !== 'search' || (control && !control.isConnected) || !selected.references.length || selected.references.length > 8) return;
    invalidate();
    const version = selected.version, epoch = state.epoch, session = state.vault?.session_id, scope = state.scope, payload = request(selected);
    await run('正在核对所选原文与出处…', async current => {
      render();
      const result = await invoke('prepare_selected_context', payload);
      if (current() && same(selected, version, epoch, session, scope)) selected.preview = validate(result, payload);
    });
  }
  async function open() {
    if (state.busy || state.page !== 'search' || !state.vault) return;
    const rows = available(); if (!rows.length) return;
    const selected = data(), key = rows.map(recordRef).join('\n'), query = state.resultQuery || '';
    if (selected.key !== key || selected.query !== query) {
      selected.candidates = rows; selected.key = key; selected.query = query;
      selected.references = rows.slice(0, 3).map(recordRef); selected.goal = query;
    }
    selected.open = true; state.mobileDetail = true; render();
    return prepare();
  }
  function focusChoice(reference) {
    [...document.querySelectorAll('.results .context-check input')]
      .find(input => input.dataset.selectionReference === reference)?.focus({ preventScroll: true });
  }
  function toggle(reference, checked, control) {
    const selected = data();
    if (state.busy || !control.isConnected || !selected.open || !selected.candidates.some(row => recordRef(row) === reference)) return;
    const keepFocus = document.activeElement === control;
    if (checked && !selected.references.includes(reference)) {
      if (selected.references.length >= 8) { showNotice('一次最多带上 8 条资料，请先取消一条'); render(); if (keepFocus) focusChoice(reference); return; }
      selected.references.push(reference);
    } else if (!checked) selected.references = selected.references.filter(value => value !== reference);
    selected.references = selected.candidates.map(recordRef).filter(value => selected.references.includes(value));
    invalidate(); render(); if (keepFocus) focusChoice(reference);
  }
  function checkbox(row) {
    if (!data().open) return null;
    const reference = recordRef(row); if (!refValid(reference)) return null;
    const position = data().candidates.findIndex(candidate => recordRef(candidate) === reference) + 1;
    const description = [`第 ${position} 条`, reference.startsWith('memory:') ? '整理记忆' : role(row.role), shorten(title(row), 60), shorten(recordText(row), 60)].filter(Boolean).join('，');
    const input = $('input', { type: 'checkbox', 'aria-label': `带上资料：${description}`, 'data-selection-reference': reference });
    input.checked = data().references.includes(reference);
    input.addEventListener('change', () => toggle(reference, input.checked, input));
    return $('label', { class: 'context-check' }, input, $('span', { class: 'sr-only' }, '带上这条资料'));
  }
  function inspect(reference, control) {
    if (state.busy || !control.isConnected) return;
    const selected = data(), candidate = selected.candidates.find(row => recordRef(row) === reference);
    if (!candidate) return;
    invalidate(); selected.open = false; ui.readRecord(candidate);
  }
  async function copy(control) {
    const selected = data(), shown = selected.preview;
    if (state.busy || !control.isConnected || !selected.open || !shown || !shown.included_count || state.page !== 'search') return;
    const payload = request(selected); invalidate();
    const version = selected.version, epoch = state.epoch, session = state.vault?.session_id, scope = state.scope;
    await run('正在再次核对资料与分享内容…', async current => {
      render();
      const response = await invoke('prepare_selected_context', payload);
      if (!current() || !same(selected, version, epoch, session, scope)) return;
      const fresh = validate(response, payload);
      if (fresh.text !== shown.text || fresh.stable_prefix !== shown.stable_prefix || fresh.background.bootstrap_version !== shown.background.bootstrap_version) throw new Error('资料、背景或访问权限已改变，请重新准备并检查交接内容');
      await invoke('write_clipboard', { text: shown.text });
      if (current() && same(selected, version, epoch, session, scope)) { selected.preview = fresh; showNotice('已复制，可粘贴到你选择的 AI'); }
    });
  }
  function pane() {
    const selected = data(), shown = selected.preview;
    const goal = $('textarea', { rows: 3, maxlength: 2000, 'aria-label': '接下来要做什么', placeholder: '接下来希望 AI 帮你做什么？' });
    goal.value = selected.goal;
    goal.addEventListener('input', () => {
      if (state.busy || !goal.isConnected) { goal.value = selected.goal; return; }
      selected.goal = goal.value; invalidate();
      document.querySelector('#selection-context-preview')?.remove();
      const copyButton = document.querySelector('#selection-context-panel .copy-continuation');
      if (copyButton) { copyButton.disabled = true; copyButton.setAttribute('data-disabled', 'true'); }
    });
    const body = $('div', { class: 'continuation-scroll', 'data-scroll': 'continuation' },
      paragraph('先按当前列表顺序带上前 3 条，可以调整选择。资料可能来自不同会话，不代表同一时间线。'),
      $('strong', {}, `已选 ${selected.references.length} / 8 条资料`),
      button('调整所选资料', () => { if (state.busy) return; state.mobileDetail = false; render(); document.querySelector('.context-check input')?.focus(); }),
      $('label', {}, '下一步目标', goal));
    if (!selected.references.length) body.append(hint('请在结果列表选择至少一条资料。'));
    if (shown) {
      const preview = $('textarea', { rows: 12, readonly: true, 'aria-label': '交接内容预览' }); preview.value = shown.text;
      const section = $('section', { id: 'selection-context-preview' }, hint(`实际带上 ${shown.included_count} / ${shown.selected_count} 条所选资料`));
      const backgroundCount = Array.isArray(shown.background.refs) ? shown.background.refs.length : 0;
      section.append($('details', { class: 'handoff-settings' }, $('summary', {}, backgroundCount ? `同时带上 ${backgroundCount} 条随身背景` : '随身背景与使用说明'), paragraph(shown.background.stable_text)));
      if (shown.truncated) section.append(hint('内容受长度限制，节选和未带入的引用已在预览里标明。', true));
      for (const record of shown.records) {
        const provenance = record.kind === 'memory' ? `整理记忆 · ${displayState(record.status)} · ${evidence(record.evidence)}` : `${role(record.role)} · ${evidence(record.evidence)} · ${displayDate(record.occurred_at)}`;
        const item = $('article', { class: 'selected-context-record', 'data-context-reference': record.ref },
          $('strong', {}, record.conversation_title || (record.kind === 'memory' ? '记忆条目，非原始逐字引文' : '原始资料')),
          paragraph(provenance), paragraph(record.text));
        if (record.content_retained === false) item.append(hint('这条文件记录没有保留正文，不能据此补写文件内容。'));
        if (record.truncated) item.append(hint('本条仅节选'));
        item.append(button('核对原文与出处', event => inspect(record.ref, event.currentTarget), false, 'small'));
        section.append(item);
      }
      if (shown.pending_refs.length) section.append(paragraph(`${shown.pending_refs.length} 条所选资料没有放入本次内容，请调整选择后重新准备。`),
        $('ul', {}, shown.pending_refs.map(reference => $('li', {}, title(selected.candidates.find(row => recordRef(row) === reference) || { text: '未命名资料' })))));
      if (!shown.included_count) section.append(hint('本次未能放入所选资料，请调整选择后重新准备。', true));
      section.append($('details', { class: 'continuation-text' }, $('summary', {}, '检查完整交接内容'), preview));
      body.append(section);
    } else body.append(paragraph('选择或目标改变后，请重新准备。只有预览中的内容会被复制。'));
    const prepareButton = button('准备交接内容', event => prepare(event.currentTarget), !shown);
    prepareButton.setAttribute('data-disabled', String(!selected.references.length));
    const copyButton = button('复制交接内容', event => copy(event.currentTarget), true, 'copy-continuation');
    copyButton.setAttribute('data-disabled', String(!shown || !shown.included_count));
    body.append(paragraph('请确认这些资料可以分享。复制后由你决定粘贴到哪个 AI，并在目标网页亲自发送。'));
    return $('aside', { id: 'selection-context-panel', class: 'continuation-pane', 'aria-label': '带到另一个AI' },
      $('div', { class: 'reader-heading' }, $('h2', {}, '带到另一个AI'), button('返回搜索结果', close)), body,
      $('div', { class: 'continuation-actions' }, prepareButton, copyButton));
  }
  return { open, close, clear, invalidate, refreshResults, checkbox, pane, isOpen: () => Boolean(state.contextSelection?.open) };
}
