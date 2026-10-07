// 独立的最小合成 DOM；不包含第三方 fixture、网页对话或账号数据。
import assert from 'node:assert/strict';
import '../../site-adapters.js';
import '../../composer-adapter.js';
export class Element {
  constructor(tag='TEXTAREA',id='',classes=[]) {
    this.tagName=tag;this.id=id;this.value='';this.children=[];this._text='';this.isConnected=true;
    this.visible=true;this.disabled=false;this.readOnly=false;this.attributes={};this.events=[];this.style={};
    this.classes=new Set(classes.length?classes:tag==='DIV'?['ProseMirror']:[]);
    this.classList={contains:(name)=>this.classes.has(name)};this.isContentEditable=tag==='DIV';
  }
  get textContent(){return this._text+this.children.map(x=>x.textContent).join('');}
  set textContent(v){this._text=v;this.children=[];}
  getClientRects(){return this.visible?[{}]:[];}
  getAttribute(name){return this.attributes[name]??null;}
  append(node){node.parentNode=this;this.children.push(node);}
  remove(){const p=this.parentNode;p.children=p.children.filter(x=>x!==this);this.parentNode=null;this.isConnected=false;}
  dispatchEvent(event){this.events.push(event.type);}
}
export function setup(tag='TEXTAREA',id='prompt-textarea',url='https://chatgpt.com/',classes=[]) {
  const node=new Element(tag,id,classes);
  const location={href:url};
  const doc={location,nodes:[node],querySelectorAll(selector){
    const selectors=selector.split(',').map(part=>part.trim().match(/^(?:([a-z]+))?([#.])([a-z0-9_-]+)$/iu));
    assert.ok(selectors.every(Boolean),'fixture 不支持此 selector');
    return this.nodes.filter(element=>selectors.some(([,tag,kind,identifier])=>(!tag||element.tagName===tag.toUpperCase())&&(kind==='#'?element.id===identifier:element.classes.has(identifier))));
  },createElement:()=>new Element('P'),defaultView:{getComputedStyle:element=>({visibility:element.style.visibility||'visible',display:element.style.display||'block',opacity:element.style.opacity||'1'}),InputEvent:class{constructor(type){this.type=type;}}}};
  return {node,doc,location,adapter:new globalThis.RecallCardComposerAdapter(doc)};
}
