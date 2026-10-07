import test from 'node:test';
import assert from 'node:assert/strict';
import '../composer-adapter.js';
class Element {
  constructor(tag='TEXTAREA',id='') {this.tagName=tag;this.id=id;this.value='';this.children=[];this._text='';this.isConnected=true;this.visible=true;this.disabled=false;this.readOnly=false;this.attributes={};this.events=[];this.style={};this.classList={contains:(x)=>x==='ProseMirror'};this.isContentEditable=tag==='DIV';}
  get textContent(){return this._text+this.children.map(x=>x.textContent).join('');}
  set textContent(v){this._text=v;this.children=[];}
  getClientRects(){return this.visible?[{}]:[];} getAttribute(name){return this.attributes[name]??null;}
  append(node){node.parentNode=this;this.children.push(node);}
  remove(){const p=this.parentNode;p.children=p.children.filter(x=>x!==this);this.parentNode=null;this.isConnected=false;}
  dispatchEvent(event){this.events.push(event.type);}
}
function setup(tag='TEXTAREA',id='prompt-textarea'){
  const node=new Element(tag,id);
  const doc={nodes:[node],querySelectorAll(selector){
    // 仅模拟这里使用的明确 id/type 选择器，不能让 fixture 忽略 selector。
    const selectors=selector.split(',').map(part=>part.trim().match(/^(?:([a-z]+))?#([a-z0-9_-]+)$/iu));
    assert.ok(selectors.every(Boolean),'fixture 不支持此 selector');
    return this.nodes.filter(element=>selectors.some(([,tag,identifier])=>element.id===identifier&&(!tag||element.tagName===tag.toUpperCase())));
  },createElement:()=>new Element('P'),defaultView:{InputEvent:class{constructor(type){this.type=type;}}}};
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


test('唯一 mobile textarea 追加和撤销均保留用户草稿，不代表已发送',()=>{
  const {node,adapter}=setup('TEXTAREA','mobile-composer-prompt');
  node.value='已有移动输入草稿🙂\n第二行';const original=node.value;
  assert.equal(adapter.find(),node);
  assert.equal(adapter.insert(capsule).status,'draft');
  assert.ok(node.value.startsWith(original));
  assert.throws(()=>adapter.confirmSent(capsule.id),/自己点击/);
  assert.throws(()=>adapter.insert(capsule),/已经插入/);
  node.value='新增前缀\n'+node.value+'\n新增后缀';
  adapter.remove(capsule.id);
  assert.equal(node.value,'新增前缀\n'+original+'\n新增后缀');
  assert.deepEqual(node.events,['input','input']);
});
test('mobile 与旧版可见编辑器同时存在时拒绝猜测目标',()=>{
  for(const tag of ['TEXTAREA','DIV']){
    const {node,doc,adapter}=setup('TEXTAREA','mobile-composer-prompt');
    const other=new Element(tag,'prompt-textarea');node.value='移动草稿';other.value='旧版草稿';doc.nodes.push(other);
    assert.throws(()=>adapter.insert(capsule),/唯一/);
    assert.equal(node.value,'移动草稿');assert.equal(other.value,'旧版草稿');
    assert.deepEqual(node.events,[]);assert.deepEqual(other.events,[]);
  }
});
test('mobile 输入框不可见、脱离文档、禁用或只读时均不写入',()=>{
  for(const modify of [
    node=>node.visible=false,
    node=>node.isConnected=false,
    node=>node.disabled=true,
    node=>node.readOnly=true,
    node=>node.attributes['aria-disabled']='true'
  ]){
    const {node,adapter}=setup('TEXTAREA','mobile-composer-prompt');node.value='保留草稿';modify(node);
    assert.throws(()=>adapter.insert(capsule));assert.equal(node.value,'保留草稿');assert.deepEqual(node.events,[]);
  }
});
test('mobile selector 只允许明确 textarea，不匹配任意编辑器',()=>{
  for(const tag of ['INPUT','DIV']){
    const {node,adapter}=setup(tag,'mobile-composer-prompt');node.value='不受支持的草稿';
    assert.throws(()=>adapter.insert(capsule),/唯一/);assert.equal(node.value,'不受支持的草稿');assert.deepEqual(node.events,[]);
  }
  const {doc,adapter}=setup('TEXTAREA','unrelated-composer');doc.nodes.push(new Element('DIV','other-editor'));
  assert.throws(()=>adapter.insert(capsule),/唯一/);assert.ok(doc.nodes.every(node=>node.events.length===0));
});
test('隐藏 mobile 不阻止唯一旧版输入框，两个可见候选中只读也不绕过歧义拒绝',()=>{
  const {node,doc,adapter}=setup();node.value='旧版草稿';
  const mobile=new Element('TEXTAREA','mobile-composer-prompt');mobile.visible=false;mobile.value='隐藏草稿';doc.nodes.push(mobile);
  adapter.insert(capsule);assert.equal(mobile.value,'隐藏草稿');assert.deepEqual(mobile.events,[]);
  adapter.remove(capsule.id);assert.equal(node.value,'旧版草稿');
  mobile.visible=true;node.readOnly=true;assert.throws(()=>adapter.insert(capsule),/唯一/);
  assert.equal(node.value,'旧版草稿');assert.equal(mobile.value,'隐藏草稿');
});
