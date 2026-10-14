import {useEffect,useRef,useState} from 'react';
import type {ScopedService} from '../service/client';
import {useResource} from '../service/hooks';
import {appError,type AppError} from '../service/contracts';
import {type LocalEventSetup,type LocalEventStatus,type LocalEventProgress,validLocalEventSettings,normalizeLocalEventSettings} from '../service/setup';
import {scopeLabel} from '../service/types';
import {Dialog,ErrorNotice,Loading} from './common';
import {CloudEventDialog} from './CloudEventSetup';

export function LocalEventSetupPanel({service}:{service:ScopedService}) {
 return <SourceIndexPanel key={`${service.workspace.session_id}:${service.scope}`} service={service}/>;
}
function SourceIndexPanel({service}:{service:ScopedService}) {
 const state=useResource(()=>service.cloudEventStatus(),[service]);
 const [editing,setEditing]=useState<'local'|'cloud'|null>(null),[stoppingCloud,setStoppingCloud]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState<AppError|null>(null),[progress,setProgress]=useState<LocalEventProgress|null>(null);
 const active=useRef(true),stop=useRef(false),lock=useRef(false);
 useEffect(()=>{active.current=true;stop.current=false;setEditing(null);setStoppingCloud(false);setProgress(null);setError(null);return()=>{active.current=false;stop.current=true;};},[service]);
 const build=async()=>{if(lock.current)return;lock.current=true;stop.current=false;setBusy(true);setError(null);try{
   while(active.current&&!stop.current){const value=await service.refreshLocalEvents();if(!active.current)break;setProgress(value);if(value.complete)break;await new Promise(resolve=>setTimeout(resolve,20));}
   if(active.current)state.refresh();
  }catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;if(active.current)setBusy(false);}};
 const disable=async(cloud:boolean)=>{if(lock.current)return;lock.current=true;setBusy(true);setError(null);try{if(cloud)await service.disableCloudEvents();else await service.disableLocalEvents();if(active.current){state.refresh();setProgress(null);setStoppingCloud(false);}}catch(reason){if(active.current)setError(appError(reason));}finally{lock.current=false;if(active.current)setBusy(false);}};
 if(state.loading)return <Loading label="正在读取普通对话与附件索引状态"/>;
 const value=state.data,cloud=value?.provider==='cloud',target=cloud?'云':'本机';
 const selected=cloud?value?.cloud.grant?.settings:value?.settings;
 return <section className="model-setting-details" aria-label="普通对话与附件语义索引"><h2>普通对话与附件语义索引</h2>
 <p>让已导入的用户原话、助手回答和独立批准的安全 UTF-8 附件正文参与向量检索，无需先变成 Memory。默认关闭，可选择已有本机 embedding 服务，或自行配置 Qwen 等 OpenAI-compatible 云服务。</p>
 <p className="field-hint">普通对话、附件和新问题分别批准。云与本机配置分别审阅，不继承 Memory 或其他范围的批准；不会自动选择提供器或云回退，不读取推理草稿。编码需要 Python 3.11 或更新版本。</p>
 {state.error&&<ErrorNotice error={state.error} retry={state.refresh}/>} {value?.service_error&&<ErrorNotice error={value.service_error}/>} {error&&<ErrorNotice error={error}/>}
 {selected&&<p>当前提供器：{target} · {selected.model} · {selected.dimensions}维 · {selected.revision} · {selected.events_approved?'普通对话编码已批准':'普通对话编码未启用'} · {selected.attachments_approved===true?'附件编码已独立批准':'附件编码未启用'} · {selected.query_approved?`问题按需${target}编码`:'问题编码未启用'}</p>}
 {value?.index_state&&<p role="status" data-testid="local-event-index-status">{value.index_state==='complete'?'当前快照已建索引':value.index_state==='stale'?'资料已变化，请更新索引':'索引尚未完成'} · {value.source_count??0}{selected?.attachments_approved?'项来源':'条原话'} / {value.chunk_count??0}个片段</p>}
 {progress&&<p role="status">{progress.complete?'本轮索引完成':busy?'正在分批处理':'已暂停，可继续'}：{progress.processed_sources}/{progress.total_sources}{selected?.attachments_approved?'项来源':'条原话'}</p>}
 {value?.cloud&&<><p role="status">{value.cloud.usage.month} 全 Vault 云向量已预留 {value.cloud.usage.reserved_calls} 次 / {value.cloud.usage.reserved_bytes} 字节（Memory 查询 {value.cloud.usage.query_calls} 次，Memory 正文 {value.cloud.usage.memory_calls} 次，来源查询 {value.cloud.usage.source_query_calls} 次，普通对话 {value.cloud.usage.event_calls} 次，附件 {value.cloud.usage.attachment_calls} 次）</p><p className="field-hint">来源与 Memory 共享累计用量，限额不能相加。{value.cloud.budget_note}</p></>}
 {cloud&&selected&&'max_calls_per_month' in selected&&<p>当前云来源月度预算：{selected.budget_mode==='unlimited'?'无限，持续计量':`${selected.max_calls_per_month} 次 / ${selected.max_bytes_per_month} 字节`}</p>}
 {cloud&&value?.credential&&<p className="field-hint">{value.credential.message}</p>}
 <p className="field-hint">保存配置不会开始编码。新增、修改、遗忘或恢复资料后，请明确更新索引。旧快照不能当作完整覆盖；缓存可重建，原始资料保留。模型质量与大库首次建索引速度应另行实测。</p>
 <div className="connection-controls"><button className="button compact" disabled={busy||!!state.error} onClick={()=>setEditing('local')}>配置普通对话本机索引</button>
 <button className="button compact" disabled={busy||!!state.error} onClick={()=>setEditing('cloud')}>配置普通对话与附件云索引</button>
 {(selected?.events_approved||selected?.attachments_approved===true)&&<button className="button compact" disabled={busy||!!state.error||!!value?.service_error} onClick={()=>void build()}>{selected?.attachments_approved===true?'更新已批准资料索引':'更新普通对话索引'}</button>}
 {busy&&<button className="button compact" onClick={()=>{stop.current=true;}}>完成当前批后暂停</button>}
 {!cloud&&value?.settings&&<button className="text-button" disabled={busy} onClick={()=>void disable(false)}>{selected?.attachments_approved===true?'停用已批准资料索引':'停用普通对话索引'}</button>}
 {value?.cloud.grant&&<button className="text-button" disabled={busy} onClick={()=>setStoppingCloud(true)}>停用云来源索引</button>}</div>
 {editing==='local'&&<LocalEventDialog key={`${service.workspace.session_id}:${service.scope}:local`} service={service} initial={cloud?null:value??null} close={()=>setEditing(null)} saved={()=>{state.refresh();}}/>}
 {editing==='cloud'&&<CloudEventDialog key={`${service.workspace.session_id}:${service.scope}:cloud`} service={service} initial={value??null} close={()=>setEditing(null)} saved={()=>{state.refresh();}}/>}
 {stoppingCloud&&<Dialog title="停用云来源索引？" close={()=>setStoppingCloud(false)} busy={busy}><div className="modal-body"><p>撤回当前范围的普通对话、附件与来源问题云编码批准，后续请求不会继续发送。已发送请求不能撤回，可能已计费。</p><p>原始资料、已有缓存、累计用量和系统保护凭据保留。此操作只停用云来源配置，不停用本机提供器或撤回 Memory 批准。</p></div><div className="modal-footer"><button className="button" disabled={busy} onClick={()=>setStoppingCloud(false)}>取消</button><button className="button primary" disabled={busy} onClick={()=>void disable(true)}>确认停用云来源索引</button></div></Dialog>}
 </section>;
}
function LocalEventDialog({service,initial,close,saved}:{service:ScopedService;initial:LocalEventStatus|null;close:()=>void;saved:(v:LocalEventStatus)=>void}) {
 const [form,setForm]=useState<LocalEventSetup>(()=>normalizeLocalEventSettings(initial?.settings??{endpoint:'http://127.0.0.1:46709/v1/embeddings',model:'',revision:'',tokenizer:'',dimensions:384,query_instruction:'',document_instruction:'',events_approved:false,attachments_approved:false,query_approved:false,chunk_bytes:1024,disk_cap_mib:256}));
 const [review,setReview]=useState(false),[confirmed,setConfirmed]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState<AppError|null>(null);const lock=useRef(false);
 const change=<K extends keyof LocalEventSetup>(key:K,value:LocalEventSetup[K])=>{setForm(v=>({...v,[key]:value}));setConfirmed(false);};
 const save=async()=>{if(!confirmed||!validLocalEventSettings(form)||lock.current)return;lock.current=true;setBusy(true);setError(null);try{const result=await service.configureLocalEvents(form);saved(result);close();}catch(reason){setError(appError(reason));setConfirmed(false);}finally{lock.current=false;setBusy(false);}};
 return <Dialog title={review?(form.attachments_approved?'确认本机资料编码范围':'确认普通对话本机编码范围'):'配置普通对话本机索引'} close={close} busy={busy} wide><div className="modal-body model-setup">{error&&<ErrorNotice error={error}/>}<p>当前范围：{scopeLabel(service.scope)}</p>{review?<>
 <dl><div><dt>本机接收地址</dt><dd>{form.endpoint}</dd></div><div><dt>模型与输入空间</dt><dd>{form.model} / {form.revision} / {form.tokenizer} / {form.dimensions}维</dd></div></dl>
 <dl aria-label="本机编码资料类型">
 <div><dt>普通对话</dt><dd>{form.events_approved?'已批准：当前范围未撤回、未被修订替换的普通用户/助手原话片段':'未批准，不发送普通对话正文'}</dd></div>
 <div><dt>附件</dt><dd>{form.attachments_approved?'已批准：仅当前范围中父事件可见、未撤回且未被修订替换的安全 UTF-8 附件正文；不支持 PDF 提取或 OCR':'未批准，不发送附件正文'}</dd></div>
 <div><dt>新问题</dt><dd>{form.query_approved?'已批准：未缓存的新问题':'未批准，不发送新问题'}</dd></div>
 </dl>
 <p>三类数据分别批准，附件批准不会同时启用普通对话。只发送到上述同一本机地址，不继承其他 Memory 或云编码批准。附件内容是独立来源，不当作上传者原话。</p>
 <p>正文每批最多8块且受64KiB限制，SQLite只留来源标识、范围、hash与向量。回环地址不保证服务内部不会转发；请使用你确认的本机模型服务。不会下载或启动模型。</p>
 <label className="checkbox-label"><input id="event-confirm" type="checkbox" checked={confirmed} onChange={e=>setConfirmed(e.target.checked)}/>我确认这个本机地址、当前范围和上述数据类型</label>
 </>:<>
 {(['endpoint','model','revision','tokenizer'] as const).map(key=><label className="field-label" key={key} htmlFor={`event-${key}`}>{({endpoint:'本机 embedding 地址',model:'模型名称',revision:'模型文件版本/摘要',tokenizer:'Tokenizer及池化版本'})[key]}<input id={`event-${key}`} className="field-input" autoComplete="off" value={form[key]} onChange={e=>change(key,e.target.value)}/></label>)}
 <label className="field-label" htmlFor="event-dimensions">向量维度<input id="event-dimensions" className="field-input" type="number" min="1" max="8192" value={form.dimensions} onChange={e=>change('dimensions',Number(e.target.value))}/></label>
 <label className="checkbox-label"><input id="event-corpus" type="checkbox" checked={form.events_approved} onChange={e=>change('events_approved',e.target.checked)}/>允许这个范围的普通对话在本机编码</label>
 <label className="checkbox-label"><input id="event-attachments" type="checkbox" aria-describedby="event-attachments-hint" checked={form.attachments_approved} onChange={e=>change('attachments_approved',e.target.checked)}/>独立允许这个范围的安全 UTF-8 附件正文在本机编码</label>
 <p id="event-attachments-hint" className="field-hint">仅包含当前范围中父事件可见、未撤回且未被修订替换的附件，原件须已保留且通过安全文本核验；不支持 PDF 提取或 OCR。附件内容是独立来源，不当作上传者原话。</p>
 <label className="checkbox-label"><input id="event-query" type="checkbox" checked={form.query_approved} onChange={e=>change('query_approved',e.target.checked)}/>允许新问题按需在本机编码</label>
 <details><summary>切块、空间上限与输入指令</summary>
 <label className="field-label">片段UTF-8字节上限<input className="field-input" type="number" min="256" max="8192" value={form.chunk_bytes} onChange={e=>change('chunk_bytes',Number(e.target.value))}/></label>
 <label className="field-label">派生数据库与日志预算 MiB<input className="field-input" type="number" min="16" max="512" value={form.disk_cap_mib} onChange={e=>change('disk_cap_mib',Number(e.target.value))}/></label>
 <label className="field-label">查询指令<textarea className="field-input" value={form.query_instruction} onChange={e=>change('query_instruction',e.target.value)}/></label>
 <label className="field-label">文档指令<textarea className="field-input" value={form.document_instruction} onChange={e=>change('document_instruction',e.target.value)}/></label></details>
 <p className="field-hint">模型、Tokenizer、维度或切块变化使用新空间。预算不包含独立模型服务的内存；此版本只保存float32向量。</p>
 </>}</div><div className="modal-footer"><button className="button" disabled={busy} onClick={review?()=>{setReview(false);setConfirmed(false);}:close}>{review?'返回修改':'取消'}</button>{review?<button className="button primary" disabled={busy||!confirmed} onClick={()=>void save()}>{form.attachments_approved?'确认保存资料配置':'确认保存普通对话配置'}</button>:<button className="button primary" disabled={!validLocalEventSettings(form)} onClick={()=>setReview(true)}>{form.attachments_approved?'核对资料范围':'核对普通对话范围'}</button>}</div></Dialog>;
}
