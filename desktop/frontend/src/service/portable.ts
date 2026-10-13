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
