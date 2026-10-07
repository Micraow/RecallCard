import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import '../site-adapters.js';
const source=(await readFile(new URL('../popup.js',import.meta.url),'utf8')).replace("import './site-adapters.js';",'');
async function popup(url){
  const buttons=['bootstrap','reset','execute','insert','remove','delivered'];
  const nodes=Object.fromEntries([...buttons,'status','binding','preview','privacy','action'].map(id=>[id,{value:'',textContent:'',disabled:false,classList:{toggle(){}},listeners:{},addEventListener(kind,fn){this.listeners[kind]=fn;}}]));
  const sent=[];let approved=false,state;
  try{const site=globalThis.RecallCardSites.forUrl(url);state={platform:site.id,platform_name:site.name,route:site.route,nonce:'a'.repeat(48),session_ref:site.id+':synthetic',preview:null};}catch{}
  const chrome={tabs:{query:async()=>[{id:1,url}]},runtime:{sendMessage:async message=>{
    sent.push(message);
    if(!approved&&message.kind==='inspect')return{ok:false,error:'请先点击重置确认当前对话'};
    if(message.kind==='reset')approved=true;
    if(message.kind==='bootstrap')state.preview={id:'r_demo',delivery:'prepared',text:'本机合成预览'};
    if(message.kind==='insert')state.preview.delivery='draft';
    return{ok:true,result:structuredClone(state)};
  }}};
  const document={getElementById:id=>nodes[id],querySelectorAll:()=>buttons.map(id=>nodes[id])};
  await vm.runInNewContext('(async()=>{'+source+'})()',{document,chrome,RecallCardSites:globalThis.RecallCardSites});
  return{nodes,sent,click:async id=>{nodes[id].listeners.click();await new Promise(setImmediate);}};
}
for(const [origin,name] of [['https://chat.qwen.ai','Qwen'],['https://chat.z.ai','Z.ai']]){
  test(`${name} popup 未绑定时仍可显式重置，并准确标明资料接收网站`,async()=>{
    const h=await popup(origin+'/');
    assert.equal(h.nodes.reset.disabled,false);assert.equal(h.nodes.bootstrap.disabled,true);assert.match(h.nodes.status.textContent,/重置/);
    await h.click('reset');assert.equal(h.nodes.bootstrap.disabled,false);
    assert.match(h.nodes.binding.textContent,new RegExp(name.replace('.','\\.')));assert.ok(h.nodes.privacy.textContent.includes(name));
    await h.click('bootstrap');assert.equal(h.nodes.preview.value,'本机合成预览');assert.equal(h.nodes.insert.disabled,false);
    await h.click('insert');assert.ok(h.nodes.status.textContent.includes(name));assert.match(h.nodes.status.textContent,/自己点击/);
    assert.ok(h.sent.every(msg=>['inspect','reset','bootstrap','insert'].includes(msg.kind)));
  });
}
test('popup 拒绝未支持 host 或登录路径，不启用重置或请求本机',async()=>{
  for(const url of ['https://chat.deepseek.com/','https://chat.qwen.ai/login','https://chat.z.ai/share/demo','https://evil.test/']){
    const h=await popup(url);assert.equal(h.nodes.reset.disabled,true);assert.equal(h.nodes.bootstrap.disabled,true);assert.equal(h.sent.length,0);assert.match(h.nodes.status.textContent,/未支持/);
  }
});
