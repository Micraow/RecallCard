import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SourceTextModule } from 'node:vm';
import { setImmediate } from 'node:timers/promises';
import net from 'node:net';
import dgram from 'node:dgram';
import { JSDOM, VirtualConsole } from 'jsdom';
import { syntheticBridge } from './fixtures.mjs';

const uiDirectory = new URL('../../ui/', import.meta.url);
const allowedModules = new Set(['app.js', 'model.js', 'memory-management.js', 'background.js', 'import-tasks.js']
  .map(name => new URL(name, uiDirectory).href));

// 每个用例加载真正的 index.html 与未改写的 ES 模块，彼此独立的 Window / 模块状态。
// 禁止任意模块来源；不拼接应用逻辑、不改 import，不把 Window 混到 Node 全局。
export async function fixture(t, { restoreResponse = null, restoreError = '', importJobs,
  holdStartup = false, configureNative } = {}) {
  const errors = [];
  const networkAttempts = [];
  // 防回归护栏，不模拟网络成功：任何连接、监听或 UDP socket 请求都会直接失败。
  for (const [target, method] of [[net.Socket.prototype, 'connect'],
    [net.Server.prototype, 'listen'], [dgram, 'createSocket']]) {
    t.mock.method(target, method, () => {
      networkAttempts.push(method);
      assert.fail(`本地 DOM 测试禁止创建 socket：${method}`);
    });
  }
  const console = new VirtualConsole();
  console.on('jsdomError', error => errors.push(error.message));
  const dom = new JSDOM(await readFile(new URL('index.html', uiDirectory), 'utf8'), {
    url: 'https://recallcard.test/', runScripts: 'outside-only', virtualConsole: console,
    // 不开启 resources: usable；不加载 script、CSS、图片或 iframe 的外部资源。
  });
  const { window } = dom;
  const { document } = window;
  const native = syntheticBridge();
  // 只在 IPC 边界提供启动响应，不访问或改写真实 ES 模块的业务状态。
  let releaseStartup;
  if (holdStartup) releaseStartup = native.hold('restore_workspace');
  else if (restoreError) native.fail('restore_workspace', restoreError);
  else native.next('restore_workspace', restoreResponse);
  if (importJobs !== undefined) native.next('list_import_jobs', importJobs);
  configureNative?.(native);
  window.__TAURI__ = { core: { invoke: native.invoke.bind(native) } };

  // jsdom 未实现原生 dialog 顶层/焦点行为。这里只模拟 open 标记，不宣称验证这些行为。
  const shownDialogs = [];
  window.HTMLDialogElement.prototype.showModal = function () { shownDialogs.push(this.id); this.open = true; };
  window.HTMLDialogElement.prototype.close = function () { this.open = false; };

  t.after(() => {
    window.close();
    assert.deepEqual(errors, [], '实际应用不得产生未处理的 DOM/JavaScript 错误');
    assert.deepEqual(native.unexpected, [], '所有原生命令必须在合成边界中明确列出');
    assert.deepEqual(networkAttempts, [], '测试运行不得尝试连接、监听或创建 UDP socket');
  });
  const modules = new Map();
  const context = dom.getInternalVMContext();
  async function load(url) {
    assert.ok(allowedModules.has(url.href), `只允许加载本地 UI 模块：${url.href}`);
    if (!modules.has(url.href)) {
      const source = await readFile(url, 'utf8');
      modules.set(url.href, new SourceTextModule(source, { context, identifier: url.href }));
    }
    return modules.get(url.href);
  }
  const app = await load(new URL('app.js', uiDirectory));
  await app.link((specifier, parent) => load(new URL(specifier, parent.identifier)));
  await app.evaluate();

  function one(selector, root = document) {
    const nodes = root.querySelectorAll(selector);
    assert.equal(nodes.length, 1, `选择器 ${selector} 应唯一，实际匹配到 ${nodes.length} 个节点`);
    return nodes[0];
  }
  function button(name, root = document) {
    const matches = [...root.querySelectorAll('button')].filter(node => node.textContent === name);
    assert.equal(matches.length, 1, `按钮「${name}」应唯一，实际匹配到 ${matches.length} 个节点`);
    return matches[0];
  }
  async function idle() {
    // 仅等待现有事件处理器/合成 Promise 完成；没有浏览器、子进程或本地服务。
    const deadline = performance.now() + 1000;
    do {
      await setImmediate();
      if (one('#operation').textContent === '准备就绪') return;
    } while (performance.now() < deadline);
    assert.fail(`应用没有回到空闲状态：${one('#operation').textContent}`);
  }
  async function click(name, root = document) {
    const node = button(name, root);
    assert.equal(node.disabled, false, `按钮「${name}」必须可用`);
    for (let ancestor = node.parentElement; ancestor; ancestor = ancestor.parentElement) if (ancestor.tagName === 'DETAILS' && !ancestor.open) ancestor.querySelector(':scope > summary').click();
    node.click();
    await idle();
  }
  async function navigate(name) {
    if (name === '添加资料') { one('#import-button').click(); await idle(); return; }
    if (name === '连接与状态') { one('#connect-button').click(); await idle(); return; }
    if (name === '随身背景') { await navigate('记忆'); await click('选择背景'); return; }
    if (name === '整理记忆') { await navigate('记忆'); await click('整理记忆'); return; }
    if (name === '查找与阅读') { one('#global-search').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true })); await idle(); return; }
    name = ({ '概览': '会话', '会话与接续': '会话', '记忆管理': '记忆' })[name] || name;

    // 导航按钮包含 aria-hidden 图标；用实际文字节点而不是自建可访问性算法定位。
    const buttons = [...one('#navigation').querySelectorAll('button')]
      .filter(node => [...node.childNodes].some(child => child.nodeType === 3 && child.textContent === name));
    assert.equal(buttons.length, 1, `导航「${name}」必须存在且唯一`);
    assert.equal(buttons[0].disabled, false);
    buttons[0].click(); await idle();
  }
  function fill(selector, value, type = 'input') {
    const node = one(selector);
    assert.equal(node.disabled, false);
    node.value = value;
    node.dispatchEvent(new window.Event(type, { bubbles: true }));
  }
  function check(selector, checked = true) {
    const node = one(selector);
    assert.equal(node.type, 'checkbox');
    assert.equal(node.disabled, false);
    if (node.checked !== checked) node.click();
  }
  function noWrites({ allowDefaultWorkspace = false } = {}) {
    // 此断言针对资料写入；明确打开库允许 remember_workspace 保存本机设置。
    const commands = ['confirm_import', 'start_import_job', 'resume_import_job', 'confirm_note', 'apply_dream', 'confirm_memory_change', 'confirm_background_change'];
    if (!allowDefaultWorkspace) commands.push('open_default_workspace');
    for (const command of commands) {
      assert.equal(native.count(command), 0, `明确的最终确认前不得调用 ${command}`);
    }
  }
  function noAutomaticWrites() {
    noWrites();
    assert.equal(native.count('remember_workspace'), 0, '只读启动恢复不得重新保存最近资料库设置');
  }
  // 默认 fixture 等待首次启动恢复结束；竞态用例显式保留原生 Promise。
  if (!holdStartup) await idle();
  return { window, document, native, one, button, click, navigate, idle, fill, check, noWrites,
    noAutomaticWrites, releaseStartup, shownDialogs,
    openVault: () => click('打开已有资料库'),
    modal: () => one('#modal'),
  };
}
