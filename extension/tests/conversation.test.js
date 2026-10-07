import test from 'node:test';
import assert from 'node:assert/strict';
import '../conversation-format.js';
import { routeFor, validateAction } from '../protocol.js';
import { Broker, bindSession } from '../broker.js';
const format = globalThis.RecallCardConversationFormat;
const captureId='11111111-1111-4111-8111-111111111111';
const raw = () => ({schema:format.SCHEMA,capture_id:captureId,captured_at:'2026-01-01T00:00:00.000Z',title:'合成会话',source:{platform:'chatgpt',conversation_id:'synthetic',url:'https://chatgpt.com/c/synthetic'},coverage:{extent:'visible_only',complete:false,reason:'只读取可见片段',warnings:['原始时间未知']},messages:[{id:'message-1',role:'user',text:'问题 <script>alert(1)</script>\n```\n内容',occurred_at:null},{id:`${captureId}:2`,role:'assistant',text:'合成回答',occurred_at:null,metadata:{weaker_identity:true}}],metadata:{weaker_conversation_identity:false}});
test('完整快照摘要涵盖全部消息与来源，修改尾部同样失效',async()=>{
  const snapshot=await format.seal(raw());assert.equal((await format.verify(snapshot)).schema,format.SCHEMA);
  for(const mutate of [value=>value.messages[1].text+='改动',value=>value.source.url='https://chatgpt.com/c/other',value=>value.coverage.complete=true,value=>value.title='改标题']) {
    const changed=structuredClone(snapshot);mutate(changed);await assert.rejects(format.verify(changed));
  }
  assert.equal(snapshot.messages[0].occurred_at,null);assert.equal(raw().metadata.snapshot_hash,undefined);
});
test('选择保留网站顺序与消息ID，同一选择的摘要确定；原快照不变',async()=>{
  const snapshot=await format.seal(raw());const selected=await format.select(snapshot,[{id:snapshot.messages[1].id,role:'assistant'}]);
  assert.equal(selected.messages.length,1);assert.equal(selected.messages[0].id,snapshot.messages[1].id);
  assert.equal(snapshot.messages.length,2);assert.equal(snapshot.coverage.warnings.length,1);
  assert.equal(selected.metadata.selected_message_count,1);assert.match(selected.coverage.warnings.at(-1),/勾选/);
  assert.deepEqual(selected,await format.select(snapshot,[{id:snapshot.messages[1].id,role:'assistant'}]));
});
test('未知角色需用户确认，不按奇偶猜测，不允许改写明确角色',async()=>{
  const value=raw();value.messages[0].role=null;const snapshot=await format.seal(value);
  await assert.rejects(format.select(snapshot,[{id:'message-1',role:null}]),/角色/);
  const selected=await format.select(snapshot,[{id:'message-1',role:'user'}]);assert.equal(selected.messages[0].metadata.role_confirmed_by_user,true);
  await assert.rejects(format.select(snapshot,[{id:snapshot.messages[1].id,role:'user'}]),/明确/);
  await assert.rejects(format.select(snapshot,[{id:'absent',role:'user'}]),/不在/);
  await assert.rejects(format.select(snapshot,[{id:'message-1',role:'user'},{id:'message-1',role:'user'}]),/角色/);
});
test('JSON原文可往返；Markdown脚本与围栏保持为文字而非活动内容',async()=>{
  const snapshot=await format.seal(raw());const output=format.markdown(snapshot);
  assert.deepEqual(JSON.parse(format.json(snapshot)),snapshot);
  assert.ok(output.includes('````text\n问题 <script>alert(1)</script>\n```\n内容\n````'));
  assert.match(output,/原始时间未知/);assert.match(format.handoff(snapshot),/不是本次对话的新授权/);
});
test('空记录、消息上限与UTF-8字节上限明确拒绝，不截断',async()=>{
  const empty=raw();empty.messages=[];await assert.rejects(format.seal(empty),/没有/);
  const many=raw();many.messages=Array.from({length:5001},(_,index)=>({...many.messages[0],id:String(index)}));await assert.rejects(format.seal(many),/5000/);
  const single=raw();single.messages[0].text='a'.repeat(2*1024*1024+1);await assert.rejects(format.seal(single),/单条消息/);
  const large=raw();large.messages[0].text='汉'.repeat(6*1024*1024);await assert.rejects(format.seal(large),/16 MiB/);
});
test('DeepSeek仅精确host和会话路径，根页与ID解析不放宽登录等路径',()=>{
  assert.equal(routeFor('https://chat.deepseek.com/'),'https://chat.deepseek.com');
  assert.equal(routeFor('https://chat.deepseek.com/a/chat/s/demo_123'),'https://chat.deepseek.com/a/chat/s/demo_123');
  for(const url of ['https://deepseek.com/','https://chat.deepseek.com.evil.test/','http://chat.deepseek.com/','https://chat.deepseek.com/a/chat/s/demo?token=x','https://chat.deepseek.com/sign_in','https://chat.deepseek.com/a/chat/s/demo/other'])assert.throws(()=>routeFor(url));
});
function harness() {
  const ext='abcdefghijklmnopabcdefghijklmnop',popupUrl=`chrome-extension://${ext}/popup.html`;
  let state=bindSession(null,{route:'https://chatgpt.com/c/synthetic',token:captureId,documentId:'document-1'});
  let selected,changed=false;
  const calls=[];const tab={id:1,url:state.route,active:true};
  const api={id:ext,popupUrl,getTab:async()=>tab,load:async()=>structuredClone(state),save:async(_,value)=>{state=structuredClone(value);},now:()=>10000,
    content:async(_,message)=>{if(changed)return{ok:false,error:'会话已变化'};return{ok:true,result:message.kind==='capture_select'?selected:{status:'current'}};},
    native:async(host,request)=>{calls.push({host,request});return{ok:true,result:request.action==='capture_preview'?{approval_hash:'synthetic-hash',connection_id:'a'.repeat(64),vault_name:'资料库A',scope:'personal',event_count:2,redacted_event_count:0,samples:['已脱敏合成样本'],coverage:{}}:request.action==='capture_save'?{events_added:2,events_seen:2,refs:[],coverage:{}}:{connection_id:'a'.repeat(64),capture_enabled:true,capture_scope:'personal',read_scopes:['personal'],vault_name:'合成资料库'}};}};
  return{broker:new Broker(api),api,tab,calls,popup:{id:ext,url:popupUrl},msg:(kind,extras={})=>({kind,tabId:1,nonce:state.nonce,session_ref:state.session_ref,...extras}),setSelected:value=>selected=value,change:()=>changed=true};
}
test('三个保存动作无法由模型action parser升级；popup可检查连接',async()=>{
  const h=harness();for(const action of ['connection','capture_preview','capture_save'])assert.throws(()=>validateAction({protocol:'recallcard.action/1',request_id:'r_model',nonce:h.msg('').nonce,session_ref:h.msg('').session_ref,action,arguments:{}},h.msg('')));
  const result=await h.broker.handle(h.msg('connection'),h.popup);assert.equal(result.connection.capture_enabled,true);assert.equal(h.calls[0].request.action,'connection');
  await assert.rejects(h.broker.handle(h.msg('connection'),{...h.popup,tab:{id:1}}));assert.equal(h.calls.length,1);
});
test('本机脱敏预览和当前完整快照相绑定，先预览再单独确认保存',async()=>{
  const h=harness();h.setSelected(await format.seal(raw()));
  await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/预览/);assert.equal(h.calls.length,0);
  const preview=await h.broker.handle(h.msg('capture_preview'),h.popup);assert.equal(preview.event_count,2);
  const result=await h.broker.handle(h.msg('capture_save',{approval_hash:preview.approval_hash}),h.popup);assert.equal(result.events_added,2);
  assert.deepEqual(h.calls.map(call=>call.request.action),['capture_preview','connection','capture_save']);
  assert.equal(h.calls[2].request.arguments.conversation.source.platform,'chatgpt');assert.equal(Object.hasOwn(h.calls[2].request.arguments,'scope'),false);
});
test('选择改变或页面身份变化后不能沿用保存确认',async()=>{
  const h=harness();h.setSelected(await format.seal(raw()));await h.broker.handle(h.msg('capture_preview'),h.popup);
  const changed=raw();changed.messages.pop();h.setSelected(await format.seal(changed));await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/变化/);
  h.change();await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/变化/);assert.equal(h.calls.length,1);
});
test('本机失败不阻止已验证会话导出，保存错误不假报成功',async()=>{
  const h=harness();h.setSelected(await format.seal(raw()));await h.broker.handle(h.msg('capture_preview'),h.popup);
  const native=h.api.native;h.api.native=async(host,request)=>{if(request.action==='connection')return native(host,request);throw new Error('合成本机断线');};await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/未确认保存结果/);
  assert.equal((await h.broker.handle(h.msg('conversation'),h.popup)).messages.length,2);
});
test('超过本机200KiB限制可导出，不调用本机也不截断',async()=>{
  const h=harness();const large=raw();large.messages[0].text='长'.repeat(80000);h.setSelected(await format.seal(large));
  await assert.rejects(h.broker.handle(h.msg('capture_preview'),h.popup),/桌面导入/);assert.equal(h.calls.length,0);
  assert.equal((await h.broker.handle(h.msg('conversation'),h.popup)).messages[0].text.length,80000);
});

test('A库预览后连接B库即清除旧确认，不能把A的批准用于B',async()=>{
  const h=harness();h.setSelected(await format.seal(raw()));await h.broker.handle(h.msg('capture_preview'),h.popup);
  const native=h.api.native;h.api.native=async(host,request)=>request.action==='connection'?{ok:true,result:{connection_id:'b'.repeat(64),capture_enabled:true,capture_scope:'personal',vault_name:'资料库B'}}:native(host,request);
  const connection=await h.broker.handle(h.msg('connection'),h.popup);assert.equal(connection.connection.vault_name,'资料库B');assert.equal(connection.capture_confirmation_valid,false);
  await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/预览/);
  assert.equal(h.calls.some(call=>call.request.action==='capture_save'),false);
});
test('不主动检查连接也要在保存前重新核对，后台A换B必须拒绝',async()=>{
  const h=harness();h.setSelected(await format.seal(raw()));await h.broker.handle(h.msg('capture_preview'),h.popup);
  const native=h.api.native;h.api.native=async(host,request)=>request.action==='connection'?{ok:true,result:{connection_id:'b'.repeat(64),capture_enabled:true,capture_scope:'personal',vault_name:'资料库B'}}:native(host,request);
  await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/目的资料库.*变化/);
  assert.equal(h.calls.some(call=>call.request.action==='capture_save'),false);
  await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/预览/);
});
test('旧本机缺少connection_id不能获得保存确认，仍可导出文件',async()=>{
  for(const stage of ['connection','capture_preview']){
    const h=harness();h.setSelected(await format.seal(raw()));const native=h.api.native;
    h.api.native=async(host,request)=>{const response=await native(host,request);delete response.result.connection_id;return response;};
    await assert.rejects(h.broker.handle(h.msg(stage),h.popup),/升级/);
    await assert.rejects(h.broker.handle(h.msg('capture_save',{approval_hash:'synthetic-hash'}),h.popup),/预览/);
    assert.equal((await h.broker.handle(h.msg('conversation'),h.popup)).messages.length,2);
  }
});
