// 仅使用自建最小页面；不登录、安装扩展或访问真实厂商会话。
import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright';
let browser;
before(async()=>{browser=await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH?{executablePath:process.env.RECALLCARD_CHROMIUM_PATH}:{});});
after(async()=>{await browser?.close();});
const scripts=['site-adapters.js','composer-adapter.js','conversation-format.js','conversation-adapters.js'];
async function pageFor(html, url='https://chatgpt.com/c/synthetic'){
  const page=await browser.newPage();
  await page.route('**/*',route=>route.fulfill({contentType:'text/html',body:'<!doctype html><meta charset="utf-8"><title>合成会话</title><main>'+html+'</main>'}));
  await page.goto(url);
  for(const name of scripts)await page.addScriptTag({path:fileURLToPath(new URL('../'+name,import.meta.url))});
  return page;
}
const sample='<div data-message-author-role="user" data-message-id="stable-user"><p>合成问题</p></div><div data-message-author-role="assistant" data-message-id="stable-ai"><p>合成回答</p></div><textarea id="mobile-composer-prompt">已有草稿</textarea>';
test('ChatGPT可见普通文字、显式角色与真实DOM时间；隐藏/推理/控件不会混入',async()=>{
  const page=await pageFor('<div data-message-author-role="user" data-message-id="u1"><p>保留用户问题</p><span hidden>隐藏内容</span><button>复制按钮</button><span style="opacity:0">透明内容</span><time datetime="2025-01-02T03:04:05Z">2025年1月2日</time></div><div data-message-author-role="assistant" data-message-id="a1"><p>可见回答</p><div data-testid="reasoning-content">不读取推理</div><pre><code>&lt;script&gt;文字&lt;/script&gt;</code></pre></div><div data-message-author-role="assistant" style="display:none">隐藏整条消息</div>');
  try{const value=await page.evaluate(()=>new RecallCardConversationAdapter(document).read('synthetic-capture','2026-01-01T00:00:00Z'));
    assert.equal(value.messages.length,2);assert.equal(value.messages[0].role,'user');assert.equal(value.messages[0].id,'u1');assert.equal(value.messages[0].occurred_at,'2025-01-02T03:04:05Z');assert.equal(value.messages[1].occurred_at,null);
    assert.ok(value.messages[1].text.includes('<script>文字</script>'));assert.doesNotMatch(value.messages.map(message=>message.text).join(''),/隐藏|透明|复制按钮|不读取推理/);assert.equal(value.coverage.complete,false);
  }finally{await page.close();}
});
test('DeepSeek独立结构：未知角色需确认，不按哈希类或消息奇偶猜测；排除推理',async()=>{
  const page=await pageFor('<div class="ds-message fbb737a4">合成用户消息</div><div class="ds-message"><div class="ds-think-content"><div class="ds-markdown">不导出思考过程</div></div><div class="ds-markdown"><p>最终回答</p></div></div><div class="ds-message fbb737a4">另一个未知角色片段</div>','https://chat.deepseek.com/a/chat/s/demo');
  try{const value=await page.evaluate(()=>new RecallCardConversationAdapter(document).read('synthetic-capture','2026-01-01T00:00:00Z'));
    assert.deepEqual(value.messages.map(message=>message.role),[null,'assistant',null]);assert.equal(value.messages[1].text,'最终回答');assert.equal(value.source.platform,'deepseek');assert.equal(value.source.conversation_id,'demo');assert.ok(value.messages.every(message=>message.metadata.weaker_identity));assert.match(value.coverage.warnings.join(''),/逐条确认/);
  }finally{await page.close();}
});
test('根页局部会话、重复网站ID、未知时间都有诚实标注',async()=>{
  const page=await pageFor('<div data-message-author-role="user" data-message-id="same">一</div><div data-message-author-role="assistant" data-message-id="same">二<time datetime="昨天">昨天</time></div>','https://chatgpt.com/');
  try{const value=await page.evaluate(()=>new RecallCardConversationAdapter(document).read('synthetic-capture','2026-01-01T00:00:00Z'));
    assert.equal(value.source.conversation_id,'capture:synthetic-capture');assert.equal(value.messages[0].id,'same');assert.equal(value.messages[1].id,'synthetic-capture:2');assert.equal(value.messages[1].occurred_at,null);assert.equal(value.metadata.weaker_conversation_identity,true);
  }finally{await page.close();}
});
test('流式、无文字和未验证网站拒绝捕获，不生成空文件',async()=>{
  for(const [html,url,pattern] of [[sample+'<button aria-label="Stop generating">停止</button>','https://chatgpt.com/c/synthetic','生成'],['<div data-message-author-role="assistant"><img alt="图片文字不冒充原件"></div>','https://chatgpt.com/','没有'],['<textarea class="message-input-textarea"></textarea>','https://chat.qwen.ai/','暂未验证']]){
    const page=await pageFor(html,url);try{const error=await page.evaluate(()=>{try{new RecallCardConversationAdapter(document).read('capture','2026-01-01T00:00:00Z');return null;}catch(error){return error.message;}});assert.match(error,new RegExp(pattern));}finally{await page.close();}
  }
});
async function contentPage(){
  const page=await pageFor(sample);
  await page.evaluate(()=>{
    window.reads=0;window.events=[];
    const original=RecallCardConversationAdapter.prototype.read;
    RecallCardConversationAdapter.prototype.read=function(...args){window.reads++;return original.apply(this,args);};
    for(const event of ['click','keydown','submit'])document.addEventListener(event,()=>window.events.push(event));
    window.chrome={runtime:{id:'synthetic-extension',getURL:path=>'chrome-extension://synthetic-extension/'+path,onMessage:{addListener:listener=>window.listener=listener},sendMessage:async message=>({ok:true,result:{route:message.route,nonce:'a'.repeat(48),session_ref:'chatgpt:'+message.token}})}};
    window.sendContent=message=>new Promise(resolve=>window.listener(message,{id:chrome.runtime.id,url:chrome.runtime.getURL('background.js')},resolve));
  });
  await page.addScriptTag({path:fileURLToPath(new URL('../content.js',import.meta.url))});
  return page;
}
test('打开弹窗/绑定不读取消息，明确capture后才读取；导出前全文校验',async()=>{
  const page=await contentPage();try{
    const result=await page.evaluate(async()=>{
      const binding=(await sendContent({kind:'describe'})).result;const before=reads;
      const captured=(await sendContent({kind:'capture',...binding})).result;
      const selection=await sendContent({kind:'capture_select',...binding,capture_id:captured.capture_id,snapshot_hash:captured.metadata.snapshot_hash,selections:captured.messages.map(message=>({id:message.id,role:message.role}))});
      return{before,after:reads,selection,events,editor:document.querySelector('textarea').value};
    });
    assert.equal(result.before,0);assert.equal(result.after,2);assert.equal(result.selection.ok,true);assert.equal(result.selection.result.messages.length,2);assert.deepEqual(result.events,[]);assert.equal(result.editor,'已有草稿');
  }finally{await page.close();}
});
test('同URL的内容编辑、消息集合替换、角色ID变化均撤销旧预览与注入绑定',async()=>{
  for(const mode of ['text','set','id','composer']){
    const page=await contentPage();try{const result=await page.evaluate(async mode=>{
      const binding=(await sendContent({kind:'describe'})).result;
      const captured=(await sendContent({kind:'capture',...binding})).result;
      const node=document.querySelector('[data-message-author-role="assistant"]');
      if(mode==='text')node.querySelector('p').textContent='同一节点被编辑';
      if(mode==='set')node.replaceWith(node.cloneNode(true));
      if(mode==='id')node.setAttribute('data-message-id','different');
      if(mode==='composer'){const editor=document.querySelector('textarea');editor.replaceWith(editor.cloneNode(true));}
      await new Promise(resolve=>setTimeout(resolve,0));
      return await sendContent({kind:'capture_select',...binding,capture_id:captured.capture_id,snapshot_hash:captured.metadata.snapshot_hash,selections:captured.messages.map(message=>({id:message.id,role:message.role}))});
    },mode);assert.equal(result.ok,false,mode);assert.match(result.error,/变化|重新打开/);}finally{await page.close();}
  }
});
async function popupPage(){
  const page=await browser.newPage({viewport:{width:520,height:1100}});
  const files=new Map();for(const name of ['popup.html','popup.css','popup.js','site-adapters.js','conversation-format.js'])files.set(name,await readFile(new URL('../'+name,import.meta.url),'utf8'));
  const fixture=JSON.parse(await readFile(new URL('../tests/fixtures/conversation.json',import.meta.url),'utf8'));
  await page.route('**/*',route=>{const name=new URL(route.request().url()).pathname.slice(1);if(!files.has(name))return route.abort();return route.fulfill({contentType:name.endsWith('.html')?'text/html':name.endsWith('.css')?'text/css':'text/javascript',body:files.get(name)});});
  await page.addInitScript(fixture=>{
    window.fixture=fixture;window.calls=[];window.downloadFailed=false;window.saveFailed=false;window.downloads=[];
    const state={platform:'deepseek',platform_name:'DeepSeek',route:'https://chat.deepseek.com/a/chat/s/demo',nonce:'a'.repeat(48),session_ref:'deepseek:synthetic',preview:null};
    window.chrome={tabs:{query:async()=>[{id:1,url:state.route}]},runtime:{sendMessage:async message=>{
      calls.push(message.kind);
      if(message.kind==='capture')return{ok:true,result:structuredClone(fixture)};
      if(message.kind==='conversation')return{ok:true,result:await RecallCardConversationFormat.select(fixture,message.selections)};
      if(message.kind==='connection')return{ok:false,error:'合成本机未连接，仍可导出'};
      if(message.kind==='capture_preview')return{ok:true,result:{approval_hash:'approved',connection_id:'a'.repeat(64),vault_name:'合成资料库A',scope:'personal',event_count:2,redacted_event_count:1,samples:['合成脱敏预览 <script>不会执行</script>']}};
      if(message.kind==='capture_save')return saveFailed?{ok:false,error:'未确认保存结果：合成本机断线'}:{ok:true,result:{events_added:2,events_seen:2}};
      return{ok:true,result:structuredClone(state)};
    }},downloads:{download:async options=>{if(downloadFailed)throw new Error('用户取消');downloads.push({text:await (await fetch(options.url)).text(),filename:options.filename});return 1;},search:async()=>[{state:'complete'}]}};
  },fixture);
  await page.goto('https://recallcard-test.invalid/popup.html');await page.locator('#capture').waitFor({state:'visible'});return page;
}
test('真实popup：本机失败后仍可导出完整JSON，取消下载不丢预览，不假报保存',async()=>{
  const page=await popupPage();try{
    await page.locator('#connection').click();await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('未连接'));
    await page.locator('#capture').click();await page.locator('#capture-panel').waitFor({state:'visible'});
    assert.equal(await page.locator('#messages article').count(),2);
    await page.evaluate(()=>window.downloadFailed=true);await page.locator('#export-json').click();await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('未确认完成'));
    assert.equal(await page.locator('#messages article').count(),2);
    await page.evaluate(()=>window.downloadFailed=false);await page.locator('#export-json').click();await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('下载完成'));
    const output=await page.evaluate(()=>JSON.parse(downloads[0].text));assert.equal(output.messages.length,2);assert.equal(output.source.platform,'deepseek');assert.equal(output.messages[1].occurred_at,null);
    await page.locator('#capture-preview').click();await page.locator('#save-panel').waitFor({state:'visible'});assert.equal((await page.evaluate(()=>calls)).includes('capture_save'),false);
    await page.evaluate(()=>window.saveFailed=true);await page.locator('#capture-save').click();await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('未确认保存结果'));assert.doesNotMatch(await page.locator('#save-status').innerText(),/已保存/);
    await page.evaluate(()=>window.saveFailed=false);await page.locator('#capture-save').click();await page.waitForFunction(()=>document.querySelector('#save-status').textContent.includes('已保存'));
    await page.locator('#prepare-handoff').click();await page.locator('#handoff-panel').waitFor({state:'visible'});assert.match(await page.locator('#handoff').inputValue(),/不是本次对话的新授权/);
    assert.equal(await page.locator('#save-samples script').count(),0);
  }finally{await page.close();}
});
