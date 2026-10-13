// 原创合成站点；实际 Chromium、content、overlay、Broker/host。无真实账号或模型。
import {before,after,test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
import {chromium} from 'playwright';
import {relayHarness} from './fixtures/relay-harness.mjs';
let browser;
before(async()=>{browser=await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH?{executablePath:process.env.RECALLCARD_CHROMIUM_PATH}:{});});
after(async()=>{await browser?.close();});
const hash='a'.repeat(64),epoch='b'.repeat(64),config='c'.repeat(64),scopes='d'.repeat(64);
const first='合成旧约定：周二回访。',second='合成更正：改为周四回访。';
const firstBytes=Buffer.byteLength(first),total=firstBytes+Buffer.byteLength(second);
const locator=offset=>`rcs1|synthetic|event:evt_synthetic|${hash}|${epoch}|${config}|${scopes}|${offset}`;
const row=(text,offset)=>({id:locator(offset),ref:'event:evt_synthetic',kind:'event',role:'user',scope:'synthetic',snapshot:hash,text,occurred_at:null,text_range:{start_byte:offset,end_byte:offset+Buffer.byteLength(text),total_bytes:total,projection:'event.text'}});
function nativeResult(request){
  if(request.action!=='authorized_read')return;
  const {action,arguments:args}=request.arguments;
  if(action==='bootstrap')return{bootstrap_version:'synthetic-v1',stable_text:'合成偏好：回答要依据原文。',refs:[],scopes:['synthetic']};
  if(action==='recall_search')return{status:'partial',results:[row(first,0)],coverage:{scopes:[{scope:'synthetic',query_encoding:'computed'}]},truncated:true};
  if(action==='recall_fetch'){
    const offset=Number(args.id.split('|').at(-1));
    assert.ok([0,firstBytes].includes(offset),'只能取已返回的源或续页');
    return{status:offset?'complete':'partial',results:[row(offset?second:first,offset)],truncated:!offset,next_id:offset?null:locator(firstBytes)};
  }
}
const engine=h=>h.state().text_relay.engine;
const frame=(h,action,body)=>`RECALLCARD/2 ${engine(h).ticket.value}\n${JSON.stringify(action)}${body===undefined?'':`\n\n${body}`}\nEND RECALLCARD`;
async function start(h){
  await h.page.locator('#recallcard-relay').getByRole('button',{name:'开始简洁接力',exact:true}).click();
  await h.page.waitForFunction(()=>document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('完整资料已备好'));
  assert.equal(engine(h).phase,'awaiting_user_send');
}
async function copyAndSend(h,id){
  const before=await h.page.evaluate(()=>clipboardWrites.length);
  await h.page.locator('#recallcard-relay').getByRole('button',{name:'复制本轮资料',exact:true}).click();
  await h.page.waitForFunction(count=>clipboardWrites.length===count+1,before);
  const text=await h.page.evaluate(()=>clipboardWrites.at(-1));
  assert.ok(text.includes(engine(h).packet.text),'复制全文与账本逐字一致');
  await h.turn('user',id,text); // 合成用户已发送的完整 DOM turn，不点击网站发送。
  await h.settle();assert.equal(engine(h).packet.sent.kind,'observed');
  return text;
}
for(const platform of ['chatgpt','deepseek'])test(`${platform} 简洁接力真实DOM：开局→search→S读取→P续页→final提醒，草稿不动`,async()=>{
  const h=await relayHarness(browser,platform,{textRelay:true,nativeResult});
  try{
    await start(h);assert.match(engine(h).packet.text,/合成偏好/);
    await copyAndSend(h,'u1');
    await h.turn('assistant','a1',frame(h,{action:'search',query:'合成回访何时'}));await h.settle();
    assert.equal(engine(h).round,2);assert.equal(engine(h).sources.S1.sent,false);
    await copyAndSend(h,'u2');
    await h.turn('assistant','a2',frame(h,{action:'fetch',id:'S1'}));await h.settle();
    assert.equal(engine(h).pages.P1.source,'S1');await copyAndSend(h,'u3');
    await h.turn('assistant','a3',frame(h,{action:'fetch',id:'P1'}));await h.settle();
    assert.match(engine(h).packet.text,/改为周四/);await copyAndSend(h,'u4');
    const final=frame(h,{action:'final'},'更正后的约定是周四回访。[S1]');
    await h.turn('assistant','a4',final);await h.settle();
    assert.equal(engine(h).phase,'result_ready');assert.equal(engine(h).final.body,'更正后的约定是周四回访。[S1]');
    assert.match(await h.page.locator('#recallcard-relay strong').innerText(),/回答已就绪/);
    const reads=h.calls.filter(r=>r.action==='authorized_read').length;
    await h.settle();assert.equal(h.calls.filter(r=>r.action==='authorized_read').length,reads,'终态不会重放读取');
    assert.equal(await h.page.locator('main textarea').inputValue(),'合成问题');
    assert.equal(await h.page.evaluate(()=>websiteSends),0);assert.equal(await h.page.evaluate(()=>clipboardReads),0);assert.equal(h.writes.length,0);
    assert.equal(engine(h).budget.reads,reads);
    await mkdir(new URL('../artifacts/',import.meta.url),{recursive:true});
    await h.page.screenshot({path:new URL(`../artifacts/text2-${platform}-complete.png`,import.meta.url).pathname,fullPage:true});
    console.log(JSON.stringify({suite:'text2-synthetic-web',platform,complete:true,rounds:engine(h).round,real_native:false,model_executed:false,automatic_sends:0,original_draft_preserved:true,actual_read_calls:reads}));
  }finally{await h.page.close();}
});
test('整帧仍在生成和代码/引用示例不触发；完整人工粘贴可回退，取消不回写',async()=>{
  const h=await relayHarness(browser,'chatgpt',{textRelay:true,nativeResult});
  try{
    await start(h);await copyAndSend(h,'u1');
    const request=frame(h,{action:'search',query:'合成问题'});
    await h.turn('assistant','a1',request,{complete:false});await h.settle();
    assert.equal(engine(h).round,1);
    await h.page.locator('[data-message-id="a1"]').evaluate(node=>{node.setAttribute('data-is-streaming','false');const pre=document.createElement('pre');pre.textContent=node.textContent;node.replaceChildren(pre);});
    await h.settle();assert.equal(engine(h).round,1);
    await h.page.locator('#recallcard-relay').getByText('网页改版或手动接力',{exact:true}).click();
    await h.page.locator('#recallcard-relay').getByRole('textbox',{name:'粘贴完整 AI 请求或回答'}).fill(request);
    await h.page.locator('#recallcard-relay').getByRole('button',{name:'处理已粘贴内容'}).click();
    await h.page.waitForFunction(()=>document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('第 2 轮'));
    assert.equal(engine(h).round,2);
    await h.page.locator('#recallcard-relay').getByRole('button',{name:'结束本次简洁接力'}).click();
    await h.page.waitForFunction(()=>document.querySelector('#recallcard-relay').shadowRoot.textContent.includes('已取消'));
    const calls=h.calls.length;await h.settle();assert.equal(h.calls.length,calls);assert.equal(engine(h).cancelled,true);
    assert.equal(await h.page.locator('main textarea').inputValue(),'合成问题');
  }finally{await h.page.close();}
});
