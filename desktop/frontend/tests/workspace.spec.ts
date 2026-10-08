import { test, expect } from '@playwright/test';
const openDemo = async (page, state = 'normal', route = '') => {
  await page.goto(`/demo.html?state=${state}${route ? '#' + route : ''}`);
  await expect(page.getByText('界面验证 · 以下均为合成示例，非真实资料，操作不会写入本机空间')).toBeVisible();
};

test('生产入口没有虚构数据或浏览器文件访问', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('请在 RecallCard 桌面应用中打开')).toBeVisible();
  await expect(page.locator('.demo-banner')).toHaveCount(0);
  await expect(page.locator('.memory-row')).toHaveCount(0);
});

test('记忆主界面可读、来源可展开且合成数据有标识', async ({ page }, info) => {
  await openDemo(page);
  await expect(page.locator('.memory-row')).toHaveCount(8);
  await expect(page.locator('.memory-article h1')).toHaveText('项目交接：保留决定，也保留原因');
  await page.locator('.evidence-summary').first().click();
  await expect(page.locator('.evidence-quote')).toContainText('这段是用于界面验证的合成原话');
  await page.screenshot({ path: info.outputPath('01-memory-dense.png'), fullPage: true });
  await page.locator('.memory-row').nth(1).click();
  await expect(page.locator('.memory-article h1')).toContainText('研究笔记');
  await expect(page.locator('.evidence-quote')).toHaveCount(0);
});

test('导航、返回和搜索指向真实内容阅读路径', async ({ page }) => {
  await openDemo(page);
  await page.getByRole('button', { name: '来源', exact: true }).click();
  await expect(page.locator('.source-row')).toHaveCount(8);
  await page.locator('.source-row').nth(1).click();
  await expect(page.locator('.conversation-article h1')).toHaveText('研究笔记与实验结论的写法');
  await page.getByRole('button', { name: '继续阅读' }).click();
  await expect(page.locator('.reader-pagination')).toContainText('5–8 / 28');
  await page.getByRole('button', { name: '回到开头' }).click();
  await expect(page.locator('.reader-pagination')).toContainText('1–4 / 28');
  await page.getByRole('button', { name: '连接', exact: true }).click();
  await page.goBack();
  await expect(page.locator('.conversation-article h1')).toHaveText('研究笔记与实验结论的写法');
  await page.getByRole('searchbox').count().then(async count => { if (!count) await page.getByLabel('搜索全部原话和记忆').fill('研究'); });
  await page.getByLabel('搜索全部原话和记忆').fill('研究');
  await page.getByLabel('搜索全部原话和记忆').press('Enter');
  await expect(page.locator('.search-result')).toHaveCount(1);
  await page.locator('.search-result').click();
  await expect(page.locator('.memory-article h1')).toContainText('研究笔记');
});

test('受保护记忆必须预览且显式确认后才能保存', async ({ page }) => {
  await openDemo(page);
  await page.getByRole('button', { name: '纠正内容' }).click();
  await page.getByLabel('记忆内容').fill('项目交接：保留决定，也保留原因\n\n更新后的合成记忆，用来验证新版本保存。');
  await page.getByRole('button', { name: '检查更改', exact: true }).click();
  await expect(page.getByRole('button', { name: '保存新版本' })).toBeDisabled();
  await page.getByRole('checkbox').check();
  await page.getByRole('button', { name: '保存新版本' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.article-byline')).toContainText('版本 4');
  await expect(page.locator('.memory-article')).toContainText('更新后的合成记忆');
});

test('取消更改不写入，Escape 关闭后恢复焦点', async ({ page }) => {
  await openDemo(page);
  const edit = page.getByRole('button', { name: '纠正内容' });
  await edit.click();
  await page.getByLabel('记忆内容').fill('不会保存的合成编辑');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(edit).toBeFocused();
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(calls.some(call => call.command === 'confirm_memory_change')).toBe(false);
});

test('停止召回、已隐藏筛选和恢复影响可完成', async ({ page }) => {
  await openDemo(page);
  await page.locator('.memory-row').nth(1).click();
  await page.getByRole('button', { name: '停止召回', exact: true }).click();
  await page.getByLabel('原因').fill('这个示例计划已取消');
  await page.getByRole('button', { name: '检查更改' }).click();
  await expect(page.locator('.impact-summary')).toContainText('2 条相关原文');
  await page.getByRole('button', { name: '确认停止召回' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.getByRole('button', { name: '已隐藏 / 历史' }).click();
  await expect(page.locator('.memory-row')).toHaveCount(2);
  await page.locator('.memory-row').first().click();
  await page.getByRole('button', { name: '检查恢复影响' }).click();
  await page.getByRole('button', { name: '确认恢复' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('导入错误靠近任务，保留文件、成员、原因和下一步', async ({ page }, info) => {
  await openDemo(page, 'failed', 'sources');
  await expect(page.locator('.source-activity .error-notice')).toContainText('conversations.json');
  await expect(page.locator('.source-activity .error-notice')).toContainText('重新下载完整的官方导出包');
  await expect(page.getByRole('button', { name: '添加来源', exact: true })).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('02-source-import-error.png'), fullPage: true });
});

test('未知总量不显示伪百分比，暂停和恢复保持任务身份', async ({ page }, info) => {
  await openDemo(page, 'running', 'activity');
  await expect(page.locator('.indeterminate-track')).toBeVisible();
  await expect(page.locator('progress')).toHaveCount(0);
  await page.getByRole('button', { name: '暂停', exact: true }).click();
  await expect(page.locator('.job-row')).toHaveAttribute('data-job-state', 'paused');
  await page.getByRole('button', { name: '继续', exact: true }).click();
  await expect(page.locator('.job-row')).toHaveAttribute('data-job-state', 'running');
  await page.getByRole('button', { name: '处理明细' }).click();
  await page.screenshot({ path: info.outputPath('03-activity-running.png'), fullPage: true });
});

test('重复添加只产生一次请求，取消选择不增加任务', async ({ page }) => {
  await openDemo(page, 'cancel');
  const before = await page.evaluate(() => (window as any).__DEMO_CALLS__.filter(x => x.command === 'import_sources').length);
  await page.getByRole('button', { name: '添加来源' }).evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
  await expect(page.getByRole('button', { name: '添加来源' })).toBeEnabled();
  const after = await page.evaluate(() => (window as any).__DEMO_CALLS__.filter(x => x.command === 'import_sources').length);
  expect(after - before).toBe(1);
  await expect(page.locator('.job-row')).toHaveCount(0);
});

test('服务不可用显示错误，不显示零任务冒充成功', async ({ page }) => {
  await openDemo(page, 'unavailable', 'activity');
  await expect(page.getByRole('alert')).toContainText('本地任务状态暂不可读取');
  await expect(page.getByText('这里会留下处理记录')).toHaveCount(0);
});

test('连接配置和注册不冒充实际连接', async ({ page }, info) => {
  await openDemo(page, 'normal', 'connections');
  await expect(page.getByText('尚未验证', { exact: true })).toHaveCount(2);
  await page.getByRole('button', { name: '设置连接' }).first().click();
  await page.getByRole('button', { name: '生成本地配置' }).click();
  await expect(page.getByRole('dialog')).toContainText('尚未收到宿主读取回执');
  await page.getByRole('button', { name: '完成', exact: true }).click();
  await expect(page.getByText('配置已生成 · 未验证', { exact: true })).toBeVisible();
  await page.screenshot({ path: info.outputPath('04-connections-unverified.png'), fullPage: true });
  await page.getByRole('button', { name: '设置连接' }).nth(1).click();
  await expect(page.getByRole('checkbox')).not.toBeChecked();
  await expect(page.getByRole('button', { name: '检查并注册连接' })).toBeDisabled();
  await page.getByLabel('RecallCard 扩展 ID').fill('a'.repeat(32));
  await page.getByRole('button', { name: '检查并注册连接' }).click();
  await expect(page.getByRole('dialog')).toContainText('已注册，等待浏览器检查');
});

test('项目范围切换不保留旧记忆或未提交编辑', async ({ page }) => {
  await openDemo(page);
  await page.getByLabel('资料范围').selectOption('project:atlas');
  await expect(page.locator('.memory-row')).toHaveCount(1);
  await expect(page.locator('.memory-article h1')).toContainText('Atlas 的离线体验');
  await expect(page.locator('.breadcrumb')).toContainText('atlas');
});

test('空状态有可执行入口，没有大面积空表单', async ({ page }, info) => {
  await openDemo(page, 'empty', 'sources');
  await expect(page.getByText('所有背景，都有来处')).toBeVisible();
  await expect(page.locator('textarea')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('05-sources-empty.png'), fullPage: true });
});

test('键盘搜索、窄窗与返回阅读列表', async ({ page }, info) => {
  await page.setViewportSize({ width: 820, height: 760 });
  await openDemo(page);
  await page.keyboard.press('Control+k');
  await expect(page.getByLabel('搜索全部原话和记忆')).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('06-memory-820px.png'), fullPage: true });
  await page.setViewportSize({ width: 620, height: 820 });
  await page.locator('.memory-row').first().click();
  await expect(page.getByRole('button', { name: '记忆列表' })).toBeVisible();
  await page.getByRole('button', { name: '记忆列表' }).click();
  await expect(page.locator('.collection-panel')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
