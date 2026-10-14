import {test,expect,type Page} from '@playwright/test';
import type {CloudEventStatus,LocalEventStatus} from '../src/service/setup';

const settings={endpoint:'https://embedding.example.invalid/v1/embeddings',model:'synthetic-choice',revision:'synthetic-version',dimensions:2,query_instruction:'',document_instruction:'',events_approved:false,attachments_approved:false,query_approved:false,budget_mode:'limited' as const,max_calls_per_month:100,max_bytes_per_month:1048576};
const index={tokenizer:'synthetic-tokenizer',chunk_bytes:1024,disk_cap_mib:256};
const usage={month:'2026-10',reserved_calls:25,reserved_bytes:2048,query_calls:10,memory_calls:10,source_query_calls:2,event_calls:2,attachment_calls:1};
const unconfigured:LocalEventStatus={state:'unconfigured',message:'合成未配置',settings:null};
const cloud:CloudEventStatus={state:'configured',message:'合成云配置',provider:'cloud',settings:{...settings,...index,attachments_approved:true,query_approved:true},cloud:{grant:{scope:'project:synthetic',revision:'synthetic-grant',accepted_at:'2026-10-14T00:00:00Z',settings:{...settings,attachments_approved:true,query_approved:true}},usage,budget_note:'共享累计用量；失败预留不退。'}};
const calls=(page:Page)=>page.evaluate(()=>(window as any).__LOCAL_EVENT_CALLS__ as {command:string;args:any}[]);
const writes=async(page:Page)=>(await calls(page)).filter(call=>!['local_event_status','cloud_event_status'].includes(call.command));
async function mount(page:Page,status:LocalEventStatus|CloudEventStatus=unconfigured,failFirstSave=false) {
 await page.route('**/cloud-event-setup-test',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><html lang="zh-CN"><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/styles.css"></head><body><main id="local-event-setup-fixture" style="max-width:900px;margin:16px"></main><script type="module">import {renderLocalEventSetup} from '/tests/local-event-setup-fixture.tsx';window.renderLocalEventSetup=renderLocalEventSetup;</script></body></html>`}));
 await page.goto('/cloud-event-setup-test');
 await page.waitForFunction(()=>typeof(window as any).renderLocalEventSetup==='function');
 await page.evaluate(({status,failFirstSave})=>(window as any).renderLocalEventSetup(status,failFirstSave),{status,failFirstSave});
 await expect(page.getByRole('button',{name:'配置普通对话与附件云索引',exact:true})).toBeVisible();
}
async function edit(page:Page){await page.getByRole('button',{name:'配置普通对话与附件云索引',exact:true}).click();}
async function fillTarget(page:Page){for(const name of ['endpoint','model','revision'] as const)await page.locator('#cloud-event-'+name).fill(settings[name]);await page.locator('#cloud-event-tokenizer').fill(index.tokenizer);await page.locator('#cloud-event-dimensions').fill('2');}
async function review(page:Page){await page.getByRole('button',{name:'审阅云来源范围',exact:true}).click();}
async function approve(page:Page){await page.locator('#cloud-event-save-os').check();await page.locator('#cloud-event-confirm').check();}

// 仅访问本机 Vite 合成夹具；.invalid 目标不会产生网络请求。
test('云附件独立二段批准，共享预算清楚，保存后才可明确开始编码',async({page},info)=>{
 const unexpected:string[]=[];page.on('request',request=>{if(request.url().startsWith('https://embedding.'))unexpected.push(request.url());});
 await mount(page);await edit(page);
 for(const name of ['endpoint','model','revision'])await expect(page.locator('#cloud-event-'+name)).toHaveValue('');
 for(const id of ['cloud-event-corpus','cloud-event-attachments','cloud-event-query'])await expect(page.locator('#'+id)).not.toBeChecked();
 await fillTarget(page);await expect(page.getByRole('button',{name:'审阅云来源范围',exact:true})).toBeDisabled();
 await page.locator('#cloud-event-attachments').check();
 await expect(page.locator('#cloud-event-corpus')).not.toBeChecked();await expect(page.locator('#cloud-event-query')).not.toBeChecked();
 await review(page);
 const types=page.getByLabel('云编码资料类型');await expect(types.locator('dt')).toHaveText(['普通对话','附件','新问题']);
 for(const text of ['未批准，不发送普通对话正文','未批准，不发送新问题','安全 UTF-8 附件正文','不支持 PDF 提取或 OCR'])await expect(types).toContainText(text);
 const dialog=page.getByRole('dialog');for(const text of [settings.endpoint,index.tokenizer,'当前范围：synthetic','共享整个 Vault','不是两套可叠加限额','失败预留不退','已发送请求不能撤回','保存配置不会开始编码'])await expect(dialog).toContainText(text);
 const submit=page.getByRole('button',{name:'确认保存云来源配置',exact:true});await expect(submit).toBeDisabled();
 await page.locator('#cloud-event-confirm').check();await expect(submit).toBeDisabled();
 await page.locator('#cloud-event-save-os').check();await expect(page.locator('#cloud-event-confirm')).not.toBeChecked();
 expect(await writes(page)).toEqual([]);
 await page.locator('#cloud-event-confirm').check();await page.screenshot({path:info.outputPath('cloud-source-approval-review.png'),fullPage:true});
 await submit.click();await expect(dialog).toHaveCount(0);
 expect(await writes(page)).toEqual([{command:'cloud_event_apply',args:{sessionId:'synthetic-session',scope:'project:synthetic',request:{settings:{...settings,attachments_approved:true},...index,save_in_os:true},apiKey:null}}]);
 await expect(page.getByText(/当前提供器：云/)).toBeVisible();
 await page.getByRole('button',{name:'更新已批准资料索引',exact:true}).click();
 await expect(page.getByText('本轮索引完成：1/1项来源',{exact:true})).toBeVisible();
 expect((await calls(page)).filter(call=>call.command==='local_event_refresh')).toHaveLength(1);expect(unexpected).toEqual([]);
});

test('本机、Memory旧批准不预选云范围，返回修改和切换提供器清除审阅',async({page})=>{
 await mount(page,{state:'configured',message:'合成本机',settings:{...settings,...index,endpoint:'http://127.0.0.1:46123/v1/embeddings',events_approved:true,attachments_approved:true,query_approved:true}});await edit(page);
 for(const id of ['cloud-event-corpus','cloud-event-attachments','cloud-event-query'])await expect(page.locator('#'+id)).not.toBeChecked();
 await fillTarget(page);await page.locator('#cloud-event-query').check();await page.locator('#cloud-event-key').fill('synthetic-test-secret');await review(page);await approve(page);
 await page.getByRole('button',{name:'返回修改',exact:true}).click();await page.locator('#cloud-event-model').fill('another-synthetic-model');await page.locator('#cloud-event-budget').selectOption('unlimited');
 await expect(page.locator('#cloud-event-calls')).toBeDisabled();await expect(page.locator('#cloud-event-calls')).toHaveValue('100');
 await review(page);await expect(page.locator('#cloud-event-confirm')).not.toBeChecked();await expect(page.locator('#cloud-event-save-os')).not.toBeChecked();await expect(page.getByRole('dialog')).toContainText('无限，持续计量，可能持续产生供应商费用');
 await page.keyboard.press('Escape');await page.getByRole('button',{name:'配置普通对话本机索引',exact:true}).click();await expect(page.locator('#event-corpus')).toBeChecked();await page.keyboard.press('Escape');await edit(page);
 await expect(page.locator('#cloud-event-key')).toHaveValue('');for(const id of ['cloud-event-corpus','cloud-event-attachments','cloud-event-query'])await expect(page.locator('#'+id)).not.toBeChecked();expect(await writes(page)).toEqual([]);
});

test('云保存失败清空密钥和确认，不自动重试，重用系统凭据仍需明确同意',async({page})=>{
 await mount(page,unconfigured,true);await edit(page);await fillTarget(page);await page.locator('#cloud-event-corpus').check();await page.locator('#cloud-event-key').fill('synthetic-test-secret');await review(page);await approve(page);
 await page.getByRole('button',{name:'确认保存云来源配置',exact:true}).click();await expect(page.getByRole('alert')).toContainText('合成保存失败');
 await expect(page.locator('#cloud-event-key')).toHaveValue('');expect(await page.evaluate(()=>({local:localStorage.length,session:sessionStorage.length}))).toEqual({local:0,session:0});
 expect((await writes(page)).map(call=>call.command)).toEqual(['cloud_event_apply']);await review(page);
 await expect(page.locator('#cloud-event-confirm')).not.toBeChecked();await expect(page.locator('#cloud-event-save-os')).not.toBeChecked();
 await approve(page);await page.getByRole('button',{name:'确认保存云来源配置',exact:true}).click();await expect(page.getByRole('dialog')).toHaveCount(0);
 const saved=(await writes(page));expect(saved).toHaveLength(2);expect(saved[0].args.apiKey).toBe('synthetic-test-secret');expect(saved[1].args.apiKey).toBeNull();expect(saved.every(call=>!('apiKey' in call.args.request))).toBe(true);
});

test('云到本机不继承批准，云状态显示共享全库分类用量，停用只发云命令',async({page})=>{
 await mount(page,cloud);
 for(const text of ['全 Vault 云向量已预留 25 次 / 2048 字节','Memory 查询 10 次','Memory 正文 10 次','来源查询 2 次','普通对话 2 次','附件 1 次'])await expect(page.getByText(/全 Vault 云向量已预留/)).toContainText(text);
 await page.getByRole('button',{name:'配置普通对话本机索引',exact:true}).click();for(const id of ['event-corpus','event-attachments','event-query'])await expect(page.locator('#'+id)).not.toBeChecked();await expect(page.locator('#event-model')).toHaveValue('');await page.keyboard.press('Escape');
 await page.getByRole('button',{name:'停用云来源索引',exact:true}).click();await expect(page.getByRole('dialog')).toContainText('不停用本机提供器');expect(await writes(page)).toEqual([]);
 await page.getByRole('button',{name:'确认停用云来源索引',exact:true}).click();await expect(page.getByRole('dialog')).toHaveCount(0);expect((await writes(page)).map(call=>call.command)).toEqual(['cloud_event_disable']);await expect(page.getByRole('button',{name:/更新.*索引/})).toHaveCount(0);
});

test('范围变化和迟到保存不恢复旧审阅或在新范围建库，仅问题批准没有更新入口',async({page})=>{
 await mount(page);await edit(page);await fillTarget(page);await page.locator('#cloud-event-query').check();await review(page);await approve(page);
 await page.evaluate(()=>(window as any).__HOLD_CLOUD_EVENT_SAVE__=true);await page.getByRole('button',{name:'确认保存云来源配置',exact:true}).click();await expect(page.getByRole('button',{name:'正在保存云来源配置',exact:true})).toBeDisabled();
 await page.evaluate(status=>(window as any).renderLocalEventSetup(status,false,'project:other-synthetic'),unconfigured);await expect(page.getByRole('dialog')).toHaveCount(0);
 await page.evaluate(()=>{(window as any).__HOLD_CLOUD_EVENT_SAVE__=false;(window as any).__RESOLVE_CLOUD_EVENT_SAVE__();});await edit(page);
 await expect(page.getByRole('dialog')).toContainText('当前范围：other-synthetic');for(const id of ['cloud-event-corpus','cloud-event-attachments','cloud-event-query'])await expect(page.locator('#'+id)).not.toBeChecked();await expect(page.locator('#cloud-event-key')).toHaveValue('');expect(await writes(page)).toEqual([]);
 await fillTarget(page);await page.locator('#cloud-event-query').check();await review(page);await approve(page);await page.getByRole('button',{name:'确认保存云来源配置',exact:true}).click();await expect(page.getByRole('dialog')).toHaveCount(0);await expect(page.getByRole('button',{name:/更新.*索引/})).toHaveCount(0);expect((await writes(page)).map(call=>call.command)).toEqual(['cloud_event_apply']);
});
