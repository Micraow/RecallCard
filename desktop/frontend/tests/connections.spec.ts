import { test, expect, type Page } from '@playwright/test';
async function open(page: Page, state = 'normal') {
  await page.goto(`/demo.html?state=${state}#connections`);
  await expect(page.getByRole('heading', { name: '把背景带到你使用的 AI' })).toBeVisible();
}
const calls = (page: Page) => page.evaluate(() => (window as any).__DEMO_CALLS__ as { command: string; args: any }[]);
async function agentConsent(page: Page) {
  await page.getByRole('button', { name: '连接 Claude Code', exact: true }).click();
  await page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' }).check();
  await page.getByRole('checkbox', { name: '允许在开始、恢复、压缩后加载背景，并执行只读查询' }).check();
  await page.getByRole('checkbox', { name: '我同意将上述范围的资料提供给这个接收方' }).check();
}
test('Agent先批准范围才生成绑定ID的两份片段，配置不冒充已安装', async ({ page }, info) => {
  await open(page);
  await page.getByRole('button', { name: '连接 Claude Code', exact: true }).click();
  await expect(page.getByRole('button', { name: '保存这次授权' })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' })).not.toBeChecked();
  await page.keyboard.press('Escape');
  expect((await calls(page)).some(call => call.command === 'connection_configure')).toBe(false);
  await agentConsent(page);
  await page.getByRole('button', { name: '保存这次授权' }).click();
  await expect(page.getByRole('dialog')).toContainText('宿主安装尚未验证');
  await page.getByText('查看 MCP 片段', { exact: true }).click();
  await page.getByText('查看 SessionStart 片段', { exact: true }).click();
  await expect(page.getByTestId('agent-mcp-config')).toContainText('--connection-id');
  await expect(page.getByTestId('agent-hooks-config')).toContainText('--connection-id');
  const requests = await calls(page); const saved = requests.find(call => call.command === 'connection_configure');
  expect(saved?.args.grant).toMatchObject({ client_kind: 'claude_code', installation_id: null, recall_scopes: ['personal'], capture_scopes: [], provider_disclosure: true, auto_recall: true });
  expect(requests.some(call => call.command === 'prepare_client_config')).toBe(false);
  await page.screenshot({ path: info.outputPath('managed-agent-unverified.png'), fullPage: true });
  await page.getByRole('button', { name: '完成', exact: true }).click();
  await expect(page.getByText('已配置 · 尚未验证', { exact: true })).toBeVisible();
});
test('浏览器配对捕获与提供资料分开，一次批准后可独立暂停和撤销', async ({ page }) => {
  await open(page);
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
test('取消后不记住未批准权限，切换空间不复用旧配对', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: '审阅连接请求' }).first().click();
  await page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' }).check();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '审阅连接请求' }).first().click();
  await expect(page.getByRole('checkbox', { name: '允许读取此范围的背景、记忆与原始来源' })).not.toBeChecked();
  await page.keyboard.press('Escape');
  await page.getByLabel('资料范围').selectOption('project:atlas');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '审阅连接请求' })).toHaveCount(0);
  expect((await calls(page)).some(call => call.command === 'connection_approve_pairing')).toBe(false);
});
