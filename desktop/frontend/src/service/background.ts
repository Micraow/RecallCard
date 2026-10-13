import type {Memory} from './types';

export interface OpeningBackground {
 bootstrap_version:string; stable_text:string; refs:string[]; truncated:boolean;
}
export interface BackgroundCandidate {
 id:string; revision:number; content:string; text_truncated:boolean; status:string;
 evidence:string; protected:boolean; source_refs:string[]; selected:boolean;
 included:boolean; can_include:boolean; can_remove:boolean; reason:string;
}
export interface BackgroundPage {
 background:OpeningBackground; copy_text:string; candidates:BackgroundCandidate[];
 selected_count:number; total:number; next_offset:number|null;
}
export interface BackgroundReview {
 preview_id:string; scope:string; include:boolean; before:Memory;
 background_before:OpeningBackground; background_after:OpeningBackground;
 copy_text_before:string; copy_text_after:string; requires_protected_approval:boolean; warning:string;
}
export function validateBackgroundPage(value:BackgroundPage):BackgroundPage {
 const b=value?.background;
 if(!b || typeof b.bootstrap_version!=='string' || !b.bootstrap_version || typeof b.stable_text!=='string' ||
   !Array.isArray(b.refs) || b.refs.some(ref=>typeof ref!=='string') || typeof b.truncated!=='boolean' ||
   typeof value.copy_text!=='string' || !value.copy_text.startsWith('recallcard.context/1\n') ||
   !value.copy_text.includes(b.stable_text) || new TextEncoder().encode(value.copy_text).length>32768 ||
   !Array.isArray(value.candidates) || value.candidates.length>30 || !Number.isSafeInteger(value.total) || value.total<0 ||
   !Number.isSafeInteger(value.selected_count) || value.selected_count<0 || value.selected_count>value.total ||
   (value.next_offset!==null&&(!Number.isSafeInteger(value.next_offset)||value.next_offset<0))) throw new Error('开局背景合同不可核验，请重新读取。');
 return value;
}
/** Copy exactly the reviewed current projection. A changed tail is a changed preview. */
export async function copyCurrentBackground(before:BackgroundPage,read:()=>Promise<BackgroundPage>,copy:(text:string)=>Promise<unknown>,active:()=>boolean) {
 const current=validateBackgroundPage(await read());
 if(!active())return null;
 if(current.copy_text!==before.copy_text || current.background.bootstrap_version!==before.background.bootstrap_version ||
   JSON.stringify(current.background.refs)!==JSON.stringify(before.background.refs)) return {current,copied:false};
 await copy(current.copy_text);
 return active()?{current,copied:true}:null;
}
