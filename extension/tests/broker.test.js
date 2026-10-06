import test from 'node:test';
import assert from 'node:assert/strict';
import { Broker, bindSession, verifyContent, verifyPopup, HOST } from '../broker.js';
const extensionId = 'abcdefghijklmnopabcdefghijklmnop';
const popupUrl = `chrome-extension://${extensionId}/popup.html`;
const popup = {id:extensionId, url:popupUrl};
const token = '11111111-1111-4111-8111-111111111111';
function harness() {
  let state = bindSession(null, {route:'https://chatgpt.com/c/demo', token, documentId:'doc-1'});
  let time = 10000;
  const tab = {id:1, active:true, url:'https://chatgpt.com/c/demo'};
  const calls = [];
  const api = { id:extensionId, popupUrl,
    getTab:async()=>tab, load:async()=>structuredClone(state), save:async(_,s)=>{state=structuredClone(s);}, now:()=>time+=2000,
    native:async(host,request)=>{calls.push({host,request}); return {ok:true,result:{stable_text:'用户偏好中文',bootstrap_version:'v1'}};},
    content:async()=>({ok:true,result:{status:'draft'}}),
  };
  const broker = new Broker(api);
  const message = (kind, rest={}) => ({kind,tabId:1, nonce:state.nonce,session_ref:state.session_ref,...rest});
  const execute = (id='r_demo') => {
    const request = {protocol:'recallcard.action/1',request_id:id,nonce:state.nonce,session_ref:state.session_ref,action:'search',arguments:{query:'决定'}};
    return message('execute',{text:'```recallcard-action\n'+JSON.stringify(request)+'\n```'});
  };
  return {broker,api,tab,calls,message,execute,state:()=>state,clear:()=>{state=null;}};
}
test('只有精确扩展弹窗可以请求 native；content script 不能升级为调用者', async()=>{
  const h=harness();
  for(const sender of [{}, {...popup,id:'other'}, {...popup,url:'https://chatgpt.com/'}, {...popup,tab:{id:1}}]) await assert.rejects(h.broker.handle(h.execute(),sender));
  assert.equal(h.calls.length,0);
  assert.throws(()=>verifyPopup({...popup,url:popupUrl+'?spoof'},extensionId,popupUrl));
});
test('content sender 的 origin、tab、顶层 frame、document、route 全部检查',()=>{
  const sender={id:extensionId,origin:'https://chatgpt.com',url:'https://chatgpt.com/c/demo',tab:{id:1,url:'https://chatgpt.com/c/demo'},frameId:0,documentId:'doc-1'};
  assert.equal(verifyContent(sender,extensionId,sender.url,sender.url),sender.url);
  for(const changes of [{origin:'https://evil.test'},{frameId:1},{documentId:null},{id:'other'},{tab:null},{url:'https://chatgpt.com/c/other'}]) assert.throws(()=>verifyContent({...sender,...changes},extensionId,sender.url,sender.url));
});
test('相同文档维持 nonce；路由、token、文档变化均轮换',()=>{
  const args={route:'https://chatgpt.com/c/demo',token,documentId:'doc-1'};
  const a=bindSession(null,args);
  assert.equal(bindSession(a,args).nonce,a.nonce);
  for(const changes of [{route:'https://chatgpt.com/c/other'},{token:'22222222-2222-4222-8222-222222222222'},{documentId:'doc-2'}]) assert.notEqual(bindSession(a,{...args,...changes}).nonce,a.nonce);
});
test('并发重复请求仅调用 native 一次，且指定固定 host',async()=>{
  const h=harness(); const msg=h.execute();
  const results=await Promise.allSettled([h.broker.handle(msg,popup),h.broker.handle(msg,popup)]);
  assert.equal(results.filter(x=>x.status==='fulfilled').length,1);
  assert.equal(h.calls.length,1); assert.equal(h.calls[0].host,HOST);
  assert.equal(h.calls[0].request.action,'search');
});
test('host 失败或 worker 重启之后，同一个请求仍不会重放',async()=>{
  const h=harness(); const msg=h.execute(); let calls=0;
  h.api.native=async()=>{calls++;throw new Error('测试断线');};
  await assert.rejects(h.broker.handle(msg,popup),/断线/);
  await assert.rejects(new Broker(h.api).handle(msg,popup),/已处理/);
  assert.equal(calls,1);
});
test('标签切换和导航拒绝请求；飞行中的过期结果丢弃',async()=>{
  const h=harness(); h.tab.active=false;
  await assert.rejects(h.broker.handle(h.execute(),popup),/标签页/); assert.equal(h.calls.length,0);
  h.tab.active=true;
  h.api.native=async()=>{h.tab.url='https://chatgpt.com/c/other';return{ok:true,result:{text:'不应显示'}};};
  await assert.rejects(h.broker.handle(h.execute(),popup),/过期/);
  assert.equal(h.state().preview,null);
});
test('Bootstrap 为会话快照，重开弹窗复用；不重复注入已发送版本',async()=>{
  const h=harness();
  const first=await h.broker.handle(h.message('bootstrap'),popup);
  await h.broker.handle(h.message('insert'),popup);
  await assert.rejects(h.broker.handle(h.message('insert'),popup),/不会重复/);
  await h.broker.handle(h.message('delivered'),popup);
  const again=await new Broker(h.api).handle(h.message('bootstrap'),popup);
  assert.equal(again.preview.text,first.preview.text);
  assert.equal(again.preview.delivery,'user_confirmed_sent');
  assert.equal(h.calls.length,3);
  assert.equal(new Set(h.calls.map(x=>x.request.request_id)).size,3);
});
test('未知 host 响应不会准备任何草稿',async()=>{
  for(const response of [{ok:true},{ok:'yes'},null,{ok:false,error:'未授权范围'}]) {
    const h=harness();h.api.native=async()=>response;
    await assert.rejects(h.broker.handle(h.execute(),popup));
    assert.equal(h.state().preview,null);
  }
});
test('原路径往返或重载导致文档绑定失效时，不调用本机桥',async()=>{
  const h=harness();h.api.content=async()=>({ok:false,error:'旧文档'});
  await assert.rejects(h.broker.handle(h.execute(),popup),/文档或会话/);
  assert.equal(h.calls.length,0);
});
test('content bind 不返回本机预览数据',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
  const result=await h.broker.handle({kind:'bind',route:h.state().route,token},{id:extensionId,origin:'https://chatgpt.com',url:h.tab.url,tab:h.tab,frameId:0,documentId:'doc-1'});
  assert.equal(result.preview,undefined);assert.ok(result.nonce);
});
test('模型再次要求 bootstrap 不能偷偷替换已经固定的会话快照',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
  const message=h.execute('r_second_bootstrap');message.text=message.text.replace('"action":"search","arguments":{"query":"决定"}', '"action":"bootstrap","arguments":{}');
  await assert.rejects(h.broker.handle(message,popup),/固定 Bootstrap/);
  assert.equal(h.calls.length,1);
});
test('插入前重新执行同一只读请求，使用当前会话的新 request_id',async()=>{
  const h=harness();const content=[];
  h.api.content=async(_,message)=>{content.push(message);return{ok:true,result:{status:'draft'}};};
  await h.broker.handle(h.execute(),popup);
  await h.broker.handle(h.message('insert'),popup);
  assert.equal(h.calls.length,2);
  assert.notEqual(h.calls[0].request.request_id,h.calls[1].request.request_id);
  for(const key of ['protocol','nonce','session_ref','action','arguments']) assert.deepEqual(h.calls[0].request[key],h.calls[1].request[key]);
  const inserted=content.filter(x=>x.kind==='insert');assert.equal(inserted.length,1);
  assert.equal(inserted[0].capsule.source_request,undefined);
  assert.equal(inserted[0].capsule.result_fingerprint,undefined);
});
test('search 结果被遗忘或缩小授权后，插入前废弃缓存且不写草稿',async()=>{
  const h=harness();const content=[];
  h.api.content=async(_,message)=>{content.push(message);return{ok:true,result:{}};};
  await h.broker.handle(h.execute(),popup);
  h.api.native=async()=>({ok:true,result:{results:[]}});
  await assert.rejects(h.broker.handle(h.message('insert'),popup),/旧预览已废弃/);
  assert.equal(h.state().preview,null);assert.equal(h.state().bootstrap,null);
  assert.equal(content.filter(x=>x.kind==='insert').length,0);
});
test('read 与 sources 都在插入前核验，新证据内容变化需要重新审核',async()=>{
  for(const kind of ['read','sources']){
    const h=harness();const message=h.execute();
    message.text=message.text.replace('"action":"search","arguments":{"query":"决定"}',`"action":"${kind}","arguments":{"refs":["event:evt_demo"]}`);
    await h.broker.handle(message,popup);
    h.api.native=async()=>({ok:true,result:{text:'已纠正的原文'}});
    await assert.rejects(h.broker.handle(h.message('insert'),popup),/资料.*变化/);
    assert.equal(h.state().preview,null);
  }
});
test('重用 Bootstrap 检查最新版本，变化后必须重新准备与审核',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
  h.api.native=async()=>({ok:true,result:{stable_text:'已撤销原偏好',bootstrap_version:'v2'}});
  await assert.rejects(h.broker.handle(h.message('bootstrap'),popup),/版本.*变化/);
  assert.equal(h.state().bootstrap,null);assert.equal(h.state().preview,null);
  const fresh=await h.broker.handle(h.message('bootstrap'),popup);
  assert.match(fresh.preview.text,/已撤销原偏好/);assert.equal(fresh.preview.delivery,'prepared');
});
test('Bootstrap 仅动态 coverage 改变时保留原稳定快照',async()=>{
  const h=harness();const first=await h.broker.handle(h.message('bootstrap'),popup);
  h.api.native=async()=>({ok:true,result:{coverage:{captured_events:42},bootstrap_version:'v1',stable_text:'用户偏好中文'}});
  const current=await h.broker.handle(h.message('bootstrap'),popup);
  assert.equal(current.preview.text,first.preview.text);assert.equal(current.preview.id,first.preview.id);
});
test('Bootstrap 即使版本相同，稳定资料或来源改变仍会废弃',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
  h.api.native=async()=>({ok:true,result:{bootstrap_version:'v1',stable_text:'改变的内容'}});
  await assert.rejects(h.broker.handle(h.message('bootstrap'),popup),/旧预览已废弃/);
  assert.equal(h.state().preview,null);
});
test('核验授权被拒绝、host断线或缺版本都不允许继续插入旧资料',async()=>{
  for(const mode of ['denied','offline','missing-version']){
    const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
    h.api.native=async()=>{
      if(mode==='offline')throw new Error('测试断线');
      return mode==='denied'?{ok:false,error:'范围已撤销'}:{ok:true,result:{stable_text:'没有版本'}};
    };
    await assert.rejects(h.broker.handle(h.message('insert'),popup),/旧预览已废弃/);
    assert.equal(h.state().preview,null);assert.equal(h.state().bootstrap,null);
  }
});
test('缓存 Bootstrap 已在草稿里，版本变化会尝试只移除旧的自有块',async()=>{
  const h=harness();const content=[];
  h.api.content=async(_,message)=>{content.push(message);return{ok:true,result:{}};};
  await h.broker.handle(h.message('bootstrap'),popup);
  await h.broker.handle(h.message('insert'),popup);
  h.api.native=async()=>({ok:true,result:{bootstrap_version:'v2',stable_text:'新的资料'}});
  await assert.rejects(h.broker.handle(h.message('bootstrap'),popup),/旧预览已废弃/);
  assert.equal(content.filter(x=>x.kind==='remove').length,1);
  assert.equal(content.filter(x=>x.kind==='insert').length,1);
  assert.equal(h.state().preview,null);
});
test('已修改的旧草稿不能安全移除时必须提示人工删除，不覆盖内容',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);await h.broker.handle(h.message('insert'),popup);
  h.api.native=async()=>({ok:true,result:{bootstrap_version:'v2',stable_text:'新的资料'}});
  h.api.content=async(_,message)=>message.kind==='remove'?{ok:false,error:'用户已改动'}:{ok:true,result:{}};
  await assert.rejects(h.broker.handle(h.message('bootstrap'),popup),/先人工删除/);
  assert.equal(h.state().preview,null);
});
test('后台重启后的旧预览也必须新鲜度核验；对象键顺序不制造假变化',async()=>{
  const h=harness();await h.broker.handle(h.execute(),popup);
  h.api.native=async()=>({ok:true,result:{bootstrap_version:'v1',stable_text:'用户偏好中文'}});
  const result=await new Broker(h.api).handle(h.message('insert'),popup);
  assert.equal(result.preview.delivery,'draft');
});
test('核验期间切换路由，迟到结果既不注入也不保留旧预览',async()=>{
  const h=harness();await h.broker.handle(h.execute(),popup);
  h.api.native=async()=>{h.tab.url='https://chatgpt.com/c/other';return{ok:true,result:{stable_text:'用户偏好中文',bootstrap_version:'v1'}};};
  await assert.rejects(h.broker.handle(h.message('insert'),popup),/旧预览已废弃/);
  assert.equal(h.state().preview,null);
});
test('仅 bootstrap_version 变化、文字相同，也必须废弃旧预览',async()=>{
  const h=harness();await h.broker.handle(h.message('bootstrap'),popup);
  h.api.native=async()=>({ok:true,result:{bootstrap_version:'v2',stable_text:'用户偏好中文'}});
  await assert.rejects(h.broker.handle(h.message('bootstrap'),popup),/旧预览已废弃/);
  assert.equal(h.state().preview,null);assert.equal(h.state().bootstrap,null);
});
test('插入核验 request_id 被预留且失败不重放；重新准备必须由新动作触发',async()=>{
  const h=harness();await h.broker.handle(h.execute(),popup);
  const used=h.state().used.length;let checked;
  h.api.native=async(_,request)=>{checked=request;throw new Error('测试超时');};
  await assert.rejects(h.broker.handle(h.message('insert'),popup),/旧预览已废弃/);
  assert.equal(h.state().used.length,used+1);assert.ok(h.state().used.includes(checked.request_id));
  await assert.rejects(h.broker.handle(h.message('insert'),popup),/先准备/);
});
