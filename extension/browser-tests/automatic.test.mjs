// Chromium 合成页面链路，真实扩展源码；不安装扩展、不登录网站、不调用云模型。
import {before,after,test} from 'node:test';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {Broker} from '../broker.js';
let browser;
before(async()=>{browser=await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH?{executablePath:process.env.RECALLCARD_CHROMIUM_PATH}:{});});
after(async()=>{await browser?.close();});
const EXT='abcdefghijklmnopabcdefghijklmnop';
const initial='<div data-message-author-role="user" data-message-id="u1"><p>合成用户决定</p></div><div data-message-author-role="assistant" data-message-id="a1" data-is-streaming="false"><p>合成回答</p></div><textarea id="mobile-composer-prompt">原有草稿</textarea>';
async function harness(authorized=true){
  const page=await browser.newPage();let state;let now=10000;
  const nativeCalls=[],saved=[],inserts=[];
  let grant=authorized?{capture:true,recall:true,provider_disclosure:true,permission_revision:1,platform:'chatgpt'}:null;
  await page.route('**/*',route=>route.fulfill({contentType:'text/html; charset=utf-8',body:'<!doctype html><meta charset="UTF-8"><title>合成对话</title>'+initial}));
  await page.goto('https://chatgpt.com/c/synthetic');
  assert.equal(await page.evaluate(()=>document.characterSet),'UTF-8','合成页面必须按UTF-8解码原始中文');
  const broker=new Broker({id:EXT,installationId:async()=>'11111111-1111-4111-8111-111111111111',popupUrl:`chrome-extension://${EXT}/popup.html`,getTab:async()=>({id:1,active:true,url:page.url()}),load:async()=>state?structuredClone(state):null,save:async(_,value)=>{state=structuredClone(value);},now:()=>now+=2000,
    content:async(_,message)=>{if(message.kind==='insert')inserts.push(message);return page.evaluate(({message,EXT})=>new Promise(resolve=>window.contentListener(message,{id:EXT,url:`chrome-extension://${EXT}/background.js`},resolve)),{message,EXT});},
    native:async(_,request)=>{nativeCalls.push(request);if(request.action==='connection')return{ok:true,result:{connection_id:'c'.repeat(64),capture_enabled:!!grant?.capture,automation:grant,state:grant?'reachable':'revoked'}};if(request.action==='automatic_capture'){saved.push(request.arguments.conversation);return{ok:true,result:{events_added:request.arguments.conversation.messages.length,events_seen:request.arguments.conversation.messages.length}};}if(request.action==='authorized_read')return{ok:true,result:request.arguments.action==='bootstrap'?{bootstrap_version:'v1',stable_text:'合成稳定背景',refs:[]}:{results:[{ref:'event:evt_synthetic',content:'合成召回'}]}};throw new Error('意外Native动作');}
  });
  await page.exposeFunction('worker',message=>broker.handle(message,{id:EXT,origin:'https://chatgpt.com',url:page.url(),tab:{id:1,url:page.url()},frameId:0,documentId:'synthetic-doc'}).then(result=>({ok:true,result}),error=>({ok:false,error:error.message})));
  for(const name of ['site-adapters.js','composer-adapter.js','conversation-format.js','conversation-adapters.js'])await page.addScriptTag({path:fileURLToPath(new URL('../'+name,import.meta.url))});
  await page.evaluate(EXT=>{
    window.syntheticClock=10000;Date.now=()=>window.syntheticClock;
    window.setInterval=callback=>{window.ticks??=[];window.ticks.push(callback);return window.ticks.length;};
    window.sendEvents=[];for(const name of ['click','keydown','submit'])document.addEventListener(name,()=>window.sendEvents.push(name));
    window.chrome={runtime:{id:EXT,getURL:path=>`chrome-extension://${EXT}/`+path,onMessage:{addListener:listener=>window.contentListener=listener},sendMessage:message=>window.worker(message)}};
  },EXT);
  await page.addScriptTag({path:fileURLToPath(new URL('../content.js',import.meta.url))});
  const tick=async()=>{await page.evaluate(()=>new Promise(resolve=>setTimeout(resolve,0)));await page.evaluate(async()=>{window.syntheticClock+=2000;await window.ticks.at(-1)();});};
  return{page,tick,nativeCalls,saved,inserts,state:()=>state,setGrant:value=>{grant=value;}};
}
test('一次授权后真实DOM完成消息自动保存/准备草稿，零click/Enter/submit且重复tick不重注入',async()=>{
  const h=await harness();try{await h.tick();assert.equal(h.saved.length,1);assert.equal(h.saved[0].messages[0].text,'合成用户决定');assert.equal(h.inserts.length,1);
    const draft=await h.page.locator('textarea').inputValue();assert.ok(draft.startsWith('原有草稿'));assert.match(draft,/合成稳定背景/);
    await h.tick();assert.equal(h.saved.length,1);assert.equal(h.inserts.length,1);assert.deepEqual(await h.page.evaluate(()=>sendEvents),[]);
  }finally{await h.page.close();}
});
test('无授权、流式和身份不明消息都不会落盘；撤权安全移除自有块保留用户草稿',async()=>{
  const off=await harness(false);try{await off.tick();assert.equal(off.saved.length,0);assert.equal(off.inserts.length,0);}finally{await off.page.close();}
  const h=await harness();try{
    await h.page.evaluate(()=>{const stop=document.createElement('button');stop.setAttribute('aria-label','Stop generating');document.body.append(stop);});await h.tick();assert.equal(h.saved.length,0);
    await h.page.evaluate(()=>{document.querySelector('button').remove();const weak=document.createElement('div');weak.setAttribute('data-message-author-role','assistant');weak.textContent='无可靠身份的回答';document.body.append(weak);});await h.tick();await h.tick();assert.equal(h.saved.length,1);assert.equal(h.saved[0].messages.length,2);
    h.setGrant(null);await h.tick();assert.equal(await h.page.locator('textarea').inputValue(),'原有草稿');assert.equal(h.state().preview,null);assert.deepEqual(await h.page.evaluate(()=>sendEvents),[]);
  }finally{await h.page.close();}
});
test('上下文块不回流为新用户事实；模型完成的只读请求自动填入下一份草稿',async()=>{
  const h=await harness();try{await h.tick();const draft=await h.page.locator('textarea').inputValue();const state=h.state();
    const request='```recallcard-action\n'+JSON.stringify({protocol:'recallcard.action/1',request_id:'r_search',nonce:state.nonce,session_ref:state.session_ref,action:'search',arguments:{query:'合成'}})+'\n```';
    await h.page.evaluate(({draft,request})=>{
      document.querySelector('textarea').value='接下来的用户草稿';
      for(const [role,id,text] of [['user','u2',draft],['assistant','a2',request]]){const node=document.createElement('div');node.setAttribute('data-message-author-role',role);node.setAttribute('data-message-id',id);if(role==='assistant')node.setAttribute('data-is-streaming','false');node.textContent=text;document.body.append(node);}
    },{draft,request});await h.tick();await h.tick();
    assert.equal(h.inserts.length,2);assert.match(await h.page.locator('textarea').inputValue(),/合成召回/);assert.ok(h.saved.at(-1).messages.every(m=>!m.text.includes('[RecallCard ')));assert.equal(h.saved.at(-1).messages.find(m=>m.id==='u2').text,'原有草稿');
    assert.equal(h.state().bootstrap.delivery,'observed_sent');assert.deepEqual(await h.page.evaluate(()=>sendEvents),[]);
  }finally{await h.page.close();}
});
test('SPA换会话自动重绑，旧nonce请求不能查询，持久授权不需要再次点击',async()=>{
  const h=await harness();try{await h.tick();const old=h.state();await h.page.evaluate(()=>history.pushState({},'', '/c/next-synthetic'));await h.tick();assert.equal(h.inserts.length,1);assert.equal(h.state().automatic_status,'waiting_for_conversation_identity');await h.page.evaluate(()=>{for(const node of document.querySelectorAll('[data-message-id]'))node.setAttribute('data-message-id','next-'+node.getAttribute('data-message-id'));});await h.tick();await h.tick();assert.notEqual(h.state().nonce,old.nonce);assert.equal(h.inserts.length,2);assert.equal(h.saved.at(-1).source.conversation_id,'next-synthetic');assert.deepEqual(await h.page.evaluate(()=>sendEvents),[]);
  }finally{await h.page.close();}
});
