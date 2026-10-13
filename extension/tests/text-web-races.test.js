// 真实 content + Broker 的原创合成竞态回归；仅内存 DOM/native，不使用真实站点或对话。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
import { Broker } from '../broker.js';
import '../site-adapters.js';
import '../conversation-format.js';
const source=await readFile(new URL('../content.js',import.meta.url),'utf8');
const EXT='abcdefghijklmnopabcdefghijklmnop';
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
async function harness(){
 let state=null,listener,overlay,clock=10000,nativeHook=null;const messages=[],calls=[],rows=[],intervals=[];
 const location={href:'https://chatgpt.com/c/synthetic-review'},window={addEventListener(){}};window.top=window;
 const doc={body:{},visibilityState:'visible'},editor={};
 class Composer{find(){return editor;}text(){return '合成未改草稿';}reset(){return [];}status(){return [];}draftIds(){return [];}}
 class Conversation{nodes(){return rows;}protocolText(n){return n.text;}streaming(){return false;}completed(n){return n.complete;}}
 class Overlay{constructor(_doc,action){overlay=this;this.action=action;}update(s){this.state=s;}reset(){this.state=null;}error(e){this.lastError=e;}}
 const sender=()=>({id:EXT,origin:'https://chatgpt.com',url:location.href,tab:{id:1,url:location.href},frameId:0,documentId:'review-document'});
 const api={id:EXT,popupUrl:`chrome-extension://${EXT}/popup.html`,getTab:async()=>({id:1,active:true,url:location.href}),load:async()=>structuredClone(state),save:async(_,s)=>state=structuredClone(s),now:()=>clock,
  content:async(_,m)=>new Promise(resolve=>listener(m,{id:EXT,url:`chrome-extension://${EXT}/background.js`},resolve)),
  native:async(_,request)=>{calls.push(request);if(nativeHook){const custom=await nativeHook(request);if(custom!==undefined)return custom;}
   if(request.action==='connection')return {ok:true,result:{connection_id:'c'.repeat(64),capture_enabled:false,read_scopes:['synthetic'],state:'reachable',automation:{recall:true,provider_disclosure:true,platform:'chatgpt',permission_revision:1}}};
   if(request.action==='authorized_read'&&request.arguments.action==='bootstrap')return {ok:true,result:{bootstrap_version:'review-v1',stable_text:'原创合成背景',refs:[],scopes:['synthetic']}};
   if(request.action==='authorized_read'&&request.arguments.action==='recall_search')return {ok:true,result:{status:'complete',results:[],coverage:{scopes:[{scope:'synthetic'}]}}};
   throw Error('Unexpected native '+request.action);
  }};
 const broker=new Broker(api);
 const chrome={runtime:{id:EXT,getURL:p=>`chrome-extension://${EXT}/`+p,onMessage:{addListener:fn=>listener=fn},sendMessage:async m=>{messages.push(m);try{return {ok:true,result:await broker.handle(m,sender())};}catch(e){return {ok:false,error:e.message};}}}};
 class Clock extends Date{static now(){return clock;}}
 vm.runInNewContext(source,{window,document:doc,location,chrome,crypto:webcrypto,TextEncoder,Date:Clock,RecallCardSites:globalThis.RecallCardSites,RecallCardComposerAdapter:Composer,RecallCardConversationAdapter:Conversation,RecallCardConversationFormat:globalThis.RecallCardConversationFormat,RecallCardRelayOverlay:Overlay,setInterval:fn=>intervals.push(fn)});
 return {action:(...a)=>overlay.action(...a),state:()=>state,messages,calls,rows,intervals,clock:()=>clock+=2000,setHook:h=>nativeHook=h,overlay,broker,api,sender};
}
test('开始后首次用户消息不依赖轮询也保持原接力绑定',async()=>{
 const h=await harness();await h.action('text_start');const initial=h.state().nonce;
 h.rows.push({text:'新用户消息',getAttribute(k){return k==='data-message-id'?'u1':k==='data-message-author-role'?'user':null;}});
 h.clock();h.intervals[0]();await h.action('inspect');assert.equal(h.state().nonce,initial);assert.ok(h.state().text_relay);
});
for(const command of ['cancel','pause'])test('真实content '+command+' 绕开阻塞读取且不返回迟到复制资料',async()=>{
 const h=await harness();await h.action('text_start');const entered=deferred(),gate=deferred();
 h.setHook(async r=>{if(r.action==='authorized_read'&&r.arguments.action==='bootstrap'){entered.resolve();await gate.promise;}});
 const copying=h.action('copy').then(v=>({ok:true,value:v}),e=>({ok:false,error:e.message}));await entered.promise;
 const stopping=h.action(command).then(v=>({ok:true,value:v}),e=>({ok:false,error:e.message}));
 for(let i=0;i<20;i++)await new Promise(setImmediate);
 const early=command==='cancel'?h.state().text_relay.engine.cancelled:h.state().text_relay.engine.paused;
 gate.resolve();const [copy,stop]=await Promise.all([copying,stopping]);
 assert.equal(early,true,'stop must persist before native is released');assert.equal(copy.ok,false,'copy must not deliver old payload');assert.equal(stop.ok,true);
});
test('简洁接力中拒绝旧弹窗与旧协议写状态入口',async()=>{
 const h=await harness();await h.action('text_start');const s=h.state(),before=structuredClone(s),nativeCount=h.calls.length;
 for(const kind of ['bootstrap','execute','insert','remove','delivered','capture','capture_check','conversation','pair','capture_preview','capture_save'])await assert.rejects(h.broker.handle({kind,tabId:1,nonce:s.nonce,session_ref:s.session_ref,route:s.route},{id:EXT,url:`chrome-extension://${EXT}/popup.html`}));
 for(const command of ['copy','execute','bootstrap','pause','resume','disable','enable','delivered'])await assert.rejects(h.broker.handle({kind:'relay_control',command,nonce:s.nonce,session_ref:s.session_ref,route:s.route},h.sender()));
 assert.deepEqual(h.state(),before);assert.equal(h.calls.length,nativeCount);
});
test('首次绑定在途取消后迟到启动不能创建新接力',async()=>{
 const h=await harness(),entered=deferred(),gate=deferred();
 const lock=h.broker.serial(1,()=>{entered.resolve();return gate.promise;});await entered.promise;
 const starting=h.action('text_start').then(v=>({ok:true}),e=>({ok:false,error:e.message}));
 const stopping=await h.action('text_cancel').then(v=>({ok:true}),e=>({ok:false,error:e.message}));
 gate.resolve();await lock;const started=await starting;
 assert.equal(started.ok,false);assert.equal(h.state()?.text_relay,undefined);assert.equal(h.calls.length,0);
});
for(const boundary of ['connection','bootstrap'])test('启动 '+boundary+' 在途取消后不能安装或返回迟到结果',async()=>{
 const h=await harness(),entered=deferred(),gate=deferred();let first=true;
 h.setHook(async r=>{if(first&&(boundary==='connection'?r.action==='connection':r.action==='authorized_read'&&r.arguments.action==='bootstrap')){first=false;entered.resolve();await gate.promise;}});
 const starting=h.action('text_start').then(v=>({ok:true}),e=>({ok:false,error:e.message}));await entered.promise;
 const stopping=await h.action('text_cancel').then(v=>({ok:true}),e=>({ok:false,error:e.message}));
 if(boundary==='bootstrap')assert.equal(h.state().text_relay.engine.cancelled,true);
 gate.resolve();const started=await starting;
 assert.equal(started.ok,false);assert.ok(!h.state().text_relay||h.state().text_relay.engine.cancelled);
 assert.equal(h.state().text_relay?.engine.packet??null,null);
});

