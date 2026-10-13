// Web text/2 宿主：只读本机资料，网页最终发送始终由用户完成。
// 纯状态账本不构成授权；每次实际读、复制和终态都重新核验本机绑定。
import { newNonce, siteFor } from './protocol.js';
import { byteLength } from './strict-json.js';
import { newTextTicket } from './text-relay-core.js';
import { capsuleText } from './relay-state.js';
import {
  createTextSession, reserveTextRead, confirmTextFreshness, prepareTextPacket,
  packetFreshnessTarget, finalFreshnessTarget, reserveTextDisclosure, confirmTextSent,
  consumeTextReply, acceptTextResult, acceptTextError, retryTextAttempt, expireTextAttempt,
  completeTextFinal, pauseTextSession, resumeTextSession, cancelTextSession,
  textSourceRequirements, discardTextResultAsBudget,
} from './text-relay-session.js';

const clone = value => JSON.parse(JSON.stringify(value));
const runtimes = new WeakMap();
const messages = Object.freeze({
  grant_required:'请在桌面确认当前网站的读取和提供资料权限',
  binding_changed:'接收方、资料范围或权限已变化，请明确重置后重新开始',
  stale_operation:'本轮已变化，迟到结果已丢弃',
  reset_required:'当前会话已有资料，请先显式重置；旧草稿不会被覆盖',
  no_session:'请先准备 text/2 使用说明',
  legacy_draft:'旧资料块已被编辑或无法完整移除，请先自行清理并重置会话；其余草稿已保留',
  invalid_command:'不支持的 text/2 操作',
  invalid_reply:'请粘贴本轮完整助手回复；引用、示例或旧票据不会执行',
  native_unavailable:'本机读取或核验未完成，不表示没有历史；不会自动重试',
  invalid_bootstrap:'开局资料缺少可核验版本，请升级或重新连接',
  source_changed:'本机资料已变化，旧预览不能继续使用；请重置后重新开始',
  invalid_result:'本机只读结果未通过来源与正文核验',
  snapshot_unavailable:'网页无法可靠确认完整本轮消息，请手动复制和粘贴完整回复',
  confirmation_required:'请先手动发送完整资料，再明确确认本轮已发送',
  wrong_packet:'发送确认不属于当前完整资料包',
  not_disclosed:'请先复制本轮完整资料，再确认发送',
  send_gate:'请先发送当前完整资料；旧票据或处理中请求不能重放',
  budget_exhausted:'本轮读取预算不足，不能继续核验；不表示没有历史',
  disclosure_budget:'累计资料披露预算已用尽，没有复制或外发',
  packet_budget:'完整资料超过本轮资料包预算，没有截断或外发',
  paused:'接力已暂停，恢复后会重新核验',
  closed:'本轮已取消或完成',
  deadline:'本轮接力已超时，请重置后重新开始',
  timeout:'本机操作已超时，结果未知；不会自动重试',
  interrupted:'上次读取未确认完成，请明确重试并重新核验',
  no_failure:'没有可明确重试的失败动作',
  unknown_alias:'只能读取或引用本 run 已发送的来源编号',
  page_citation:'续页 P 不能作为来源引用，请引用原 S 编号',
  host_unavailable:'本轮操作未完成，请检查连接后再试',
});
const fail = code => { const error = new Error(messages[code] || messages.host_unavailable); error.code = code; throw error; };
const safeCode = (error, fallback='host_unavailable') => Object.hasOwn(messages,error?.code) ? error.code : fallback;
const ctx = (broker,engine,extra={}) => ({bindingKey:engine.bindingKey,now:()=>broker.api.now(),...extra});
function runtime(broker,tabId) {
  if (!runtimes.has(broker)) runtimes.set(broker,new Map());
  const tabs=runtimes.get(broker);
  if (!tabs.has(tabId)) tabs.set(tabId,{generation:0,writes:Promise.resolve(),active:new Set()});
  return tabs.get(tabId);
}
function tokenFor(broker,tabId,state) {
  const host=state.text_relay;
  return {nonce:state.nonce,sessionRef:state.session_ref,route:state.route,documentId:state.documentId,installationId:state.installation_id??null,
    runId:host?.engine.runId??null,epoch:host?.engine.epoch??null,hostEpoch:host?.hostEpoch??null,generation:runtime(broker,tabId).generation};
}
function assertCurrent(broker,tabId,state,token,{allowPaused=false,allowClosed=false,allowInvalidated=false}={}) {
  const host=state.text_relay;
  if (runtime(broker,tabId).generation!==token.generation || state.nonce!==token.nonce || state.session_ref!==token.sessionRef || state.route!==token.route || state.documentId!==token.documentId || (state.installation_id??null)!==token.installationId || (host?.engine.runId??null)!==token.runId || (host?.engine.epoch??null)!==token.epoch || (host?.hostEpoch??null)!==token.hostEpoch) fail('stale_operation');
  if (!allowInvalidated && host?.invalidated) fail('binding_changed');
  if (!allowClosed && (host?.engine.cancelled || host?.engine.completed)) fail('closed');
  if (!allowPaused && host?.engine.paused) fail('paused');
  return state;
}
async function current(broker,tabId,message,token,options) {
  return assertCurrent(broker,tabId,await broker.current(tabId,message),token,options);
}
// 只串行短存储提交，不把 cancel/pause 排在 native 或 DOM I/O 后面。
// 生命周期操作先同步提高 generation；即使先前 save 在途，最后一次写也必定是取消。
async function mutate(broker,tabId,message,token,fn,options) {
  const rt=runtime(broker,tabId);
  const work=rt.writes.catch(()=>{}).then(async()=>{
    const state=await current(broker,tabId,message,token,options);
    const result=fn(state);
    assertCurrent(broker,tabId,state,token,{allowClosed:true,allowPaused:true,allowInvalidated:true});
    await broker.api.save(tabId,state);
    return {state,result};
  });
  rt.writes=work.catch(()=>{});
  return work;
}
async function saveEngine(broker,tabId,message,token,engine,sequence,update=()=>{}) {
  return mutate(broker,tabId,message,token,state=>{
    if (state.text_relay.engine.sequence!==sequence) fail('stale_operation');
    state.text_relay.engine=engine; update(state.text_relay,state);
  });
}
function grantBinding(state,connection) {
  const grant=connection?.automation;
  if (!connection || connection.state==='revoked' || connection.setup_required===true || !/^[a-f0-9]{64}$/u.test(connection.connection_id??'') || !grant?.recall || !grant.provider_disclosure || grant.platform!==siteFor(state.route).id || !Number.isSafeInteger(grant.permission_revision) || grant.permission_revision<1 || !Array.isArray(connection.read_scopes) || !connection.read_scopes.length || connection.read_scopes.some(scope=>typeof scope!=='string'||!scope)) fail('grant_required');
  return {connection_id:connection.connection_id,grant_id:grant.grant_id??null,permission_revision:grant.permission_revision,read_scopes:[...new Set(connection.read_scopes)].sort(),platform:grant.platform,
    nonce:state.nonce,session_ref:state.session_ref,route:state.route,document_id:state.documentId,installation_id:state.installation_id??null,extension_id:state.extension_id??null};
}
async function freshGrant(broker,tabId,message,token,options) {
  let state=await current(broker,tabId,message,token,options),connection;
  try { connection=await broker.userNative(tabId,message,state,'connection',{}); }
  catch { await current(broker,tabId,message,token,options); fail('native_unavailable'); }
  state=await current(broker,tabId,message,token,options);
  const binding=grantBinding(state,connection);
  const bindingKey=await broker.fingerprint('text_binding',binding);
  state=await current(broker,tabId,message,token,options);
  if (state.text_relay && state.text_relay.engine.bindingKey!==bindingKey) fail('binding_changed');
  await mutate(broker,tabId,message,token,value=>{value.connection=connection;},options);
  return {binding,bindingKey,revision:binding.permission_revision};
}
async function dataRead(broker,tabId,message,token,request,{target='binding',precharged=false}={}) {
  const grant=await freshGrant(broker,tabId,message,token);
  let readId=null;
  if (!precharged) ({result:readId}=await mutate(broker,tabId,message,token,state=>reserveTextRead(state.text_relay.engine,ctx(broker,state.text_relay.engine),target).id));
  const state=await current(broker,tabId,message,token);
  let result;
  try { result=await broker.userNative(tabId,message,state,'authorized_read',{action:request.action,arguments:clone(request.arguments),permission_revision:grant.revision}); }
  catch { await current(broker,tabId,message,token); fail('native_unavailable'); }
  await current(broker,tabId,message,token);
  await freshGrant(broker,tabId,message,token);
  return {result,readId};
}
async function bootstrapFingerprint(broker,result) {
  if (typeof result?.bootstrap_version!=='string' || !result.bootstrap_version) fail('invalid_bootstrap');
  return broker.fingerprint('bootstrap',result);
}
const bootstrapRequest = () => ({action:'bootstrap',arguments:{}});
async function prepare(broker,tabId,message,token,bootstrap=null) {
  let state=await current(broker,tabId,message,token),engine=clone(state.text_relay.engine),sequence=engine.sequence;
  try { await prepareTextPacket(engine,{packetId:`p_${newNonce()}`,ticket:newTextTicket(),bootstrap},ctx(broker,engine)); }
  catch (error) {
    if (error.code!=='packet_budget' || !engine.ready) throw error;
    discardTextResultAsBudget(engine,ctx(broker,engine));
    await prepareTextPacket(engine,{packetId:`p_${newNonce()}`,ticket:newTextTicket()},ctx(broker,engine));
  }
  await saveEngine(broker,tabId,message,token,engine,sequence,host=>{host.sentMessageId=null;host.error=null;});
}
async function revalidatePacket(broker,tabId,message,token) {
  let state=await current(broker,tabId,message,token),host=state.text_relay;
  if (!host.engine.packet || !host.packetRequest || !host.packetFingerprint) fail('source_changed');
  const target=packetFreshnessTarget(host.engine),checks=[];
  // 首包的 source request 本身就是 bootstrap，不能重复计费或伪造少一次调用。
  const fresh=await dataRead(broker,tabId,message,token,host.packetRequest,{target});checks.push(fresh.readId);
  const fingerprint=host.packetRequest.action==='bootstrap'?await bootstrapFingerprint(broker,fresh.result):await broker.fingerprint(host.packetRequest.action,fresh.result);
  if (fingerprint!==host.packetFingerprint) fail('source_changed');
  if (host.packetRequest.action!=='bootstrap') {
    const background=await dataRead(broker,tabId,message,token,bootstrapRequest(),{target});checks.push(background.readId);
    if (await bootstrapFingerprint(broker,background.result)!==host.bootstrapFingerprint) fail('source_changed');
  }
  await freshGrant(broker,tabId,message,token);
  await mutate(broker,tabId,message,token,value=>{
    const engine=value.text_relay.engine;
    for (const id of checks) if (!confirmTextFreshness(engine,id,ctx(broker,engine))) fail('stale_operation');
  });
}
async function validateFinalSource(broker,engine,attempt,required,result) {
  // 用同一纯核验器验证 rcs1、正文散列、范围与 metadata；只在隔离副本上投影。
  const checked=clone(engine);
  checked.pending={...attempt,action:'fetch',tool:'recall_fetch',source:required.id,args:{id:required.locator}};
  let accepted;
  try { accepted=await acceptTextResult(checked,attempt.id,result,ctx(broker,checked,{verified:true})); }
  catch { fail('invalid_result'); }
  if (!accepted || result.results.length!==1 || result.results[0].id!==required.locator || result.results[0].snapshot!==required.snapshot || result.results[0].ref!==required.ref) fail('source_changed');
}
async function executeAttempt(broker,tabId,message,token,attempt) {
  const rt=runtime(broker,tabId);rt.active.add(attempt.id);
  try {
    let state=await current(broker,tabId,message,token),host=state.text_relay;
    if (attempt.action==='final') {
      const required=textSourceRequirements(host.engine,'final');
      // 先保证整组真实授权读的额度，不在读取一半后冒充全部来源已核验。
      if (host.engine.budget.reads+1+required.length>host.engine.limits.reads) fail('budget_exhausted');
      const target=finalFreshnessTarget(host.engine),checks=[];
      const background=await dataRead(broker,tabId,message,token,bootstrapRequest(),{target});checks.push(background.readId);
      if (await bootstrapFingerprint(broker,background.result)!==host.bootstrapFingerprint) fail('source_changed');
      for (const source of required) {
        const fresh=await dataRead(broker,tabId,message,token,{action:'recall_fetch',arguments:{id:source.locator}},{target});checks.push(fresh.readId);
        state=await current(broker,tabId,message,token);
        await validateFinalSource(broker,state.text_relay.engine,attempt,source,fresh.result);
      }
      await freshGrant(broker,tabId,message,token);
      await mutate(broker,tabId,message,token,value=>{
        const engine=value.text_relay.engine;
        for (const id of checks) if (!confirmTextFreshness(engine,id,ctx(broker,engine))) fail('stale_operation');
        if (!completeTextFinal(engine,attempt.id,ctx(broker,engine))) fail('stale_operation');
        value.text_relay.error=null;
      },{allowClosed:true});
    } else {
      const request={action:attempt.tool,arguments:clone(attempt.args)};
      const {result}=await dataRead(broker,tabId,message,token,request,{precharged:true});
      const fingerprint=await broker.fingerprint(request.action,result);
      state=await current(broker,tabId,message,token);
      const engine=clone(state.text_relay.engine),sequence=engine.sequence;
      let accepted;
      try { accepted=await acceptTextResult(engine,attempt.id,result,ctx(broker,engine,{verified:true})); }
      catch { fail('invalid_result'); }
      if (!accepted) fail('stale_operation');
      await saveEngine(broker,tabId,message,token,engine,sequence,host=>{host.packetRequest=request;host.packetFingerprint=fingerprint;});
      await prepare(broker,tabId,message,token);
    }
  } finally { rt.active.delete(attempt.id); }
}
async function snapshot(broker,tabId,message,token) {
  const state=await current(broker,tabId,message,token),host=state.text_relay,packet=host.engine.packet;
  let response;
  try { response=await broker.api.content(tabId,{kind:'text_snapshot',route:state.route,nonce:state.nonce,session_ref:state.session_ref,packet:packet?{id:packet.id,text:packet.text,hash:packet.hash}:null,sent_message_id:host.sentMessageId??packet?.sent?.messageId??null},state.documentId); }
  catch { fail('snapshot_unavailable'); }
  await current(broker,tabId,message,token);
  if (!response?.ok || !response.result) fail('snapshot_unavailable');
  return response.result;
}
async function observeSent(broker,tabId,message,token,observation) {
  if (observation.status!=='stable' || !observation.sent) return false;
  const {result}=await mutate(broker,tabId,message,token,state=>{
    const host=state.text_relay,engine=host.engine,packet=engine.packet,sent=observation.sent;
    if (!packet || sent.packet_hash!==packet.hash || typeof sent.message_id!=='string' || !sent.message_id || byteLength(sent.message_id)>1024 || !packet.disclosureAttempts) return false;
    confirmTextSent(engine,{packetId:packet.id,packetHash:packet.hash,kind:'observed',messageId:sent.message_id},ctx(broker,engine));
    host.sentMessageId=sent.message_id;host.error=null;return true;
  });
  return result;
}
async function recordError(broker,tabId,message,token,code) {
  if (!token?.runId || ['stale_operation','closed','paused'].includes(code)) return;
  try {
    await mutate(broker,tabId,message,token,state=>{
      const host=state.text_relay,engine=host.engine;
      if (engine.pending) {
        if (!expireTextAttempt(engine,ctx(broker,engine))) acceptTextError(engine,engine.pending.id,{code,message:messages[code]},ctx(broker,engine));
      }
      if (['grant_required','binding_changed','source_changed','invalid_bootstrap'].includes(code)) {host.invalidated=true;engine.phase='blocked';engine.freshness=null;}
      else if (!['invalid_reply','wrong_packet','confirmation_required','not_disclosed','send_gate','unknown_alias','page_citation','no_failure'].includes(code)) engine.phase='blocked';
      host.error=code;
    });
  } catch { /* 已切换/取消的会话不能被旧错误复活。 */ }
}
async function recoverInterrupted(broker,tabId,message,token) {
  const state=await current(broker,tabId,message,token),pending=state.text_relay?.engine.pending;
  if (!pending || runtime(broker,tabId).active.has(pending.id)) return;
  await mutate(broker,tabId,message,token,value=>{
    const engine=value.text_relay.engine;
    if (!expireTextAttempt(engine,ctx(broker,engine))) acceptTextError(engine,pending.id,{code:'interrupted',message:messages.interrupted},ctx(broker,engine));
    value.text_relay.error=engine.failure?.code??'interrupted';
  });
}

export function textPublic(state) {
  const host=state?.text_relay;if (!host) return null;
  const engine=host.engine,packet=host.invalidated?null:engine.packet;
  const details={idle:'等待准备本轮资料',preparing:'正在按完整助手请求核验资料',awaiting_user_send:'完整资料已备好，请复制、检查并手动发送',waiting_reply:host.manualFallback?'请粘贴本轮完整助手回复继续':'等待本轮完整助手回复',result_ready:'AI 已标记本轮回答完成；回答内容仍需你判断',paused:messages.paused,blocked:engine.cancelled?'本轮接力已取消':messages[host.error]??'本轮已阻塞，请检查连接或明确重试'};
  return {protocol:'text/2',phase:engine.phase,cancelled:engine.cancelled,completed:engine.completed,detail:messages[host.error]??details[engine.phase],round:engine.round,budget:clone(engine.budget),limits:clone(engine.limits),
    packet:packet?{id:packet.id,text:packet.text,hash:packet.hash,bytes:packet.bytes,sent:packet.sent?clone(packet.sent):null}:null,
    final:engine.final?clone(engine.final):null,canRetry:!!engine.failure&&!engine.paused&&!engine.cancelled&&!host.invalidated,error:host.error??null,manual_fallback:!!host.manualFallback};
}

export async function textCommand(broker,tabId,message) {
  // 由 Broker 完成 sender/document 验证及普通操作串行。取消/暂停须放 serial 外。
  const command=message.command;
  if (['cancel','pause'].includes(command)) runtime(broker,tabId).generation++;
  let state=await broker.current(tabId,message),token=tokenFor(broker,tabId,state);
  try {
    if (command==='inspect') return textPublic(state);
    if (['cancel','pause'].includes(command)) {
      if (!state.text_relay) fail('no_session');
      // 停止不依赖已撤销的本机权限；允许已失效 run 被用户关闭。
      const rt=runtime(broker,tabId);
      const work=rt.writes.catch(()=>{}).then(async()=>{
        const latest=await broker.current(tabId,message),host=latest.text_relay;
        if (!host || host.engine.runId!==token.runId) fail('stale_operation');
        if (command==='cancel') cancelTextSession(host.engine);
        else if (!host.engine.cancelled&&!host.engine.completed) pauseTextSession(host.engine,ctx(broker,host.engine));
        host.hostEpoch++;host.error=null;
        await broker.api.save(tabId,latest);return textPublic(latest);
      });rt.writes=work.catch(()=>{});return await work;
    }
    if (command==='start') {
      if (state.text_relay) fail('reset_required');
      // 显式选择 text/2 可替换旧 prepared 预览；draft 只删除完整未编辑的自有块。
      const previous=state.preview;
      if (previous && !['prepared','draft'].includes(previous.delivery)) fail('reset_required');
      if (previous?.delivery==='draft') {
        let removed;
        try { removed=await broker.api.content(tabId,{kind:'remove',route:state.route,nonce:state.nonce,session_ref:state.session_ref,id:previous.id},state.documentId); }
        catch { fail('legacy_draft'); }
        await current(broker,tabId,message,token);
        if (!removed?.ok) fail('legacy_draft');
      }
      await mutate(broker,tabId,message,token,value=>{
        if (JSON.stringify(value.preview??null)!==JSON.stringify(previous??null)) fail('stale_operation');
        value.preview=null;value.bootstrap=null;value.relay_sent_message=null;
      });
      const grant=await freshGrant(broker,tabId,message,token);
      const engine=createTextSession({runId:`r_${newNonce()}`,bindingKey:grant.bindingKey,now:broker.api.now()});
      // 初次创建没有旧 run，可在短写锁内安装；随后立即换为新 run token。
      const rt=runtime(broker,tabId);
      const work=rt.writes.catch(()=>{}).then(async()=>{
        const latest=await current(broker,tabId,message,token);
        if (latest.text_relay || latest.preview || latest.bootstrap) fail('reset_required');
        latest.text_relay={engine,grantBinding:grant.binding,hostEpoch:0,bootstrapFingerprint:null,packetRequest:null,packetFingerprint:null,sentMessageId:null,error:null,manualFallback:true};
        await broker.api.save(tabId,latest);return latest;
      });rt.writes=work.catch(()=>{});state=await work;token={...tokenFor(broker,tabId,state),generation:token.generation};
      const {result}=await dataRead(broker,tabId,message,token,bootstrapRequest());
      const fingerprint=await bootstrapFingerprint(broker,result);
      await mutate(broker,tabId,message,token,value=>{const host=value.text_relay;host.bootstrapFingerprint=fingerprint;host.packetRequest=bootstrapRequest();host.packetFingerprint=fingerprint;});
      await prepare(broker,tabId,message,token,result);
    } else {
      if (!state.text_relay) fail('no_session');
      if (command==='resume') {
        await freshGrant(broker,tabId,message,token,{allowPaused:true});
        await mutate(broker,tabId,message,token,value=>{resumeTextSession(value.text_relay.engine,ctx(broker,value.text_relay.engine));value.text_relay.error=null;},{allowPaused:true});
      } else {
        await current(broker,tabId,message,token);
        await recoverInterrupted(broker,tabId,message,token);
        await freshGrant(broker,tabId,message,token);
        if (command==='copy') {
          await revalidatePacket(broker,tabId,message,token);
          const {result}=await mutate(broker,tabId,message,token,value=>{
            const host=value.text_relay,engine=host.engine,packet=engine.packet;
            const text=capsuleText({nonce:value.nonce,id:packet.id,text:packet.text});
            reserveTextDisclosure(engine,ctx(broker,engine),{overheadBytes:byteLength(text)-packet.bytes});host.error=null;
            return {text,request_id:packet.id,packet_id:packet.id,packet_hash:packet.hash};
          });
          await current(broker,tabId,message,token);return result;
        }
        if (command==='delivered') {
          state=await current(broker,tabId,message,token);
          const packet=state.text_relay.engine.packet;
          if (!packet || message.packet_id!==packet.id || message.packet_hash!==packet.hash) fail('wrong_packet');
          if (message.confirmed===true) {
            await mutate(broker,tabId,message,token,value=>{const host=value.text_relay;confirmTextSent(host.engine,{packetId:message.packet_id,packetHash:message.packet_hash,kind:'manual'},ctx(broker,host.engine));host.manualFallback=true;host.error=null;});
          } else if (!await observeSent(broker,tabId,message,token,await snapshot(broker,tabId,message,token))) fail('confirmation_required');
        } else if (command==='execute'||command==='retry') {
          const {result:attempt}=await mutate(broker,tabId,message,token,value=>{
            const engine=value.text_relay.engine;
            try { return command==='retry'?retryTextAttempt(engine,ctx(broker,engine)):consumeTextReply(engine,message.text,ctx(broker,engine)); }
            catch(error) { fail(safeCode(error,'invalid_reply')); }
          });
          await executeAttempt(broker,tabId,message,token,attempt);
        } else if (command!=='copy') fail('invalid_command');
      }
    }
    return textPublic(await current(broker,tabId,message,token,{allowClosed:true,allowPaused:true}));
  } catch(error) {
    const code=safeCode(error);await recordError(broker,tabId,message,token,code);fail(code);
  }
}

export async function textCycle(broker,tabId,message) {
  let state=await broker.current(tabId,message);if (!state.text_relay) return {status:'idle'};
  let token=tokenFor(broker,tabId,state),engine=state.text_relay.engine;
  if (engine.paused || engine.cancelled || engine.completed || state.text_relay.invalidated) return {status:engine.phase,account_identity:'unverified'};
  try {
    await recoverInterrupted(broker,tabId,message,token);
    await freshGrant(broker,tabId,message,token);
    const observation=await snapshot(broker,tabId,message,token);
    if (observation.status!=='stable') {
      await mutate(broker,tabId,message,token,value=>{value.text_relay.manualFallback=observation.status==='manual_fallback';});
      return {status:observation.status==='manual_fallback'?'manual_fallback':'waiting_reply',account_identity:'unverified'};
    }
    await observeSent(broker,tabId,message,token,observation);
    state=await current(broker,tabId,message,token);engine=state.text_relay.engine;
    const assistant=observation.assistant;
    // 人工确认没有强用户消息身份，不能据此接纳网页中残留的旧助手回复。
    if (!engine.packet?.sent || !state.text_relay.sentMessageId || !assistant?.complete || typeof assistant.id!=='string' || !assistant.id || typeof assistant.text!=='string' || engine.pending || engine.failure) return {status:engine.phase,account_identity:'unverified'};
    let attempt;
    try {
      ({result:attempt}=await mutate(broker,tabId,message,token,value=>consumeTextReply(value.text_relay.engine,assistant.text,ctx(broker,value.text_relay.engine))));
    } catch(error) {
      // 旧、引用、示例、重放、未完成完整帧均不执行，不把 DOM 内容当授权。
      if (['stale_operation','paused','closed'].includes(error?.code)) throw error;
      return {status:'waiting_reply',account_identity:'unverified'};
    }
    await executeAttempt(broker,tabId,message,token,attempt);
    state=await current(broker,tabId,message,token,{allowClosed:true});
    return {status:state.text_relay.engine.phase,account_identity:'unverified'};
  } catch(error) {
    const code=safeCode(error);await recordError(broker,tabId,message,token,code);fail(code);
  }
}
