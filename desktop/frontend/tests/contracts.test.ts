import assert from "node:assert/strict";
import test from "node:test";
import {
  appError,
  isRunning,
  validateJobs,
  importRecovery,
  coverageNotes,
} from "../src/service/contracts.ts";
import { validModelTarget } from "../src/service/setup.ts";
import {
  dateLabel,
  eventText,
  evidenceLabel,
  scopeLabel,
} from "../src/service/types.ts";

test("应用错误保留原因、来源成员、恢复动作和提交边界", () => {
  const error = appError({
    code: "invalid_json",
    message: "会话不完整",
    action: "重新下载",
    file_name: "synthetic.zip",
    member: "conversations.json",
    retryable: true,
    committed_events: 32,
  });
  assert.equal(error.code, "invalid_json");
  assert.equal(error.member, "conversations.json");
  assert.equal(error.committed_events, 32);
  assert.equal(error.action, "重新下载");
});
test("旧服务字符串错误不被泛化成没有原因的通知", () => {
  assert.equal(appError("当前资料范围已失效").message, "当前资料范围已失效");
  assert.equal(appError(new Error("需要重新读取")).message, "需要重新读取");
});
test("不兼容任务合同明确失败，不当作零任务", () => {
  assert.throws(() => validateJobs({ jobs: [] }), /格式不受支持/);
  assert.throws(
    () =>
      validateJobs([
        {
          schema: "old",
          job_id: "synthetic",
          state: "completed",
          progress: {},
        },
      ]),
    /合同版本不一致/,
  );
  assert.deepEqual(validateJobs([]), []);
});
test("未知任务状态不伪造完成", () => {
  assert.throws(
    () =>
      validateJobs([
        {
          schema: "recallcard.application-job/1",
          job_id: "synthetic",
          state: "interrupted",
          progress: {},
        },
      ]),
    /合同版本不一致/,
  );
});
test("暂停与需要输入不是后台正在运行", () => {
  assert.equal(isRunning({ state: "paused" } as any), false);
  assert.equal(isRunning({ state: "needs_input" } as any), false);
  assert.equal(isRunning({ state: "queued" } as any), true);
});
test("未知时间不使用当前导入时间补写", () => {
  assert.equal(dateLabel(null), "时间未知");
  assert.equal(dateLabel("invalid"), "时间未知");
});
test("来源正文按内容片段读取而不执行 HTML", () => {
  assert.equal(
    eventText({
      content: "<script>unsafe</script>",
      parts: [{ text: "<script>raw</script>" }, { text: "正文" }],
    } as any),
    "<script>raw</script>\n正文",
  );
});
test("观察和 AI 建议不标为明确事实，个人和项目标签分开", () => {
  assert.equal(evidenceLabel("assistant_suggestion"), "AI 建议");
  assert.equal(evidenceLabel("observed"), "观察所得");
  assert.equal(scopeLabel("personal"), "个人空间");
  assert.equal(scopeLabel("project:atlas"), "atlas");
});

test("损坏来源必须重新选择，暂停任务才可以继续", () => {
  assert.equal(
    importRecovery({
      can_resume: true,
      error: { code: "invalid_json" },
    } as any),
    "new_source",
  );
  assert.equal(
    importRecovery({
      can_resume: true,
      error: { code: "source_changed" },
    } as any),
    "new_source",
  );
  assert.equal(
    importRecovery({ can_resume: true, error: null } as any),
    "resume",
  );
  assert.equal(
    importRecovery({ can_resume: false, error: null } as any),
    "none",
  );
});

test("导入覆盖明确文件元数据和缺失正文，不声称附件原件已保存", () => {
  const notes = coverageNotes({
    file_references: 3,
    trace_placeholders: 2,
    hidden_fragments: 4,
  } as any);
  assert.ok(notes.some((note) => note.includes("导出未包含文件原件")));
  assert.ok(notes.some((note) => note.includes("没有工具输出正文")));
  assert.ok(notes.some((note) => note.includes("隐藏推理片段未收集")));
});

test("模型目的地只接受不夹带凭据的明确 HTTPS 地址", () => {
  assert.equal(
    validModelTarget("https://model.example/v1/chat/completions", "model-a"),
    true,
  );
  assert.equal(
    validModelTarget("http://model.example/v1/chat/completions", "model-a"),
    false,
  );
  assert.equal(
    validModelTarget(
      "https://user:secret@model.example/v1/chat/completions",
      "model-a",
    ),
    false,
  );
  assert.equal(
    validModelTarget(
      "https://model.example/v1/chat/completions?key=secret",
      "model-a",
    ),
    false,
  );
  assert.equal(validModelTarget("https://model.example/", "model-a"), false);
  assert.equal(
    validModelTarget("https://model.example/v1/chat/completions", ""),
    false,
  );
});


test("模型等待时间单独校验连接、读取与整个请求，不改变token预算", async () => {
  const {validMemoryTimeouts} = await import("../src/service/runtime.ts");
  assert.equal(validMemoryTimeouts({connect_seconds:15,read_seconds:300,operation_seconds:600}),true);
  for(const value of [{connect_seconds:121,read_seconds:300,operation_seconds:600},{connect_seconds:15,read_seconds:301,operation_seconds:300},{connect_seconds:0,read_seconds:30,operation_seconds:30},{connect_seconds:30,read_seconds:30,operation_seconds:1801}]) assert.equal(validMemoryTimeouts(value),false);
});

import { canRetryMemoryJob } from "../src/service/runtime.ts";
test("仅可恢复的供应商错误显示重试，不把证据冲突当网络失败", () => {
  const job:any={state:"needs_input",can_resume:true,error:{retryable:true}};
  assert.equal(canRetryMemoryJob(job),true);
  assert.equal(canRetryMemoryJob({...job,state:"failed"}),true);
  for(const state of ["running","queued","completed","cancelled","paused"])
    assert.equal(canRetryMemoryJob({...job,state}),false);
  assert.equal(canRetryMemoryJob({...job,error:{retryable:false}}),false);
  assert.equal(canRetryMemoryJob({...job,error:null}),false);
  assert.equal(canRetryMemoryJob({...job,can_resume:false}),false);
});

// 离线源码合同只防默认配置回退；实际按钮交互另由 Playwright 验收。
test("模型配置入口明确采用两阶段且不授予自动写回", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(new URL("../src/components/ModelSetup.tsx", import.meta.url), "utf8");
  assert.match(source, /pipeline: "staged_review"/);
  assert.match(source, /auto_apply: false/);
  assert.match(source, /共两次模型请求/);
  assert.match(source, /候选须经人工审核后发布/);
  assert.doesNotMatch(source, /auto_apply: true/);
});

import { canApproveMemoryReview, validateMemoryJobReview } from '../src/service/runtime.ts';
const pendingReview = () => ({job:{schema:'recallcard.application-job/1',job_id:'memory_test',scope:'personal',state:'needs_input'},source_snapshots:[],review:{result_hash:'a'.repeat(64),can_apply:true,already_applied:false,requires_protected_approval:false,changes:[],diagnostics:[]}} as any);
test('人工批准必须绑定完整结果、显式勾选及独立保护批准',()=>{
  const value=pendingReview();
  assert.equal(canApproveMemoryReview(value,false,false),false);
  assert.equal(canApproveMemoryReview(value,true,false),true);
  value.review.requires_protected_approval=true;
  assert.equal(canApproveMemoryReview(value,true,false),false);
  assert.equal(canApproveMemoryReview(value,true,true),true);
  for(const mutation of [(v:any)=>v.review.can_apply=false,(v:any)=>v.review.already_applied=true,(v:any)=>v.review.result_hash='short',(v:any)=>v.job.state='cancelled',(v:any)=>v.review=null]){const changed=pendingReview();mutation(changed);assert.equal(canApproveMemoryReview(changed,true,true),false);}
});
test('审阅响应缺少来源或来自旧任务/范围时拒绝显示和批准',()=>{
  assert.equal(validateMemoryJobReview(pendingReview(),'memory_test','personal').job.job_id,'memory_test');
  for(const mutation of [(v:any)=>v.job.job_id='old',(v:any)=>v.job.scope='project:other',(v:any)=>v.source_snapshots=null,(v:any)=>v.review.result_hash='short']){const value=pendingReview();mutation(value);assert.throws(()=>validateMemoryJobReview(value,'memory_test','personal'),/身份、范围/);}
});
test('真实客户端只传当前会话、范围与批准摘要，不让页面上传修改后的候选正文',async()=>{
  const {build}=await import('esbuild');
  const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],platform:'node',format:'esm',bundle:true,write:false});
  const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
  const calls:any[]=[];const client=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return command==='memory_job_review'?pendingReview():{state:'completed'};}},{session_id:'current_session'},'personal');
  const review=await client.reviewMemoryJob('memory_test');
  await client.approveMemoryJob('memory_test',review.review.result_hash,false);
  assert.deepEqual(calls[1],{command:'memory_job_approve',args:{sessionId:'current_session',scope:'personal',jobId:'memory_test',resultHash:'a'.repeat(64),approveProtected:false}});
});

test('有正文变更却缺少任何所引原话时拒绝不完整审阅',()=>{
 const value=pendingReview();value.review.changes=[{before:null,after:{source_refs:['evt_missing']}}];
 assert.throws(()=>validateMemoryJobReview(value,'memory_test','personal'),/缺少完整来源/);
 value.source_snapshots=[{ref:'event:evt_missing',event:{id:'evt_other',content:'不同原话'}}];
 assert.throws(()=>validateMemoryJobReview(value,'memory_test','personal'),/来源快照不完整/);
});

test('索引失败状态独立可见，重试只绑定当前会话范围且不上传语料或配置',async()=>{
 const {build}=await import('esbuild');
 const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],platform:'node',format:'esm',bundle:true,write:false});
 const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
 const calls:any[]=[];const client=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return {state:'pending'};}},{session_id:'index_session'},'personal');
 await client.retryMemoryIndex();assert.deepEqual(calls,[{command:'memory_index_retry',args:{sessionId:'index_session',scope:'personal'}}]);
 const {readFile}=await import('node:fs/promises');const source=await readFile(new URL('../src/components/ModelRuntimePanel.tsx',import.meta.url),'utf8');
 assert.match(source,/runtime\.index_refresh\.can_retry/);assert.match(source,/已发布/);assert.match(source,/索引失败不会撤回已批准记忆/);assert.match(source,/if\(indexLock\.current\)return/);
});

import {validLocalSemanticTarget, semanticSearchNote} from '../src/service/setup.ts';
const localTarget=()=>({endpoint:'http://127.0.0.1:80/v1/embeddings',model:'synthetic',dimensions:1024,revision:'v1',query_instruction:'',document_instruction:''});
test('本机语义表单只接受数字回环、明确端口与有界模型空间',()=>{
 assert.equal(validLocalSemanticTarget(localTarget()),true);
 assert.equal(validLocalSemanticTarget({...localTarget(),endpoint:'http://[::1]:5123/v1/embeddings'}),true);
 for(const endpoint of ['https://external.invalid/v1/embeddings','http://localhost:1/v1/embeddings','http://127.0.0.1/v1/embeddings','http://127.0.0.1:0/v1/embeddings','http://127.0.0.1:65536/v1/embeddings','http://u:p@127.0.0.1:1/v1/embeddings','http://127.0.0.1:1/v1/embeddings?key=x']) assert.equal(validLocalSemanticTarget({...localTarget(),endpoint}),false,endpoint);
 for(const value of [{dimensions:0},{dimensions:8193},{dimensions:1.5},{model:''},{revision:'\n'},{model:'汉'.repeat(86)},{document_instruction:'汉'.repeat(683)}]) assert.equal(validLocalSemanticTarget({...localTarget(),...value}),false);
});
test('语义配置客户端仅提交受限设置和当前范围，不允许页面指定 worker 或索引路径',async()=>{
 const {build}=await import('esbuild');const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],platform:'node',format:'esm',bundle:true,write:false});
 const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
 const calls:any[]=[];const client=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return {state:'configured'};}},{session_id:'semantic_session'},'project:synthetic');
 const request={...localTarget(),query_approved:true,memory_approved:false,resume_background:false};await client.semanticStatus();await client.configureSemantic(request);await client.disableSemantic();
 assert.deepEqual(calls,[{command:'semantic_setup_status',args:{sessionId:'semantic_session',scope:'project:synthetic'}},{command:'semantic_setup_apply',args:{sessionId:'semantic_session',scope:'project:synthetic',request}},{command:'semantic_setup_disable',args:{sessionId:'semantic_session',scope:'project:synthetic'}}]);
 const {readFile}=await import('node:fs/promises');const source=await readFile(new URL('../src/components/SemanticSetup.tsx',import.meta.url),'utf8');
 assert.match(source,/if\(!valid\|\|!confirmed\|\|lock.current\)return/);assert.match(source,/setConfirmed\(false\)/);assert.match(source,/包括已启用的整理与索引/);assert.match(source,/配置过期并退回目录\/词法读取/);assert.match(source,/暂定条目/);assert.match(source,/无需编写 JSON/);
 const native=await readFile(new URL('../../src-tauri/src/main.rs',import.meta.url),'utf8');assert.match(native,/operation.configure_semantic_with_binary\(request, &binary\)/);assert.match(native,/runtime::register_packaged_cli/);
});
test('读取结果显示语义退化原因与新查询编码，排序不等同回答正确率',()=>{
 assert.match(semanticSearchNote({semantic_search:'available',semantic_query_encoding:'local_encoded'}),/本次新查询已在本机编码/);
 assert.match(semanticSearchNote({semantic_search:'available'}),/排序不代表答案准确率/);
 assert.match(semanticSearchNote({semantic_search:'unavailable',semantic_error:'worker_failed'}),/worker_failed.*保留词法结果/);
 assert.match(semanticSearchNote({semantic_search:'not_configured'}),/词法匹配/);
});

test("无限月度预算显式展示且保留原有限值，不伪造巨大整数", async () => {
  const {monthlyBudgetLabel}=await import("../src/service/runtime.ts");
  const finite={max_calls_per_month:100,max_reserved_tokens_per_month:1000000,max_output_tokens_per_call:4096,max_request_bytes_per_call:1048576};
  assert.match(monthlyBudgetLabel(finite),/100 次请求/);
  assert.match(monthlyBudgetLabel({...finite,mode:"unlimited"}),/无限/);
  assert.match(monthlyBudgetLabel({...finite,mode:"unlimited"}),/持续计量/);
  assert.equal(finite.max_calls_per_month,100);
});

test("云向量需要独立数据类型和明确预算模式，不从 LLM 权限推断", async()=>{
 const {validCloudSemanticSettings}=await import('../src/service/setup.ts');
 const base={endpoint:'https://embedding.example.invalid/v1/embeddings',model:'synthetic',dimensions:2,revision:'v1',query_instruction:'',document_instruction:'',query_approved:false,memory_approved:false,budget_mode:'limited' as const,max_calls_per_month:1,max_bytes_per_month:1};
 assert.equal(validCloudSemanticSettings(base),false);
 assert.equal(validCloudSemanticSettings({...base,query_approved:true}),true);
 assert.equal(validCloudSemanticSettings({...base,memory_approved:true,budget_mode:'unlimited'}),true);
 assert.equal(validCloudSemanticSettings({...base,query_approved:true,endpoint:'https://key@example.invalid/api'}),false);
 assert.equal(validCloudSemanticSettings({...base,query_approved:true,budget_mode:'infinite' as any}),false);
});

import {canRefreshBrowserBridge,refreshBrowserBridge,type ConnectionEntry} from '../src/service/connections.ts';
const reconnectEntry = (): ConnectionEntry => ({id:'conn_synthetic',permission_revision:3,state:'read_succeeded',revoked:false,configured_at:'2026-01-01T00:00:00Z',last_handshake_at:null,last_bootstrap_at:null,last_read_at:'2026-01-02T00:00:00Z',last_capture_at:null,last_error:null,grant:{client_kind:'browser',host_identity:'a'.repeat(32),installation_id:'11111111-1111-4111-8111-111111111111',platform:'chatgpt',recall_scopes:['personal'],capture_scopes:['project:other'],provider_disclosure:true,auto_capture:false,auto_recall:false}});
const reconnectInventory=(entry:ConnectionEntry)=>({entries:[entry],pending_pairings:[],supported_protocols:[],identity_notice:'synthetic'});
test('已有浏览器更新只保留当前范围能力，暂停和回执不重写',async()=>{
 const entry=reconnectEntry();const before=structuredClone(entry);const calls:any[]=[];
 assert.equal(canRefreshBrowserBridge(entry,'personal'),true);
 const result=await refreshBrowserBridge(entry,'personal','chromium',async()=>reconnectInventory(entry),async(...args)=>{calls.push(args);return {registered:true};});
 assert.deepEqual(result,{registered:true});assert.deepEqual(calls,[['a'.repeat(32),'chromium',false]]);assert.deepEqual(entry,before);
 entry.grant.capture_scopes.push('personal');await refreshBrowserBridge(entry,'personal','brave',async()=>reconnectInventory(entry),async(...args)=>calls.push(args));assert.deepEqual(calls[1],['a'.repeat(32),'brave',true]);
});
test('撤权、授权变更和收窄在系统登记之前停止，错误浏览器与越界范围亦拒绝',async()=>{
 const entry=reconnectEntry();let writes=0;const install=async()=>++writes;
 for(const change of [(e:ConnectionEntry)=>{e.revoked=true;},(e:ConnectionEntry)=>{e.permission_revision++;},(e:ConnectionEntry)=>{e.grant.recall_scopes=[];},(e:ConnectionEntry)=>{e.grant.capture_scopes.push('personal');},(e:ConnectionEntry)=>{e.grant.host_identity='b'.repeat(32);}]){
  const current=structuredClone(entry);change(current);await assert.rejects(refreshBrowserBridge(entry,'personal','chrome',async()=>reconnectInventory(current),install),/授权已变化/);
 }
 await assert.rejects(refreshBrowserBridge(entry,'personal','firefox',async()=>reconnectInventory(entry),install),/不能更新/);
 await assert.rejects(refreshBrowserBridge(entry,'project:other','chrome',async()=>reconnectInventory(entry),install),/不能更新/);
 await assert.rejects(refreshBrowserBridge(entry,'personal','chrome',async()=>{throw Error('本机服务离线');},install),/离线/);
 assert.equal(writes,0);assert.equal(canRefreshBrowserBridge({...entry,grant:{...entry.grant,client_kind:'codex'}},'personal'),false);
});
test('取消系统登记返回取消，不制造连接成功或修改授权',async()=>{
 const entry=reconnectEntry();const before=structuredClone(entry);const result=await refreshBrowserBridge(entry,'personal','chrome',async()=>reconnectInventory(entry),async()=>null);
 assert.equal(result,null);assert.deepEqual(entry,before);
});
test('页面离开重建服务后保留本机桥待定结果，阻止跨范围重复登记且可恢复',async()=>{
 const {build}=await import('esbuild');const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
 const {RecallService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
 let finish:(value:any)=>void=()=>{};let installs=0;let installedArgs:any;const entry=reconnectEntry();
 const service=new RecallService({invoke:async(command:string,args:any)=>{if(command==='connection_inventory')return reconnectInventory(entry);if(command==='install_browser_connection'){installs++;installedArgs=args;return new Promise(resolve=>{finish=resolve;});}throw Error(command);}});
 const workspace={session_id:'synthetic-reconnect'};const first=service.scoped(workspace,'personal');const pending=first.refreshBrowserConnection(entry,'chromium');
 await new Promise(resolve=>setTimeout(resolve,0));const returned=service.scoped(workspace,'personal');const other=service.scoped(workspace,'project:other');
 assert.equal(returned.browserReconnectState(entry.id)?.pending,pending);assert.equal(other.browserReconnectState(entry.id),null);
 await assert.rejects(returned.refreshBrowserConnection(entry,'chromium'),/不要重复/);await assert.rejects(other.refreshBrowserConnection(entry,'chrome'),/不要重复/);assert.equal(installs,1);assert.deepEqual(installedArgs,{sessionId:'synthetic-reconnect',scope:'personal',extensionId:'a'.repeat(32),browser:'chromium',allowCapture:false});
 returned.dismissBrowserReconnect(entry.id);assert.equal(returned.browserReconnectPending,pending);
 finish({registered:true,registration:'/synthetic/native.json',capture_enabled:false,note:'未验证扩展'});await pending;
 assert.equal(returned.browserReconnectPending,null);assert.equal(returned.browserReconnectState(entry.id).result.registered,true);
 returned.dismissBrowserReconnect(entry.id);assert.equal(returned.browserReconnectState(entry.id),null);
});

import {parseMemoryTarget} from '../src/service/types.ts';
test('搜索的带版本 Memory ref 拆为裸 ID 和明确版本，拒绝坏格式且不吞掉版本',()=>{
 const id=`mem_${'a'.repeat(32)}`;assert.deepEqual(parseMemoryTarget(`memory:${id}@3`),{id,revision:3});assert.deepEqual(parseMemoryTarget(`${id}@3`),{id,revision:3});assert.deepEqual(parseMemoryTarget(id),{id,revision:null});
 for(const value of [`memory:${id}@0`,`memory:${id}@-1`,`memory:${id}@9007199254740992`,`memory:${id}@3/../../control`,`event:${id}`,`${id}@3@4`,'../vault'])assert.equal(parseMemoryTarget(value),null,value);
});
async function sourceChainClient(invoke:(command:string,args:any)=>Promise<any>){
 const {build}=await import('esbuild');const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
 const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);return new ScopedService({invoke},{session_id:'source_chain'},'personal');
}
test('真实前端客户端打开版本化搜索命中只发裸 ID，旧版本或隐藏不悄悄替换',async()=>{
 const id=`mem_${'b'.repeat(32)}`;let revision=4;let hidden=false;const calls:any[]=[];
 const client=await sourceChainClient(async(command,args)=>{calls.push({command,args});return {memory:{id,revision,source_refs:[],content:'合成候选'},hidden,can_restore:false};});
 assert.equal((await client.memory(`memory:${id}@4`)).revision,4);assert.deepEqual(calls[0],{command:'managed_memory_view',args:{sessionId:'source_chain',scope:'personal',id}});
 revision=5;await assert.rejects(client.memory(`memory:${id}@4`),error=>error.code==='stale_reference');
 revision=4;hidden=true;await assert.rejects(client.memory(`memory:${id}@4`),error=>error.code==='stale_reference');assert.equal((await client.memory(id)).hidden,true);
 const previous=calls.length;await assert.rejects(client.memory(`${id}@0`));assert.equal(calls.length,previous);
});
test('引用回看先重验所属 Memory，再按同 scope 的当前 Event 定位，不猜标题或页码',async()=>{
 const memoryId=`mem_${'c'.repeat(32)}`,eventId=`evt_${'d'.repeat(64)}`;const calls:any[]=[];
 const client=await sourceChainClient(async(command,args)=>{calls.push({command,args});return command==='managed_memory_source'?{id:eventId,role:'assistant',content:'建议不等于采用'}:{ref:`event:${eventId}`,conversation_ref:'真实返回的会话键',offset:27};});
 const location=await client.memorySourceLocation(memoryId,eventId);assert.equal(location.offset,27);assert.equal(location.event_ref,`event:${eventId}`);
 assert.deepEqual(calls,[{command:'managed_memory_source',args:{sessionId:'source_chain',scope:'personal',memoryId,eventId}},{command:'event_location',args:{sessionId:'source_chain',scope:'personal',reference:`event:${eventId}`}}]);
});
test('所属来源变更或当前定位失败不会返回导航目标，错误消息引用亦拒绝',async()=>{
 const memoryId=`mem_${'e'.repeat(32)}`,eventId=`evt_${'f'.repeat(64)}`;let mode='unrelated';let locations=0;
 const client=await sourceChainClient(async(command)=>{if(command==='managed_memory_source')return {id:mode==='unrelated'?'evt_other':eventId};locations++;if(mode==='suppressed')throw Error('原话已停止召回');return {ref:'event:evt_other',conversation_ref:'不能跳到这里',offset:0};});
 await assert.rejects(client.memorySourceLocation(memoryId,eventId),/来源关联/);assert.equal(locations,0);mode='suppressed';await assert.rejects(client.memorySourceLocation(memoryId,eventId),/停止召回/);mode='wrong-location';await assert.rejects(client.memorySourceLocation(memoryId,eventId),/定位结果/);
});

import {memoryPredecessors} from '../src/service/types.ts';
test('桌面历史关联只使用明确裸 ID，不伪造旧版本快照或修补路径',()=>{
 const id=`mem_${'1'.repeat(32)}`,previous=`mem_${'2'.repeat(32)}`;
 assert.deepEqual(memoryPredecessors({id,supersedes:[previous,previous,`${previous}@1`,`memory:${previous}`,'../../vault',id]}),{ids:[previous],invalid:4});
 assert.deepEqual(memoryPredecessors({id}),{ids:[],invalid:0});assert.deepEqual(memoryPredecessors({id,supersedes:'bad' as any}),{ids:[],invalid:1});
});

import {memoryJobWindow,type MemoryJob} from '../src/service/runtime.ts';
const queueJob=(id:string,state:MemoryJob['state'],created_at:string)=>({job_id:id,state,created_at} as MemoryJob);
test('旧待审候选不会被十条新完成记录藏住，全部任务仍可分十项读取',()=>{
 const old=queueJob('memory_old_review','needs_input','2026-01-01T00:00:00Z');const jobs=[old,...Array.from({length:12},(_,i)=>queueJob(`memory_done_${i}`,'completed',`2026-01-02T00:${String(i).padStart(2,'0')}:00Z`))];const before=jobs.map(j=>j.job_id);
 const pending=memoryJobWindow(jobs,'pending',0);assert.deepEqual(pending.rows,[old]);assert.equal(pending.pendingCount,1);assert.equal(pending.total,1);
 const first=memoryJobWindow(jobs,'all',0),second=memoryJobWindow(jobs,'all',1);assert.equal(first.rows.length,10);assert.equal(second.rows.length,3);assert.equal(second.rows.at(-1)?.job_id,old.job_id);assert.equal(second.end,13);assert.deepEqual(jobs.map(j=>j.job_id),before);
});
test('超过十个待处理任务均可达，完成后页码收缩且同时间排序稳定',()=>{
 const states:MemoryJob['state'][]=['needs_input','failed','paused','running','queued'];const jobs=Array.from({length:13},(_,i)=>queueJob(`memory_${String(i).padStart(2,'0')}`,states[i%5],'2026-01-01T00:00:00Z'));
 const first=memoryJobWindow([...jobs].reverse(),'pending',0),last=memoryJobWindow(jobs,'pending',1);assert.equal(first.rows.length,10);assert.equal(last.rows.length,3);assert.equal(new Set([...first.rows,...last.rows].map(j=>j.job_id)).size,13);assert.equal(first.rows[0].job_id,'memory_00');
 assert.equal(memoryJobWindow(jobs.slice(0,1),'pending',1).page,0);assert.equal(memoryJobWindow([],'pending',8).start,0);assert.equal(memoryJobWindow(jobs.map(j=>({...j,state:'completed'})),'pending',0).total,0);
});

import {ManualDreamFlow,MANUAL_DREAM_LIMIT} from '../src/service/manualDream.ts';
function manualFixture() {
 const calls:any[]=[];const input='a'.repeat(64),result='b'.repeat(64),id='evt_manual';
 const task:any={job_id:`dream_${input}`,input_hash:input,scope:'personal',source_count:1,memory_count:0,text:'完整合成任务：用户原话与 AI 建议分别标记。',sources:[{ref:`event:${id}`,role:'user',text:'用户原话',truncated:false,occurred_at:null}]};task.byte_count=new TextEncoder().encode(task.text).length;
 const preview:any={preview_id:'preview_manual',session_id:'session_manual',scope:'personal',review:{job_id:task.job_id,input_hash:input,result_hash:result,can_apply:true,already_applied:false,requires_protected_approval:false,diagnostics:[],changes:[{operation:'add',before:null,after:{id:'mem_manual',revision:1,content:'合成可追溯记忆',scope:'personal',source_refs:[id],evidence:'user_explicit'}}]}};
 const receipt:any={job_id:task.job_id,input_hash:input,result_hash:result,already_applied:false,changes:[{id:'mem_manual',revision:1,content_hash:'c'.repeat(64)}]};
 const commands:any={prepareDreamTask:async(s:any,m:any)=>{calls.push(['prepare',s,m]);return structuredClone(task);},cancelPreviews:async()=>{calls.push(['cancel']);},copy:async(text:string)=>{calls.push(['copy',text]);},reviewDreamText:async(text:string)=>{calls.push(['review',text]);return structuredClone(preview);},applyDream:async(...args:any[])=>{calls.push(['apply',...args]);return structuredClone(receipt);},read:async(ref:string)=>{calls.push(['read',ref]);return {results:[{record:{id,content:'完整合成原话',role:'user',occurred_at:null}}]};}};
 return {calls,task,preview,receipt,commands,flow:new ManualDreamFlow(commands,'session_manual','personal')};
}
test('现代手动流程按当前来源生成、重验复制、审查、回读与单次批准',async()=>{
 const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);assert.equal(f.flow.canApply(true,true),false);
 await f.flow.copyTask();assert.deepEqual(f.calls.slice(-3).map(c=>c[0]),['cancel','prepare','copy']);assert.equal(f.calls.at(-1)[1],f.task.text);
 await f.flow.review('```json\n合成结果由核心严格解析\n```');assert.equal(f.flow.canApply(false,true),false);assert.equal((await f.flow.source('evt_manual')).role,'user');
 await f.flow.apply(true,false);assert.deepEqual(f.calls.at(-1),['apply','preview_manual',false]);assert.equal(f.flow.receipt?.changes[0].id,'mem_manual');assert.equal(f.flow.preview,null);await assert.rejects(f.flow.apply(true,false),/审阅/);
});
test('来源改变、撤回或剪贴板失败均不保留旧复制任务与批准',async()=>{
 for(const mode of ['changed','revoked','clipboard']){const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);await f.flow.review('{}');if(mode==='changed'){f.task.text+='已变化';f.task.byte_count=new TextEncoder().encode(f.task.text).length;}if(mode==='revoked')f.commands.prepareDreamTask=async()=>{throw Error('来源已撤回');};if(mode==='clipboard')f.commands.copy=async()=>{throw Error('剪贴板失败');};await assert.rejects(f.flow.copyTask());assert.equal(f.flow.task,null);assert.equal(f.flow.preview,null);assert.equal(f.calls.filter(c=>c[0]==='copy').length,0);}
});
test('粘贴结果拒绝其他任务、会话、范围和不完整差异，取消服务器旧预览',async()=>{
 for(const mutation of [(p:any)=>p.review.job_id='dream_other',(p:any)=>p.review.input_hash='c'.repeat(64),(p:any)=>p.session_id='old_session',(p:any)=>p.scope='project:other',(p:any)=>p.review.can_apply=undefined,(p:any)=>p.review.changes[0].after.scope='project:other']){const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);mutation(f.preview);await assert.rejects(f.flow.review('{}'),/不属于当前任务/);assert.equal(f.flow.preview,null);assert.equal(f.calls.at(-1)[0],'cancel');assert.equal(f.calls.filter(c=>c[0]==='apply').length,0);}
});
test('手动来源和 UTF-8 字节预算超限时不会请求模型或提交结果',async()=>{
 const f=manualFixture();await assert.rejects(f.flow.prepare([],[]));await assert.rejects(f.flow.prepare(['event:../../secret'],[]));await assert.rejects(f.flow.prepare(Array.from({length:65},(_,i)=>`event:evt_${i}`),[]));await assert.rejects(f.flow.prepare(['event:evt_manual'],['memory:mem_manual']));assert.equal(f.calls.filter(c=>c[0]==='prepare').length,0);
 await f.flow.prepare(['event:evt_manual'],[]);await assert.rejects(f.flow.review('中'.repeat(Math.ceil(MANUAL_DREAM_LIMIT/3))),/1 MiB/);assert.equal(f.calls.filter(c=>c[0]==='review').length,0);
});
test('受保护审批独立，编辑撤销旧预览，失败不自动重试，双击不重复写回',async()=>{
 const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);f.preview.review.requires_protected_approval=true;await f.flow.review('{}');await assert.rejects(f.flow.apply(true,false));await f.flow.invalidate();assert.equal(f.flow.canApply(true,true),false);await f.flow.review('{}');
 let reject!:(e:Error)=>void;f.commands.applyDream=async()=>{f.calls.push(['apply']);return await new Promise((_,r)=>{reject=r;});};const pending=f.flow.apply(true,true);await assert.rejects(f.flow.apply(true,true));reject(Error('已撤回来源，未写回'));await assert.rejects(pending,/已撤回/);assert.equal(f.flow.preview,null);assert.equal(f.flow.receipt,null);assert.equal(f.calls.filter(c=>c[0]==='apply').length,1);
});
test('原话错配/预算耗尽撤销批准；关闭或核心幂等回执禁止继续写回',async()=>{
 for(const response of [{results:[{record:{id:'wrong'}}]},{results:[],status:'budget_exhausted'}]){const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);await f.flow.review('{}');f.commands.read=async()=>response;await assert.rejects(f.flow.source('evt_manual'));assert.equal(f.flow.preview,null);}
 const f=manualFixture();await f.flow.prepare(['event:evt_manual'],[]);f.preview.review.already_applied=true;await f.flow.review('{}');assert.equal(f.flow.canApply(true,true),false);await f.flow.close();await assert.rejects(f.flow.prepare(['event:evt_manual'],[]),/窗口已关闭/);
});
test('手动整理客户端复用真实注册命令，只批准服务器预览号且保持当前范围',async()=>{
 const {build}=await import('esbuild');const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);const calls:any[]=[];const service=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return {};}},{session_id:'manual_session'},'project:manual');
 await service.prepareDreamTask(['event:evt_test'],['memory:mem_test@2']);await service.reviewDreamText('{}');await service.applyDream('preview_checked',true);
 assert.deepEqual(calls,[{command:'prepare_dream_task',args:{sessionId:'manual_session',scope:'project:manual',sourceRefs:['event:evt_test'],memoryRefs:['memory:mem_test@2']}},{command:'review_dream_text',args:{sessionId:'manual_session',scope:'project:manual',text:'{}'}},{command:'apply_dream',args:{sessionId:'manual_session',scope:'project:manual',previewId:'preview_checked',approveProtected:true}}]);
});
