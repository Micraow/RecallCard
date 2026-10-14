import {useEffect,useRef,useState} from 'react';
import type {ScopedService} from '../service/client';
import {appError,type AppError} from '../service/contracts';
import {initialCloudEventSetup,validCloudEventSetup,type CloudEventSetup,type CloudEventStatus,type SourceCloudSettings} from '../service/setup';
import {scopeLabel} from '../service/types';
import {Dialog,ErrorNotice} from './common';

export function CloudEventDialog({service,initial,close,saved}:{service:ScopedService;initial:CloudEventStatus|null;close:()=>void;saved:(value:CloudEventStatus)=>void}) {
 const [form,setForm]=useState<CloudEventSetup>(()=>initialCloudEventSetup(initial,service.scope));
 const [apiKey,setApiKey]=useState(''),[review,setReview]=useState(false),[confirmed,setConfirmed]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState<AppError|null>(null);
 const lock=useRef(false),active=useRef(true),keyInput=useRef<HTMLInputElement>(null);
 useEffect(()=>{active.current=true;return()=>{active.current=false;if(keyInput.current)keyInput.current.value='';};},[]);
 const change=<K extends keyof CloudEventSetup>(name:K,value:CloudEventSetup[K])=>{setForm(old=>({...old,[name]:value}));setConfirmed(false);};
 const setting=<K extends keyof SourceCloudSettings>(name:K,value:SourceCloudSettings[K])=>{setForm(old=>({...old,settings:{...old.settings,[name]:value}}));setConfirmed(false);};
 const clearKey=()=>{if(keyInput.current)keyInput.current.value='';setApiKey('');};
 const dismiss=()=>{clearKey();setConfirmed(false);close();};
 const save=async()=>{
  if(lock.current||!confirmed||!form.save_in_os||!validCloudEventSetup(form))return;
  lock.current=true;setBusy(true);setError(null);
  const secret=apiKey||null;clearKey();
  try {const result=await service.configureCloudEvents(form,secret);if(result.service_error)throw result.service_error;if(active.current){saved(result);close();}}
  catch(reason){if(active.current){setError(appError(reason));setConfirmed(false);setForm(old=>({...old,save_in_os:false}));setReview(false);}}
  finally{lock.current=false;if(active.current)setBusy(false);}
 };
 const settings=form.settings,valid=validCloudEventSetup(form);
 const budget=settings.budget_mode==='unlimited'?'无限，持续计量，可能持续产生供应商费用':`${settings.max_calls_per_month} 次 / ${settings.max_bytes_per_month} 字节`;
 return <Dialog title={review?'审阅云来源编码范围':'配置普通对话与附件云索引'} close={dismiss} busy={busy} wide>
  <div className="modal-body model-setup">
   {error&&<ErrorNotice error={error}/>}
   <p>当前范围：{scopeLabel(service.scope)}</p>
   {review?<>
    <dl><div><dt>唯一云接收接口</dt><dd>{settings.endpoint}</dd></div><div><dt>模型与输入空间</dt><dd>{settings.model} / {settings.revision} / {form.tokenizer} / {settings.dimensions}维</dd></div><div><dt>片段与本机索引空间</dt><dd>{form.chunk_bytes} UTF-8 字节 / {form.disk_cap_mib} MiB</dd></div></dl>
    <dl aria-label="云编码资料类型">
     <div><dt>普通对话</dt><dd>{settings.events_approved?'已批准：当前范围未撤回、未被修订替换的普通用户/助手原话片段及文档指令':'未批准，不发送普通对话正文'}</dd></div>
     <div><dt>附件</dt><dd>{settings.attachments_approved?'已批准：仅当前范围中父事件可见、未撤回且未被修订替换的安全 UTF-8 附件正文及文档指令；不支持 PDF 提取或 OCR':'未批准，不发送附件正文'}</dd></div>
     <div><dt>新问题</dt><dd>{settings.query_approved?'已批准：当前范围桌面与已授权浏览器/MCP 来源检索中未缓存的新问题及查询指令，可能包含私人信息':'未批准，不发送新问题'}</dd></div>
    </dl>
    <p>三类数据分别批准，附件批准不会同时启用普通对话。此批准不继承 Memory、其他范围或本机来源批准。附件内容是独立来源，不当作上传者原话。</p>
    <p>只向上述云接口发送已批准的数据。保存配置不会开始编码或建立索引；之后需明确点击更新已批准资料索引。已批准的新问题会在检索时按需编码。已发送请求不能撤回，可能已计费；停用只阻止后续发送。</p>
    <p className="context-note">月度预算：{budget}。来源、Memory 和各范围云向量请求共享整个 Vault 的累计用量，不是两套可叠加限额；本次请求按其当前批准的限额核对共享用量。失败预留不退，重启和重新保存不清零；这不是账单金额保证。</p>
    <details><summary>核对输入指令</summary><p>查询指令：{settings.query_instruction||'无'}</p><p>文档指令：{settings.document_instruction||'无'}</p></details>
    <label className="checkbox-label"><input id="cloud-event-save-os" type="checkbox" checked={form.save_in_os} onChange={e=>change('save_in_os',e.target.checked)}/>允许保存或使用此向量目标的系统保护凭据，并由上述已授权本机进程用于云来源编码</label>
    <label className="checkbox-label"><input id="cloud-event-confirm" type="checkbox" checked={confirmed} onChange={e=>setConfirmed(e.target.checked)}/>我确认这个云接收者、当前范围、三类发送选择和共享月度预算</label>
   </>:<>
    <p>可使用 Qwen 或其他 OpenAI-compatible embedding 服务。接口、模型、版本与维度由你核对填写；不会替你选定供应商或发送探测请求。</p>
    {(['endpoint','model','revision'] as const).map(name=><label className="field-label" key={name} htmlFor={`cloud-event-${name}`}>{({endpoint:'云 embedding HTTPS 接口',model:'向量模型名称',revision:'模型版本标识'})[name]}<input id={`cloud-event-${name}`} className="field-input" autoComplete="off" spellCheck={false} value={settings[name]} onChange={e=>setting(name,e.target.value)}/></label>)}
    <label className="field-label" htmlFor="cloud-event-tokenizer">Tokenizer及池化版本<input id="cloud-event-tokenizer" className="field-input" autoComplete="off" value={form.tokenizer} onChange={e=>change('tokenizer',e.target.value)}/></label>
    <label className="field-label" htmlFor="cloud-event-dimensions">向量维度<input id="cloud-event-dimensions" className="field-input" type="number" min="1" max="8192" value={settings.dimensions} onChange={e=>setting('dimensions',Number(e.target.value))}/></label>
    <label className="field-label" htmlFor="cloud-event-key">独立向量 API 密钥<input ref={keyInput} id="cloud-event-key" className="field-input" type="password" autoComplete="off" spellCheck={false} maxLength={8192} value={apiKey} onChange={e=>{setApiKey(e.target.value);setConfirmed(false);}} placeholder="已有同目标 embedding 系统凭据时可留空"/></label>
    <p className="field-hint">留空只请求复用同一向量目标的系统保护凭据，不读取或显示密钥，不借用整理模型凭据。跨进程编码要求系统保护存储可用；不可用时停止配置。密钥不会写入配置 JSON。</p>
    <label className="checkbox-label"><input id="cloud-event-corpus" type="checkbox" checked={settings.events_approved} onChange={e=>setting('events_approved',e.target.checked)}/>允许这个范围的普通对话发送到上述云接口编码</label>
    <label className="checkbox-label"><input id="cloud-event-attachments" type="checkbox" checked={settings.attachments_approved} aria-describedby="cloud-event-attachments-hint" onChange={e=>setting('attachments_approved',e.target.checked)}/>独立允许这个范围的安全 UTF-8 附件正文发送到上述云接口编码</label>
    <p id="cloud-event-attachments-hint" className="field-hint">仅包含当前范围中父事件可见、未撤回且未被修订替换的附件，原件须已保留且通过安全文本核验；不支持 PDF 提取或 OCR。附件内容是独立来源，不当作上传者原话。</p>
    <label className="checkbox-label"><input id="cloud-event-query" type="checkbox" checked={settings.query_approved} onChange={e=>setting('query_approved',e.target.checked)}/>允许这个范围来源检索的未缓存新问题按需发送到上述云接口编码</label>
    <label className="field-label" htmlFor="cloud-event-budget">云向量月度预算<select id="cloud-event-budget" className="field-input" value={settings.budget_mode||'limited'} onChange={e=>setting('budget_mode',e.target.value as 'limited'|'unlimited')}><option value="limited">有限配额</option><option value="unlimited">无限预算，持续计量</option></select></label>
    <div className="budget-fields"><label className="field-label" htmlFor="cloud-event-calls">每月最多请求<input id="cloud-event-calls" className="field-input" type="number" min="1" max="1000000" disabled={settings.budget_mode==='unlimited'} value={settings.max_calls_per_month} onChange={e=>setting('max_calls_per_month',Number(e.target.value))}/></label><label className="field-label" htmlFor="cloud-event-bytes">每月请求体字节<input id="cloud-event-bytes" className="field-input" type="number" min="1" max="4294967296" disabled={settings.budget_mode==='unlimited'} value={settings.max_bytes_per_month} onChange={e=>setting('max_bytes_per_month',Number(e.target.value))}/></label></div>
    <p className="field-hint">整个 Vault 的来源、Memory 和各范围云向量请求共用累计用量，限额不能相加；无限模式仍计量，可能持续产生费用。</p>
    {!settings.events_approved&&!settings.attachments_approved&&<p className="context-note">仅批准新问题不会发送普通对话或附件。没有当前模型空间的来源向量时，语义候选仍不可用。</p>}
    <details><summary>切块、本机空间上限与输入指令</summary>
     <label className="field-label" htmlFor="cloud-event-chunk">片段UTF-8字节上限<input id="cloud-event-chunk" className="field-input" type="number" min="256" max="8192" value={form.chunk_bytes} onChange={e=>change('chunk_bytes',Number(e.target.value))}/></label>
     <label className="field-label" htmlFor="cloud-event-disk">派生数据库与日志预算 MiB<input id="cloud-event-disk" className="field-input" type="number" min="16" max="512" value={form.disk_cap_mib} onChange={e=>change('disk_cap_mib',Number(e.target.value))}/></label>
     <label className="field-label" htmlFor="cloud-event-query-instruction">查询指令<textarea id="cloud-event-query-instruction" className="field-input" value={settings.query_instruction} onChange={e=>setting('query_instruction',e.target.value)}/></label>
     <label className="field-label" htmlFor="cloud-event-document-instruction">文档指令<textarea id="cloud-event-document-instruction" className="field-input" value={settings.document_instruction} onChange={e=>setting('document_instruction',e.target.value)}/></label>
    </details>
   </>}
  </div>
  <div className="modal-footer"><button className="button" disabled={busy} onClick={review?()=>{setReview(false);setConfirmed(false);setForm(old=>({...old,save_in_os:false}));}:dismiss}>{review?'返回修改':'取消'}</button>{review?<button className="button primary" disabled={busy||!valid||!confirmed||!form.save_in_os} onClick={()=>void save()}>{busy?'正在保存云来源配置':'确认保存云来源配置'}</button>:<button className="button primary" disabled={busy||!valid} onClick={()=>{setConfirmed(false);setReview(true);}}>审阅云来源范围</button>}</div>
 </Dialog>;
}
