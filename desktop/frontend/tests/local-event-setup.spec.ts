import {test,expect,type Page} from '@playwright/test';
import type {LocalEventStatus} from '../src/service/setup';

const settings={endpoint:'http://127.0.0.1:46123/v1/embeddings',model:'synthetic',revision:'fake-v1',tokenizer:'test-only',dimensions:2,query_instruction:'',document_instruction:'',events_approved:false,attachments_approved:false,query_approved:false,chunk_bytes:1024,disk_cap_mib:256};
const calls=(page:Page)=>page.evaluate(()=>(window as any).__LOCAL_EVENT_CALLS__ as {command:string;args:any}[]);
async function mount(page:Page,status:LocalEventStatus,failFirstSave=false) {
 await page.route('**/local-event-setup-test',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><html lang="zh-CN"><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/styles.css"></head><body><main id="local-event-setup-fixture" style="max-width:900px;margin:16px"></main><script type="module">import {renderLocalEventSetup} from '/tests/local-event-setup-fixture.tsx';window.renderLocalEventSetup=renderLocalEventSetup;</script></body></html>`}));
 await page.goto('/local-event-setup-test');
 await page.waitForFunction(()=>typeof(window as any).renderLocalEventSetup==='function');
 await page.evaluate(({status,failFirstSave})=>(window as any).renderLocalEventSetup(status,failFirstSave),{status,failFirstSave});
 await expect(page.getByRole('button',{name:'配置普通对话本机索引',exact:true})).toBeVisible();
}
async function edit(page:Page) {await page.getByRole('button',{name:'配置普通对话本机索引',exact:true}).click();}
async function fillTarget(page:Page) {
 for(const field of ['endpoint','model','revision','tokenizer'] as const)await page.locator('#event-'+field).fill(settings[field]);
 await page.locator('#event-dimensions').fill('2');
}

test('附件默认关闭，独立两步批准后才可更新，保存不触发编码',async({page},info)=>{
 await mount(page,{state:'unconfigured',message:'合成状态',settings:null});
 await edit(page);
 for(const id of ['event-corpus','event-attachments','event-query'])await expect(page.locator('#'+id)).not.toBeChecked();
 await fillTarget(page);
 await expect(page.getByRole('button',{name:'核对普通对话范围',exact:true})).toBeDisabled();
 await page.locator('#event-attachments').check();
 await expect(page.locator('#event-corpus')).not.toBeChecked();
 await expect(page.locator('#event-query')).not.toBeChecked();
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 const review=page.getByLabel('本机编码资料类型');
 await expect(review.locator('dt')).toHaveText(['普通对话','附件','新问题']);
 await expect(review).toContainText('未批准，不发送普通对话正文');
 await expect(review).toContainText('未批准，不发送新问题');
 await expect(review).toContainText('父事件可见、未撤回且未被修订替换');
 await expect(review).toContainText('安全 UTF-8 附件正文');
 await expect(review).toContainText('不支持 PDF 提取或 OCR');
 await expect(page.getByRole('dialog')).toContainText(settings.endpoint);
 await expect(page.getByRole('dialog')).toContainText('附件内容是独立来源，不当作上传者原话');
 await expect(page.getByRole('button',{name:'确认保存资料配置',exact:true})).toBeDisabled();
 expect((await calls(page)).filter(c=>!['local_event_status','cloud_event_status'].includes(c.command))).toEqual([]);
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'返回修改',exact:true}).click();
 await expect(page.locator('#event-attachments')).toBeChecked();
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 await expect(page.locator('#event-confirm')).not.toBeChecked();
 await expect(page.getByRole('button',{name:'确认保存资料配置',exact:true})).toBeDisabled();
 await page.screenshot({path:info.outputPath('attachment-approval-review.png'),fullPage:true});
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'确认保存资料配置',exact:true}).click();
 await expect(page.getByRole('dialog')).toHaveCount(0);
 const writes=(await calls(page)).filter(c=>!['local_event_status','cloud_event_status'].includes(c.command));
 expect(writes).toHaveLength(1);
 expect(writes[0]).toEqual({command:'local_event_apply',args:{sessionId:'synthetic-session',scope:'project:synthetic',request:{...settings,attachments_approved:true}}});
 await expect(page.getByRole('button',{name:'更新普通对话索引',exact:true})).toHaveCount(0);
 await page.getByRole('button',{name:'更新已批准资料索引',exact:true}).click();
 await expect(page.getByText('本轮索引完成：1/1项来源',{exact:true})).toBeVisible();
 expect((await calls(page)).filter(c=>c.command==='local_event_refresh')).toHaveLength(1);
});

test('旧普通对话配置不继承附件批准，原更新入口与新问题批准保持兼容',async({page})=>{
 const {attachments_approved:_,...legacy}=settings;
 await mount(page,{state:'configured',message:'旧版合成状态',settings:{...legacy,events_approved:true,query_approved:true}});
 await expect(page.getByRole('button',{name:'更新普通对话索引',exact:true})).toBeVisible();
 await expect(page.getByRole('button',{name:'更新已批准资料索引',exact:true})).toHaveCount(0);
 await expect(page.getByText(/附件编码未启用/)).toBeVisible();
 await edit(page);
 await expect(page.locator('#event-corpus')).toBeChecked();
 await expect(page.locator('#event-query')).toBeChecked();
 await expect(page.locator('#event-attachments')).not.toBeChecked();
 await page.getByRole('button',{name:'核对普通对话范围',exact:true}).click();
 await expect(page.getByLabel('本机编码资料类型')).toContainText('未批准，不发送附件正文');
 await expect(page.getByLabel('本机编码资料类型')).toContainText('已批准：未缓存的新问题');
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'确认保存普通对话配置',exact:true}).click();
 await expect(page.getByRole('dialog')).toHaveCount(0);
 const applies=(await calls(page)).filter(c=>c.command==='local_event_apply');
 expect(applies).toHaveLength(1);
 expect(applies[0].args.request).toEqual({...settings,events_approved:true,query_approved:true,attachments_approved:false});
});

test('取消和保存失败不授予附件批准，再次保存必须重新确认',async({page})=>{
 await mount(page,{state:'configured',message:'合成状态',settings},true);
 await edit(page);
 await page.locator('#event-attachments').check();
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 await page.locator('#event-confirm').check();
 await page.keyboard.press('Escape');
 await expect(page.getByRole('dialog')).toHaveCount(0);
 expect((await calls(page)).filter(c=>!['local_event_status','cloud_event_status'].includes(c.command))).toEqual([]);
 await edit(page);
 await expect(page.locator('#event-attachments')).not.toBeChecked();
 await page.locator('#event-attachments').check();
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'确认保存资料配置',exact:true}).click();
 await expect(page.getByRole('alert')).toContainText('合成保存失败，需要重新确认');
 await expect(page.locator('#event-confirm')).not.toBeChecked();
 await expect(page.getByRole('button',{name:'确认保存资料配置',exact:true})).toBeDisabled();
 expect((await calls(page)).filter(c=>c.command==='local_event_apply')).toHaveLength(1);
 expect((await calls(page)).filter(c=>c.command==='local_event_refresh')).toHaveLength(0);
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'确认保存资料配置',exact:true}).click();
 await expect(page.getByRole('dialog')).toHaveCount(0);
 expect((await calls(page)).filter(c=>c.command==='local_event_apply')).toHaveLength(2);
});

test('仅问题批准不能建资料索引，修改目标与范围后不得沿用复核',async({page})=>{
 await mount(page,{state:'configured',message:'合成状态',settings:{...settings,query_approved:true}});
 await expect(page.getByRole('button',{name:/更新.*索引/})).toHaveCount(0);
 await edit(page);
 await page.locator('#event-attachments').check();
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 await page.locator('#event-confirm').check();
 await page.getByRole('button',{name:'返回修改',exact:true}).click();
 await page.locator('#event-endpoint').fill('https://example.invalid/v1/embeddings');
 await expect(page.getByRole('button',{name:'核对资料范围',exact:true})).toBeDisabled();
 await page.locator('#event-endpoint').fill('http://[::1]:46124/v1/embeddings');
 await page.getByRole('button',{name:'核对资料范围',exact:true}).click();
 await expect(page.locator('#event-confirm')).not.toBeChecked();
 await expect(page.getByRole('dialog')).toContainText('http://[::1]:46124/v1/embeddings');
 await page.evaluate(status=>(window as any).renderLocalEventSetup(status,false,'project:other-synthetic'),{state:'unconfigured',message:'另一合成范围',settings:null});
 await expect(page.getByRole('dialog')).toHaveCount(0);
 await edit(page);
 await expect(page.locator('#event-attachments')).not.toBeChecked();
 await expect(page.locator('#event-corpus')).not.toBeChecked();
 await expect(page.locator('#event-query')).not.toBeChecked();
 expect((await calls(page)).filter(c=>!['local_event_status','cloud_event_status'].includes(c.command))).toEqual([]);
});
