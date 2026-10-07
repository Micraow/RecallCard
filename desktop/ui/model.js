export const pages = [
  ['home', '概览', '▦'], ['search', '查找与阅读', '⌕'], ['import', '导入资料', '↥'],
  ['dream', '整理记忆', '✧'], ['connect', '连接与状态', '⌘'],
];
export function shorten(value, limit = 90) { const text = String(value ?? ''); return text.length > limit ? `${text.slice(0, limit)}…` : text; }
export function displayDate(value) { if (!value) return '时间未知'; const date = new Date(value); return Number.isNaN(date.getTime()) ? '时间未知' : date.toLocaleString('zh-CN', { hour12: false }); }
export function recordText(item) {
  if (item?.record) return recordText(item.record);
  if (item?.parts?.length) return item.parts.map(part => part.text || '').join('\n');
  return item?.content ?? item?.text ?? item?.snippet ?? item?.data?.content ?? '';
}
export function recordRef(item) { return item?.ref ?? item?.reference ?? ''; }
export function newState() { return { vault: null, page: 'home', scope: 'personal', busy: false, query: '', target: 'all', results: [], selected: null, sources: [], importPreview: null, dreamPreview: null, selectedRefs: [], epoch: 0 }; }
export function activateVault(state, vault) { state.epoch += 1; state.vault = vault; state.scope = vault.scopes?.[0] || 'personal'; state.results = []; state.selected = null; state.sources = []; state.importPreview = null; state.dreamPreview = null; state.selectedRefs = []; state.query = ''; state.page = 'home'; }
export function resetScope(state, scope) { state.epoch += 1; state.scope = scope; state.results = []; state.selected = null; state.sources = []; state.importPreview = null; state.dreamPreview = null; state.selectedRefs = []; }
export function scopeOptions(vault, selected) { return [...new Set([selected, ...(vault?.scopes || []), 'personal'])].filter(Boolean); }
export function nativeInstructions(root, scope, extensionId = '替换为扩展ID') {
  const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
  return `./recallcard --vault ${quote(root)} native-install --scope ${quote(scope)} --extension-id ${quote(extensionId)} --output-dir "$HOME/.local/share/recallcard-native"`;
}

export function displayState(value) { return ({ active: '有效', tentative: '待确认', stale: '已过期', retracted: '已撤回', captured: '原始记录', superseded: '已被替代' })[value] || value || '原始记录'; }
