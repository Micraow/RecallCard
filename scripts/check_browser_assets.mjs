// 只读取真实浏览器 fixture 的资源表并解析模块；不启动浏览器或执行应用代码。
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { resolve, relative, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { SourceTextModule } from 'node:vm';

const root = resolve(process.argv[2] || '.');
const ui = resolve(root, 'desktop/ui');
const { workspaceAssets } = await import(pathToFileURL(resolve(root, 'desktop/tests/workspace-browser-helpers.mjs')));
const assets = await workspaceAssets();
assert.ok(assets instanceof Map, '真实浏览器资源 helper 必须返回 Map');
const files = await readdir(ui, { recursive: true, withFileTypes: true });
const expected = files.filter(file => file.isFile()).map(file =>
  '/' + relative(ui, resolve(file.parentPath, file.name)).split(sep).join('/')).sort();
assert.deepEqual([...assets.keys()].sort(), expected, '浏览器资源白名单必须覆盖全部真实 UI 文件，且不能含陈旧路径');
for (const path of expected) {
  assert.deepEqual(Buffer.from(assets.get(path)), await readFile(resolve(ui, '.' + path)), `资源必须来自真实应用文件：${path}`);
}
const modules = new Map();
for (const [path, source] of assets) {
  if (path.endsWith('.js') || path.endsWith('.mjs')) {
    modules.set(path, new SourceTextModule(String(source), { identifier: path }));
  }
}
// V8 自己解析静态 import/export；模块只 link，不 evaluate。
for (const module of modules.values()) {
  if (module.status !== 'unlinked') continue;
  await module.link((specifier, parent) => {
    assert.ok(specifier.startsWith('.'), `UI 模块不得引用资源表以外的依赖：${specifier}`);
    const path = new URL(specifier, 'https://recallcard.test' + parent.identifier).pathname;
    assert.ok(modules.has(path), `浏览器资源白名单缺少模块：${path}`);
    return modules.get(path);
  });
}
console.log(JSON.stringify({ assets: expected, modules: [...modules.keys()], application_evaluated: false }));
