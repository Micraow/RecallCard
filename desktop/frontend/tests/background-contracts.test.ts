import assert from 'node:assert/strict';
import test from 'node:test';
import {build} from 'esbuild';
import {validateBackgroundPage,copyCurrentBackground,type BackgroundPage} from '../src/service/background.ts';
function page(text='合成长期背景'):BackgroundPage{return {background:{bootstrap_version:'synthetic-v1',stable_text:text,refs:['memory:mem_synthetic@1'],truncated:false},copy_text:'recallcard.context/1\n'+text,candidates:[],selected_count:1,total:1,next_offset:null};}
test('开局复制前比较全文及来源版本，改变末尾或引用不能静默复制',async()=>{
 const writes:string[]=[];const old=page('相同开头'.repeat(90)+'旧尾');const newer=page('相同开头'.repeat(90)+'新尾');
 assert.equal((await copyCurrentBackground(old,async()=>newer,async s=>{writes.push(s);},()=>true))?.copied,false);
 const changed={...old,background:{...old.background,refs:['memory:mem_other@2']}};
 assert.equal((await copyCurrentBackground(old,async()=>changed,async s=>{writes.push(s);},()=>true))?.copied,false);
 assert.deepEqual(writes,[]);
 assert.equal((await copyCurrentBackground(newer,async()=>newer,async s=>{writes.push(s);},()=>true))?.copied,true);assert.deepEqual(writes,[newer.copy_text]);
});
test('关闭和切换范围后迟到读取不复制，来源撤回或无效合同不冒充成功',async()=>{
 let done:(p:BackgroundPage)=>void=()=>{};let active=true,writes=0;
 const task=copyCurrentBackground(page(),()=>new Promise(resolve=>{done=resolve;}),async()=>{writes++;},()=>active);
 active=false;done(page());assert.equal(await task,null);assert.equal(writes,0);
 await assert.rejects(copyCurrentBackground(page(),async()=>{throw Error('范围已撤回');},async()=>{writes++;},()=>true),/撤回/);
 for(const change of [{copy_text:'正文缺少上下文标记'},{copy_text:'recallcard.context/1\n另一份正文'},{total:-1},{selected_count:2}])assert.throws(()=>validateBackgroundPage({...page(),...change}),/不可核验/);
});
test('实际开局预览显示截断、来源和排除项，正文不能执行HTML',async()=>{
 const b=await build({stdin:{contents:`import {createElement} from 'react';import {renderToStaticMarkup} from 'react-dom/server';import {OpeningPreview} from './src/components/OpeningBackground.tsx';export const render=page=>renderToStaticMarkup(createElement(OpeningPreview,{page}));`,resolveDir:new URL('..',import.meta.url).pathname,loader:'tsx'},bundle:true,platform:'node',format:'esm',write:false,loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"'},banner:{js:`import {createRequire} from 'node:module';const require=createRequire(${JSON.stringify(import.meta.url)});`}});
 const {render}=await import(`data:text/javascript;base64,${Buffer.from(b.outputFiles[0].text).toString('base64')}`);
 const p=page('<script>synthetic()</script>');p.background.truncated=true;const html=render(p);
 for(const pattern of [/字节 UTF-8/,/部分选择未带上/,/memory:mem_synthetic@1/,/不会复制未选记忆/,/自动摘要/,/&lt;script&gt;/])assert.match(html,pattern);assert.doesNotMatch(html,/<script>/);
});
test('后台选择前端桥绑定当前session范围及精确取消票据',async()=>{
 const b=await build({entryPoints:[new URL('../src/service/client.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false});
 const {ScopedService}=await import(`data:text/javascript;base64,${Buffer.from(b.outputFiles[0].text).toString('base64')}`);const calls:any[]=[];
 const service=new ScopedService({invoke:async(command:string,args:any)=>{calls.push({command,args});return command==='read_background_page'?page():{};}},{session_id:'synthetic-session'},'project:synthetic');
 await service.openingBackground(30);await service.reviewBackground('mem_synthetic',2,true);await service.confirmBackground('preview-exact',true);await service.cancelBackground('preview-exact');
 assert.deepEqual(calls.map(c=>c.command),['read_background_page','review_background_change','confirm_background_change','cancel_background_change']);
 assert.equal(calls[3].args.previewId,'preview-exact');assert.equal(calls[0].args.offset,30);assert.equal(calls[2].args.approveProtected,true);
 for(const c of calls){assert.equal(c.args.scope,'project:synthetic');assert.equal(c.args.sessionId,'synthetic-session');}
});
