import { test, expect, type Page } from '@playwright/test';
async function open(page: Page, state = 'normal') {
  await page.goto(`/demo.html?state=${state}#connections`);
  await expect(page.getByRole('heading', { name: '连接你常用的 AI' })).toBeVisible();
}
const calls = (page: Page) => page.evaluate(() => (window as any).__DEMO_CALLS__ as { command: string; args: any }[]);
async function agentPlan(page: Page, client = 'Claude Code') {
  await page.getByRole('button', { name: `连接 ${client}`, exact: true }).click();
  await page.getByRole('button', { name: '选择项目文件夹' }).click();
  await expect(page.getByRole('dialog')).toContainText('将合并这些文件');
}
test('Agent项目预览只读，批准后一次安装并区分本机试读与客户端读取', async ({ page }, info) => {
  await open(page);
  await agentPlan(page);
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: '授权并安装连接' })).toBeDisabled();
  await expect(dialog.getByRole('checkbox')).not.toBeChecked();
  expect((await calls(page)).some(call => call.command === 'apply_connection_setup')).toBe(false);
  await dialog.getByRole('checkbox').check();
  await dialog.getByRole('button', { name: '授权并安装连接' }).evaluate(button => { (button as HTMLButtonElement).click(); (button as HTMLButtonElement).click(); });
  await expect(dialog).toContainText('项目接入配置已写入');
  await expect(dialog).toContainText('本机试读通过');
  await expect(dialog).not.toContainText('收到客户端读取');
  const requests = await calls(page);
  expect(requests.filter(call => call.command === 'apply_connection_setup')).toHaveLength(1);
  expect(requests.find(call => call.command === 'connection_setup_plan')?.args).toMatchObject({ client: 'claude_code', projectDir: '/合成示例/projects/atlas', scope: 'personal', sessionId: 'demo_session' });
  expect(requests.some(call => call.command === 'prepare_client_config')).toBe(false);
  await page.screenshot({ path: info.outputPath('agent-local-proof.png'), fullPage: true });
  await page.evaluate(() => (window as any).__DEMO_HOST_READ__());
  await dialog.getByRole('button', { name: '检测连接状态' }).click();
  await expect(dialog).toContainText('收到客户端读取');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: '连接 Claude Code', exact: true })).toBeFocused();
});
test('返回和取消清除未批准选择，原生选目录取消不产生计划', async ({ page }) => {
  await open(page);
  await agentPlan(page, 'Codex');
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '返回', exact: true }).click();
  await page.getByRole('button', { name: '选择项目文件夹' }).click();
  await expect(page.getByRole('dialog').getByRole('checkbox')).not.toBeChecked();
  await page.keyboard.press('Escape');
  expect((await calls(page)).some(call => call.command === 'apply_connection_setup')).toBe(false);
  await open(page, 'cancel-project');
  await page.getByRole('button', { name: '连接 Codex', exact: true }).click();
  await page.getByRole('button', { name: '选择项目文件夹' }).click();
  await expect(page.getByRole('button', { name: '选择项目文件夹' })).toBeEnabled();
  expect((await calls(page)).some(call => call.command === 'connection_setup_plan')).toBe(false);
});
test('安装已写但试读失败只重试读取，不重复安装', async ({ page }) => {
  await open(page, 'verify-failed');
  await agentPlan(page, 'Codex');
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '授权并安装连接' }).click();
  await expect(page.getByRole('dialog')).toContainText('配置已写入，但本机读取组件暂不可用');
  await page.getByRole('button', { name: '重新试读', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('本机试读通过');
  const requests = await calls(page);
  expect(requests.filter(call => call.command === 'apply_connection_setup')).toHaveLength(1);
  expect(requests.filter(call => call.command === 'verify_connection')).toHaveLength(1);
});
test('不确定安装结果只允许检查，不盲目重发配置', async ({ page }) => {
  await open(page, 'install-error');
  await agentPlan(page);
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '授权并安装连接' }).click();
  await expect(page.getByRole('dialog')).toContainText('安装结果暂不可确认');
  await expect(page.getByRole('button', { name: '授权并安装连接' })).toHaveCount(0);
  await page.getByRole('button', { name: '检测连接状态' }).click();
  expect((await calls(page)).filter(call => call.command === 'apply_connection_setup')).toHaveLength(1);
});
test('浏览器测试版先定位真实包，编号只出现在开发版高级配对', async ({ page }, info) => {
  await open(page);
  await page.getByRole('button', { name: '安装测试扩展', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('尚未上架商店');
  await expect(page.getByLabel('扩展连接编号')).not.toBeVisible();
  await page.getByRole('button', { name: '打开测试扩展文件夹' }).click();
  expect((await calls(page)).filter(call => call.command === 'open_browser_extension_directory')).toHaveLength(1);
  await page.getByText('开发版手动配对（高级）', { exact: true }).click();
  await page.getByLabel('扩展连接编号').fill(`recallcard-connect/1:${'a'.repeat(32)}:11111111-1111-4111-8111-111111111111`);
  await page.getByRole('button', { name: '登记本机桥并继续' }).click();
  await expect(page.getByRole('dialog')).toContainText('等待扩展发来请求');
  await expect(page.getByRole('dialog')).not.toContainText('已连接');
  await page.evaluate(() => (window as any).__DEMO_PAIR__());
  await page.getByRole('button', { name: '检测连接请求', exact: true }).click();
  await expect(page.getByRole('button', { name: '审阅这个连接' })).toBeVisible();
  await page.screenshot({ path: info.outputPath('browser-pairing-request.png'), fullPage: true });
  await page.getByRole('button', { name: '审阅这个连接' }).click();
  await expect(page.getByRole('dialog')).toContainText('审阅连接请求');
  await expect(page.getByRole('dialog')).toContainText('11111111-1111-4111-8111-111111111111');
});
test('缺少测试包不显示可安装或已连接，只保留清楚的恢复办法', async ({ page }) => {
  await open(page, 'missing-extension');
  await page.getByRole('button', { name: '安装测试扩展', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('未找到可核验的测试扩展');
  await expect(page.getByRole('button', { name: '打开测试扩展文件夹' })).toBeDisabled();
});
test('浏览器捕获与资料提供分别批准，之后可独立暂停和撤销', async ({ page }) => {
  await open(page, 'pairing');
  await page.getByRole('button', { name: '审阅连接请求' }).first().click();
  await expect(page.getByRole('button', { name: '批准此连接' })).toBeDisabled();
  await page.getByRole('checkbox', { name: '允许将这个网站的对话保存到此范围' }).check();
  await page.getByRole('checkbox', { name: '在对话稳定结束后自动保存' }).check();
  await page.getByRole('button', { name: '批准此连接' }).evaluate(button => { (button as HTMLButtonElement).click(); (button as HTMLButtonElement).click(); });
  await expect(page.getByRole('dialog')).toHaveCount(0);
  const approvals = (await calls(page)).filter(call => call.command === 'connection_approve_pairing');
  expect(approvals).toHaveLength(1);
  expect(approvals[0].args.grant).toMatchObject({ recall_scopes: [], capture_scopes: ['personal'], provider_disclosure: false, auto_capture: true, auto_recall: false });
  await expect(page.getByText('读取：未授权', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '暂停自动保存', exact: true }).click();
  await expect(page.getByRole('button', { name: '开启自动保存', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '撤销访问', exact: true }).click();
  await page.getByRole('button', { name: '确认撤销访问', exact: true }).click();
  await expect(page.locator('[data-connection-state="revoked"]')).toHaveCount(1);
});
test('取消不保留浏览器未批准权限，切换空间不复用旧配对', async ({ page }) => {
  await open(page, 'pairing');
  await page.getByRole('button', { name: '审阅连接请求' }).first().click();
  await page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' }).check();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '审阅连接请求' }).first().click();
  await expect(page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' })).not.toBeChecked();
  await page.keyboard.press('Escape');
  await page.getByLabel('资料范围').selectOption('project:atlas');
  await page.getByRole('button', { name: '连接', exact: true }).click();
  await expect(page.getByRole('button', { name: '审阅连接请求' })).toHaveCount(0);
  expect((await calls(page)).some(call => call.command === 'connection_approve_pairing')).toBe(false);
});
test('断线后隐藏旧健康状态，重新检测恢复', async ({ page }) => {
  await open(page, 'connected');
  await expect(page.getByText('最近读取成功', { exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__DEMO_CONNECTIONS_OFFLINE__(true));
  await page.getByRole('button', { name: '刷新', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('连接状态暂不可读取');
  await expect(page.getByText('最近读取成功', { exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as any).__DEMO_CONNECTIONS_OFFLINE__(false));
  await page.getByRole('button', { name: '重试', exact: true }).click();
  await expect(page.getByText('最近读取成功', { exact: true })).toBeVisible();
});
test('820px连接中心和授权预览保持完整，键盘可返回', async ({ page }, info) => {
  await page.setViewportSize({ width: 820, height: 760 });
  await open(page);
  await page.screenshot({ path: info.outputPath('connections-820.png'), fullPage: true });
  await agentPlan(page, 'Codex');
  await expect(page.getByRole('button', { name: '授权并安装连接' })).toBeVisible();
  await page.screenshot({ path: info.outputPath('agent-consent-820.png'), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: '连接 Codex', exact: true })).toBeFocused();
});

test('安装中导航离开再返回不会提交第二次，取消会等待正在写入的真实结果', async ({ page }) => {
  await page.goto('/demo.html?state=slow-install#background');
  await page.getByRole('button', { name: '连接 AI', exact: true }).click();
  await agentPlan(page, 'Codex');
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '授权并安装连接' }).click();
  await expect(page.locator('.modal-footer').getByRole('button', { name: '关闭', exact: true })).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('heading', { name: '让下一次对话接得上' })).toBeVisible();
  await page.goForward();
  await page.getByRole('button', { name: '连接 Codex', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('项目接入配置已写入');
  await expect(page.getByRole('dialog')).toContainText('本机试读通过');
  await expect(page.getByRole('button', { name: '选择项目文件夹' })).toHaveCount(0);
  expect((await calls(page)).filter(call => call.command === 'apply_connection_setup')).toHaveLength(1);
});

test('检查权限原样关闭不递增授权版本、不清空读取回执', async ({ page }) => {
  await open(page, 'connected');
  await page.getByRole('button', { name: '检查权限', exact: true }).click();
  await expect(page.getByRole('button', { name: '保存这次授权' })).toBeDisabled();
  await expect(page.getByRole('dialog')).toContainText('当前权限没有变化');
  await page.keyboard.press('Escape');
  await expect(page.getByText('最近读取成功', { exact: true })).toBeVisible();
  expect((await calls(page)).filter(call => call.command === 'connection_configure')).toHaveLength(0);
});

test('安装期间离开再返回仍显示配置已写与真实试读错误', async ({ page }) => {
  await page.goto('/demo.html?state=slow-verify-error#background');
  await page.getByRole('button', { name: '连接 AI', exact: true }).click();
  await agentPlan(page, 'Codex');
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '授权并安装连接' }).click();
  await page.goBack();
  await page.goForward();
  await page.getByRole('button', { name: '连接 Codex', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('项目接入配置已写入');
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText('本机读取程序暂不可用');
  await expect(page.getByRole('button', { name: '重新试读', exact: true })).toBeVisible();
  expect((await calls(page)).filter(call => call.command === 'apply_connection_setup')).toHaveLength(1);
});
test('安装结果页轮询短暂断线后自动恢复，不保留旧网络错误', async ({ page }) => {
  await open(page);
  await agentPlan(page, 'Codex');
  await page.getByRole('dialog').getByRole('checkbox').check();
  await page.getByRole('button', { name: '授权并安装连接' }).click();
  await expect(page.getByRole('dialog')).toContainText('本机试读通过');
  await page.evaluate(() => (window as any).__DEMO_CONNECTIONS_OFFLINE__(true));
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText('连接状态暂不可读取', { timeout: 6000 });
  await page.evaluate(() => { (window as any).__DEMO_CONNECTIONS_OFFLINE__(false); (window as any).__DEMO_HOST_READ__(); });
  await expect(page.getByRole('dialog').getByRole('alert')).toHaveCount(0, { timeout: 6000 });
  await expect(page.getByRole('dialog')).toContainText('收到客户端读取');
});
test('批准期间权限表单冻结，显示的范围与实际提交保持一致', async ({ page }) => {
  await open(page, 'slow-pairing');
  await page.getByRole('button', { name: '审阅连接请求' }).click();
  const capture = page.getByRole('checkbox', { name: '允许将这个网站的对话保存到此范围' });
  await capture.check();
  await page.getByRole('button', { name: '批准此连接' }).click();
  await expect(capture).toBeDisabled();
  await expect(capture).toBeChecked();
  await expect(page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' })).toBeDisabled();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect((await calls(page)).filter(call => call.command === 'connection_approve_pairing')[0].args.grant.capture_scopes).toEqual(['personal']);
});
