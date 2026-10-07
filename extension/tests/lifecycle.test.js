import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
import '../site-adapters.js';
const contentSource = await readFile(new URL('../content.js',import.meta.url),'utf8');
const manifest = JSON.parse(await readFile(new URL('../manifest.json',import.meta.url),'utf8'));
function setup(url='https://chatgpt.com/c/first'){
  let listener, interval, resets=0, inserts=0;
  const handlers = {}, messages=[];
  const extensionId='abcdefghijklmnopabcdefghijklmnop';
  const parsed=new URL(url);
  const location={origin:parsed.origin,pathname:parsed.pathname,get href(){return this.origin+this.pathname;}};
  let editor={};
  let bindHook=async()=>{};
  const window={addEventListener:(kind,fn)=>handlers[kind]=fn};window.top=window;
  class Adapter{find(){if(!editor)throw new Error("输入框不可用");return editor;}reset(){resets++;return[];}status(){return[];}insert(){inserts++;return{status:'draft'};}}
  const chrome={runtime:{id:extensionId,getURL:(path)=>`chrome-extension://${extensionId}/${path}`,onMessage:{addListener:(fn)=>listener=fn},sendMessage:async(msg)=>{messages.push(msg);await bindHook(msg);return{ok:true,result:{route:msg.route,nonce:'a'.repeat(48),session_ref:globalThis.RecallCardSites.forUrl(msg.route).id+':'+msg.token}};}}};
  vm.runInNewContext(contentSource,{RecallCardSites:globalThis.RecallCardSites,window,document:{},location,crypto:webcrypto,chrome,RecallCardComposerAdapter:Adapter,setInterval:(fn)=>interval=fn});
  const sender={id:extensionId,url:chrome.runtime.getURL('background.js')};
  const send=(msg,who=sender)=>new Promise(resolve=>{const pending=listener(msg,who,resolve);if(!pending)resolve(undefined);});
  return{send,location,interval,handlers,messages,delayBind:(fn)=>{bindHook=fn;},replaceEditor:(node={})=>{editor=node;},counts:()=>({resets,inserts})};
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
test('manifest 权限仅列出明确主机，没有泛域、脚本注入权限或对外消息接口',()=>{
  assert.equal(manifest.manifest_version,3);
  assert.deepEqual(manifest.permissions,['nativeMessaging','storage']);
  assert.deepEqual(manifest.host_permissions,['https://chatgpt.com/*','https://chat.qwen.ai/*','https://chat.z.ai/*']);
  assert.deepEqual(manifest.content_scripts[0].matches,manifest.host_permissions);
  assert.deepEqual(manifest.content_scripts[0].js,['site-adapters.js','composer-adapter.js','content.js']);
  assert.equal(manifest.content_scripts[0].all_frames,false);
  assert.equal(manifest.content_scripts[0].world,'ISOLATED');
  assert.equal(manifest.externally_connectable,undefined);
  assert.equal(manifest.web_accessible_resources,undefined);
  assert.match(manifest.content_security_policy.extension_pages,/connect-src 'none'/);
});
test('生产代码没有发送按钮、Enter 提交、网页输出采集或网络调用',async()=>{
  const names=['site-adapters.js','content.js','composer-adapter.js','broker.js','background.js','popup.js','protocol.js'];
  for(const name of names){
    const source=await readFile(new URL('../'+name,import.meta.url),'utf8');
    for(const forbidden of [/\.click\s*\(/u,/\.submit\s*\(/u,/requestSubmit/u,/KeyboardEvent/u,/send-button/u,/data-message-author-role/u,/\beval\s*\(/u,/new Function/u,/\bfetch\s*\(/u,/XMLHttpRequest/u,/postMessage\s*\(/u,/innerHTML\s*=/u]) assert.doesNotMatch(source,forbidden,name);
  }
});

test('同 URL 更换 composer 也会轮换 nonce 并拒绝旧 capsule',async()=>{
  const h=setup();const first=(await h.send({kind:'describe'})).result;
  h.replaceEditor();h.interval();
  const stale=await h.send({kind:'insert',...first,capsule:{id:'r_old',...first,text:'合成旧内容'}});
  assert.equal(stale.ok,false);assert.equal(h.counts().inserts,0);
  const second=(await h.send({kind:'describe'})).result;
  assert.notEqual(second.session_ref,first.session_ref);assert.equal(h.counts().resets,1);
});
for(const origin of ['https://chat.qwen.ai','https://chat.z.ai']){
  test(`${origin} 入口先要求显式重置，替换输入框或变路由后再次要求确认`,async()=>{
    const h=setup(origin+'/');
    const first=await h.send({kind:'describe'});assert.equal(first.ok,false);assert.match(first.error,/重置/);assert.equal(h.messages.length,0);
    const bound=(await h.send({kind:'reset'})).result;assert.ok(bound.nonce);
    assert.equal((await h.send({kind:'check',...bound})).ok,true);
    h.replaceEditor();h.interval();
    assert.equal((await h.send({kind:'check',...bound})).ok,false);
    assert.equal((await h.send({kind:'describe'})).ok,false);
    const rebound=(await h.send({kind:'reset'})).result;assert.notEqual(rebound.session_ref,bound.session_ref);
    h.location.pathname='/c/11111111-1111-4111-8111-111111111111';h.interval();
    assert.equal((await h.send({kind:'check',...rebound})).ok,false);assert.equal((await h.send({kind:'describe'})).ok,false);
  });
}
test('未支持路径和编辑器消失时不会创建绑定或执行旧插入',async()=>{
  const h=setup();const first=(await h.send({kind:'describe'})).result;
  h.replaceEditor(null);h.interval();assert.equal((await h.send({kind:'check',...first})).ok,false);
  assert.equal((await h.send({kind:'reset'})).ok,false);assert.equal(h.counts().inserts,0);
  for(const url of ['https://chat.qwen.ai/login','https://chat.z.ai/share/demo','https://chat.deepseek.com/']){
    const invalid=setup(url);assert.equal((await invalid.send({kind:'describe'})).ok,false);assert.equal((await invalid.send({kind:'reset'})).ok,false);assert.equal(invalid.messages.length,0);
  }
});


test('绑定返回前同 URL 输入框被替换时，不复活迟到的旧绑定',async()=>{
  const h=setup();let release;h.delayBind(()=>new Promise(resolve=>release=resolve));
  const pending=h.send({kind:'describe'});h.replaceEditor();h.interval();release();
  const stale=await pending;assert.equal(stale.ok,false);assert.match(stale.error,/绑定期间/);
  h.delayBind(async()=>{});assert.equal((await h.send({kind:'describe'})).ok,true);
});
test('实验平台同 URL 同节点无法自行判断换会话，用户重置确实使旧请求失效',async()=>{
  for(const origin of ['https://chat.qwen.ai','https://chat.z.ai']){
    const h=setup(origin+'/');const first=(await h.send({kind:'reset'})).result;
    const next=(await h.send({kind:'reset'})).result;
    assert.notEqual(next.session_ref,first.session_ref);
    assert.equal((await h.send({kind:'insert',...first,capsule:{id:'old',...first,text:'旧资料'}})).ok,false);
    assert.equal(h.counts().inserts,0);
  }
});
