import test from 'node:test';
import assert from 'node:assert/strict';
import {installationIdentity, INSTALLATION_KEY} from '../installation.js';
import {Broker,bindSession} from '../broker.js';
function storage(){const values={};return{values,get:async key=>({[key]:values[key]}),set:async next=>Object.assign(values,next)};}
test('安装编号在同profile重启保持，另一个profile或清除重装绝不继承',async()=>{
  const a=storage(),b=storage();const identity=installationIdentity(a);
  const [first,concurrent]=await Promise.all([identity.get(),identity.get()]);assert.equal(first,concurrent);assert.equal(await installationIdentity(a).get(),first);
  assert.notEqual(await installationIdentity(b).get(),first);delete a.values[INSTALLATION_KEY];identity.reset();assert.notEqual(await identity.get(),first);
});
test('损坏安装编号不自动退回无身份授权',async()=>{const a=storage();a.values[INSTALLATION_KEY]='forged';await assert.rejects(installationIdentity(a).get(),/损坏/);});
test('worker只用本地安装编号，忽略页面伪造；编号改变使在途结果失效',async()=>{
  const ext='abcdefghijklmnopabcdefghijklmnop',route='https://chatgpt.com/c/synthetic',token='11111111-1111-4111-8111-111111111111';
  let installation='22222222-2222-4222-8222-222222222222';
  let state=bindSession(null,{route,token,documentId:'doc',installationId:installation});const calls=[];
  const api={id:ext,popupUrl:`chrome-extension://${ext}/popup.html`,installationId:async()=>installation,getTab:async()=>({id:1,active:true,url:route}),load:async()=>structuredClone(state),save:async(_,next)=>{state=structuredClone(next);},now:()=>10000,content:async()=>({ok:true,result:{status:'current'}}),native:async(_,request)=>{calls.push(request);installation='33333333-3333-4333-8333-333333333333';return{ok:true,result:{bootstrap_version:'v1',stable_text:'合成'}};}};
  const broker=new Broker(api);const bound=await broker.handle({kind:'bind',route,token,installation_id:'forged'},{id:ext,origin:'https://chatgpt.com',url:route,tab:{id:1,url:route},frameId:0,documentId:'doc'});
  assert.equal(bound.installation_id,installation);
  await assert.rejects(broker.handle({kind:'bootstrap',tabId:1,nonce:bound.nonce,session_ref:bound.session_ref},{id:ext,url:api.popupUrl}),/安装绑定/);
  assert.equal(calls[0].installation_id,'22222222-2222-4222-8222-222222222222');assert.equal(state.preview,null);
});
test('配对请求只从扩展弹窗发起，固定安装身份且请求本身不给数据权限',async()=>{
  const ext='abcdefghijklmnopabcdefghijklmnop',route='https://chatgpt.com/c/synthetic',installation='22222222-2222-4222-8222-222222222222';
  let state=bindSession(null,{route,token:'11111111-1111-4111-8111-111111111111',documentId:'doc',installationId:installation});const calls=[];
  const api={id:ext,popupUrl:`chrome-extension://${ext}/popup.html`,installationId:async()=>installation,getTab:async()=>({id:1,active:true,url:route}),load:async()=>structuredClone(state),save:async(_,next)=>{state=structuredClone(next);},now:()=>10000,content:async()=>({ok:true,result:{status:'current'}}),native:async(_,request)=>{calls.push(request);return{ok:true,result:{status:'pending',request_id:'pair_synthetic',data_access:false}};}};
  const broker=new Broker(api),message={kind:'pair',tabId:1,nonce:state.nonce,session_ref:state.session_ref,scope:'secret'};
  await assert.rejects(broker.handle(message,{id:ext,url:route,tab:{id:1}}));assert.equal(calls.length,0);
  const response=await broker.handle(message,{id:ext,url:api.popupUrl});assert.equal(response.data_access,false);assert.equal(calls[0].action,'request_pairing');assert.deepEqual(calls[0].arguments,{});assert.equal(calls[0].installation_id,installation);assert.equal(state.preview,null);
});
