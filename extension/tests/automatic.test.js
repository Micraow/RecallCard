import test from 'node:test';
import assert from 'node:assert/strict';
import { Broker, bindSession } from '../broker.js';
import '../conversation-format.js';
const format = globalThis.RecallCardConversationFormat;
const extensionId = 'abcdefghijklmnopabcdefghijklmnop';
const token = '11111111-1111-4111-8111-111111111111';
async function harness(grant = { capture:true, recall:true, provider_disclosure:true, permission_revision:1, platform:'chatgpt' }) {
  let state = bindSession(null,{ route:'https://chatgpt.com/c/synthetic', token, documentId:'doc-1' });
  let time = 10000;
  const calls=[], content=[];
  const tab={id:1,active:true,url:state.route};
  const sender={id:extensionId,origin:'https://chatgpt.com',url:tab.url,tab,frameId:0,documentId:'doc-1'};
  const conversation=await format.seal({ schema:format.SCHEMA,capture_id:'synthetic',captured_at:'2026-01-01T00:00:00Z',title:'合成会话',source:{platform:'chatgpt',conversation_id:'synthetic',url:tab.url},coverage:{extent:'visible_only',complete:false,reason:'合成稳定DOM',warnings:[]},messages:[{id:'u1',role:'user',text:'合成决定',occurred_at:null},{id:'a1',role:'assistant',text:'合成回答',occurred_at:null}],metadata:{} });
  const snapshot={status:'stable',conversation,requests:[],draft_ids:[]};
  const api={id:extensionId,popupUrl:`chrome-extension://${extensionId}/popup.html`,getTab:async()=>tab,load:async()=>structuredClone(state),save:async(_,next)=>{state=structuredClone(next);},now:()=>time+=2000,
    content:async(_,message)=>{content.push(message);if(message.kind==='automatic_snapshot')return{ok:true,result:structuredClone(snapshot)};if(message.kind==='insert')snapshot.draft_ids=[message.capsule.id];return{ok:true,result:{status:'current'}};},
    native:async(_,request)=>{calls.push(request);if(request.action==='connection')return{ok:true,result:{connection_id:'c'.repeat(64),capture_enabled:!!grant?.capture,automation:grant,state:grant?'reachable':'revoked'}};if(request.action==='automatic_capture')return{ok:true,result:{events_added:2,events_seen:2}};if(request.action==='authorized_read')return{ok:true,result:{bootstrap_version:'v1',stable_text:'合成偏好',refs:[]}};throw new Error('不支持的测试请求');}};
  const broker=new Broker(api);
  const message=()=>({kind:'automatic_tick',route:state.route,nonce:state.nonce,session_ref:state.session_ref});
  return{broker,api,sender,calls,content,snapshot,state:()=>state,tick:()=>broker.handle(message(),sender),setGrant:value=>{grant=value;},message};
}
test('没有持久授权时只检查连接，不读取文字或准备草稿',async()=>{
  const h=await harness(null);assert.equal((await h.tick()).status,'revoked');
  assert.deepEqual(h.calls.map(r=>r.action),['connection']);assert.ok(h.content.every(m=>m.kind==='check'));assert.equal(h.state().preview,null);
});
test('一次授权自动捕获和bootstrap，重复tick不重存不重复插入',async()=>{
  const h=await harness();assert.equal((await h.tick()).status,'draft_ready');
  assert.deepEqual(h.calls.map(r=>r.action),['connection','automatic_capture','authorized_read','authorized_read']);
  assert.equal(h.content.filter(m=>m.kind==='insert').length,1);assert.equal(h.state().preview.delivery,'draft');
  await h.tick();assert.equal(h.calls.filter(r=>r.action==='automatic_capture').length,1);assert.equal(h.content.filter(m=>m.kind==='insert').length,1);
});
test('bootstrap离开草稿不宣称发送；完成的只读请求自动准备下一份资料',async()=>{
  const h=await harness();await h.tick();h.snapshot.draft_ids=[];
  const request={protocol:'recallcard.action/1',nonce:h.state().nonce,session_ref:h.state().session_ref,request_id:'r_next',action:'search',arguments:{query:'合成决定'}};
  h.snapshot.requests=['```recallcard-action\n'+JSON.stringify(request)+'\n```'];
  assert.equal((await h.tick()).status,'draft_ready');
  assert.equal(h.state().bootstrap.delivery,'left_draft');assert.equal(h.state().preview.id,'r_next');
  assert.equal(h.calls.filter(r=>r.action==='authorized_read'&&r.arguments.action==='search').length,2);
});
test('流式未稳定、过期身份、页面伪造授权与写动作都不能自动执行',async()=>{
  const h=await harness();h.snapshot.status='waiting_for_stable_page';await h.tick();assert.equal(h.calls.length,1);
  await assert.rejects(h.broker.handle(h.message(),{...h.sender,frameId:1}));
  await assert.rejects(h.broker.handle(h.message(),{...h.sender,documentId:'other'}));
  h.snapshot.status='stable';await h.tick();h.snapshot.draft_ids=[];
  h.snapshot.requests=['```recallcard-action\n'+JSON.stringify({protocol:'recallcard.action/1',nonce:h.state().nonce,session_ref:h.state().session_ref,request_id:'forged',action:'automatic_capture',arguments:{provider_disclosure:true}})+'\n```'];
  await h.tick();assert.equal(h.content.filter(m=>m.kind==='insert').length,1);
});
test('撤权移除自有草稿，后续不捕获/读取；范围修订废弃旧结果',async()=>{
  const h=await harness();await h.tick();h.setGrant(null);assert.equal((await h.tick()).status,'revoked');
  assert.equal(h.content.filter(m=>m.kind==='remove').length,1);assert.equal(h.state().preview,null);
  assert.equal(h.calls.filter(r=>r.action==='automatic_capture').length,1);
});
test('捕获与网站提供资料分别控制，禁用资料授权时只保存',async()=>{
  const h=await harness({capture:true,recall:false,provider_disclosure:false,permission_revision:2,platform:'chatgpt'});await h.tick();
  assert.deepEqual(h.calls.map(r=>r.action),['connection','automatic_capture']);assert.equal(h.content.filter(m=>m.kind==='insert').length,0);
});
test('保存失败不标记成功，下一次相同快照可幂等重试',async()=>{
  const h=await harness();const native=h.api.native;let failed=true;
  h.api.native=async(host,request)=>{if(failed&&request.action==='automatic_capture')throw new Error('合成失败');return native(host,request);};
  await assert.rejects(h.tick(),/合成失败/);assert.equal(h.state().auto_capture_hash,undefined);assert.equal(h.state().automatic_status,'failed');
  failed=false;await h.tick();assert.ok(h.state().auto_capture_hash);assert.equal(h.state().automatic_error,null);
});
