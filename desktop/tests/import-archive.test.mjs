// 合成备份的真实 Chromium DOM；原生命令由边界 fixture 响应，Rust 边界另有集成测试。
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { captureBrowserEvidence, workspaceAssets, idle, button, navigate, openImport, expandDetails } from './workspace-browser-helpers.mjs';
import { chromium } from '../../extension/node_modules/playwright/index.mjs';
const vault = {session_id:'synthetic',root:'/synthetic/vault',display_name:'合成资料库',scopes:['personal'],event_count:0,memory_count:0,health:{ok:true}};
const selection = {selection_id:'synthetic-archive',session_id:vault.session_id,file_name:'synthetic.zip',scope:'personal',file_hash:'synthetic-hash',byte_count:1024,
  coverage:{conversations_available:2,events_available:4,recognized_json_files:2,markdown_files_skipped:1,other_files_skipped:1,messages:{hidden_reasoning_messages_skipped:1},notes:['只导入当前分支可见文本，附件原件不导入']},
  conversations:[{source_id:'one',title:'合成第一会话',event_count:2,user_messages:1,assistant_messages:1,tool_messages:0},{source_id:'two',title:'<img src=x onerror=alert(1)>',event_count:2,user_messages:1,assistant_messages:1,tool_messages:0}]};
const preview = {preview_id:'synthetic-preview',file_name:selection.file_name,scope:'personal',byte_count:1024,event_count:2,redacted_event_count:0,truncated:false,warning:'尚未写入，确认后仅处理已选会话',coverage:selection.coverage,conversations:[selection.conversations[0]],samples:[{role:'user',source:{platform:'chatgpt-export'},occurred_at:'2023-11-14T22:13:20Z',content:'合成消息原文'}]};
const importedConversation = { session_ref: 'imported-first-conversation', title: '合成第一会话',
  platform: 'chatgpt-export', message_count: 2, captured_at: '2026-10-07T10:00:00Z',
  coverage: 'partial' };
let browser;
let assets;
before(async()=>{
  assets = await workspaceAssets();
  browser=await chromium.launch(process.env.RECALLCARD_CHROMIUM_PATH?{executablePath:process.env.RECALLCARD_CHROMIUM_PATH}:{});
});
after(async()=>{await browser?.close();});
async function fixture(t, chosen=selection){
  const page=await browser.newPage({locale:'zh-CN',timezoneId:'UTC'});page.setDefaultTimeout(5000);
  const calls=[];const errors=[];let failure=null;let imported=false;page.on('pageerror',e=>errors.push(e.message));
  t.after(async()=>{if(!t.passed||errors.length)await captureBrowserEvidence(page,`import-failure-${t.name}`);await page.close();assert.deepEqual(errors,[]);});
  await page.exposeFunction('__invoke',async(command,payload)=>{
    calls.push({command,payload});
    if(failure?.command===command){const error=failure.message;failure=null;throw new Error(error);}
    switch(command){
      case 'restore_workspace': case 'remember_workspace': return null;
      case 'list_import_jobs': return [];
      case 'choose_vault':return vault;
      case 'vault_status':return {...vault,event_count:imported?2:0};
      case 'list_conversations':return {conversations:imported?[importedConversation]:[],total:imported?1:0,next_offset:null};
      case 'conversation_messages':return {session_ref:importedConversation.session_ref,title:importedConversation.title,platform:importedConversation.platform,messages:[
        {ref:'event:imported-first-message',role:'user',text:'合成消息原文',occurred_at:'2023-11-14T22:13:20Z'},
        {ref:'event:imported-second-message',role:'assistant',text:'合成AI回复',occurred_at:'2023-11-14T22:13:21Z'}
      ],total:2,next_offset:null,offset:0,order_known:true};
      case 'cancel_previews':case 'return_import_selection':return null;
      case 'pick_import':return chosen;
      case 'preview_import_selection':return preview;
      case 'confirm_import':imported=true;return {events_added:2,events_seen:2,events_duplicates:0,conversation_refs:[importedConversation.session_ref],conversations:[importedConversation]};
      default:throw new Error(`未定义测试命令 ${command}`);
    }
  });
  await page.addInitScript(()=>{window.__TAURI__={core:{invoke:(command,payload)=>window.__invoke(command,payload)}};});
  await page.route('**/*',async route=>{
    const url=new URL(route.request().url());const body=url.origin==='https://recallcard.test'&&assets.get(url.pathname);
    if(!body)return route.abort();
    await route.fulfill({status:200,body,contentType:url.pathname.endsWith('.js')?'text/javascript; charset=utf-8':url.pathname.endsWith('.css')?'text/css; charset=utf-8':'text/html; charset=utf-8'});
  });
  await page.goto('https://recallcard.test/index.html');
  await button(page,'打开已有资料库');await idle(page);
  await openImport(page);
  await button(page,'选择文件并预览');await idle(page);
  return {page,calls,count:cmd=>calls.filter(c=>c.command===cmd).length,fail:(command,message)=>{failure={command,message}}};
}
async function chooseFirst(page){await page.getByRole('checkbox',{name:'选择会话：合成第一会话',exact:true}).check();}

test('ZIP 首先展示会话与覆盖范围，默认不选择且标题只作文本',async t=>{
  const {page,count}=await fixture(t);
  assert.equal(await page.getByRole('heading',{name:'选择本批要导入的会话'}).count(),1);
  assert.equal(await page.getByRole('checkbox').count(),2);
  assert.equal(await page.getByRole('checkbox').first().isChecked(),false);
  assert.equal(await page.getByRole('button',{name:'预览所选会话',exact:true}).isDisabled(),true);
  const text=await page.locator('.archive-selection').textContent();
  assert.match(text,/2 个会话 \/ 4 条消息/);assert.match(text,/Markdown 文件已跳过：1/);assert.match(text,/隐藏推理消息未收集：1/);
  assert.match(text,/<img src=x/);assert.equal(await page.locator('.archive-selection img').count(),0);
  assert.equal(count('preview_import_selection'),0);assert.equal(count('confirm_import'),0);
});

test('仅提交勾选会话、审查来源时间，确认一次后定位本批并可继续下一批',async t=>{
  const {page,calls,count}=await fixture(t);await chooseFirst(page);await button(page,'预览所选会话');await idle(page);
  assert.deepEqual(calls.filter(c=>c.command==='preview_import_selection')[0].payload,{sessionId:vault.session_id,selectionId:selection.selection_id,sourceIds:['one']});
  assert.match(await page.locator('.sample').textContent(),/chatgpt-export/);assert.match(await page.locator('.sample').textContent(),/2023/);
  assert.equal(count('confirm_import'),0);
  assert.match(await page.locator('.file-preview').textContent(),/写入资料库合成资料库.*写入范围personal/);
  await button(page,'取消这次导入');await idle(page);assert.equal(count('confirm_import'),0);
  await button(page,'选择文件并预览');await idle(page);await chooseFirst(page);await button(page,'预览所选会话');await idle(page);
  const confirm=await page.getByRole('button',{name:'确认导入 2 条记录',exact:true}).elementHandle();
  await confirm.evaluate(node=>{node.click();node.click()});await idle(page);
  assert.equal(await page.locator('#modal').evaluate(node=>node.open),false);
  await confirm.evaluate(node=>{node.disabled=false;node.click()});await idle(page);
  assert.equal(count('confirm_import'),1);assert.deepEqual(calls.filter(c=>c.command==='confirm_import')[0].payload,{sessionId:vault.session_id,previewId:preview.preview_id});
  assert.equal(await page.locator('#location').textContent(),'会话');
  assert.equal(await page.locator('.import-batch-summary').count(),1);
  assert.equal(await page.locator('.archive-selection').count(),0);
  assert.equal(await page.locator('.conversation-message').count(),2);
  assert.deepEqual(await page.locator('.conversation-message .message-role').allTextContents(),['用户原话','AI 回复']);
  assert.match((await page.locator('.conversation-message').allTextContents()).join(' '),/合成消息原文/);
  assert.equal(calls.filter(c=>c.command==='conversation_messages').at(-1).payload.conversationRef,importedConversation.session_ref);
  await openImport(page);
  assert.equal(await page.locator('.archive-selection').count(),0);
  await button(page,'选择文件并预览');await idle(page);
  assert.equal(await page.getByRole('heading',{name:'选择本批要导入的会话'}).count(),1);
  assert.equal(await page.getByRole('checkbox').first().isChecked(),false);
});

test('全选超过 100000 条时禁用预览，减少批次后才生成预览',async t=>{
  const huge=structuredClone(selection);huge.conversations.forEach(c=>c.event_count=60000);huge.coverage.events_available=120000;
  const {page,count}=await fixture(t,huge);await button(page,'选择全部可导入会话');
  assert.equal(await page.getByRole('button',{name:'预览所选会话',exact:true}).isDisabled(),true);assert.match(await page.locator('.archive-selection').textContent(),/超过 100000/);
  assert.equal(count('preview_import_selection'),0);await page.getByRole('checkbox').last().uncheck();
  assert.equal(await page.getByRole('button',{name:'预览所选会话',exact:true}).isDisabled(),false);await button(page,'预览所选会话');await idle(page);assert.equal(count('preview_import_selection'),1);
});

test('返回会话选择撤销旧预览；取消后往返页面不恢复清单',async t=>{
  const {page,count}=await fixture(t);await chooseFirst(page);await button(page,'预览所选会话');await idle(page);await button(page,'返回会话选择');await idle(page);
  assert.equal(count('return_import_selection'),1);assert.equal(await page.getByRole('heading',{name:'确认导入',exact:true}).count(),0);
  await button(page,'取消这次导入');await idle(page);
  await navigate(page,'会话');await openImport(page);
  assert.equal(await page.locator('.archive-selection').count(),0);assert.equal(count('confirm_import'),0);
});

test('原文件改变导致预览失败时没有确认写入按钮，改格式同时清除旧清单',async t=>{
  const {page,count,fail}=await fixture(t);await chooseFirst(page);fail('preview_import_selection','文件已改变或被替换，请重新选择文件并审查');await button(page,'预览所选会话');await idle(page);
  assert.match(await page.locator('#notice').textContent(),/文件已改变/);assert.equal(await page.getByRole('heading',{name:'确认导入',exact:true}).count(),0);assert.equal(count('confirm_import'),0);
  await expandDetails(page, '#file-import-details');
  await page.locator('#import-format').selectOption('manual-jsonl');await idle(page);assert.equal(await page.locator('.archive-selection').count(),0);
});


test('确认时原文件改变，旧确认按钮与会话清单都立即失效', async t => {
  const { page, count, fail } = await fixture(t);
  await chooseFirst(page); await button(page, '预览所选会话'); await idle(page);
  fail('confirm_import', '文件已改变或被替换，请重新选择文件并审查');
  const confirm = await page.getByRole('button', { name: '确认导入 2 条记录', exact: true }).elementHandle();
  await button(page, '确认导入 2 条记录');
  await idle(page);
  await confirm.evaluate(node => { node.disabled = false; node.click(); }); await idle(page);
  assert.equal(count('confirm_import'), 1);
  assert.equal(await page.getByRole('heading', { name: '确认导入', exact: true }).count(), 0);
  assert.equal(await page.locator('.archive-selection').count(), 0);
  assert.match(await page.locator('#notice').textContent(), /文件已改变/);
});
