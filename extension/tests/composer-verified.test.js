import test from 'node:test';
import assert from 'node:assert/strict';
import { Element, setup } from './fixtures/composer.mjs';
const capsule={id:'verified',nonce:'c'.repeat(48),text:'合成资料🙂\n完整结尾，不可丢失'};
function fixture(tag='TEXTAREA') {
  const f=setup(tag); const {node,doc}=f;
  if(tag==='DIV')Object.defineProperty(node,'innerText',{get:()=>node.textContent});
  node.getBoundingClientRect=()=>({left:0,top:0,right:600,bottom:120});
  doc.elementFromPoint=()=>node; doc.visibilityState='visible';
  Object.assign(doc.defaultView,{innerWidth:800,innerHeight:600,setTimeout,clearTimeout,requestAnimationFrame:fn=>setTimeout(fn,0),cancelAnimationFrame:clearTimeout});
  return f;
}
test('等待框架刷新后核验全文，保留原草稿、Unicode、换行且没有发送事件',async()=>{
  const {node,adapter}=fixture();node.value='已有草稿🙂\r\n下一行';
  const result=await adapter.insertVerified(capsule);
  assert.equal(result.full_text_verified,true);assert.equal(result.existing_draft_preserved,true);
  assert.ok(node.value.startsWith('已有草稿🙂\r\n下一行'));assert.ok(node.value.includes(capsule.text));assert.deepEqual(node.events,['input']);
});
test('异步截断尾部、改动原草稿即使保留整个新增块，也不能误报成功或自动再写',async()=>{
  for(const change of [text=>text.slice(0,-1),text=>text.replace('原草稿','被框架改写'),text=>text.replace('完整结尾','错误结尾')]) {
    const {node,adapter}=fixture();node.value='原草稿';node.dispatchEvent=e=>{node.events.push(e.type);setTimeout(()=>{node.value=change(node.value);},0);};
    await assert.rejects(()=>adapter.insertVerified(capsule),/完整草稿读回不一致/);
    const after=node.value;await assert.rejects(()=>adapter.insertVerified({...capsule,id:'retry'}),/不会再自动写入/);assert.equal(node.value,after);assert.deepEqual(node.events,['input']);
  }
});
test('填框期间换路由、换输入框、取消会话均不能确认旧结果',async()=>{
  for(const kind of ['route','node','cancel']) {
    const {node,doc,adapter,location}=fixture();let current=true;
    node.dispatchEvent=()=>setTimeout(()=>{if(kind==='route')location.href='https://chatgpt.com/c/another';if(kind==='node'){node.isConnected=false;doc.nodes=[new Element('TEXTAREA','prompt-textarea')];}if(kind==='cancel')current=false;},0);
    await assert.rejects(()=>adapter.insertVerified(capsule,()=>current),/会话或输入框已变化/);
  }
});
test('遮挡、隐藏、多目标在写入前回退，连续点击不追加两次',async()=>{
  for(const change of [f=>f.doc.elementFromPoint=()=>null,f=>f.doc.visibilityState='hidden',f=>f.doc.nodes.push(new Element('TEXTAREA','prompt-textarea'))]) {
    const f=fixture();f.node.value='不动的草稿';change(f);await assert.rejects(()=>f.adapter.insertVerified(capsule));assert.equal(f.node.value,'不动的草稿');assert.deepEqual(f.node.events,[]);
  }
  const {node,adapter}=fixture();const pending=adapter.insertVerified(capsule);await assert.rejects(()=>adapter.insertVerified(capsule),/正在核验/);await pending;assert.deepEqual(node.events,['input']);
});
test('富文本全文验证保留已有节点与原始草稿，未承诺网站框架兼容性',async()=>{
  const {node,doc,adapter}=fixture('DIV');const original=doc.createElement('p');original.textContent='原有段落';node.append(original);
  assert.equal((await adapter.insertVerified(capsule)).full_text_verified,true);assert.equal(node.children[0],original);
  adapter.remove(capsule.id);assert.equal(node.textContent,'原有段落');
});
test('已成功的相同请求拒绝重复但不污损后续新请求状态',async()=>{
  const {adapter}=fixture();await adapter.insertVerified(capsule);
  await assert.rejects(()=>adapter.insertVerified(capsule),/已经插入/);
  adapter.remove(capsule.id);
  assert.equal((await adapter.insertVerified({...capsule,id:'next'})).full_text_verified,true);
});
test('核验等待时页面reset不得回滚草稿或抹掉本次写入标记，后续不能再次自动写入',async()=>{
  const {node,adapter}=fixture();node.value='保留原草稿';let current=true;
  const pending=adapter.insertVerified(capsule,()=>current);
  assert.throws(()=>adapter.remove(capsule.id),/仍在核验/);
  assert.throws(()=>adapter.confirmSent(capsule.id),/仍在核验/);
  const filled=node.value;assert.equal(adapter.reset().length,1);assert.equal(node.value,filled);current=false;
  await assert.rejects(()=>pending,/会话或输入框已变化/);
  assert.equal(adapter.verificationFailed,true);
  assert.equal(adapter.reset().length,1);assert.equal(node.value,filled);
  await assert.rejects(()=>adapter.insertVerified({...capsule,id:'late'}),/不会再自动写入/);
  assert.deepEqual(node.events,['input']);
});

test('原富文本节点身份或格式变动不能仅凭textContent相等报成功',async()=>{
  const {node,doc,adapter}=fixture('DIV');const original=doc.createElement('p');original.textContent='两行草稿';node.append(original);
  node.dispatchEvent=()=>setTimeout(()=>{const replacement=doc.createElement('p');replacement.textContent=original.textContent;replacement.parentNode=node;node.children[0]=replacement;},0);
  await assert.rejects(()=>adapter.insertVerified(capsule),/富文本草稿结构已变化/);
  assert.equal(adapter.verificationFailed,true);
});
test('最终content会话确认失效也受保护，不得由reset回滚或逃过安全停止',async()=>{
  const {node,adapter}=fixture();await adapter.insertVerified(capsule);const filled=node.value;
  assert.throws(()=>adapter.confirmVerifiedContext(()=>{adapter.reset();return false;}),/会话已变化/);
  assert.equal(node.value,filled);assert.deepEqual(node.events,['input']);assert.equal(adapter.verificationFailed,true);
  await assert.rejects(()=>adapter.insertVerified({...capsule,id:'late'}),/不会再自动写入/);
});
