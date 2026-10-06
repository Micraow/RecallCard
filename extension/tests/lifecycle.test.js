import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
const contentSource = await readFile(new URL('../content.js',import.meta.url),'utf8');
const manifest = JSON.parse(await readFile(new URL('../manifest.json',import.meta.url),'utf8'));
function setup(){
  let listener, interval, resets=0, inserts=0;
  const handlers = {}, messages=[];
  const extensionId='abcdefghijklmnopabcdefghijklmnop';
  const location={origin:'https://chatgpt.com',pathname:'/c/first'};
  const window={addEventListener:(kind,fn)=>handlers[kind]=fn};window.top=window;
  class Adapter{reset(){resets++;return[];}status(){return[];}insert(){inserts++;return{status:'draft'};}}
  const chrome={runtime:{id:extensionId,getURL:(path)=>`chrome-extension://${extensionId}/${path}`,onMessage:{addListener:(fn)=>listener=fn},sendMessage:async(msg)=>{messages.push(msg);return{ok:true,result:{route:msg.route,nonce:'a'.repeat(48),session_ref:'chatgpt:'+msg.token}};}}};
  vm.runInNewContext(contentSource,{window,document:{},location,crypto:webcrypto,chrome,RecallCardComposerAdapter:Adapter,setInterval:(fn)=>interval=fn});
  const sender={id:extensionId,url:chrome.runtime.getURL('background.js')};
  const send=(msg,who=sender)=>new Promise(resolve=>{const pending=listener(msg,who,resolve);if(!pending)resolve(undefined);});
  return{send,location,interval,handlers,messages,counts:()=>({resets,inserts})};
}
test('真正 content 脚本的 SPA 路由变化使旧 nonce 操作失效',async()=>{
  const h=setup();const first=(await h.send({kind:'describe'})).result;
  h.location.pathname='/c/second';h.interval();
  const stale=await h.send({kind:'insert',...first,capsule:{id:'r_old',...first,text:'旧结果'}});
  assert.equal(stale.ok,false);assert.equal(h.counts().inserts,0);
  const second=(await h.send({kind:'describe'})).result;
  assert.notEqual(second.session_ref,first.session_ref);assert.equal(h.counts().resets,1);
  assert.ok(h.messages.every(x=>x.kind==='bind'));
});
test('content 拒绝网页/其他扩展 sender，页面隐藏后不会继续操作',async()=>{
  const h=setup();assert.equal(await h.send({kind:'describe'},{id:'other',url:'https://chatgpt.com'}),undefined);
  const first=(await h.send({kind:'describe'})).result;h.handlers.pagehide();
  assert.equal((await h.send({kind:'check',...first})).ok,false);
  assert.equal(h.counts().inserts,0);
});
test('manifest 权限限定主站，没有脚本注入权限或对外消息接口',()=>{
  assert.equal(manifest.manifest_version,3);
  assert.deepEqual(manifest.permissions,['nativeMessaging','storage']);
  assert.deepEqual(manifest.host_permissions,['https://chatgpt.com/*']);
  assert.deepEqual(manifest.content_scripts[0].matches,['https://chatgpt.com/*']);
  assert.equal(manifest.content_scripts[0].all_frames,false);
  assert.equal(manifest.content_scripts[0].world,'ISOLATED');
  assert.equal(manifest.externally_connectable,undefined);
  assert.equal(manifest.web_accessible_resources,undefined);
  assert.match(manifest.content_security_policy.extension_pages,/connect-src 'none'/);
});
test('生产代码没有发送按钮、Enter 提交、网页输出采集或网络调用',async()=>{
  const names=['content.js','composer-adapter.js','broker.js','background.js','popup.js','protocol.js'];
  for(const name of names){
    const source=await readFile(new URL('../'+name,import.meta.url),'utf8');
    for(const forbidden of [/\.click\s*\(/u,/\.submit\s*\(/u,/requestSubmit/u,/KeyboardEvent/u,/send-button/u,/data-message-author-role/u,/\beval\s*\(/u,/new Function/u,/\bfetch\s*\(/u,/XMLHttpRequest/u,/postMessage\s*\(/u,/innerHTML\s*=/u]) assert.doesNotMatch(source,forbidden,name);
  }
});
