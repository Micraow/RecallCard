import assert from 'node:assert/strict';
import test from 'node:test';
import {validLocalEventSettings,normalizeLocalEventSettings,validCloudSemanticSettings,recallCoverageNotes,type LocalEventSetup} from '../src/service/setup.ts';
import type {SearchPage} from '../src/service/types.ts';
const settings:LocalEventSetup={endpoint:'http://127.0.0.1:46123/v1/embeddings',model:'synthetic',revision:'fake-v1',tokenizer:'test-only',dimensions:2,query_instruction:'',document_instruction:'',events_approved:false,attachments_approved:false,query_approved:false,chunk_bytes:1024,disk_cap_mib:256};
test('普通对话本机索引需独立明确范围和版本，拒绝云、凭据地址与越界预算',()=>{
 assert.equal(validLocalEventSettings(settings),false);
 const approved={...settings,events_approved:true};assert.equal(validLocalEventSettings(approved),true);
 for(const change of [{endpoint:'https://example.invalid/v1/embeddings'},{endpoint:'http://localhost:1234/v1/embeddings'},{endpoint:'http://key@127.0.0.1:1234/v1/embeddings'},{tokenizer:''},{revision:''},{chunk_bytes:255},{disk_cap_mib:513}])assert.equal(validLocalEventSettings({...approved,...change}),false);
});
test('附件授权默认关闭，与普通对话和新问题独立且不扩展云批准',()=>{
 assert.equal(settings.attachments_approved,false);
 for(const field of ['events_approved','attachments_approved','query_approved'] as const) {
  const approved={...settings,[field]:true};
  assert.equal(validLocalEventSettings(approved),true);
  assert.equal(Object.values({events:approved.events_approved,attachments:approved.attachments_approved,query:approved.query_approved}).filter(Boolean).length,1);
 }
 const attachmentsOnly={...settings,attachments_approved:true};
 for(const change of [{endpoint:'https://example.invalid/v1/embeddings'},{endpoint:'http://localhost:1234/v1/embeddings'},{endpoint:'http://127.0.0.1:0/v1/embeddings'},{endpoint:'http://127.0.0.1:65536/v1/embeddings'},{endpoint:'http://127.0.0.1:1234/v1/embeddings?scope=all'},{dimensions:0},{dimensions:8193},{dimensions:1.5},{tokenizer:''},{revision:''},{chunk_bytes:255},{disk_cap_mib:513}])assert.equal(validLocalEventSettings({...attachmentsOnly,...change}),false);
 assert.equal(validLocalEventSettings({...attachmentsOnly,endpoint:'http://[::1]:46123/v1/embeddings'}),true);
 const cloud={endpoint:'https://example.invalid/v1/embeddings',model:'synthetic',revision:'fake-v1',dimensions:2,query_instruction:'',document_instruction:'',query_approved:false,memory_approved:false,max_calls_per_month:5,max_bytes_per_month:1024,attachments_approved:true};
 assert.equal(validCloudSemanticSettings(cloud),false);
});
test('旧状态不继承附件授权，显式false和显式true保持独立',()=>{
 const {attachments_approved:_,...legacy}=settings;
 for(const events_approved of [false,true])for(const query_approved of [false,true]) {
  const old={...legacy,events_approved,query_approved};
  const normalized=normalizeLocalEventSettings(old);
  assert.equal(normalized.attachments_approved,false);
  assert.equal(normalized.events_approved,events_approved);
  assert.equal(normalized.query_approved,query_approved);
  assert.equal(Object.hasOwn(old,'attachments_approved'),false);
  assert.equal(normalizeLocalEventSettings({...old,attachments_approved:false}).attachments_approved,false);
  assert.equal(normalizeLocalEventSettings({...old,attachments_approved:true}).attachments_approved,true);
 }
 assert.equal(normalizeLocalEventSettings({...settings,attachments_approved:'true' as unknown as boolean}).attachments_approved,false);
});
test('本机Event覆盖不再误报只有Memory语义，也不把片段候选当完整答案',()=>{
 const notes=recallCoverageNotes({results:[],truncated:true,coverage:{local_event_semantic:true,semantic_query_encoding:'local',chunk_selection:'per_event_best_chunk'}}).join('\n');
 assert.match(notes,/普通对话已直接参与本机语义检索/);assert.match(notes,/没有云回退/);assert.match(notes,/一处来源已核验的匹配片段/);assert.doesNotMatch(notes,/得分最高/);assert.match(notes,/本机编码服务/);assert.doesNotMatch(notes,/只覆盖已建索引的记忆/);
});

test('共享入口缓存与未覆盖如实显示，不把缓存说成新外发或回到云配置',()=>{
 const cached=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,cloud_fallback:false,scopes:[{scope:'personal',state:'ready',query_encoding:'cache'}]}}).join('\n');
 assert.match(cached,/已缓存/);assert.doesNotMatch(cached,/已发送/);
 const missing=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,event_search:'local_only'}}).join('\n');assert.match(missing,/未覆盖/);assert.match(missing,/不会改用云编码/);
 const lexical=recallCoverageNotes({results:[],coverage:{cloud_fallback:false}}).join('\n');assert.match(lexical,/本地按关键词/);
});

test('未配置本机索引的自然问题分支仍解释否定时间与同义词限制',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,lexical_match:'natural_question'}}).join('\n');
 for(const expected of ['本地按关键词','不会改用云编码','按问题寻找字面候选','否定、相对时间和同义改写','候选不等于答案'])assert.ok(notes.includes(expected),expected);
 assert.doesNotMatch(notes,/已发送到.*编码服务|已结合 Memory 语义/);
});

import { recallSupportNote } from '../src/service/portable.ts';
test('candidate support labels never claim relevance, absence, or answerability',()=>{
 const row={ref:'event:synthetic',kind:'event',text:'synthetic'};
 assert.equal(recallSupportNote(row),'');
 assert.match(recallSupportNote({...row,retrieval_support:{kind:'semantic_candidate'}}),/可能无关/);
 assert.match(recallSupportNote({...row,retrieval_support:{kind:'semantic_candidate',protected_anchors:1}}),/不表示整个事件不含/);
 assert.match(recallSupportNote({...row,retrieval_support:{protected:true}}),/仍未判断是否足以回答/);
 assert.match(recallSupportNote({...row,retrieval_support:{limited:true}}),/超出.*预算/);
 assert.match(recallSupportNote({...row,retrieval_support:{kind:'lexical_candidate'}}),/仍需核对/);
});

test('verification resource limits are not mislabeled as only a text cap',()=>{
 const notes=recallCoverageNotes({results:[],truncated:true,coverage:{local_event_semantic:true,scopes:[{scope:'personal',state:'ready',verification_limited:true}]}}).join('\n');
 assert.match(notes,/时间或资源门槛限制/);assert.match(notes,/部分候选尚未核验/);
});


test('独立词法覆盖经共享入口显示旧库、长问句退回与候选预算',()=>{
 const note=(status:string,truncated=false)=>recallCoverageNotes({results:[],coverage:{local_event_semantic:true,scopes:[{scope:'personal',state:'ready',lexical:{status,candidates_truncated:truncated}}]}}).join('\n');
 assert.match(note('available'),/独立寻找完整编号/);
 assert.match(note('legacy_unavailable'),/显式刷新.*复用已有向量/);
 assert.match(note('limited_query'),/完整问题仍用于语义检索/);
 assert.match(note('unsupported_query'),/没有受支持的词法词项/);
 assert.match(note('available',true),/未展示的精确来源仍可能存在/);
 assert.doesNotMatch(note('available'),/得分最高/);
 const direct=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,lexical:{status:'limited_query'}}}).join('\n');
 assert.match(direct,/没有截短或改写问题/);
});

test('编码器离线词法降级不宣称语义成功或只因未配置',()=>{
 const scope={scope:'personal',state:'degraded',query_encoding:'unavailable',fusion:'lexical_only',semantic:{status:'unavailable',reason:'local_provider_unavailable'},lexical:{status:'available'}};
 const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:false,cloud_fallback:false,scopes:[scope]}}).join('\n');
 assert.match(notes,/本机编码服务不可用/);assert.match(notes,/没有语义分数/);assert.match(notes,/不会改用云编码/);assert.match(notes,/未命中不表示/);
 assert.doesNotMatch(notes,/已直接参与本机语义|尚未配置|已发送到|结合语义排序/);
 const limited=recallCoverageNotes({results:[],coverage:{fusion:'lexical_only',lexical:{status:'limited_query'}}}).join('\n');
 assert.match(limited,/未能执行这两条候选路线/);assert.doesNotMatch(limited,/完整问题仍用于语义/);
 const unsupported=recallCoverageNotes({results:[],coverage:{scopes:[{...scope,lexical:{status:'unsupported_query'}}]}}).join('\n');
 assert.match(unsupported,/没有可用的候选路线/);
 const mixed=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,scopes:[scope,{scope:'project:ok',state:'ready',query_encoding:'cache',lexical:{status:'available'}}]}}).join('\n');
 assert.match(mixed,/本机编码服务不可用/);assert.match(mixed,/已缓存/);assert.match(mixed,/独立寻找完整编号/);
});

test('附件不完整覆盖区分扫描上限和不可读原件，不承诺换关键词解决',()=>{
 for(const extra of [{},{cloud_fallback:false},{local_event_semantic:true},{fusion:'lexical_only'}]) {
  const notes=recallCoverageNotes({results:[],truncated:true,coverage:{...extra,attachments:{status:'partial',limited:true,skipped:2,semantic:false}}}).join('\n');
  for(const text of ['UTF-8','PDF','OCR','换关键词不会解除','原件缺失','未命中不能当作附件内容不存在'])assert.ok(notes.includes(text),text+': '+notes);
 }
});


test('附件语义覆盖只根据实际成功标记声明，空命中不冒充没有运行',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,attachments:{semantic:true}}}).join('\n');
 for(const text of ['至少一个独立批准范围','附件正文已参与本机语义检索','安全 UTF-8','不含 PDF 提取或 OCR','独立来源，不当作上传者原话','父事件须在当前范围可见、未撤回且未被修订替换'])assert.ok(notes.includes(text),text);
 assert.doesNotMatch(notes,/未使用附件语义向量|不含附件语义向量|附件正文仅在本机按词法/);
 for(const semantic of [false,undefined]) {
  const lexical=recallCoverageNotes({results:[{ref:'attachment:synthetic',kind:'attachment',text:'合成附件'}],coverage:{local_event_semantic:true,attachments:{semantic}}}).join('\n');
  assert.match(lexical,/本次附件正文仅在本机按词法/);
  assert.match(lexical,/未使用附件语义向量/);
  assert.doesNotMatch(lexical,/附件正文已参与本机语义检索/);
 }
});
test('附件语义成功不掩盖词法扫描上限与跳过原件，也不宣称全部范围覆盖',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,attachments:{semantic:true,status:'partial',limited:true,skipped:2}}}).join('\n');
 for(const text of ['至少一个独立批准范围','换关键词不会解除','原件缺失','未命中不能当作附件内容不存在'])assert.ok(notes.includes(text),text);
 assert.doesNotMatch(notes,/全部附件.*已.*检索|未使用附件语义向量/);
});


test('附件-only成功不冒称普通对话已批准，旧普通对话成功文案保留',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,attachments:{semantic:true},lexical:{status:'available'}}}).join('\n');
 assert.match(notes,/本次独立批准的资料已参与本机语义检索/);
 assert.match(notes,/每项资料展示一处来源已核验/);
 assert.match(notes,/已批准资料的词法索引已独立/);
 assert.doesNotMatch(notes,/普通对话已直接参与|普通对话词法索引已独立|每条原话展示/);
 const old=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,lexical:{status:'available'}}}).join('\n');
 assert.match(old,/普通对话已直接参与本机语义检索/);
 assert.match(old,/普通对话词法索引已独立寻找完整编号和汉字词组候选，并结合语义排序/);
});


test('scope附件索引覆盖不能被顶部完整词法扫描覆盖掩盖',()=>{
 for(const attachment of [{status:'partial'},{status:'ready',limited:true},{status:'ready',skipped:1}]) {
  const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,attachments:{status:'ready',semantic:true,limited:false,skipped:0},scopes:[{scope:'project:approved',state:'ready',attachments:{...attachment,semantic:true,mode:'bounded_local_semantic_index',projection:'attachment.safe_utf8/v1',files_examined:2,documents:1,bytes_examined:64}}]}}).join('\n');
  assert.match(notes,/附件语义索引来源覆盖不完整/);
  assert.match(notes,/至少一个独立批准范围/);
  assert.match(notes,/不能据此推断其他范围已获批准或全部覆盖/);
  if('limited' in attachment)assert.match(notes,/换关键词不会解除这项来源检查上限/);
  if('skipped' in attachment)assert.match(notes,/未进入语义索引/);
 }
});
test('仅scope的部分附件覆盖也显示，编码降级不冒称附件语义成功',()=>{
 for(const extra of [{},{cloud_fallback:false},{local_event_semantic:true},{fusion:'lexical_only'}]) {
  const notes=recallCoverageNotes({results:[],truncated:true,coverage:{...extra,scopes:[{scope:'project:approved',state:'degraded',attachments:{status:'partial',semantic:false,limited:true,skipped:1}}]}}).join('\n');
  for(const text of ['附件语义索引来源覆盖不完整','来源检查达到文件数量或总字节上限','未进入语义索引','未使用附件语义向量'])assert.ok(notes.includes(text),text);
  assert.doesNotMatch(notes,/附件正文已参与本机语义检索/);
 }
});
test('未批准范围的来源计数不被当作批准、语义成功或已检查覆盖',()=>{
 for(const status of ['not_approved','not_requested']) {
  const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,attachments:{status:'ready',semantic:false},scopes:[{scope:'project:unapproved',state:'ready',attachments:{status,semantic:false,files_examined:100,documents:50,skipped:7,limited:true}}]}}).join('\n');
  assert.doesNotMatch(notes,/附件正文已参与|附件语义索引来源覆盖不完整|附件扫描达到|来源检查达到|未进入语义索引/);
 }
 const countsOnly=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,scopes:[{scope:'project:approved',state:'ready',attachments:{status:'ready',semantic:false,files_examined:100,documents:50}}]}}).join('\n');
 assert.doesNotMatch(countsOnly,/附件正文已参与本机语义检索/);
});
test('scope明确成功可以报告附件通道，混合未批准范围不扩大覆盖声明',()=>{
 const notes=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,scopes:[{scope:'project:approved',state:'ready',attachments:{status:'partial',semantic:true,skipped:1}},{scope:'project:unapproved',state:'ready',attachments:{status:'not_approved',semantic:false,documents:0}}]}}).join('\n');
 assert.match(notes,/至少一个独立批准范围的附件正文已参与本机语义检索/);
 assert.match(notes,/本次独立批准的资料已参与本机语义检索/);
 assert.match(notes,/附件语义索引来源覆盖不完整/);
 assert.doesNotMatch(notes,/普通对话已直接参与|全部范围.*已覆盖/);
});

test('紧凑索引元数据在所有召回分支解释预算，不误报正文或来源截断',()=>{
 for(const extra of [{},{semantic_search:'available'},{cloud_fallback:false},{local_event_semantic:true},{fusion:'lexical_only'},{local_event_semantic:true,scopes:[{scope:'personal',state:'degraded',fusion:'lexical_only'}]}]) {
  const notes=recallCoverageNotes({results:[],truncated:true,coverage:{...extra,budget_limited:true,presentation:'compact_index_metadata_full_source_via_fetch',budget_omitted_by_kind:{event:0,attachment:0,memory:0}}}).join('\n');
  assert.match(notes,/紧凑索引元数据，保留片段正文和来源定位信息/);
  assert.match(notes,/完整来源可按引用读取/);
  assert.doesNotMatch(notes,/预算省略|文字预算|片段或核验达到|正文.*截断|来源.*移除/);
 }
});
test('按种类如实说明预算省略数量，不承诺每类候选必保留',()=>{
 for(const extra of [{},{cloud_fallback:false},{local_event_semantic:true},{fusion:'lexical_only'}]) {
  const notes=recallCoverageNotes({results:[],truncated:true,coverage:{...extra,budget_limited:true,presentation:'compact_index_metadata_full_source_via_fetch',budget_omitted_by_kind:{event:2,attachment:1,memory:3},candidate_budget_policy:'protected_anchor_tier_then_source_kind_diversity'}}).join('\n');
  assert.match(notes,/预算省略了普通对话 2 项、附件 1 项、记忆 3 项候选/);
  assert.match(notes,/不保证每种来源类型均有保留/);
  assert.match(notes,/未展示的来源不代表不存在/);
 }
});
test('预算省略计数忽略未知种类和非法数字，未知展示方式不冒充紧凑元数据',()=>{
 for(const invalid of [undefined,null,-1,0,1.5,NaN,Infinity,Number.MAX_SAFE_INTEGER+1,'2']) {
  const omitted={event:invalid,attachment:2,memory:invalid,unknown:9} as unknown as NonNullable<SearchPage['coverage']>['budget_omitted_by_kind'];
  const notes=recallCoverageNotes({results:[],coverage:{budget_limited:true,presentation:'unknown',budget_omitted_by_kind:omitted}}).join('\n');
  assert.match(notes,/预算省略了附件 2 项候选/);
  assert.doesNotMatch(notes,/普通对话 .*项|记忆 .*项|NaN|Infinity|unknown|紧凑索引元数据/);
 }
 const notes=recallCoverageNotes({results:[],truncated:true,coverage:{budget_limited:true,budget_omitted_by_kind:{event:-1,attachment:NaN,memory:0}}}).join('\n');
 assert.match(notes,/本次返回受到预算限制/);
 assert.doesNotMatch(notes,/预算省略|NaN|文字预算/);
});
