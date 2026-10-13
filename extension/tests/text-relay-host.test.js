import test from 'node:test';
import assert from 'node:assert/strict';
import { Broker, bindSession } from '../broker.js';
import { textCommand, textCycle, textPublic } from '../text-relay-host.js';

const token='11111111-1111-4111-8111-111111111111';
const hash='a'.repeat(64),epoch='b'.repeat(64),config='c'.repeat(64),scopes='d'.repeat(64);
const locator=(offset=0,snapshot=hash)=>`rcs1|synthetic|event:evt_synthetic|${snapshot}|${epoch}|${config}|${scopes}|${offset}`;
const row=(text='合成文本',offset=0,total=100)=>({id:locator(offset),ref:'event:evt_synthetic',kind:'event',scope:'synthetic',snapshot:hash,text,text_range:{start_byte:offset,end_byte:offset+Buffer.byteLength(text),total_bytes:total}});
const result=()=>({status:'partial',results:[row()],coverage:{scopes:[{scope:'synthetic',query_encoding:'computed'}]},truncated:true});
const code=expected=>error=>error.code===expected&&!error.message.includes('/private/');
function deferred(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
async function harness(){
 let state=bindSession(null,{route:'https://chatgpt.com/c/synthetic',token,documentId:'doc-synthetic',installationId:'installation-synthetic'}),time=10000;
 state.extension_id='extension-synthetic';
 const calls=[],content=[],tab={id:1,active:true,url:state.route};
 let connection={connection_id:'c'.repeat(64),capture_enabled:false,read_scopes:['synthetic'],automation:{recall:true,provider_disclosure:true,platform:'chatgpt',permission_revision:1,grant_id:'synthetic-grant'},state:'reachable'};
 let bootstrap={bootstrap_version:'synthetic-v1',stable_text:'合成背景',refs:[],scopes:['synthetic']},search=result();
 let snapshot={status:'manual_fallback',sent:null,assistant:null};
 let nativeHook=null;
 const api={getTab:async()=>tab,load:async()=>structuredClone(state),save:async(_,value)=>{state=structuredClone(value);},now:()=>time,
 content:async(_,message)=>{content.push(message);return message.kind==='text_snapshot'?{ok:true,result:structuredClone(snapshot)}:{ok:true,result:{}};},
 native:async(_,request)=>{
  calls.push(structuredClone(request));
  if(nativeHook){const value=await nativeHook(request);if(value!==undefined)return value;}
  if(request.action==='connection')return{ok:true,result:structuredClone(connection)};
  if(request.action!=='authorized_read')throw new Error('/private/synthetic: 未授权工具');
  const {action,arguments:args}=request.arguments;
  if(action==='bootstrap')return{ok:true,result:structuredClone(bootstrap)};
  if(action==='recall_search')return{ok:true,result:structuredClone(search)};
  if(action==='recall_fetch')return{ok:true,result:{status:'partial',results:[row('abcd',Number(args.id.split('|').at(-1)),100)],next_id:locator(Number(args.id.split('|').at(-1))+4)}};
  throw new Error('/private/synthetic: 错误');
 }};
 const broker=new Broker(api),message=()=>({nonce:state.nonce,session_ref:state.session_ref,route:state.route});
 const command=(command,extra={})=>textCommand(broker,1,{...message(),command,...extra});
 const frame=(action,body)=>`RECALLCARD/2 ${state.text_relay.engine.ticket.value}\n${JSON.stringify(action)}${body===undefined?'':`\n\n${body}`}\nEND RECALLCARD`;
 const delivered=()=>command('delivered',{confirmed:true,packet_id:state.text_relay.engine.packet.id,packet_hash:state.text_relay.engine.packet.hash});
 const send=async()=>{await command('copy');await delivered();};
 return {broker,api,calls,content,tab,state:()=>state,setState:fn=>fn(state),command,message,frame,delivered,send,tick:()=>textCycle(broker,1,message()),setConnection:fn=>fn(connection),setBootstrap:fn=>fn(bootstrap),setSearch:fn=>fn(search),setSnapshot:value=>{snapshot=value;},setNativeHook:fn=>{nativeHook=fn;},advance:ms=>{time+=ms;}};
}
const reads=h=>h.calls.filter(r=>r.action==='authorized_read');

test('首包只读且不自动填入；公开状态不暴露绑定、nonce、long locator',async()=>{
 const h=await harness(),value=await h.command('start');
 assert.equal(value.protocol,'text/2');assert.equal(value.phase,'awaiting_user_send');assert.equal(value.budget.reads,1);
 assert.match(value.packet.text,/合成背景/);assert.equal(value.packet.sent,null);
 assert.deepEqual(reads(h).map(r=>r.arguments.action),['bootstrap']);assert.ok(h.content.every(m=>m.kind==='check'));
 const text=JSON.stringify(value);for(const internal of ['bindingKey','grantBinding','packetFingerprint',h.state().nonce,'rcs1|'])assert.ok(!text.includes(internal));
 assert.equal(h.state().preview,null);assert.equal(h.state().bootstrap,null);
 await assert.rejects(h.command('start'),code('reset_required'));
});

test('复制重核真实背景并按wrap全文扣费；复制和草稿消失均不算发送',async()=>{
 const h=await harness();await h.command('start');const copy=await h.command('copy');
 assert.match(copy.text,/^\[RecallCard /);assert.match(copy.text,/\[\/RecallCard /);assert.equal(copy.packet_id,h.state().text_relay.engine.packet.id);
 assert.equal(h.state().text_relay.engine.budget.reads,2);assert.equal(h.state().text_relay.engine.budget.disclosureBytes,Buffer.byteLength(copy.text));
 assert.equal(h.state().text_relay.engine.ticket.active,false);assert.equal(h.state().text_relay.engine.packet.sent,null);
 await assert.rejects(h.command('execute',{text:h.frame({action:'search',query:'合成问题'})}),code('send_gate'));
 await assert.rejects(h.command('delivered',{packet_id:copy.packet_id,packet_hash:copy.packet_hash}),code('confirmation_required'));
 await h.delivered();assert.equal(h.state().text_relay.engine.ticket.active,true);
});

test('人工确认须匹配当前packet/hash且必须已真实发起复制',async()=>{
 const h=await harness();await h.command('start');await assert.rejects(h.delivered(),code('not_disclosed'));
 await h.command('copy');await assert.rejects(h.command('delivered',{confirmed:true,packet_id:'old',packet_hash:h.state().text_relay.engine.packet.hash}),code('wrong_packet'));
 await h.delivered();assert.equal(h.state().text_relay.engine.packet.sent.kind,'manual');
});

test('人工 start→search→fetch→final 完整闭环，final复核全部真实引用且不写入资料',async()=>{
 const h=await harness();await h.command('start');await h.send();
 await h.command('execute',{text:h.frame({action:'search',query:'合成问题'})});
 assert.equal(h.state().text_relay.engine.sources.S1.sent,false);assert.ok(!JSON.stringify(textPublic(h.state())).includes('rcs1|'));
 await h.send();await h.command('execute',{text:h.frame({action:'fetch',id:'S1'})});
 assert.equal(h.state().text_relay.engine.pages.P1.source,'S1');await h.send();
 const before=reads(h).length,final=h.frame({action:'final'},'基于合成来源回答。[S1]');
 const done=await h.command('execute',{text:final});assert.equal(done.phase,'result_ready');assert.equal(done.final.body,'基于合成来源回答。[S1]');
 assert.deepEqual(reads(h).slice(before).map(r=>r.arguments.action),['bootstrap','recall_fetch']);assert.equal(done.budget.reads,reads(h).length);
 await assert.rejects(h.command('execute',{text:final}),code('closed'));assert.equal((await h.tick()).status,'result_ready');
 assert.ok(h.calls.every(r=>['connection','authorized_read'].includes(r.action)));assert.ok(h.content.every(m=>['check','text_snapshot'].includes(m.kind)));
});

test('无需历史可直接final；零引用也真实复核bootstrap且只完成一次',async()=>{
 const h=await harness();await h.command('start');await h.send();
 const done=await h.command('execute',{text:h.frame({action:'final'},'无需历史资料。')});assert.deepEqual(done.final.sources,[]);assert.equal(done.budget.reads,3);
 const before=reads(h).length;await h.tick();assert.equal(reads(h).length,before);
});

test('DOM完整收据后才消费最后完整assistant；旧帧、引用、streaming零动作',async()=>{
 const h=await harness();await h.command('start');await h.command('copy');
 const packet=h.state().text_relay.engine.packet,good=h.frame({action:'search',query:'合成问题'}),before=reads(h).length;
 for(const value of [
 {status:'streaming',sent:{message_id:'u1',packet_hash:packet.hash},assistant:{id:'a1',text:good,complete:true}},
 {status:'stable',sent:{message_id:'u1',packet_hash:'changed'},assistant:{id:'a1',text:good,complete:true}},
 {status:'stable',sent:{message_id:'u1',packet_hash:packet.hash},assistant:{id:'a1',text:good,complete:false}},
 {status:'stable',sent:{message_id:'u1',packet_hash:packet.hash},assistant:{id:'a1',text:'```\n'+good+'\n```',complete:true}},
 {status:'stable',sent:{message_id:'u1',packet_hash:packet.hash},assistant:{id:'a1',text:good.replace(h.state().text_relay.engine.ticket.value,'T_0000000000000000'),complete:true}},
 ]){h.setSnapshot(value);await h.tick();assert.equal(reads(h).length,before);}
 h.setSnapshot({status:'stable',sent:{message_id:'u1',packet_hash:packet.hash},assistant:{id:'a1',text:good,complete:true}});
 assert.equal((await h.tick()).status,'awaiting_user_send');assert.equal(reads(h).length,before+1);
 await h.tick();assert.equal(reads(h).length,before+1);
});

test('仅人工发送确认无DOM强收据时，不消费网页旧assistant',async()=>{
 const h=await harness();await h.command('start');await h.send();
 h.setSnapshot({status:'stable',sent:null,assistant:{id:'old',text:h.frame({action:'search',query:'合成'}),complete:true}});
 const before=reads(h).length;assert.equal((await h.tick()).status,'waiting_reply');assert.equal(reads(h).length,before);
});

test('每次资料read当前授权；撤权、修订、scope和connection变动均阻塞复制',async()=>{
 for(const change of [c=>{c.automation.recall=false;},c=>{c.automation.provider_disclosure=false;},c=>{c.automation.permission_revision++;},c=>{c.read_scopes=['other'];},c=>{c.connection_id='e'.repeat(64);},c=>{c.automation.platform='deepseek';}]){
  const h=await harness();await h.command('start');const before=reads(h).length;h.setConnection(change);
  await assert.rejects(h.command('copy'));assert.equal(reads(h).length,before);assert.equal(h.state().text_relay.engine.budget.disclosureBytes,0);
  assert.equal(h.state().text_relay.invalidated,true);assert.equal(textPublic(h.state()).packet,null);
 }
});

test('复制前重跑原source request；仅query_encoding诊断差异允许',async()=>{
 const h=await harness();await h.command('start');await h.send();await h.command('execute',{text:h.frame({action:'search',query:'合成问题'})});
 h.setSearch(value=>{value.coverage.scopes[0].query_encoding='cache';});await h.command('copy');
 h.setSearch(value=>{value.results[0].text='新文本';});const before=h.state().text_relay.engine.budget.disclosureBytes;
 await assert.rejects(h.command('copy'),code('source_changed'));assert.equal(h.state().text_relay.engine.budget.disclosureBytes,before);assert.equal(h.state().text_relay.invalidated,true);
});

test('新搜索包复制也重核开局背景；真实来源定位符在final时重新授权读取',async()=>{
 const h=await harness();await h.command('start');await h.send();await h.command('execute',{text:h.frame({action:'search',query:'合成问题'})});
 h.setBootstrap(value=>{value.bootstrap_version='changed';});await assert.rejects(h.command('copy'),code('source_changed'));
 const other=await harness();await other.command('start');await other.send();await other.command('execute',{text:other.frame({action:'search',query:'合成'})});await other.send();
 other.setNativeHook(request=>{if(request.arguments?.action==='recall_fetch')throw new Error('/private/synthetic 被遗忘');});
 await assert.rejects(other.command('execute',{text:other.frame({action:'final'},'引用 [S1]')}),code('native_unavailable'));assert.equal(other.state().text_relay.engine.completed,false);assert.equal(other.state().text_relay.engine.failure.code,'native_unavailable');
});

test('final引用来源逐个预先预算，不足不会做部分核验或伪称完成',async()=>{
 const h=await harness();await h.command('start');await h.send();await h.command('execute',{text:h.frame({action:'search',query:'合成问题'})});await h.send();
 h.setState(value=>{value.text_relay.engine.budget.reads=15;});const before=reads(h).length;
 await assert.rejects(h.command('execute',{text:h.frame({action:'final'},'引用 [S1]')}),code('budget_exhausted'));assert.equal(reads(h).length,before);assert.equal(h.state().text_relay.engine.completed,false);
});

test('读取过程中取消立即完成；旧成功或错误均不能复活状态或披露',async()=>{
 for(const failure of [false,true]){
  const h=await harness();await h.command('start');await h.send();const entered=deferred(),gate=deferred();
  h.setNativeHook(async request=>{if(request.arguments?.action==='recall_search'){entered.resolve();await gate.promise;if(failure)throw new Error('/private/late');return{ok:true,result:result()};}});
  const pending=h.command('execute',{text:h.frame({action:'search',query:'合成'})});await entered.promise;
  const cancelled=await h.command('cancel');assert.equal(cancelled.phase,'blocked');const after=structuredClone(h.state().text_relay);
  gate.resolve();await assert.rejects(pending,code('stale_operation'));assert.deepEqual(h.state().text_relay,after);assert.equal(h.state().text_relay.engine.cancelled,true);
 }
});

test('读取中暂停恢复丢弃旧响应，明确retry新attempt才继续',async()=>{
 const h=await harness();await h.command('start');await h.send();const entered=deferred(),gate=deferred();
 h.setNativeHook(async request=>{if(request.arguments?.action==='recall_search'){entered.resolve();await gate.promise;return{ok:true,result:result()};}});
 const pending=h.command('execute',{text:h.frame({action:'search',query:'合成'})});await entered.promise;
 await h.command('pause');assert.equal((await h.tick()).status,'paused');await h.command('resume');gate.resolve();await assert.rejects(pending,code('stale_operation'));
 assert.equal(h.state().text_relay.engine.phase,'blocked');assert.equal(h.state().text_relay.engine.failure.code,'paused');
 h.setNativeHook(null);const resultValue=await h.command('retry');assert.equal(resultValue.phase,'awaiting_user_send');assert.equal(resultValue.budget.actions,2);
});

test('copy复核在途被取消，取消前预扣读取不退还也不交出资料',async()=>{
 const h=await harness();await h.command('start');const entered=deferred(),gate=deferred();
 h.setNativeHook(async request=>{if(request.arguments?.action==='bootstrap'){entered.resolve();await gate.promise;return{ok:true,result:{bootstrap_version:'synthetic-v1',stable_text:'合成背景',refs:[],scopes:['synthetic']}};}});
 const pending=h.command('copy');await entered.promise;await h.command('cancel');gate.resolve();await assert.rejects(pending,code('stale_operation'));
 assert.equal(h.state().text_relay.engine.budget.reads,2);assert.equal(h.state().text_relay.engine.budget.disclosureBytes,0);
});

test('已知失败仅显式retry，旧票据消费后不能用另一个请求绕过',async()=>{
 const h=await harness();await h.command('start');await h.send();h.setNativeHook(request=>{if(request.arguments?.action==='recall_search')throw new Error('/private/synthetic unavailable');});
 const frame=h.frame({action:'search',query:'合成'});await assert.rejects(h.command('execute',{text:frame}),code('native_unavailable'));
 assert.equal(h.state().text_relay.engine.ticket.consumed,true);await assert.rejects(h.command('execute',{text:frame}),code('send_gate'));
 h.setNativeHook(null);await h.command('retry');assert.equal(h.state().text_relay.engine.budget.actions,2);
});

test('显式start可替换prepared预览；draft只移除完整自有块，失败保留编辑',async()=>{
 for(const delivery of ['prepared','draft']){
  const h=await harness();h.setState(value=>{value.preview={id:'v1-existing',text:'合成旧资料',delivery};value.bootstrap=value.preview;});
  await h.command('start');assert.equal(h.state().preview,null);assert.equal(h.state().bootstrap,null);
  assert.equal(h.content.filter(m=>m.kind==='remove').length,delivery==='draft'?1:0);assert.equal(h.state().text_relay.engine.phase,'awaiting_user_send');
 }
 const h=await harness();h.setState(value=>{value.preview={id:'v1-existing',text:'编辑后的合成草稿',delivery:'draft'};});
 const content=h.api.content;h.api.content=async(tab,message)=>message.kind==='remove'?{ok:false,error:'/private/edited'}:content(tab,message);
 const prior=structuredClone(h.state().preview);await assert.rejects(h.command('start'),code('legacy_draft'));assert.deepEqual(h.state().preview,prior);assert.equal(h.calls.length,0);
});

test('旧资料送达状态不明和已存在text run必须显式reset，不自动重启预算',async()=>{
 for(const delivery of ['left_draft','user_confirmed_sent']){
  const h=await harness();h.setState(value=>{value.preview={id:'v1-existing',text:'合成草稿',delivery};});
  const prior=structuredClone(h.state().preview);await assert.rejects(h.command('start'),code('reset_required'));assert.deepEqual(h.state().preview,prior);assert.equal(h.calls.length,0);
 }
 const h=await harness();await h.command('start');const done=await h.command('cancel');assert.equal(done.cancelled,true);assert.equal(done.completed,false);await assert.rejects(h.command('start'),code('reset_required'));
});

test('取消与已排队storage.save交错，最后持久状态仍为cancelled且copy不返回正文',async()=>{
 const h=await harness();await h.command('start');const entered=deferred(),gate=deferred(),originalSave=h.api.save;let blocked=false;
 h.api.save=async(tab,value)=>{if(!blocked&&value.text_relay?.engine.budget.disclosureBytes>0){blocked=true;entered.resolve();await gate.promise;}return originalSave(tab,value);};
 const pending=h.command('copy');await entered.promise;const cancelling=h.command('cancel');await new Promise(resolve=>setImmediate(resolve));gate.resolve();
 await assert.rejects(pending,code('stale_operation'));await cancelling;assert.equal(h.state().text_relay.engine.cancelled,true);assert.equal(h.state().text_relay.engine.ticket.active,false);
});

test('connection检查在途被取消，不用旧connection快照覆盖生命周期',async()=>{
 const h=await harness();await h.command('start');const entered=deferred(),gate=deferred();let intercepted=false;
 h.setNativeHook(async request=>{if(!intercepted&&request.action==='connection'){intercepted=true;entered.resolve();await gate.promise;}});
 const pending=h.command('copy');await entered.promise;await h.command('cancel');const after=structuredClone(h.state().text_relay);gate.resolve();
 await assert.rejects(pending,code('stale_operation'));assert.deepEqual(h.state().text_relay,after);
});

test('final多引用逐个严格原locator fetch并按真实调用累计预算',async()=>{
 const h=await harness();h.setSearch(value=>{value.results.push({...row(),id:locator(0,'e'.repeat(64)),snapshot:'e'.repeat(64)});});
 await h.command('start');await h.send();await h.command('execute',{text:h.frame({action:'search',query:'合成'})});await h.send();
 const requirements=structuredClone(h.state().text_relay.engine.sources),before=reads(h).length;
 h.setNativeHook(request=>{if(request.arguments?.action==='recall_fetch'){
  const id=request.arguments.arguments.id,snapshot=id.split('|')[3];return {ok:true,result:{status:'complete',results:[{...row('abcd',0,4),id,snapshot}],next_id:null}};
 }});
 const done=await h.command('execute',{text:h.frame({action:'final'},'两份合成资料。[S1][S2][S1]')});
 assert.equal(done.phase,'result_ready');assert.deepEqual(done.final.sources,['S1','S2']);assert.equal(reads(h).length-before,3);assert.equal(done.budget.reads,reads(h).length);
 assert.deepEqual(reads(h).slice(before+1).map(r=>r.arguments.arguments.id),[requirements.S1.locator,requirements.S2.locator]);
});

test('final来源返回同ref不同snapshot或scope必须拒绝，不能用成功状态冒充授权',async()=>{
 for(const change of [r=>{r.snapshot='e'.repeat(64);},r=>{r.scope='other';},r=>{r.id=locator(0,'e'.repeat(64));r.snapshot='e'.repeat(64);},r=>{r.text_hash='f'.repeat(64);}]){
  const h=await harness();await h.command('start');await h.send();await h.command('execute',{text:h.frame({action:'search',query:'合成'})});await h.send();
  h.setNativeHook(request=>{if(request.arguments?.action==='recall_fetch'){const value=row('abcd',0,4);change(value);return{ok:true,result:{status:'complete',results:[value]}};}});
  await assert.rejects(h.command('execute',{text:h.frame({action:'final'},'合成来源。[S1]')}),code('invalid_result'));assert.equal(h.state().text_relay.engine.completed,false);
 }
});

test('worker恢复遗留pending不自动重放，明确retry维持原参数和新attempt',async()=>{
 const h=await harness();await h.command('start');await h.send();h.setNativeHook(request=>{if(request.arguments?.action==='recall_search')throw new Error('合成离线');});
 await assert.rejects(h.command('execute',{text:h.frame({action:'search',query:'合成待恢复'})}));
 h.setState(state=>{const engine=state.text_relay.engine;engine.pending=engine.failure.attempt;engine.failure=null;engine.phase='preparing';});h.setNativeHook(null);
 const restored=new Broker(h.api),before=reads(h).length;
 await textCycle(restored,1,h.message());assert.equal(reads(h).length,before);assert.equal(h.state().text_relay.engine.failure.code,'interrupted');assert.equal(textPublic(h.state()).canRetry,true);
 await textCommand(restored,1,{...h.message(),command:'retry'});assert.equal(h.state().text_relay.engine.phase,'awaiting_user_send');assert.equal(reads(h).at(-1).arguments.arguments.query,'合成待恢复');
});

test('超过attempt期限的响应丢弃；下一次tick记录timeout，绝不自动重试',async()=>{
 const h=await harness();await h.command('start');await h.send();const entered=deferred(),gate=deferred();
 h.setNativeHook(async request=>{if(request.arguments?.action==='recall_search'){entered.resolve();await gate.promise;return{ok:true,result:result()};}});
 const pending=h.command('execute',{text:h.frame({action:'search',query:'合成超时'})});await entered.promise;h.advance(60001);gate.resolve();
 await assert.rejects(pending,code('stale_operation'));assert.equal(h.state().text_relay.engine.sources.S1,undefined);const before=reads(h).length;
 await h.tick();assert.equal(h.state().text_relay.engine.failure.code,'timeout');assert.equal(h.state().text_relay.engine.phase,'blocked');assert.equal(reads(h).length,before);
});
