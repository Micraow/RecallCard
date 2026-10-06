import test from 'node:test';
import assert from 'node:assert/strict';
import '../composer-adapter.js';
class Element {
  constructor(tag='TEXTAREA') {this.tagName=tag;this.value='';this.children=[];this._text='';this.isConnected=true;this.events=[];this.style={};this.classList={contains:(x)=>x==='ProseMirror'};this.isContentEditable=tag==='DIV';}
  get textContent(){return this._text+this.children.map(x=>x.textContent).join('');}
  set textContent(v){this._text=v;this.children=[];}
  getClientRects(){return[{}];} getAttribute(){return null;}
  append(node){node.parentNode=this;this.children.push(node);}
  remove(){const p=this.parentNode;p.children=p.children.filter(x=>x!==this);this.parentNode=null;this.isConnected=false;}
  dispatchEvent(event){this.events.push(event.type);}
}
function setup(tag='TEXTAREA'){
  const node=new Element(tag);
  const doc={querySelectorAll:()=>[node],createElement:()=>new Element('P'),defaultView:{InputEvent:class{constructor(type){this.type=type;}}}};
  return {node,doc,adapter:new globalThis.RecallCardComposerAdapter(doc)};
}
const capsule={id:'r_demo',nonce:'a'.repeat(48),text:'来源 event:evt_demo\n待审核上下文'};
test('追加不覆盖已有草稿，移除不丢失前后新编辑',()=>{
  const {node,adapter}=setup();node.value='原有草稿🙂\r\n第二行';const original=node.value;
  adapter.insert(capsule);assert.ok(node.value.startsWith(original));
  node.value='前面新增\n'+node.value+'\n后面新增';
  adapter.remove(capsule.id);assert.equal(node.value,'前面新增\n'+original+'\n后面新增');
  assert.deepEqual(node.events,['input','input']);
});
test('同一请求不会双重注入，不尝试自动发送或 Enter',()=>{
  const {node,adapter}=setup();adapter.insert(capsule);const before=node.value;
  assert.throws(()=>adapter.insert(capsule),/已经插入/);assert.equal(node.value,before);
  assert.deepEqual(node.events,['input']);
});
test('用户改动注入块后安全拒绝移除；不回滚整个草稿',()=>{
  const {node,adapter}=setup();node.value='保留';adapter.insert(capsule);
  node.value=node.value.replace('待审核上下文','我修改了内容');const before=node.value;
  assert.throws(()=>adapter.remove(capsule.id),/人工|手动/);assert.equal(node.value,before);
});
test('多个相同块不猜测移除目标',()=>{
  const {node,adapter}=setup();adapter.insert(capsule);node.value+=node.value;const before=node.value;
  assert.throws(()=>adapter.remove(capsule.id));assert.equal(node.value,before);
});
test('contenteditable 保持原有富文本节点身份，撤销只移除自己的节点',()=>{
  const {node,doc,adapter}=setup('DIV');const existing=doc.createElement('p');existing.textContent='原草稿';node.append(existing);
  adapter.insert(capsule);assert.equal(node.children[0],existing);assert.equal(node.children.length,2);
  const extra=doc.createElement('p');extra.textContent='新的个人内容';node.append(extra);
  adapter.remove(capsule.id);assert.deepEqual(node.children,[existing,extra]);
});
test('页面结构缺失、模糊、不支持时不做写入',()=>{
  for(const nodes of [[],[new Element(),new Element()],[new Element('INPUT')]]) {
    const {doc,adapter}=setup();doc.querySelectorAll=()=>nodes;assert.throws(()=>adapter.insert(capsule));
    for(const node of nodes)assert.equal(node.value,'');
  }
});
test('导航重置仅移除完整的自有块；不会删除改动过的用户文本',()=>{
  const {node,adapter}=setup();node.value='原草稿';adapter.insert(capsule);
  assert.deepEqual(adapter.reset(),[]);assert.equal(node.value,'原草稿');
  adapter.insert(capsule);node.value=node.value.replace('待审核','改动');const before=node.value;
  assert.equal(adapter.reset().length,1);assert.equal(node.value,before);
});
test('只有显式人工确认才记录发送，草稿还在时拒绝假确认',()=>{
  const {node,adapter}=setup();adapter.insert(capsule);
  assert.throws(()=>adapter.confirmSent(capsule.id),/自己点击/);
  node.value='';assert.equal(adapter.status().length,1);
  assert.equal(adapter.confirmSent(capsule.id).status,'user_confirmed_sent');assert.equal(adapter.status().length,0);
});
test('草稿消失仅释放旧会话，不推断已发送；编辑过的块仍阻止假确认',()=>{
  const {node,adapter}=setup();adapter.insert(capsule);node.value='';
  assert.deepEqual(adapter.reset(),[]);assert.deepEqual(adapter.status(),[]);
  adapter.insert(capsule);node.value=node.value.replace('待审核','用户编辑');
  assert.throws(()=>adapter.confirmSent(capsule.id),/自己点击/);
});
test('注入块新增非文本内容后，不会因为 textContent 相同就删除用户图片',()=>{
  const {node,adapter}=setup('DIV');adapter.insert(capsule);
  const injected=node.children[0];injected.innerHTML='用户在上下文内加入了图片';
  assert.throws(()=>adapter.remove(capsule.id),/富文本结构/);
  assert.equal(node.children[0],injected);
});
