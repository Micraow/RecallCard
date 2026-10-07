// 共享真实浏览器交互：仅点击、填写与等待公开 UI，不从 helper 调用原生桥。
// UI 资源保持明确白名单，任一页面请求到外网或未列出的资源都会由 fixture 阻断。
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function captureBrowserEvidence(page, label) {
  const directory = process.env.RECALLCARD_BROWSER_ARTIFACTS;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  const name = label.replace(/[^\p{L}\p{N}-]+/gu, '-').slice(0, 100);
  const layout = await page.evaluate(() => ({
    viewport: { width: innerWidth, height: innerHeight },
    workspace: document.querySelector('.workspace-split')?.className,
    controls: ['#notice', '#continuation-panel', '#selection-context-panel', '.copy-continuation', '.conversation-list', '.reader-scroll'].map(selector => {
      const node = document.querySelector(selector);
      if (!node) return { selector, present: false };
      const rect = node.getBoundingClientRect();
      return { selector, present: true, rectangle: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, display: getComputedStyle(node).display };
    }),
  }));
  await writeFile(join(directory, `${name}.json`), JSON.stringify(layout, null, 2));
  await writeFile(join(directory, `${name}.html`), await page.content());
  await page.screenshot({ path: join(directory, `${name}.png`), fullPage: false });
}

export async function workspaceAssets() {
  const assets = new Map();
  for (const name of ['index.html', 'app.js', 'model.js', 'memory-management.js', 'background.js', 'import-tasks.js', 'context-selection.js', 'styles.css']) {
    assets.set(`/${name}`, await readFile(new URL(`../ui/${name}`, import.meta.url)));
  }
  return assets;
}

export async function idle(page) {
  await page.waitForFunction(() => document.querySelector('#operation').textContent === '准备就绪');
}

export async function expandDetails(page, selector) {
  const details = page.locator(selector);
  if (!await details.evaluate(node => node.open)) await details.locator(':scope > summary').click();
}

export async function button(page, name) {
  if (/^查看出处 \d+$/.test(name)) await expandDetails(page, '.memory-sources');
  if (['选择这条资料', '已选择 · 点击移除'].includes(name)) await expandDetails(page, '.reading-pane .organize-details');
  if (name === '选择文件并预览') await expandDetails(page, '#file-import-details');
  if (['选择结果并审阅', '导出本次来源包'].includes(name)) await expandDetails(page, '#dream-file-options');
  await page.getByRole('button', { name, exact: true }).click();
}

export async function navigate(page, name) {
  if (!['会话', '记忆'].includes(name)) throw new Error(`非主工作区入口：${name}`);
  await page.locator('#navigation').getByRole('button', { name, exact: true }).click();
  await idle(page);
}

export async function openImport(page) {
  await page.locator('#import-button').click();
  await idle(page);
}

export async function openConnections(page) {
  await page.locator('#connect-button').click();
  await idle(page);
}

export async function openSearch(page, query = '') {
  await page.locator('#query').fill(query);
  await button(page, '查找');
  await idle(page);
}

export async function openDream(page) {
  await navigate(page, '记忆');
  await button(page, '整理记忆');
  await idle(page);
}

export async function openContinuation(page) {
  await page.locator('[data-action="open-continuation"]').click();
  await page.locator('#continuation-panel').waitFor();
  await idle(page);
}

export async function closeContinuation(page) {
  await page.locator('[data-action="close-continuation"]').click();
  await page.locator('#continuation-panel').waitFor({ state: 'detached' });
  await idle(page);
}

export async function openNote(page) {
  await expandDetails(page, '#note-import-details');
}
