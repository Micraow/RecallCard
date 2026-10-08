import { test, expect } from "@playwright/test";

// Browser tests use the explicit synthetic entry. Native first_use_native.py
// imports the same official-shaped archive through the real file chooser.
test("首次导入不要求选择格式、空间或模型，直接显示有出处的原话", async ({
  page,
}, info) => {
  await page.goto("/demo.html?state=first-use");
  await expect(
    page.getByRole("button", { name: "导入聊天记录" }),
  ).toBeVisible();
  await expect(page.getByLabel("资料范围")).not.toBeVisible();
  await page.getByRole("button", { name: "导入聊天记录" }).click();
  await expect(
    page.getByRole("heading", { name: "从这些原话继续" }),
  ).toBeVisible();
  await expect(page.locator(".context-conversation")).toHaveCount(4);
  await expect(page.locator(".context-excerpt").first()).toContainText(
    "正文撰写可以标为完成",
  );
  await expect(page.locator(".background-fact")).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(
    calls.filter((call: any) => call.command === "open_default_workspace"),
  ).toHaveLength(1);
  expect(
    calls.filter((call: any) => call.command === "import_sources"),
  ).toHaveLength(1);
  expect(
    calls.some((call: any) => /configure|dream|model_setup/.test(call.command)),
  ).toBe(false);
  await page.screenshot({
    path: info.outputPath("01-first-value.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "回到出处" }).first().click();
  await expect(page.locator(".focused-message")).toContainText(
    "正文撰写可以标为完成",
  );
  await expect(page.locator(".source-position")).toHaveText("已定位到这条原话");
  await page.screenshot({
    path: info.outputPath("02-exact-source.png"),
    fullPage: true,
  });
});

test("近况一次保存，立即搜索可见；取消不保存", async ({ page }, info) => {
  await page.goto("/demo.html?state=imported");
  await page.getByRole("button", { name: "补充最新情况" }).click();
  await page.getByLabel("最新情况").fill("这段草稿不会保存");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "补充最新情况" }).click();
  await page
    .getByLabel("最新情况")
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

test("ChatGPT 原生入口明确待连接，检查不创建授权", async ({ page }, info) => {
  await page.goto("/demo.html?state=imported");
  await page.getByRole("button", { name: "连接 ChatGPT" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("本机读取待授权");
  await expect(dialog).toContainText(
    "实际检索到的背景、原文和出处会提供给 OpenAI",
  );
  await expect(dialog).toContainText("长期使用的电脑");
  await expect(dialog).toContainText("待连接");
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(
    calls.some((call: any) =>
      /connection_configure|approve_pairing/.test(call.command),
    ),
  ).toBe(false);
  await page.screenshot({
    path: info.outputPath("04-chatgpt-pending.png"),
    fullPage: true,
  });
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "连接 ChatGPT" }),
  ).toBeFocused();
});

test("820px 加载后的背景、出处详情与返回列表都可阅读", async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 820, height: 760 });
  await page.goto("/demo.html?state=imported");
  await expect(page.locator(".context-excerpt").first()).toContainText(
    "正文撰写可以标为完成",
  );
  await page.screenshot({
    path: info.outputPath("05-background-820-loaded.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "回到出处" }).first().click();
  await expect(page.locator(".focused-message")).toBeVisible();
  await expect(page.locator(".collection-panel")).not.toBeVisible();
  await page.screenshot({
    path: info.outputPath("06-source-820-loaded.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "来源列表", exact: true }).click();
  await expect(page.locator(".collection-panel")).toBeVisible();
  await expect(page.locator(".reader-panel")).not.toBeVisible();
  await page.screenshot({
    path: info.outputPath("07-source-list-820.png"),
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("定位失败保留原因，不假装已跳到原话", async ({ page }) => {
  await page.goto("/demo.html?state=location-error");
  await expect(page.locator(".context-excerpt").first()).toContainText("正文撰写可以标为完成");
  await page.getByRole("button", { name: "回到出处" }).first().click();
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
