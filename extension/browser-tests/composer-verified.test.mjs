// 原创合成页面上的真实 Chromium；所有请求都被阻断，不代表厂商网页/账号已验收。
import { before, after, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';
let browser;
before(async()=>{browser=await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH?{executablePath:process.env.RECALLCARD_CHROMIUM_PATH}:{});});
after(async()=>{await browser?.close();});
async function pageFor(html) {
  const page=await browser.newPage();await page.route('**/*',route=>route.abort());
  await page.setContent('<!doctype html><meta charset="utf-8"><style>body{font:16px system-ui;margin:40px;color:#30354c}textarea,[contenteditable]{display:block;width:700px;min-height:250px;padding:16px;white-space:pre-wrap}h1{font-size:23px}</style><h1>合成填框验收 · 保留草稿，完整回读，不发送</h1>'+html);
  for(const name of ['site-adapters.js','composer-adapter.js'])await page.addScriptTag({path:fileURLToPath(new URL('../'+name,import.meta.url))});
  return page;
}
for(const [label,url,html] of [
  ['ChatGPT mobile','https://chatgpt.com/','<textarea id="mobile-composer-prompt"></textarea>'],
  ['DeepSeek','https://chat.deepseek.com/','<textarea class="ds-scroll-area" name="search"></textarea>'],
  ['Qwen','https://chat.qwen.ai/','<textarea class="message-input-textarea"></textarea>'],
  ['Z.ai','https://chat.z.ai/','<textarea id="chat-input"></textarea>'],
  ['ChatGPT rich','https://chatgpt.com/','<div id="prompt-textarea" class="ProseMirror" contenteditable="true"><p>已有富文本草稿🙂</p></div>']
]) test(label+'：框架刷新后全文一致且不执行发送',async()=>{
  const page=await pageFor(html);
  try {
    const result=await page.evaluate(async({url})=>{
      const adapter=new RecallCardComposerAdapter(document,{href:url}),node=adapter.find();
      if(node.tagName==='TEXTAREA')node.value='原草稿🙂\n必须保留的第二行';
      const before=adapter.text(node),events=[];for(const type of ['click','keydown','submit'])document.addEventListener(type,()=>events.push(type));
      const result=await adapter.insertVerified({id:'synthetic',nonce:'d'.repeat(48),text:'合成背景：已核对来源。\n最后一行🙂必须保留。'});
      return {result,before,after:adapter.text(node),events};
    },{url});
    assert.equal(result.result.full_text_verified,true);assert.equal(result.result.existing_draft_preserved,true);assert.ok(result.after.startsWith(result.before));assert.match(result.after,/最后一行🙂必须保留。/);assert.deepEqual(result.events,[]);
    if(label==='ChatGPT rich'){await mkdir('artifacts',{recursive:true});await page.screenshot({path:'artifacts/synthetic-rich-draft-verified.png'});}
  } finally {await page.close();}
});
test('真实 DOM 异步截断尾部、改写原草稿、重建节点均不能伪报成功',async()=>{
  for(const mode of ['tail','original','replace']) {
    const page=await pageFor('<textarea id="prompt-textarea"></textarea>');
    try {
      const result=await page.evaluate(async mode=>{
        const adapter=new RecallCardComposerAdapter(document,{href:'https://chatgpt.com/'}),node=adapter.find();node.value='不能被替换的原草稿';
        node.addEventListener('input',()=>setTimeout(()=>{if(mode==='tail')node.value=node.value.slice(0,-1);if(mode==='original')node.value=node.value.replace('原草稿','坏改写');if(mode==='replace'){const next=node.cloneNode();next.value=node.value;node.replaceWith(next);}},0),{once:true});
        let error='',retry='';try{await adapter.insertVerified({id:'first',nonce:'e'.repeat(48),text:'合成资料'+'.'.repeat(500)+'必须检查尾部'});}catch(e){error=e.message;}
        const beforeRetry=document.querySelector('textarea').value;
        try{await adapter.insertVerified({id:'retry',nonce:'e'.repeat(48),text:'不能再追加'});}catch(e){retry=e.message;}
        return {error,retry,unchanged:beforeRetry===document.querySelector('textarea').value};
      },mode);
      assert.match(result.error,/不一致|输入框已变化/);assert.match(result.retry,/不会再自动写入/);assert.equal(result.unchanged,true);
    } finally {await page.close();}
  }
});
test('目标被遮挡和存在两个可见编辑器时零写入，保留手动复制退路',async()=>{
  for(const mode of ['overlay','ambiguous']) {
    const page=await pageFor('<textarea id="prompt-textarea">不应改动的草稿</textarea>'+(mode==='overlay'?'<div style="position:fixed;inset:0;background:#fff8">合成遮挡层</div>':'<textarea id="mobile-composer-prompt">另一份草稿</textarea>'));
    try {
      const result=await page.evaluate(async()=>{const adapter=new RecallCardComposerAdapter(document,{href:'https://chatgpt.com/'});let error='';try{await adapter.insertVerified({id:'blocked',nonce:'f'.repeat(48),text:'不得写入'});}catch(e){error=e.message;}return{error,text:document.querySelector('textarea').value};});
      assert.match(result.error,/遮挡|唯一/);assert.equal(result.text,'不应改动的草稿');
    } finally {await page.close();}
  }
});
