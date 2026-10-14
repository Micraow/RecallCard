import assert from 'node:assert/strict';
import test from 'node:test';
import {validLocalEventSettings,recallCoverageNotes,type LocalEventSetup} from '../src/service/setup.ts';
const settings:LocalEventSetup={endpoint:'http://127.0.0.1:46123/v1/embeddings',model:'synthetic',revision:'fake-v1',tokenizer:'test-only',dimensions:2,query_instruction:'',document_instruction:'',events_approved:false,query_approved:false,chunk_bytes:1024,disk_cap_mib:256};
test('普通对话本机索引需独立明确范围和版本，拒绝云、凭据地址与越界预算',()=>{
 assert.equal(validLocalEventSettings(settings),false);
 const approved={...settings,events_approved:true};assert.equal(validLocalEventSettings(approved),true);
 for(const change of [{endpoint:'https://example.invalid/v1/embeddings'},{endpoint:'http://localhost:1234/v1/embeddings'},{endpoint:'http://key@127.0.0.1:1234/v1/embeddings'},{tokenizer:''},{revision:''},{chunk_bytes:255},{disk_cap_mib:513}])assert.equal(validLocalEventSettings({...approved,...change}),false);
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
