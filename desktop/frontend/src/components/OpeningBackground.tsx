import {useEffect,useRef,useState} from 'react';
import type {ScopedService} from '../service/client';
import {useResource} from '../service/hooks';
import {appError,type AppError} from '../service/contracts';
import {copyCurrentBackground,type BackgroundPage,type BackgroundReview,type BackgroundCandidate} from '../service/background';
import {utf8Bytes} from '../service/portable';
import {evidenceLabel,scopeLabel} from '../service/types';
import {navigate} from '../App';
import {Badge,Dialog,ErrorNotice,Loading,Prose} from './common';

export function OpeningPreview({page}:{page:Pick<BackgroundPage,'background'|'copy_text'>&{selected_count?:number}}) {
 return <section className="opening-preview" aria-label="实际开局背景">
 <h3>实际会带上的背景</h3>
 <p>{page.selected_count!==undefined&&<>已选 {page.selected_count} 条 · </>}本次完整带上 {page.background.refs.length} 条 · {utf8Bytes(page.copy_text).toLocaleString()} 字节 UTF-8</p>
 {page.background.truncated&&<p role="status">长度有限，部分选择未带上；以这份实际文字和引用为准，其他资料仍可按需查找。</p>}
 <textarea id="opening-preview" className="text-area" readOnly value={page.copy_text} aria-label="完整开局背景，可核对后复制"/>
 <details><summary>本次带上的来源与版本</summary>{page.background.refs.length?<ul>{page.background.refs.map(ref=><li key={ref}>{ref}</li>)}</ul>:<p>尚未带上个人记忆；当前只有使用说明和资料目录。</p>}<small>版本：{page.background.bootstrap_version}</small></details>
 <p className="field-hint">不会复制未选记忆、完整聊天、附件原件或原始会话的自动摘要。参考文字不是新的用户事实。</p>
 </section>;
}

export function OpeningBackgroundDialog({service,close,changed}:{service:ScopedService;close:()=>void;changed:()=>void}) {
 const [offset,setOffset]=useState(0),[override,setOverride]=useState<BackgroundPage|null>(null);
 const data=useResource(()=>service.openingBackground(offset),[service,offset]);
 const [review,setReview]=useState<BackgroundReview|null>(null),[approved,setApproved]=useState(false),[busy,setBusy]=useState(false);
 const [error,setError]=useState<AppError|null>(null),[status,setStatus]=useState('');
 const active=useRef(true),lock=useRef(false),pending=useRef<string|null>(null);
 useEffect(()=>{active.current=true;return()=>{active.current=false;const id=pending.current;if(id)void service.cancelBackground(id).catch(()=>{});};},[service]);
 useEffect(()=>{setOverride(null);},[data.data]);
 const page=override??data.data;
 const refresh=()=>{setOverride(null);setStatus('');data.refresh();};
 const cancel=async(closeDialog:boolean)=>{
  if(lock.current)return;lock.current=true;setBusy(true);setError(null);
  try{const id=pending.current;if(id)await service.cancelBackground(id);pending.current=null;
   if(active.current){setReview(null);setApproved(false);if(closeDialog)close();}
  }catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;if(active.current)setBusy(false);}
 };
 const choose=async(row:BackgroundCandidate,include:boolean)=>{
  if(lock.current)return;lock.current=true;setBusy(true);setError(null);setStatus('');
  try{const value=await service.reviewBackground(row.id,row.revision,include);
   if(!active.current){await service.cancelBackground(value.preview_id);return;}
   pending.current=value.preview_id;
   if(value.scope!==service.scope||value.before.id!==row.id||value.before.revision!==row.revision||value.include!==include)throw new Error('预览与本次选择不一致，请返回后重新读取。');
   setReview(value);setApproved(false);
  }catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;if(active.current)setBusy(false);}
 };
 const confirm=async()=>{
  if(!review||lock.current||(review.requires_protected_approval&&!approved))return;
  lock.current=true;setBusy(true);setError(null);
  try{await service.confirmBackground(review.preview_id,approved);pending.current=null;
   if(active.current){setReview(null);setApproved(false);refresh();setStatus(review.include?'已加入开局背景；请核对实际带上的文字。':'已移出开局背景；记忆和来源仍保留。');changed();}
  }catch(reason){if(active.current){setError(appError(reason));setApproved(false);}}
  finally{lock.current=false;if(active.current)setBusy(false);}
 };
 const copy=async()=>{
  if(!page||lock.current||data.loading||data.error)return;lock.current=true;setBusy(true);setError(null);setStatus('');
  try{const result=await copyCurrentBackground(page,()=>service.openingBackground(offset),text=>service.copy(text),()=>active.current);
   if(result){setOverride(result.current);setStatus(result.copied?'开局背景已复制。到你选择的 AI 粘贴，检查后由你发送。':'背景已变化，尚未复制。请核对新预览后再点击复制。');}
  }catch(reason){if(active.current){setError(appError(reason));setOverride(null);}}
  finally{lock.current=false;if(active.current)setBusy(false);}
 };
 return <Dialog title={review?(review.include?'核对加入开局背景':'核对移出开局背景'):'开局背景'} close={()=>void cancel(true)} busy={busy} wide>
 <div className="modal-body opening-background">
 <p>当前范围：{scopeLabel(service.scope)}</p>
 {error&&<ErrorNotice error={error}/>} {status&&<p role="status">{status}</p>}
 {review?<>
 <p>{review.warning}</p><Badge>{evidenceLabel(review.before.evidence)}</Badge><Prose text={review.before.content}/>
 <OpeningPreview page={{background:review.background_after,copy_text:review.copy_text_after}}/>
 {review.requires_protected_approval&&<label className="checkbox-label"><input id="opening-protected-approval" type="checkbox" checked={approved} onChange={event=>setApproved(event.target.checked)}/>我已核对，允许本次修改这条受保护记忆的背景选择</label>}
 </>:<>
 <p>选好一次，默认用于“准备这次对话”和 bootstrap 开局入口。不需要先输入问题。</p>
 <p className="field-hint">网页可复制后发送，扩展可按已有授权准备草稿；MCP 需要宿主调用 bootstrap，不保证所有客户端自动采用。普通会话摘要尚未自动加入。</p>
 {data.loading?<Loading label="正在读取开局背景"/>:data.error?<ErrorNotice error={data.error} retry={refresh}/>:page&&<>
 <OpeningPreview page={page}/>
 <div className="opening-heading"><h3>从现有记忆选择</h3><button className="text-button" disabled={busy} onClick={refresh}>重新读取</button></div>
 {!page.candidates.length&&<p>当前没有可选记忆。先在记忆页整理并确认有用信息；不会把普通聊天或助手回答自动当作稳定事实。</p>}
 <div className="opening-candidates">{page.candidates.map(row=><article key={`${row.id}:${row.revision}`} data-background-id={row.id}>
 <div><Badge tone={row.included?'success':'muted'}>{row.included?'本次带上':row.selected?'已选，未带上':'未选择'}</Badge> <Badge>{evidenceLabel(row.evidence)}</Badge></div>
 <Prose text={row.content}/>{row.text_truncated&&<p>这里只显示节选，请打开记忆核对原文。</p>}<p className="field-hint">{row.reason}</p>
 <div className="connection-controls">{row.can_include&&<button className="button compact" disabled={busy} onClick={()=>void choose(row,true)}>{row.selected?'保护并带入背景':'加入开局背景'}</button>}
 {row.selected&&<button className="button compact" disabled={busy||!row.can_remove} onClick={()=>void choose(row,false)}>移出开局背景</button>}
 <button className="text-button" disabled={busy} onClick={()=>{close();navigate('memories',`memory:${row.id}@${row.revision}`);}}>查看记忆与来源</button></div>
 </article>)}</div>
 <div className="opening-pagination"><span>共 {page.total} 条可查看记忆</span><button className="text-button" disabled={busy||offset===0} onClick={()=>{setOverride(null);setOffset(Math.max(0,offset-30));}}>上一页</button><button className="text-button" disabled={busy||page.next_offset===null} onClick={()=>{setOverride(null);setOffset(page.next_offset!);}}>下一页</button></div>
 </>}
 </>}
 </div><div className="modal-footer"><button className="button" disabled={busy} onClick={()=>void cancel(!review)}>{review?'取消本次选择':'关闭'}</button>
 {review?<button className="button primary" disabled={busy||(review.requires_protected_approval&&!approved)} onClick={()=>void confirm()}>{review.include?'确认加入背景':'确认移出背景'}</button>:<button className="button primary" disabled={busy||data.loading||!!data.error||!page} onClick={()=>void copy()}>核对并复制开局背景</button>}</div>
 </Dialog>;
}
