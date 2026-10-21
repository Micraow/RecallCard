export const pages = [['search', '搜索', '⌕'], ['memories', '记忆', '▥']];
export function shorten(value, limit = 90) { const text = String(value ?? ''); return text.length > limit ? `${text.slice(0, limit)}…` : text; }
export function displayDate(value) { if (!value) return '时间未知'; const date = new Date(value); return Number.isNaN(date.getTime()) ? '时间未知' : date.toLocaleString('zh-CN', { hour12: false }); }
export function recordText(item) {
  if (item?.record) return recordText(item.record);
  if (item?.parts?.length) return item.parts.map(part => part.text || '').join('\n');
  return item?.content ?? item?.text ?? item?.snippet ?? item?.data?.content ?? '';
}
export function recordRef(item) { return item?.ref ?? item?.reference ?? ''; }
export function newMemoryState() { return { rows: [], total: 0, offset: 0, nextOffset: null, includeHidden: false, loaded: false, error: '', selected: null, selectedRow: null, source: null, sourceId: '', mode: '', draft: null, reason: '', review: null, reviewKey: '' }; }
export function newBackgroundState() { return { actualOpen: false, snapshot: null, rows: [], total: 0, selectedCount: 0, offset: 0, nextOffset: null, loaded: false, error: '', selected: null, selectedRow: null, source: null, draft: null, review: null, reviewKey: '' }; }
export function newState() { return { vault: null, page: 'home', workspace: 'search', memoryFilter: 'all', mobileDetail: false, viewport: {}, searchLoaded: false, scope: 'personal', busy: false, query: '', target: 'all', results: [], selected: null, sources: [], dreamPreview: null, dreamTask: null, dreamResultText: '', notePreview: null, noteText: '', selectedRefs: [], memory: newMemoryState(), background: newBackgroundState(), epoch: 0 }; }
export function activateVault(state, vault) { state.epoch += 1; state.vault = vault; state.scope = vault.scopes?.[0] || 'personal'; state.results = []; state.selected = null; state.sources = []; state.dreamPreview = null; state.dreamTask=null;state.dreamResultText='';state.notePreview = null; state.selectedRefs = []; state.query = ''; state.noteText = ''; state.clientConfig = null; state.memory = newMemoryState(); state.background = newBackgroundState(); state.page = 'search'; state.workspace = 'search'; resetWorkspace(state); }
export function resetScope(state, scope) { state.epoch += 1; state.scope = scope; state.results = []; state.selected = null; state.sources = []; state.dreamPreview = null; state.dreamTask=null;state.dreamResultText='';state.notePreview = null; state.selectedRefs = []; state.clientConfig = null; state.memory = newMemoryState(); state.background = newBackgroundState(); resetWorkspace(state); }
function resetWorkspace(state) { state.resultQuery = '';  state.dreamEvidence = {}; state.pendingMemoryId = null; state.focusedEvent = ''; state.noteText = ''; state.noteOpen = false; state.memoryFilter = 'all'; state.mobileDetail = false; state.viewport = {}; state.searchLoaded = false; state.readerReturn = ''; }
export function scopeOptions(vault, selected) { return [...new Set([selected, ...(vault?.scopes || []), 'personal'])].filter(Boolean); }

export function displayState(value) { return ({ active: '有效', tentative: '待确认', stale: '已过期', retracted: '已撤回', captured: '原始记录', superseded: '已被替代' })[value] || value || '原始记录'; }



