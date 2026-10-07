import test from 'node:test';
import assert from 'node:assert/strict';
import { routeFor, siteFor, parseAction } from '../protocol.js';
import { bindSession, verifyContent, Broker } from '../broker.js';
import { Element, setup } from './fixtures/composer.mjs';
const id='11111111-1111-4111-8111-111111111111';
const ext='abcdefghijklmnopabcdefghijklmnop';
const capsule={id:'r_site',nonce:'a'.repeat(48),text:'合成来源 event:evt_demo\n本机参考资料'};
const cases=[
  {site:'qwen',name:'Qwen',origin:'https://chat.qwen.ai',id:'',classes:['message-input-textarea']},
  {site:'zai',name:'Z.ai',origin:'https://chat.z.ai',id:'chat-input',classes:[]},
];
for(const spec of cases){
  const fixture=()=>setup('TEXTAREA',spec.id,`${spec.origin}/c/${id}`,spec.classes);
  test(`${spec.name} 只接受明确 HTTPS origin、根页和 UUID 对话路由`,()=>{
    assert.equal(routeFor(spec.origin+'/'),spec.origin);
    assert.equal(routeFor(`${spec.origin}/c/${id}/`),`${spec.origin}/c/${id}`);
    assert.equal(siteFor(spec.origin).id,spec.site);
    for(const suffix of ['/login','/auth/login','/share/'+id,'/settings','/c/demo','/c/'+id+'/extra','//','/?chat='+id,'/#chat','/C/'+id]) assert.throws(()=>routeFor(spec.origin+suffix),suffix);
    for(const url of [spec.origin.replace('https:','http:'),spec.origin+'.evil.test/',spec.origin.replace('https://','https://user:password@'),spec.origin+':444/'])assert.throws(()=>routeFor(url));
  });
  test(`${spec.name} 仅匹配本站精确编辑器，追加撤销保留合成草稿与新增内容`,()=>{
    const {node,doc,adapter}=fixture();
    const monaco=new Element('TEXTAREA','monaco');monaco.readOnly=true;monaco.visible=false;doc.nodes.push(monaco);
    node.value='原草稿🙂\r\n第二行';const before=node.value;
    assert.equal(adapter.insert(capsule).status,'draft');assert.ok(node.value.startsWith(before));
    assert.throws(()=>adapter.confirmSent(capsule.id),/自己点击/);
    node.value='前增\n'+node.value+'\n后增';adapter.remove(capsule.id);
    assert.equal(node.value,'前增\n'+before+'\n后增');assert.deepEqual(node.events,['input','input']);assert.equal(monaco.value,'');
  });
  test(`${spec.name} 多个候选、禁用、只读、隐藏、脱离文档均安全拒绝`,()=>{
    for(const modify of [
      ({node})=>node.disabled=true,({node})=>node.readOnly=true,
      ({node})=>node.attributes['aria-disabled']='true',({node})=>node.attributes['aria-readonly']='true',
      ({node})=>node.visible=false,({node})=>node.isConnected=false,
      ({node})=>node.style.visibility='hidden',({node})=>node.style.display='none',({node})=>node.style.opacity='0',
      ({doc})=>doc.nodes.push(new Element('TEXTAREA',spec.id,spec.classes)),
    ]){
      const f=fixture();f.node.value='不覆盖';modify(f);
      assert.throws(()=>f.adapter.insert(capsule));assert.equal(f.node.value,'不覆盖');assert.deepEqual(f.node.events,[]);
    }
  });
  test(`${spec.name} 不接收其他站点 selector，也不接受擅自换成富文本的节点`,()=>{
    for(const [tag,nodeId,classes] of [['TEXTAREA','prompt-textarea',[]],['DIV',spec.id,spec.classes],['INPUT',spec.id,spec.classes]]){
      const {node,adapter}=setup(tag,nodeId,spec.origin+'/',classes);node.value='不可写入';
      assert.throws(()=>adapter.insert(capsule));assert.equal(node.value,'不可写入');assert.deepEqual(node.events,[]);
    }
    const f=fixture();f.location.href='https://example.org/';assert.throws(()=>f.adapter.insert(capsule));assert.equal(f.node.value,'');
  });
  test(`${spec.name} sender origin、live tab 和站点前缀必须一致`,()=>{
    const url=spec.origin+'/c/'+id;
    const sender={id:ext,origin:spec.origin,url,tab:{id:1,url},frameId:0,documentId:'doc'};
    assert.equal(verifyContent(sender,ext,url,url),url);
    const state=bindSession(null,{route:url,token:id,documentId:'doc'});
    assert.equal(state.session_ref,`${spec.site}:${id}`);
    for(const change of [{origin:'https://chatgpt.com'},{url:'https://chatgpt.com/c/'+id},{tab:{id:1,url:'https://chatgpt.com/c/'+id}},{frameId:1},{documentId:null}])assert.throws(()=>verifyContent({...sender,...change},ext,url,url));
    const old=bindSession(null,{route:'https://chatgpt.com/c/'+id,token:id,documentId:'doc'});
    const request={protocol:'recallcard.action/1',request_id:'r_old',nonce:old.nonce,session_ref:old.session_ref,action:'search',arguments:{query:'合成'}};
    assert.throws(()=>parseAction('```recallcard-action\n'+JSON.stringify(request)+'\n```',state));
  });
  test(`${spec.name} 本机失败和读取途中跨站导航不产生可插入预览`,async()=>{
    for(const mode of ['failure','navigate']){
      let state=bindSession(null,{route:spec.origin+'/c/'+id,token:id,documentId:'doc'});
      const tab={id:1,active:true,url:state.route};let native=0;
      const broker=new Broker({id:ext,popupUrl:`chrome-extension://${ext}/popup.html`,getTab:async()=>tab,load:async()=>state,save:async(_,value)=>state=value,now:()=>10000,content:async()=>({ok:true,result:{status:'current'}}),native:async()=>{native++;if(mode==='failure')return{ok:false,error:'合成拒绝'};tab.url='https://chatgpt.com/c/'+id;return{ok:true,result:{stable_text:'不可插入',bootstrap_version:'v1'}};}});
      await assert.rejects(broker.handle({kind:'bootstrap',tabId:1,nonce:state.nonce,session_ref:state.session_ref},{id:ext,url:`chrome-extension://${ext}/popup.html`}));
      assert.equal(native,1);assert.equal(state.preview,null);
    }
  });
}
test('未启用 DeepSeek、apex 或泛域，不能因为有相同 textarea 就放行',()=>{
  for(const url of ['https://chat.deepseek.com/','https://chat.deepseek.com/a/chat/s/'+id,'https://qwen.ai/','https://www.qwen.ai/','https://qwenlm.ai/','https://z.ai/','https://api.z.ai/']){
    assert.throws(()=>routeFor(url));const {node,adapter}=setup('TEXTAREA','chat-input',url,['ds-scroll-area']);
    assert.throws(()=>adapter.insert(capsule));assert.equal(node.value,'');
  }
});
