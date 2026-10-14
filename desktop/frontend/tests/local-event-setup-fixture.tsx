import {createRoot} from 'react-dom/client';
import {LocalEventSetupPanel} from '../src/components/LocalEventSetup';
import {ScopedService,type Transport} from '../src/service/client';
import type {CloudEventSetup,CloudEventStatus,LocalEventSetup,LocalEventStatus} from '../src/service/setup';

// 合成浏览器夹具：只记录 IPC 请求，不接入真实资料库、文件或编码服务。
const root=createRoot(document.getElementById('local-event-setup-fixture')!);
let revision=0;
export function renderLocalEventSetup(initial:LocalEventStatus|CloudEventStatus,failFirstSave=false,scope='project:synthetic') {
 let status:CloudEventStatus='cloud' in initial?initial:{...initial,provider:initial.settings?'local':'none',cloud:{grant:null,usage:{month:'2026-10',reserved_calls:0,reserved_bytes:0,query_calls:0,memory_calls:0,source_query_calls:0,event_calls:0,attachment_calls:0},budget_note:'失败预留不退，重启不清零。'}};
 const calls:{command:string;args?:Record<string,unknown>}[]=[];
 (window as any).__LOCAL_EVENT_CALLS__=calls;
 const transport:Transport={invoke:async<T,>(command:string,args?:Record<string,unknown>):Promise<T>=>{
  calls.push({command,args});
  if(command==='local_event_status'||command==='cloud_event_status')return structuredClone(status) as T;
  if(command==='local_event_apply'||command==='cloud_event_apply') {
   if(failFirstSave){failFirstSave=false;throw new Error('合成保存失败，需要重新确认');}
   if(command==='cloud_event_apply') {
    if((window as any).__HOLD_CLOUD_EVENT_SAVE__)await new Promise(resolve=>(window as any).__RESOLVE_CLOUD_EVENT_SAVE__=resolve);
    const request=structuredClone(args?.request as CloudEventSetup);
    status={...status,state:'configured',message:'合成云配置',provider:'cloud',settings:{...request.settings,tokenizer:request.tokenizer,chunk_bytes:request.chunk_bytes,disk_cap_mib:request.disk_cap_mib},cloud:{...status.cloud,grant:{revision:'synthetic-grant-v1',scope,accepted_at:'2026-10-14T00:00:00Z',settings:request.settings}}};
   }else status={...status,state:'configured',message:'合成本机配置',provider:'local',settings:structuredClone(args?.request as LocalEventSetup),cloud:{...status.cloud,grant:null}};
   return structuredClone(status) as T;
  }
  if(command==='local_event_refresh')return {state:'complete',complete:true,processed_sources:1,total_sources:1} as T;
  if(command==='local_event_disable'){status={...status,state:'disabled',message:'已停用',settings:null,provider:'none'};return status as T;}
  if(command==='cloud_event_disable') {
   status={...status,...(status.provider==='cloud'?{state:'disabled',message:'已停用云来源',settings:null,provider:'none' as const}:{}),cloud:{...status.cloud,grant:null}};
   return structuredClone(status) as T;
  }
  throw new Error('合成夹具拒绝未预期的操作：'+command);
 }};
 const service=new ScopedService(transport,{session_id:'synthetic-session',root:'synthetic-no-files',display_name:'合成资料库',scopes:[scope],event_count:0,memory_count:0,health:null},scope);
 root.render(<LocalEventSetupPanel key={++revision} service={service}/>);
}

export function unmountLocalEventSetup() {root.unmount();}
