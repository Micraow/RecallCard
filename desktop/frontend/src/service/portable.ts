import type { SearchPage } from "./types";

export interface PortableRecord {
  ref: string; kind: string; role: string | null; evidence: string; text: string;
  conversation_title: string | null; truncated: boolean; content_retained: boolean;
}
export interface PortableContext {
  text: string; scope: string; selected_refs: string[]; selected_count: number;
  included_count: number; pending_refs: string[]; partial: boolean; truncated: boolean;
  records: PortableRecord[];
}
export interface ReplyPreview {
  preview_id: string; session_id: string; scope: string; content: string;
  redacted: boolean; role: "assistant"; automatic_memory: false;
}
export const utf8Bytes = (text: string) => new TextEncoder().encode(text).length;

export function validatePortableContext(value: PortableContext, scope: string, refs: string[]) {
  if (!value || typeof value.text !== "string" || !value.text || value.scope !== scope
    || !Array.isArray(value.records) || !Array.isArray(value.pending_refs)
    || JSON.stringify(value.selected_refs) !== JSON.stringify(refs)
    || value.selected_count !== refs.length || value.included_count !== value.records.length
    || value.records.some(record => !refs.includes(record.ref))
    || value.pending_refs.some(ref => !refs.includes(ref)))
    throw new Error("上下文预览与当前选择不一致，请重新准备。");
  return value;
}

// 每次点击复制先读取当前正本；页面已离开或内容发生变化时不写入剪贴板。
export async function revalidateAndCopy(
  preview: PortableContext,
  prepare: () => Promise<PortableContext>,
  copy: (text: string) => Promise<void>,
  active: () => boolean,
): Promise<{ copied: boolean; current: PortableContext } | null> {
  const current = await prepare();
  if (!active()) return null;
  if (current.scope !== preview.scope || current.text !== preview.text
    || JSON.stringify(current.selected_refs) !== JSON.stringify(preview.selected_refs))
    return { copied: false, current };
  await copy(current.text);
  return active() ? { copied: true, current } : null;
}


export function recallChoices(page: SearchPage | null | undefined) {
  const seen = new Set<string>();
  return (page?.results || []).filter(row => {
    if (seen.has(row.ref)) return false;
    seen.add(row.ref); return true;
  }).map(row => ({
    ref: row.ref,
    label: row.conversation_title || Array.from(row.text || row.content || row.snippet || row.ref).slice(0, 72).join(""),
    snippet: Array.from(row.text || row.content || row.snippet || "").slice(0, 240).join(""),
    role: row.role, evidence: row.evidence || "", kind: row.kind,
    support_note: recallSupportNote(row),
  }));
}
export const currentRecallQuery = (goal: string, searched: string, page: SearchPage | null | undefined) =>
  !!page && !!goal.trim() && goal.trim() === searched.trim();

export function recallSupportNote(row: import('./types').SearchRow): string {
 const support=row.retrieval_support;
 if(!support)return '';
 if(support.limited)return '问题超出片段锚点核对预算，保留原候选顺序；请缩短问题或打开来源。';
 if(support.protected)return '检索片段含区分大小写的精确锚点，已在候选窗口中优先；仍未判断是否足以回答。';
 if(support.kind==='lexical_candidate')return '字面候选，相关性与当前是否成立仍需核对来源。';
 return (support.protected_anchors||0)>0?'这段检索片段没有完整匹配精确锚点；只保留为语义候选，不表示整个事件不含该标识。':'仅按语义相似度入选，可能无关；请核对原文后选择。';
}
