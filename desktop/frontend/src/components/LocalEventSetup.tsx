import {useEffect,useRef,useState} from 'react';
import type {ScopedService} from '../service/client';
import {useResource} from '../service/hooks';
import {appError,type AppError} from '../service/contracts';
import {type LocalEventSetup,type LocalEventStatus,type LocalEventProgress,validLocalEventSettings} from '../service/setup';
import {scopeLabel} from '../service/types';
import {Dialog,ErrorNotice,Loading} from './common';

export function LocalEventSetupPanel({service}:{service:ScopedService}) {
 const state=useResource(()=>service.localEventStatus(),[service]);
 const [editing,setEditing]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState<AppError|null>(null),[progress,setProgress]=useState<LocalEventProgress|null>(null);
 const active=useRef(true),stop=useRef(false),lock=useRef(false);
 useEffect(()=>{active.current=true;stop.current=false;setEditing(false);setProgress(null);return()=>{active.current=false;stop.current=true;};},[service]);
 const build=async()=>{if(lock.current)return;lock.current=true;stop.current=false;setBusy(true);setError(null);try{
   while(active.current&&!stop.current){const value=await service.refreshLocalEvents();if(!active.current)break;setProgress(value);if(value.complete)break;await new Promise(resolve=>setTimeout(resolve,20));}
   if(active.current)state.refresh();
  }catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;if(active.current)setBusy(false);}};
 const disable=async()=>{if(lock.current)return;lock.current=true;setError(null);try{await service.disableLocalEvents();if(active.current){state.refresh();setProgress(null);}}catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;}};
 if(state.loading)return <Loading label="正在读取普通对话索引状态"/>;
 const value=state.data;
 return <section className="model-setting-details" aria-label="普通对话本机语义索引"><h2>普通对话本机语义索引</h2>
 <p>让已导入的用户原话和助手回答直接参与本机向量检索，无需先变成 Memory。默认关闭，需要 Python 3.11 或更新版本，以及已有的本机 embedding 服务。</p>
 <p className="field-hint">仅向明确批准的数字回环地址发送当前范围的普通对话片段和新问题；服务本身是否完全离线仍需你核实。此入口不使用云回退，也不读取附件原件或推理草稿。</p>
 {state.error&&<ErrorNotice error={state.error} retry={state.refresh}/>} {error&&<ErrorNotice error={error}/>}
 {value?.settings&&<p>{value.settings.model} · {value.settings.dimensions}维 · {value.settings.revision} · {value.settings.query_approved?'问题按需本机编码':'问题编码未启用'}</p>}
 {value?.index_state&&<p role="status" data-testid="local-event-index-status">{value.index_state==='complete'?'当前快照已建索引':value.index_state==='stale'?'资料已变化，请更新索引':'索引尚未完成'} · {value.source_count??0}条原话 / {value.chunk_count??0}个片段</p>}
 {progress&&<p role="status">{progress.complete?'本轮索引完成':busy?'正在分批处理':'已暂停，可继续'}：{progress.processed_sources}/{progress.total_sources}条原话</p>}
 <p className="field-hint">新增、修改、遗忘或恢复资料后，请更新索引。旧快照不能当作完整覆盖；缓存可重建，原话保留。模型质量与大库首次建索引速度应另行实测。</p>
 <div className="connection-controls"><button className="button compact" disabled={busy} onClick={()=>setEditing(true)}>配置普通对话本机索引</button>
 {value?.settings?.events_approved&&<button className="button compact" disabled={busy} onClick={()=>void build()}>更新普通对话索引</button>}
 {busy&&<button className="button compact" onClick={()=>{stop.current=true;}}>完成当前批后暂停</button>}
 {value?.settings&&<button className="text-button" disabled={busy} onClick={()=>void disable()}>停用普通对话索引</button>}</div>
 {editing&&<LocalEventDialog key={`${service.workspace.session_id}:${service.scope}`} service={service} initial={value??null} close={()=>setEditing(false)} saved={()=>{state.refresh();}}/>}
 </section>;
}
function LocalEventDialog({service,initial,close,saved}:{service:ScopedService;initial:LocalEventStatus|null;close:()=>void;saved:(v:LocalEventStatus)=>void}) {
 const [form,setForm]=useState<LocalEventSetup>(initial?.settings??{endpoint:'http://127.0.0.1:46709/v1/embeddings',model:'',revision:'',tokenizer:'',dimensions:384,query_instruction:'',document_instruction:'',events_approved:false,query_approved:false,chunk_bytes:1024,disk_cap_mib:256});
 const [review,setReview]=useState(false),[confirmed,setConfirmed]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState<AppError|null>(null);const lock=useRef(false);
 const change=<K extends keyof LocalEventSetup>(key:K,value:LocalEventSetup[K])=>{setForm(v=>({...v,[key]:value}));setConfirmed(false);};
 const save=async()=>{if(!confirmed||!validLocalEventSettings(form)||lock.current)return;lock.current=true;setBusy(true);setError(null);try{const result=await service.configureLocalEvents(form);saved(result);close();}catch(reason){setError(appError(reason));setConfirmed(false);}finally{lock.current=false;setBusy(false);}};
 return <Dialog title={review?'确认普通对话本机编码范围':'配置普通对话本机索引'} close={close} busy={busy} wide><div className="modal-body model-setup">{error&&<ErrorNotice error={error}/>}<p>当前范围：{scopeLabel(service.scope)}</p>{review?<>
 <dl><div><dt>本机接收地址</dt><dd>{form.endpoint}</dd></div><div><dt>模型与输入空间</dt><dd>{form.model} / {form.revision} / {form.tokenizer} / {form.dimensions}维</dd></div></dl>
 <p>你选择发送：{[form.events_approved?'当前范围未撤回、未被修订替换的普通用户/助手原话片段':null,form.query_approved?'未缓存的新问题':null].filter(Boolean).join('；')}。不继承其他 Memory 或云编码批准。</p>
 <p>正文每批最多8块且受64KiB限制，SQLite只留来源标识、范围、hash与向量。回环地址不保证服务内部不会转发；请使用你确认的本机模型服务。不会下载或启动模型。</p>
 <label className="checkbox-label"><input id="event-confirm" type="checkbox" checked={confirmed} onChange={e=>setConfirmed(e.target.checked)}/>我确认这个本机地址、当前范围和上述数据类型</label>
 </>:<>
 {(['endpoint','model','revision','tokenizer'] as const).map(key=><label className="field-label" key={key} htmlFor={`event-${key}`}>{({endpoint:'本机 embedding 地址',model:'模型名称',revision:'模型文件版本/摘要',tokenizer:'Tokenizer及池化版本'})[key]}<input id={`event-${key}`} className="field-input" autoComplete="off" value={form[key]} onChange={e=>change(key,e.target.value)}/></label>)}
 <label className="field-label" htmlFor="event-dimensions">向量维度<input id="event-dimensions" className="field-input" type="number" min="1" max="8192" value={form.dimensions} onChange={e=>change('dimensions',Number(e.target.value))}/></label>
 <label className="checkbox-label"><input id="event-corpus" type="checkbox" checked={form.events_approved} onChange={e=>change('events_approved',e.target.checked)}/>允许这个范围的普通对话在本机编码</label>
 <label className="checkbox-label"><input id="event-query" type="checkbox" checked={form.query_approved} onChange={e=>change('query_approved',e.target.checked)}/>允许新问题按需在本机编码</label>
 <details><summary>切块、空间上限与输入指令</summary>
 <label className="field-label">片段UTF-8字节上限<input className="field-input" type="number" min="256" max="8192" value={form.chunk_bytes} onChange={e=>change('chunk_bytes',Number(e.target.value))}/></label>
 <label className="field-label">派生数据库与日志预算 MiB<input className="field-input" type="number" min="16" max="512" value={form.disk_cap_mib} onChange={e=>change('disk_cap_mib',Number(e.target.value))}/></label>
 <label className="field-label">查询指令<textarea className="field-input" value={form.query_instruction} onChange={e=>change('query_instruction',e.target.value)}/></label>
 <label className="field-label">文档指令<textarea className="field-input" value={form.document_instruction} onChange={e=>change('document_instruction',e.target.value)}/></label></details>
 <p className="field-hint">模型、Tokenizer、维度或切块变化使用新空间。预算不包含独立模型服务的内存；此版本只保存float32向量。</p>
 </>}</div><div className="modal-footer"><button className="button" disabled={busy} onClick={review?()=>{setReview(false);setConfirmed(false);}:close}>{review?'返回修改':'取消'}</button>{review?<button className="button primary" disabled={busy||!confirmed} onClick={()=>void save()}>确认保存普通对话配置</button>:<button className="button primary" disabled={!validLocalEventSettings(form)} onClick={()=>setReview(true)}>核对普通对话范围</button>}</div></Dialog>;
}
