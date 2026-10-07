export const pages = [['conversations', '会话', '▤'], ['memories', '记忆', '▥']];
export function shorten(value, limit = 90) { const text = String(value ?? ''); return text.length > limit ? `${text.slice(0, limit)}…` : text; }
export function displayDate(value) { if (!value) return '时间未知'; const date = new Date(value); return Number.isNaN(date.getTime()) ? '时间未知' : date.toLocaleString('zh-CN', { hour12: false }); }
export function recordText(item) {
  if (item?.record) return recordText(item.record);
  if (item?.parts?.length) return item.parts.map(part => part.text || '').join('\n');
  return item?.content ?? item?.text ?? item?.snippet ?? item?.data?.content ?? '';
}
export function recordRef(item) { return item?.ref ?? item?.reference ?? ''; }
export function newMemoryState() { return { rows: [], total: 0, offset: 0, nextOffset: null, includeHidden: false, loaded: false, error: '', selected: null, selectedRow: null, source: null, sourceId: '', mode: '', draft: null, reason: '', review: null, reviewKey: '' }; }
export function newBackgroundState() { return { snapshot: null, rows: [], total: 0, selectedCount: 0, offset: 0, nextOffset: null, loaded: false, error: '', selected: null, selectedRow: null, source: null, draft: null, review: null, reviewKey: '' }; }
export function newState() { return { vault: null, page: 'home', workspace: 'conversations', memoryFilter: 'all', continuationOpen: false, mobileDetail: false, viewport: {}, importBatch: null, searchLoaded: false, scope: 'personal', busy: false, query: '', target: 'all', results: [], selected: null, sources: [], importPreview: null, importSelection: null, importSelectedIds: [], dreamPreview: null, dreamTask: null, dreamResultText: '', notePreview: null, noteText: '', selectedRefs: [], conversations: [], conversation: null, conversationRows: [], conversationOffset: 0, continuation: null, continuationGoal: '', memory: newMemoryState(), background: newBackgroundState(), epoch: 0 }; }
export function activateVault(state, vault) { state.epoch += 1; state.vault = vault; state.scope = vault.scopes?.[0] || 'personal'; state.results = []; state.selected = null; state.sources = []; state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.dreamPreview = null; state.dreamTask=null;state.dreamResultText='';state.notePreview = null; state.selectedRefs = []; state.query = ''; state.noteText = ''; state.conversations = []; state.conversation = null; state.conversationRows = []; state.continuation = null; state.continuationGoal = ''; state.clientConfig = null; state.connectionResult = null; state.memory = newMemoryState(); state.background = newBackgroundState(); state.page = 'conversations'; state.workspace = 'conversations'; resetWorkspace(state); }
export function resetScope(state, scope) { state.epoch += 1; state.scope = scope; state.results = []; state.selected = null; state.sources = []; state.importPreview = null; state.importSelection = null; state.importSelectedIds = []; state.dreamPreview = null; state.dreamTask=null;state.dreamResultText='';state.notePreview = null; state.selectedRefs = []; state.conversations = []; state.conversation = null; state.conversationRows = []; state.continuation = null; state.continuationGoal = ''; state.clientConfig = null; state.connectionResult = null; state.memory = newMemoryState(); state.background = newBackgroundState(); resetWorkspace(state); }
function resetWorkspace(state) { state.dreamEvidence = {}; state.resumeConversation = null; state.pendingMemoryId = null; state.focusedEvent = ''; state.noteText = ''; state.noteOpen = false; state.fileImportOpen = false; state.allowBrowserCapture = false; state.memoryFilter = 'all'; state.continuationOpen = false; state.mobileDetail = false; state.viewport = {}; state.importBatch = null; state.searchLoaded = false; state.readerReturn = ''; state.conversationListOffset = 0; state.conversationListNext = null; state.conversationTotal = 0; state.conversationOffset = 0; state.conversationNext = null; }
export function scopeOptions(vault, selected) { return [...new Set([selected, ...(vault?.scopes || []), 'personal'])].filter(Boolean); }
export function nativeInstructions(root, scope, extensionId = '替换为扩展ID') {
  const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
  return `./recallcard --vault ${quote(root)} native-install --scope ${quote(scope)} --extension-id ${quote(extensionId)} --output-dir "$HOME/.local/share/recallcard-native"`;
}

export function displayState(value) { return ({ active: '有效', tentative: '待确认', stale: '已过期', retracted: '已撤回', captured: '原始记录', superseded: '已被替代' })[value] || value || '原始记录'; }


export function importSelectionStats(selection, selectedIds) {
  const ids = new Set(selectedIds);
  const selected = (selection?.conversations || []).filter(c => ids.has(c.source_id));
  const events = selected.reduce((total, conversation) => total + conversation.event_count, 0);
  return { conversations: selected.length, events, valid: events > 0 && events <= 5000 && selected.length === ids.size };
}

export function importCoverageLines(coverage) {
  if (!coverage) return [];
  const lines = [];
  const counts = [
    ['recognized_json_files', '已识别会话 JSON'], ['invalid_json_files_skipped', '损坏 JSON 已跳过'],
    ['unrecognized_json_values_skipped', '不支持的 JSON 项已跳过'], ['markdown_files_skipped', 'Markdown 文件已跳过'],
    ['markdown_copies_skipped', '其中同名 Markdown 副本'], ['other_files_skipped', '其他文件已跳过'],
    ['directories_skipped', '目录项已跳过'], ['duplicate_events_skipped', '重复消息版本已跳过'],
  ];
  for (const [key, title] of counts) if (coverage[key]) lines.push(`${title}：${coverage[key]}`);
  const messages = coverage.messages || {};
  for (const [key, title] of [
    ['other_branch_messages_skipped', '其他分支消息已跳过'], ['hidden_reasoning_messages_skipped', '隐藏推理消息未收集'],
    ['unsupported_messages_skipped', '不支持的消息已跳过'], ['empty_messages_skipped', '空消息已跳过'],
    ['unsupported_content_parts_skipped', '不支持的内容片段已跳过'],
  ]) if (messages[key]) lines.push(`${title}：${messages[key]}`);
  return [...lines, ...(coverage.notes || [])];
}
