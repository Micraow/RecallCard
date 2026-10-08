import { test, expect } from "@playwright/test";

// Browser tests use the explicit synthetic entry. Native first_use_native.py
// imports the same official-shaped archive through the real file chooser.
test("首次导入直接完成，首页呈现连接状态而非铺陈旧原话", async ({ page }, info) => {
  await page.goto("/demo.html?state=first-use");
  await expect(page.getByRole("button", { name: "导入聊天记录" })).toBeVisible();
  await expect(page.getByLabel("资料范围")).not.toBeVisible();
  await page.getByRole("button", { name: "导入聊天记录" }).click();
  await expect(page.getByRole("heading", { name: "让下一次对话接得上" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "你的 AI 连接" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "最近增强回执" })).toBeVisible();
  await expect(page.locator(".context-conversation")).toHaveCount(0);
  await expect(page.locator(".home-library-links")).toContainText("8");
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(calls.filter((call: any) => call.command === "open_default_workspace")).toHaveLength(1);
  expect(calls.filter((call: any) => call.command === "import_sources")).toHaveLength(1);
  expect(calls.some((call: any) => /configure|apply_connection/.test(call.command))).toBe(false);
  expect(calls.some((call: any) => call.command === "conversation_messages")).toBe(false);
  await page.screenshot({ path: info.outputPath("01-first-value.png"), fullPage: true });
  await page.getByRole("button", { name: "原始资料", exact: true }).click();
  await expect(page.locator(".source-row")).toHaveCount(8);
  await page.locator(".source-row").first().click();
  await expect(page.locator(".conversation-article")).toContainText("正文撰写可以标为完成");
  await page.screenshot({ path: info.outputPath("02-exact-source.png"), fullPage: true });
});

test("近况一次保存，立即搜索可见；取消不保存", async ({ page }, info) => {
  await page.goto("/demo.html?state=imported");
  await page.getByRole("button", { name: "补充最新情况" }).click();
  await page.getByRole("textbox", { name: "最新情况", exact: true }).fill("这段草稿不会保存");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "补充最新情况" }).click();
  await page
    .getByRole("textbox", { name: "最新情况", exact: true })
    .fill("合成最新情况：琥珀计划已经交付，改为周四回访。");
  await page.getByRole("button", { name: "保存近况" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByText("已保存你的补充，新的查找可以立即读到；原对话保持原样。"),
  ).toBeVisible();
  await page.getByLabel("搜索全部原话和记忆").fill("周四回访");
  await page.getByLabel("搜索全部原话和记忆").press("Enter");
  await expect(page.locator(".search-result")).toHaveCount(1);
  await expect(page.locator(".search-result")).toContainText("改为周四回访");
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(
    calls.filter((call: any) => call.command === "confirm_note"),
  ).toHaveLength(1);
  await page.screenshot({
    path: info.outputPath("03-update-search.png"),
    fullPage: true,
  });
});

test("首页主动作打开真实连接中心，返回和前进保留导航", async ({ page }, info) => {
  await page.goto("/demo.html?state=imported");
  await page.getByRole("button", { name: "连接 AI", exact: true }).click();
  await expect(page.getByRole("heading", { name: "连接你常用的 AI" })).toBeVisible();
  await expect(page.getByRole("button", { name: "连接 Codex", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "连接 Claude Code", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect((await page.evaluate(() => (window as any).__DEMO_CALLS__)).some((call: any) => /connection_configure|approve_pairing|apply_connection/.test(call.command))).toBe(false);
  await page.screenshot({ path: info.outputPath("04-connection-center.png"), fullPage: true });
  await page.goBack();
  await expect(page.getByRole("button", { name: "连接 AI", exact: true })).toBeVisible();
  await page.goForward();
  await expect(page.getByRole("heading", { name: "连接你常用的 AI" })).toBeVisible();
});

test("820px首页和原始资料保持可读，侧栏底部无需更多展开", async ({ page }, info) => {
  await page.setViewportSize({ width: 820, height: 760 });
  await page.goto("/demo.html?state=imported");
  await expect(page.getByRole("heading", { name: "最近增强回执" })).toBeVisible();
  await expect(page.getByRole("button", { name: "连接", exact: true })).toBeVisible();
  await expect(page.getByText("更多", { exact: true })).toHaveCount(0);
  const sidebar = page.locator(".sidebar");
  const top = await sidebar.getByRole('button', { name: '首页', exact: true }).boundingBox();
  const bottom = await sidebar.getByRole('button', { name: '连接', exact: true }).boundingBox();
  expect(bottom!.y - top!.y).toBeGreaterThan(250);
  await page.screenshot({ path: info.outputPath("05-background-820-loaded.png"), fullPage: true });
  await page.getByRole("button", { name: "原始资料", exact: true }).click();
  await page.locator('.source-row').first().click();
  await expect(page.locator(".collection-panel")).not.toBeVisible();
  await page.screenshot({ path: info.outputPath("06-source-820-loaded.png"), fullPage: true });
  await page.getByRole("button", { name: "来源列表", exact: true }).click();
  await expect(page.locator(".collection-panel")).toBeVisible();
  await expect(page.locator(".reader-panel")).not.toBeVisible();
  await page.screenshot({ path: info.outputPath("07-source-list-820.png"), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test("定位失败保留原因，不假装已跳到原话", async ({ page }) => {
  await page.goto("/demo.html?state=location-error#sources?item=source_demo_1&at=event%3Asource_demo_1_message_0");
  await expect(page.getByRole("alert")).toContainText("原话暂时无法定位");
  await expect(page.locator(".source-position")).toHaveCount(0);
});

test("坏文件给重新选择动作而非继续损坏快照", async ({ page }, info) => {
  await page.goto("/demo.html?state=failed#background");
  await expect(page.locator(".background-import")).toContainText(
    "conversations.json",
  );
  await expect(
    page.getByRole("button", { name: "重新选择文件" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "继续", exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: info.outputPath("08-import-recovery.png"),
    fullPage: true,
  });
});


test("首页只用已发生回执，不把握手或配置当作增强成功", async ({ page }, info) => {
  await page.goto("/demo.html?state=connected");
  await expect(page.getByRole('region', { name: '最近增强回执' })).toContainText('收到按需读取');
  await expect(page.getByRole('region', { name: '最近增强回执' })).toContainText('已保存原始对话');
  await expect(page.getByRole('region', { name: '最近增强回执' })).toContainText('不代表已完成记忆整理');
  await expect(page.locator('.context-conversation')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('09-home-receipts.png'), fullPage: true });
  await page.getByRole('button', { name: '1 条记忆待核对' }).click();
  await expect(page.getByRole('button', { name: '待确认', exact: true })).toHaveAttribute('aria-pressed', 'true');
});
