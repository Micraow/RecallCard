// 原创内存 DOM，真实 content listener；没有浏览器、网络或私人会话。
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {webcrypto,createHash} from 'node:crypto';
import './fixtures/composer.mjs';
import '../conversation-format.js';
const source=await readFile(new URL('../content.js',import.meta.url),'utf8');
const nonce='a'.repeat(48), packet={id:'p_synthetic',text:'完整合成资料\n尾部不能丢失'};
packet.hash=createHash('sha256').update(packet.text).digest('hex');
const wrapped=`[RecallCard ${nonce}:${packet.id}]\n${packet.text}\n[/RecallCard ${nonce}:${packet.id}]`;
const frame='RECALLCARD/2 T_abcdefghijklmnop\n{"action":"search","query":"合成问题"}\nEND RECALLCARD';
const row=(role,id,text,complete=true)=>({role,id,text,complete,getAttribute(key){return key==='data-message-id'?id:key==='data-message-author-role'?role:'';},querySelector(){return null;}});
async function harness(rows, options={}) {
  let listener,clock=10000,overlayAction;
  const calls=[];
  const location={href:'https://chatgpt.com/c/synthetic'},window={addEventListener(){}};window.top=window;
  const doc={visibilityState:'visible',body:options.overlay?{}:null};
  class Overlay { constructor(_doc,action){overlayAction=action;}update(value){this.state=value;}reset(){}error(){} }
  class Composer{find(){return null;}text(){return '';}reset(){return [];}status(){return [];}draftIds(){return [];} }
  class Conversation{nodes(){return rows;}protocolText(node){return node.text;}streaming(){return false;}completed(node){return node.complete;}}
  const chrome={runtime:{id:'test-extension',getURL:path=>'chrome-extension://test-extension/'+path,onMessage:{addListener:fn=>listener=fn},sendMessage:async m=>{calls.push(m);if(options.send){const value=await options.send(m);if(value!==undefined)return value;}return{ok:true,result:{route:m.route,nonce,session_ref:'chatgpt:'+m.token,...(options.overlay?{text_relay:{phase:'waiting_reply'}}:{})}};}}};
  class Clock extends Date {static now(){return clock;}}
  vm.runInNewContext(source,{window,document:doc,location,chrome,crypto:webcrypto,TextEncoder,Date:Clock,RecallCardSites:globalThis.RecallCardSites,RecallCardComposerAdapter:Composer,RecallCardConversationAdapter:Conversation,RecallCardConversationFormat:globalThis.RecallCardConversationFormat,RecallCardRelayOverlay:options.overlay?Overlay:undefined,setInterval(){}});
  const send=m=>new Promise(resolve=>listener(m,{id:'test-extension',url:chrome.runtime.getURL('background.js')},resolve));
  const binding=(await send({kind:'describe'})).result;clock+=2000;
  return {snapshot:()=>send({kind:'text_snapshot',...binding,packet}),rows,action:(...args)=>overlayAction(...args),calls};
}
test('完整唯一用户包才证明发送，整条完成助手回复关联在发送之后',async()=>{
  const h=await harness([row('user','u1','问题\n'+wrapped),row('assistant','a1',frame)]);
  const r=await h.snapshot();assert.equal(r.ok,true);assert.equal(r.result.sent.message_id,'u1');assert.equal(r.result.sent.packet_hash,packet.hash);assert.equal(r.result.assistant.text,frame);
});
test('有相同标记但正文尾部缺失或变更不能冒充完整发送',async()=>{
  for(const text of [wrapped.replace('尾部不能丢失','尾部'),wrapped.replace('资料','材料'),wrapped+wrapped]){
    const h=await harness([row('user','u1',text),row('assistant','a1',frame)]);const r=await h.snapshot();assert.equal(r.result.sent,null);assert.equal(r.result.assistant,null);
  }
});
test('重复发送包、重复消息ID、未知角色和流式助手不能执行',async()=>{
  for(const rows of [[row('user','u1',wrapped),row('user','u2',wrapped),row('assistant','a1',frame)],[row('user','same',wrapped),row('assistant','same',frame)],[row('user','u1',wrapped),row(null,'a1',frame)],[row('user','u1',wrapped),row('assistant','a1',frame,false)]]){
    const r=await (await harness(rows)).snapshot();assert.equal(r.ok,true);assert.ok(!r.result.assistant);
  }
});
test('历史助手在发送之前、引用无法无损恢复时回退，不剥离可执行片段',async()=>{
  for(const rows of [[row('assistant','a0',frame),row('user','u1',wrapped)],[row('user','u1',wrapped),row('assistant','a1',null)]]){
    const r=await (await harness(rows)).snapshot();assert.equal(r.result.assistant,null);
  }
});

test('真实content暂停不先bind排队，并阻止取消后迟到的普通操作继续',async()=>{
  let release,waiting=false,block=false;
  const gate=new Promise(resolve=>release=resolve);
  const h=await harness([],{overlay:true,send:async message=>{
    if(message.kind==='bind'&&block){waiting=true;await gate;}
    if(message.kind==='text_control')return{ok:true,result:{text_relay:{phase:'paused'}}};
  }});
  block=true;
  const pending=h.action('execute',{text:frame});
  await new Promise(resolve=>setTimeout(resolve,0));assert.equal(waiting,true);
  const before=h.calls.length;
  await h.action('pause');
  assert.equal(h.calls.length,before+1);assert.equal(h.calls.at(-1).kind,'text_control');assert.equal(h.calls.at(-1).command,'pause');
  release();await assert.rejects(pending,/暂停|变化/);
  assert.equal(h.calls.some(m=>m.kind==='text_control'&&m.command==='execute'),false);
});
