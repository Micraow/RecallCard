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
 assert.match(notes,/普通对话已直接参与本机语义检索/);assert.match(notes,/没有云回退/);assert.match(notes,/得分最高的一处片段/);assert.match(notes,/本机编码服务/);assert.doesNotMatch(notes,/只覆盖已建索引的记忆/);
});

test('共享入口缓存与未覆盖如实显示，不把缓存说成新外发或回到云配置',()=>{
 const cached=recallCoverageNotes({results:[],coverage:{local_event_semantic:true,cloud_fallback:false,scopes:[{scope:'personal',state:'ready',query_encoding:'cache'}]}}).join('\n');
 assert.match(cached,/已缓存/);assert.doesNotMatch(cached,/已发送/);
 const missing=recallCoverageNotes({results:[],coverage:{cloud_fallback:false,event_search:'local_only'}}).join('\n');assert.match(missing,/未覆盖/);assert.match(missing,/不会改用云编码/);
 const lexical=recallCoverageNotes({results:[],coverage:{cloud_fallback:false}}).join('\n');assert.match(lexical,/本地按关键词/);
});
