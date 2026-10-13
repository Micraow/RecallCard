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
