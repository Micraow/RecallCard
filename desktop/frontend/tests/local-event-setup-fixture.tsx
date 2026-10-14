import {createRoot} from 'react-dom/client';
import {LocalEventSetupPanel} from '../src/components/LocalEventSetup';
import {ScopedService,type Transport} from '../src/service/client';
import type {LocalEventSetup,LocalEventStatus} from '../src/service/setup';

// 合成浏览器夹具：只记录 IPC 请求，不接入真实资料库、文件或编码服务。
const root=createRoot(document.getElementById('local-event-setup-fixture')!);
let revision=0;
export function renderLocalEventSetup(initial:LocalEventStatus,failFirstSave=false,scope='project:synthetic') {
 let status=initial;
 const calls:{command:string;args?:Record<string,unknown>}[]=[];
 (window as any).__LOCAL_EVENT_CALLS__=calls;
 const transport:Transport={invoke:async<T,>(command:string,args?:Record<string,unknown>):Promise<T>=>{
  calls.push({command,args});
  if(command==='local_event_status')return structuredClone(status) as T;
  if(command==='local_event_apply') {
   if(failFirstSave){failFirstSave=false;throw new Error('合成保存失败，需要重新确认');}
   status={state:'configured',message:'合成配置',settings:structuredClone(args?.request as LocalEventSetup)};
   return structuredClone(status) as T;
  }
  if(command==='local_event_refresh')return {state:'complete',complete:true,processed_sources:1,total_sources:1} as T;
  if(command==='local_event_disable'){status={state:'disabled',message:'已停用',settings:null};return status as T;}
  throw new Error('合成夹具拒绝未预期的操作：'+command);
 }};
 const service=new ScopedService(transport,{session_id:'synthetic-session',root:'synthetic-no-files',display_name:'合成资料库',scopes:[scope],event_count:0,memory_count:0,health:null},scope);
 root.render(<LocalEventSetupPanel key={++revision} service={service}/>);
}

export function unmountLocalEventSetup() {root.unmount();}
