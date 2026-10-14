import assert from 'node:assert/strict';
import test from 'node:test';
import {build} from 'esbuild';
import {initialCloudEventSetup,validSourceCloudSettings,validCloudEventSetup,recallCoverageNotes,type CloudEventStatus,type SourceCloudSettings} from '../src/service/setup.ts';

const settings:SourceCloudSettings={endpoint:'https://embedding.example.invalid/v1/embeddings',model:'synthetic-choice',revision:'synthetic-version',dimensions:2,query_instruction:'',document_instruction:'',events_approved:false,attachments_approved:false,query_approved:false,budget_mode:'limited',max_calls_per_month:100,max_bytes_per_month:1048576};
const usage={month:'2026-10',reserved_calls:25,reserved_bytes:2048,query_calls:10,memory_calls:10,source_query_calls:2,event_calls:2,attachment_calls:1};
const initial:CloudEventStatus={state:'configured',message:'合成',provider:'cloud',settings:{...settings,tokenizer:'synthetic-tokenizer',chunk_bytes:1024,disk_cap_mib:256},cloud:{grant:{scope:'personal',revision:'synthetic-grant',accepted_at:'2026-10-14T00:00:00Z',settings:{...settings,attachments_approved:true}},usage,budget_note:'共享预算'}};

test('云来源新表单无真实目标，三项批准独立且不继承本机或其他范围',()=>{
 for(const status of [null,{...initial,provider:'local' as const},{...initial,cloud:{...initial.cloud,grant:null}}]) {
  const form=initialCloudEventSetup(status,'personal');
  assert.equal(form.settings.endpoint,'');assert.equal(form.settings.model,'');assert.equal(form.settings.revision,'');
  assert.equal(form.settings.events_approved,false);assert.equal(form.settings.attachments_approved,false);assert.equal(form.settings.query_approved,false);assert.equal(form.save_in_os,false);
 }
 assert.equal(initialCloudEventSetup(initial,'project:other').settings.attachments_approved,false);
 const existing=initialCloudEventSetup(initial,'personal');
 assert.equal(existing.settings.attachments_approved,true);assert.equal(existing.settings.events_approved,false);assert.equal(existing.settings.query_approved,false);assert.equal(existing.save_in_os,false);
 assert.notEqual(existing.settings,initial.cloud.grant!.settings);
 const legacy={...initial,cloud:{...initial.cloud,grant:{...initial.cloud.grant!,settings:{...settings,memory_approved:true,query_approved:undefined,events_approved:undefined,attachments_approved:undefined} as unknown as SourceCloudSettings}}};
 const missing=initialCloudEventSetup(legacy,'personal').settings;
 assert.equal('memory_approved' in missing,false);assert.equal(missing.query_approved,false);assert.equal(missing.events_approved,false);assert.equal(missing.attachments_approved,false);
});

test('云来源目标和预算有界，三种单独批准都合法而Memory批准无效',()=>{
 assert.equal(validSourceCloudSettings(settings),false);
 for(const field of ['events_approved','attachments_approved','query_approved'] as const) {
  const approved={...settings,[field]:true};assert.equal(validSourceCloudSettings(approved),true);
  assert.equal([approved.events_approved,approved.attachments_approved,approved.query_approved].filter(Boolean).length,1);
 }
 assert.equal(validSourceCloudSettings({...settings,memory_approved:true} as SourceCloudSettings),false);
 const approved={...settings,attachments_approved:true};
 for(const patch of [{endpoint:'http://127.0.0.1:46123/v1/embeddings'},{endpoint:'https://key@example.invalid/v1/embeddings'},{endpoint:'https://example.invalid/v1/embeddings?key=synthetic'},{endpoint:'https://example.invalid/v1/@embeddings'},{endpoint:'https://example.invalid/v1/embeddings#synthetic'},{endpoint:'https://example.invalid:0/v1/embeddings'},{endpoint:'https://example.invalid/v1/embeddings/'},{model:''},{model:'汉'.repeat(86)},{revision:''},{dimensions:8193},{dimensions:1.5},{max_calls_per_month:0},{max_calls_per_month:1000001},{max_bytes_per_month:4294967297},{budget_mode:'invalid'},{query_instruction:'synthetic\u0000instruction'}])assert.equal(validSourceCloudSettings({...approved,...patch} as SourceCloudSettings),false,JSON.stringify(patch));
 assert.equal(validSourceCloudSettings({...approved,budget_mode:'unlimited'}),true);
 const form={settings:approved,tokenizer:'synthetic-tokenizer',chunk_bytes:1024,disk_cap_mib:256,save_in_os:false};
 assert.equal(validCloudEventSetup(form),true);
 for(const patch of [{tokenizer:''},{tokenizer:'汉'.repeat(86)},{chunk_bytes:255},{disk_cap_mib:513}])assert.equal(validCloudEventSetup({...form,...patch}),false);
});

test('来源云服务桥绑定当前会话与范围，配置不触发编码且停用使用独立命令',async()=>{
 const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
 const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
 const calls:any[]=[];
 const service=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return initial;}},{session_id:'synthetic-session'},'personal');
 const request={...initialCloudEventSetup(initial,'personal'),save_in_os:true};
 await service.cloudEventStatus();await service.configureCloudEvents(request,null);await service.disableCloudEvents();
 assert.deepEqual(calls.map(call=>call.command),['cloud_event_status','cloud_event_apply','cloud_event_disable']);
 assert.deepEqual(calls[1].args,{sessionId:'synthetic-session',scope:'personal',request,apiKey:null});
 for(const call of calls){assert.equal(call.args.scope,'personal');assert.equal(call.args.sessionId,'synthetic-session');}
 assert.equal('apiKey' in request,false);
});

test('云附件语义成功与新问题云编码分别报告，不冒充本机或Memory-only',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{cloud_event_semantic:true,local_event_semantic:false,cloud_fallback:false,scopes:[{scope:'personal',state:'ready',provider:'cloud',query_encoding:'cloud',attachments:{semantic:true,status:'ready'},lexical:{status:'available'}}]}}).join('\n');
 assert.match(notes,/附件正文已参与云语义检索/);assert.match(notes,/问题已按当前范围的独立批准发送到云编码服务/);assert.match(notes,/本次独立批准的资料已参与独立批准的云语义检索/);
 assert.match(notes,/没有自动云回退/);assert.match(notes,/不含 PDF 提取或 OCR/);
 assert.doesNotMatch(notes,/本机编码服务|附件正文已参与本机|普通对话已直接参与|只覆盖已建索引的记忆|尚未配置/);
});

test('云cache命中不冒充本机编码或本次外发，mixed各范围分别描述',()=>{
 const cached=recallCoverageNotes({results:[],coverage:{cloud_event_semantic:true,cloud_fallback:false,scopes:[{scope:'personal',state:'ready',provider:'cloud',query_encoding:'cache'}]}}).join('\n');
 assert.match(cached,/已缓存的问题向量/);assert.doesNotMatch(cached,/已发送|发送到|本机问题向量|本机编码服务/);
 const mixed=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,cloud_event_semantic:true,scopes:[{scope:'personal',state:'ready',provider:'cloud',query_encoding:'cache',attachments:{semantic:true}},{scope:'project:local',state:'ready',provider:'local',query_encoding:'local',attachments:{semantic:true}},{scope:'project:cloud',state:'ready',provider:'cloud',query_encoding:'cloud'}]}}).join('\n');
 for(const expected of ['本机及云语义检索','已缓存的问题向量','本次部分范围的问题已发送到你批准的本机编码服务','本次部分范围的问题已按当前范围的独立批准发送到云编码服务','没有自动云回退'])assert.ok(mixed.includes(expected),mixed);
});

test('云降级与无自动回退不等同未配置云或本机服务离线',()=>{
 const degraded=recallCoverageNotes({results:[],coverage:{local_event_semantic:false,cloud_event_semantic:false,cloud_fallback:false,scopes:[{scope:'personal',state:'degraded',provider:'cloud',query_encoding:'unavailable',fusion:'lexical_only',semantic:{status:'unavailable',reason:'network_budget_exhausted'},lexical:{status:'available'}}]}}).join('\n');
 assert.match(degraded,/云编码服务不可用/);assert.match(degraded,/没有语义分数/);assert.match(degraded,/不会自动更换提供器/);
 assert.doesNotMatch(degraded,/本机编码服务不可用|已发送|已直接参与|尚未配置/);
 const none=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,provider:'cloud'}}).join('\n');
 assert.match(none,/自动回退/);assert.doesNotMatch(none,/尚未配置普通对话本机索引/);
});

test('云与混合语义路径仍显示预算省略和不完整附件覆盖',()=>{
 for(const local of [false,true]) {
  const notes=recallCoverageNotes({results:[],truncated:true,coverage:{local_event_semantic:local,cloud_event_semantic:true,presentation:'compact_index_metadata_full_source_via_fetch',budget_limited:true,budget_omitted_by_kind:{event:2,attachment:1,memory:0},scopes:[{scope:'personal',state:'ready',provider:'cloud',query_encoding:'cache',attachments:{semantic:true,status:'partial',skipped:1,limited:true}}]}}).join('\n');
  for(const expected of ['紧凑索引元数据','普通对话 2 项、附件 1 项','不保证每种来源类型均有保留','附件语义索引来源覆盖不完整','换关键词不会解除这项来源检查上限'])assert.ok(notes.includes(expected),notes);
  assert.doesNotMatch(notes,/文字预算|片段或核验达到/);
 }
});


test('已知云来源失败披露可能外发计费，混合本机成功不掩盖云路无结果',()=>{
 const failed={scope:'project:cloud',state:'unavailable',provider:'cloud' as const};
 for(const localSuccess of [false,true]) {
  const notes=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,event_search:localSuccess?'explicit_local_and_cloud':'explicit_cloud',local_event_semantic:localSuccess,cloud_event_semantic:false,scopes:[failed,...(localSuccess?[{scope:'personal',state:'ready',provider:'local' as const,query_encoding:'cache',lexical:{status:'available'}}]:[])]}}).join('\n');
  assert.match(notes,/所选云来源检索未完成/);
  assert.match(notes,/可能已经发送已批准请求并产生费用/);
  assert.match(notes,/本次没有这些云范围的语义结果/);
  assert.doesNotMatch(notes,/本次只在本地|尚未配置|已参与独立批准的云语义检索|已发送到.*云编码服务/);
  if(localSuccess){assert.match(notes,/普通对话已直接参与本机语义检索/);assert.match(notes,/已缓存的问题向量/);assert.match(notes,/没有自动云回退/);}
 }
 const degraded=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,event_search:'explicit_cloud',scopes:[{...failed,state:'degraded',fusion:'lexical_only'}]}}).join('\n');
 assert.match(degraded,/可能已经发送已批准请求并产生费用/);assert.match(degraded,/云编码服务不可用/);assert.doesNotMatch(degraded,/本次只在本地/);
});
