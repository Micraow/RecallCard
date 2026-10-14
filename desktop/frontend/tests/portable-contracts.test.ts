import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { revalidateAndCopy, utf8Bytes, validatePortableContext, type PortableContext } from "../src/service/portable.ts";

function context(text = "合成上下文🙂"): PortableContext {
  return {text,scope:"personal",selected_refs:["event:synthetic"], selected_count:1,included_count:1,
    pending_refs:[],partial:false,truncated:false,records:[{ref:"event:synthetic",kind:"event",role:"assistant",evidence:"AssistantSuggestion",text:"合成回答",conversation_title:"合成会话",truncated:false,content_retained:true}]};
}
test("复制前重验完整文字，资料改变时先展示新预览而不写剪贴板", async () => {
  const writes:string[]=[];
  const before=context("开头".repeat(200)+"旧结尾");
  const after=context("开头".repeat(200)+"新结尾");
  const first=await revalidateAndCopy(before,async()=>after,async text=>{writes.push(text);},()=>true);
  assert.equal(first?.copied,false); assert.equal(first?.current,after); assert.deepEqual(writes,[]);
  const second=await revalidateAndCopy(after,async()=>after,async text=>{writes.push(text);},()=>true);
  assert.equal(second?.copied,true); assert.deepEqual(writes,[after.text]);
});
test("关闭或切换页面后迟到结果不触碰剪贴板，读源错误不返回成功", async () => {
  let finish:(value:PortableContext)=>void=()=>{}; let active=true, writes=0;
  const pending=revalidateAndCopy(context(),()=>new Promise(resolve=>{finish=resolve;}),async()=>{writes++;},()=>active);
  active=false; finish(context()); assert.equal(await pending,null); assert.equal(writes,0);
  await assert.rejects(()=>revalidateAndCopy(context(),async()=>{throw new Error("来源已遗忘");},async()=>{writes++;},()=>true),/已遗忘/);
  await assert.rejects(()=>revalidateAndCopy(context(),async()=>context(),async()=>{throw new Error("剪贴板失败");},()=>true),/剪贴板失败/);
});
test("跨空间、换引用与不完整预览合同不能复制，长度按 UTF-8 而非 token 声称", () => {
  assert.equal(utf8Bytes("中文🙂"),10);
  assert.equal(validatePortableContext(context(),"personal",["event:synthetic"]).selected_count,1);
  for(const bad of [{...context(),scope:"project:other"},{...context(),selected_refs:["event:other"]},{...context(),included_count:2},{...context(),records:[{...context().records[0],ref:"event:other"}]}])
    assert.throws(()=>validatePortableContext(bad,"personal",["event:synthetic"]),/不一致/);
});
test("真实预览组件同时显示 AI 来源、长度、节选及未复制引用，恶意正文仅纯文本", async () => {
  const bundle=await build({stdin:{contents:`import {createElement} from 'react'; import {renderToStaticMarkup} from 'react-dom/server'; import {PortablePreview} from './src/components/PortableContext.tsx'; export const render=preview=>renderToStaticMarkup(createElement(PortablePreview,{preview}));`,resolveDir:new URL("..",import.meta.url).pathname,loader:"tsx"},bundle:true,platform:"node",format:"esm",write:false,loader:{".css":"empty"},define:{"process.env.NODE_ENV":'"production"'},banner:{js:`import {createRequire} from 'node:module'; const require=createRequire(${JSON.stringify(import.meta.url)});`}});
  const {render}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
  const item=context('<script>synthetic()</script>'); item.truncated=true; item.pending_refs=["event:omitted"];item.records[0].truncated=true;
  const html=render(item);
  assert.match(html,/AI 回答，非用户事实/);assert.match(html,/字节 UTF-8/);assert.match(html,/event:omitted/);assert.match(html,/正文仅节选/);assert.match(html,/&lt;script&gt;/);assert.doesNotMatch(html,/<script>/);
});

test("真实服务桥按当前 session/scope 准备、保存与定向取消，不允许前端确认时换正文", async () => {
  const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
  const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
  const calls:any[]=[];
  const service=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return command==='prepare_selected_context'?context():{};}},{session_id:'portable_session'},'personal');
  await service.prepareContext(['event:synthetic'],'当前问题');
  await service.previewAiReply('合成普通回答');
  await service.cancelAiReply('synthetic_preview');
  await service.confirmAiReply('synthetic_preview');
  assert.deepEqual(calls.map(call=>call.command),['prepare_selected_context','preview_ai_reply','cancel_ai_reply','confirm_ai_reply']);
  for(const call of calls) {assert.equal(call.args.sessionId,'portable_session');assert.equal(call.args.scope,'personal');}
  assert.equal(calls[0].args.budgetTokens,32768);
  assert.deepEqual(calls[3].args,{sessionId:'portable_session',scope:'personal',previewId:'synthetic_preview'});
});

import { recallChoices, currentRecallQuery } from '../src/service/portable.ts';
import { recallCoverageNotes } from '../src/service/setup.ts';
import type { SearchPage } from '../src/service/types.ts';
test('问题变化或未返回有效检索不能沿用旧来源，空结果仍是本次真实检索', () => {
  const page:SearchPage={results:[{ref:'event:first',kind:'event',text:'合成来源'}]};
  assert.equal(currentRecallQuery('问题二','问题一',page),false);
  assert.equal(currentRecallQuery('问题二','问题二',null),false);
  assert.equal(currentRecallQuery('   ','',page),false);
  assert.equal(currentRecallQuery(' 问题一 ','问题一',page),true);
  assert.equal(currentRecallQuery('问题二','问题二',{results:[]}),true);
});
test('保留第九条候选供换选，同会话来源仍有独立摘录、角色和唯一引用', () => {
  const page:SearchPage={results:Array.from({length:9},(_,i)=>({ref:`event:synthetic_${i}`,kind:'event',text:`独立合成摘录${i}`,conversation_title:'相同合成会话',role:i%2?'assistant':'user'}))};
  page.results.push(page.results[0]);
  const choices=recallChoices(page);
  assert.equal(choices.length,9);assert.equal(choices[8].ref,'event:synthetic_8');
  assert.equal(choices[1].role,'assistant');assert.equal(choices[1].snippet,'独立合成摘录1');
  assert.deepEqual(recallChoices({results:[]}),[]);
});
test('检索覆盖分别说明本地、已批准云、退化、未知和预算，不声称原始聊天已全量语义化', () => {
  for(const [coverage,pattern] of [
    [{semantic_search:'available',semantic_query_encoding:'local_encoded'},/本机编码/],
    [{semantic_search:'available',semantic_query_encoding:'cloud_encoded'},/独立云批准编码/],
    [{semantic_search:'unavailable',semantic_error:'worker_failed'},/worker_failed.*保留词法结果/],
    [{semantic_search:'unavailable',semantic_error:'offline_config_invalid'},/offline_config_invalid/],
    [{semantic_search:'unavailable'},/词法匹配/],
    [undefined,/未提供语义检索状态/],
  ] as const) {
    const notes=recallCoverageNotes({results:[],coverage,truncated:true}).join('\n');
    assert.match(notes,pattern);assert.match(notes,/原始聊天仍通过词法查找/);assert.match(notes,/预算上限/);
  }
});

test('问题入口显式选择问题词法模式，普通关键词搜索不改变批准或模式', async () => {
  const bundle=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
  const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
  const calls:any[]=[];
  const service=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return {results:[]};}},{session_id:'synthetic_question'},'personal');
  await service.search('本地备份');await service.searchQuestion('我之前怎么做本地备份');
  assert.deepEqual(calls.map(c=>c.command),['search_records','search_records']);
  assert.equal(calls[0].args.questionMode,undefined);assert.equal(calls[1].args.questionMode,true);
  for(const call of calls){assert.equal(call.args.scope,'personal');assert.equal(call.args.sessionId,'synthetic_question');}
  assert.match(recallCoverageNotes({results:[],coverage:{lexical_match:'natural_question',semantic_search:'unavailable'}}).join('\n'),/候选不等于答案/);
});

test('附件正文不会被复制包冒充为所属用户原话', () => {
  const choices = recallChoices({results:[
    {ref:'attachment:evt_synthetic/'+'a'.repeat(64),kind:'attachment',text:'合成文件正文',role:'user'},
    {ref:'event:evt_synthetic',kind:'event',text:'合成消息',role:'user'},
  ]});
  assert.equal(choices.length,2);
  assert.equal(choices[0].kind,'attachment'); assert.equal(choices[0].role,null);
  assert.equal(choices[1].role,'user');
});

function attachmentContext(): PortableContext {
  const parent = 'event:evt_'+'a'.repeat(64), hash = 'b'.repeat(64);
  const ref = 'attachment:'+parent.slice(6)+'/'+hash;
  return {...context('附件正文，非用户原话\n'+ref+'\n合成原件'), selected_refs:[ref], records:[{
    ...context().records[0],ref,kind:'attachment',role:null,evidence:'Attachment/Content',
    source_refs:[parent],object_sha256:hash,
  }]};
}
test('附件预览校验独立证据、父消息和原件摘要，不能伪装为消息', () => {
  const good=attachmentContext();
  assert.equal(validatePortableContext(good,'personal',good.selected_refs),good);
  for(const fields of [{role:'user'},{evidence:'UserExplicit'},{kind:'event'},
    {source_refs:['event:foreign']},{object_sha256:'c'.repeat(64)},{source_refs:undefined}]) {
    const bad={...good,records:[{...good.records[0],...fields}]};
    assert.throws(()=>validatePortableContext(bad,'personal',good.selected_refs),/不一致/);
  }
});
test('附件复制前原件变化或撤回均不写剪贴板，稳定原件只复制明确预览', async () => {
  const before=attachmentContext(), writes:string[]=[];
  const changed={...before,text:before.text+'新内容'};
  assert.equal((await revalidateAndCopy(before,async()=>changed,async text=>{writes.push(text);},()=>true))?.copied,false);
  await assert.rejects(()=>revalidateAndCopy(before,async()=>{throw new Error('附件原件无法核验');},async text=>{writes.push(text);},()=>true),/无法核验/);
  assert.deepEqual(writes,[]);
  assert.equal((await revalidateAndCopy(before,async()=>before,async text=>{writes.push(text);},()=>true))?.copied,true);
  assert.deepEqual(writes,[before.text]);
});
test('真实附件复制预览优先附件标签，不把上传者角色当正文作者', async () => {
  const bundle=await build({stdin:{contents:`import {createElement} from 'react'; import {renderToStaticMarkup} from 'react-dom/server'; import {PortablePreview} from './src/components/PortableContext.tsx'; export const render=preview=>renderToStaticMarkup(createElement(PortablePreview,{preview}));`,resolveDir:new URL('..',import.meta.url).pathname,loader:'tsx'},bundle:true,platform:'node',format:'esm',write:false,loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"'},banner:{js:`import {createRequire} from 'node:module'; const require=createRequire(${JSON.stringify(import.meta.url)});`}});
  const {render}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
  const item=attachmentContext(); item.records[0].role='user';
  const html=render(item);
  assert.match(html,/附件正文，非用户原话/); assert.doesNotMatch(html,/>用户原话</);
});


test('真实上下文入口说明可选本机和云来源及独立批准，条件提示不冒称当前已外发',async()=>{
 const bundle=await build({stdin:{contents:`import {createElement} from 'react'; import {renderToStaticMarkup} from 'react-dom/server'; import {PortableContextDialog} from './src/components/PortableContext.tsx'; export const render=service=>renderToStaticMarkup(createElement(PortableContextDialog,{service,initialGoal:'合成问题',close:()=>{}}));`,resolveDir:new URL('..',import.meta.url).pathname,loader:'tsx'},bundle:true,platform:'node',format:'esm',write:false,loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"'},banner:{js:`import {createRequire} from 'node:module'; const require=createRequire(${JSON.stringify(import.meta.url)});`}});
 const {render}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
 for(const scope of ['personal','project:synthetic']) {
  let calls=0;
  const html=render({scope,searchQuestion:()=>{calls++;throw new Error('本合同只渲染条件说明，不执行检索');}});
  for(const text of ['当前范围明确选择的本机或云来源索引','普通对话、附件与新问题分别独立批准','未配置来源索引时只做本地关键词查找','不会自动回退到云编码','若已选择云提供器并批准新问题','未缓存的问题可能发送到该云接口','是否实际发送以本次检索覆盖为准'])assert.ok(html.includes(text),text);
  assert.doesNotMatch(html,/问题仅使用.*本机普通对话索引|此入口不会改用云编码|问题已发送|问题已经发送|已按独立云批准编码/);
  assert.equal(calls,0);
 }
});
