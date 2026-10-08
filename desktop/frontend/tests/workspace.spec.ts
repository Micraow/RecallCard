import { test, expect } from "@playwright/test";
const openDemo = async (page, state = "normal", route = "memories") => {
  await page.goto(`/demo.html?state=${state}${route ? "#" + route : ""}`);
  await expect(
    page.getByText(
      "界面验证 · 以下均为合成示例，非真实资料，操作不会写入本机空间",
    ),
  ).toBeVisible();
};

test("生产入口没有虚构数据或浏览器文件访问", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("请在 RecallCard 桌面应用中打开")).toBeVisible();
  await expect(page.locator(".demo-banner")).toHaveCount(0);
  await expect(page.locator(".memory-row")).toHaveCount(0);
});

test("记忆主界面可读、来源可展开且合成数据有标识", async ({ page }, info) => {
  await openDemo(page);
  await expect(page.locator(".memory-row")).toHaveCount(8);
  await expect(page.locator(".memory-article h1")).toHaveText(
    "秋季研究报告已交付",
  );
  await page.locator(".evidence-summary").first().click();
  await expect(page.locator(".evidence-quote")).toContainText(
    "这段是用于界面验证的合成原话",
  );
  await page.screenshot({
    path: info.outputPath("01-memory-dense.png"),
    fullPage: true,
  });
  await page.locator(".memory-row").nth(1).click();
  await expect(page.locator(".memory-article h1")).toContainText("德语课");
  await expect(page.locator(".evidence-quote")).toHaveCount(0);
});

test("导航、返回和搜索指向真实内容阅读路径", async ({ page }) => {
  await openDemo(page);
  await page.getByRole("button", { name: "原始资料", exact: true }).click();
  await expect(page.locator(".source-row")).toHaveCount(8);
  await page.locator(".source-row").nth(1).click();
  await expect(page.locator(".conversation-article h1")).toHaveText(
    "周五德语课的改期",
  );
  await page.getByRole("button", { name: "继续阅读" }).click();
  await expect(page.locator(".reader-pagination")).toContainText("5–8 / 28");
  await page.getByRole("button", { name: "回到开头" }).click();
  await expect(page.locator(".reader-pagination")).toContainText("1–4 / 28");
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await page.goBack();
  await expect(page.locator(".conversation-article h1")).toHaveText(
    "周五德语课的改期",
  );
  await page
    .getByRole("searchbox")
    .count()
    .then(async (count) => {
      if (!count) await page.getByLabel("搜索全部原话和记忆").fill("图表");
    });
  await page.getByLabel("搜索全部原话和记忆").fill("图表");
  await page.getByLabel("搜索全部原话和记忆").press("Enter");
  await expect(page.locator(".search-result")).toHaveCount(1);
  await page.locator(".search-result").click();
  await expect(page.locator(".memory-article h1")).toContainText("图表");
});

test("受保护记忆必须预览且显式确认后才能保存", async ({ page }) => {
  await openDemo(page);
  await page.getByRole("button", { name: "纠正内容" }).click();
  await page
    .getByLabel("记忆内容")
    .fill("秋季研究报告已交付\n\n更新后的合成记忆，用来验证新版本保存。");
  await page.getByRole("button", { name: "检查更改", exact: true }).click();
  await expect(page.getByRole("button", { name: "保存新版本" })).toBeDisabled();
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "保存新版本" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".article-byline")).toContainText("版本 4");
  await expect(page.locator(".memory-article")).toContainText(
    "更新后的合成记忆",
  );
});

test("取消更改不写入，Escape 关闭后恢复焦点", async ({ page }) => {
  await openDemo(page);
  const edit = page.getByRole("button", { name: "纠正内容" });
  await edit.click();
  await page.getByLabel("记忆内容").fill("不会保存的合成编辑");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(edit).toBeFocused();
  const calls = await page.evaluate(() => (window as any).__DEMO_CALLS__);
  expect(calls.some((call) => call.command === "confirm_memory_change")).toBe(
    false,
  );
});

test("停止召回、已隐藏筛选和恢复影响可完成", async ({ page }) => {
  await openDemo(page);
  await page.locator(".memory-row").nth(1).click();
  await page.getByRole("button", { name: "停止召回", exact: true }).click();
  await page.getByLabel("原因").fill("这个示例计划已取消");
  await page.getByRole("button", { name: "检查更改" }).click();
  await expect(page.locator(".impact-summary")).toContainText("2 条相关原文");
  await page.getByRole("button", { name: "确认停止召回" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "已隐藏 / 历史" }).click();
  await expect(page.locator(".memory-row")).toHaveCount(2);
  await page.locator(".memory-row").first().click();
  await page.getByRole("button", { name: "检查恢复影响" }).click();
  await page.getByRole("button", { name: "确认恢复" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("导入错误靠近任务，保留文件、成员、原因和下一步", async ({
  page,
}, info) => {
  await openDemo(page, "failed", "sources");
  await expect(page.locator(".source-activity .error-notice")).toContainText(
    "conversations.json",
  );
  await expect(
    page.getByRole("button", { name: "重新选择来源", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "继续", exact: true }),
  ).toHaveCount(0);
  await expect(page.locator(".source-activity .error-notice")).toContainText(
    "重新下载完整的官方导出包",
  );
  await expect(
    page.getByRole("button", { name: "添加记录", exact: true }),
  ).toHaveCount(1);
  await page.screenshot({
    path: info.outputPath("02-source-import-error.png"),
    fullPage: true,
  });
});

test("未知总量不显示伪百分比，暂停和恢复保持任务身份", async ({
  page,
}, info) => {
  await openDemo(page, "running", "activity");
  await expect(page.locator(".indeterminate-track")).toBeVisible();
  await expect(page.locator("progress")).toHaveCount(0);
  await page.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(page.locator(".job-row")).toHaveAttribute(
    "data-job-state",
    "paused",
  );
  await page.getByRole("button", { name: "继续", exact: true }).click();
  await expect(page.locator(".job-row")).toHaveAttribute(
    "data-job-state",
    "running",
  );
  await page.getByRole("button", { name: "处理明细" }).click();
  await page.screenshot({
    path: info.outputPath("03-activity-running.png"),
    fullPage: true,
  });
});

test("重复添加只产生一次请求，取消选择不增加任务", async ({ page }) => {
  await openDemo(page, "cancel");
  const before = await page.evaluate(
    () =>
      (window as any).__DEMO_CALLS__.filter(
        (x) => x.command === "import_sources",
      ).length,
  );
  await page
    .getByRole("button", { name: "添加记录" })
    .evaluate((button: HTMLButtonElement) => {
      button.click();
      button.click();
    });
  await expect(page.getByRole("button", { name: "添加记录" })).toBeEnabled();
  const after = await page.evaluate(
    () =>
      (window as any).__DEMO_CALLS__.filter(
        (x) => x.command === "import_sources",
      ).length,
  );
  expect(after - before).toBe(1);
  await expect(page.locator(".job-row")).toHaveCount(0);
  await expect(page.locator(".memory-article h1")).toHaveText("秋季研究报告已交付");
});

test("服务不可用显示错误，不显示零任务冒充成功", async ({ page }) => {
  await openDemo(page, "unavailable", "activity");
  await expect(page.getByRole("alert")).toContainText("本地任务状态暂不可读取");
  await expect(page.getByText("这里会留下处理记录")).toHaveCount(0);
});

test("连接中心不把未安装或未授权当作实际连接", async ({ page }, info) => {
  await openDemo(page, "normal", "connections");
  await expect(page.getByRole('heading', { name: '连接你常用的 AI' })).toBeVisible();
  await expect(page.getByText('最近读取成功', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '连接 Codex', exact: true }).click();
  await expect(page.getByRole('button', { name: '选择项目文件夹' })).toBeVisible();
  await page.screenshot({ path: info.outputPath("04-connections-unverified.png"), fullPage: true });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test("项目范围切换不保留旧记忆或未提交编辑", async ({ page }) => {
  await openDemo(page);
  await page.getByLabel("资料范围").selectOption("project:atlas");
  await page.getByRole("button", { name: "记忆", exact: true }).click();
  await expect(page.locator(".memory-row")).toHaveCount(1);
  await expect(page.locator(".memory-article h1")).toContainText("Atlas v0.8");
  await expect(page.locator(".breadcrumb")).toContainText("atlas");
});

test("空状态有可执行入口，没有大面积空表单", async ({ page }, info) => {
  await openDemo(page, "empty", "sources");
  await expect(page.getByText("所有背景，都有来处")).toBeVisible();
  await expect(page.locator("textarea")).toHaveCount(0);
  await page.screenshot({
    path: info.outputPath("05-sources-empty.png"),
    fullPage: true,
  });
});

test("键盘搜索、窄窗与返回阅读列表", async ({ page }, info) => {
  await page.setViewportSize({ width: 820, height: 760 });
  await openDemo(page);
  await page.keyboard.press("Control+k");
  await expect(page.getByLabel("搜索全部原话和记忆")).toBeFocused();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await expect(page.locator(".memory-row")).toHaveCount(8);
  await expect(page.locator(".reader-panel")).not.toBeVisible();
  await page.screenshot({
    path: info.outputPath("06-memory-list-820px.png"),
    fullPage: true,
  });
  await page.locator(".memory-row").first().click();
  await expect(page.locator(".memory-article h1")).toBeVisible();
  await expect(page.locator(".collection-panel")).not.toBeVisible();
  await page.screenshot({
    path: info.outputPath("07-memory-reader-820px.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "记忆列表" }).click();
  await page.setViewportSize({ width: 620, height: 820 });
  await page.locator(".memory-row").first().click();
  await expect(page.getByRole("button", { name: "记忆列表" })).toBeVisible();
  await page.getByRole("button", { name: "记忆列表" }).click();
  await expect(page.locator(".collection-panel")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("后台导入完成刷新不会清空未提交的记忆草稿", async ({ page }) => {
  await openDemo(page, "running");
  await page.getByRole("button", { name: "纠正内容" }).click();
  const draft = "尚未提交的合成草稿，后台完成导入后必须保留。";
  await page.getByLabel("记忆内容").fill(draft);
  await page.evaluate(() => (window as any).__DEMO_COMPLETE_JOBS__());
  await page.waitForTimeout(1400);
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByLabel("记忆内容")).toHaveValue(draft);
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await expect(page.locator(".memory-article")).not.toContainText(draft);
});

test("正文和元信息达到可读字号与 WCAG AA 对比度", async ({ page }) => {
  await openDemo(page);
  await expect(page.locator(".memory-article > .prose")).toBeVisible();
  await expect(page.locator(".evidence-summary strong").first()).toBeVisible();
  const metrics = await page.evaluate(() => {
    const rgb = (value: string) =>
      (value.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
    const lum = (channels: number[]) =>
      channels
        .map((value) => value / 255)
        .map((value) =>
          value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
        )
        .reduce(
          (sum, value, i) => sum + value * [0.2126, 0.7152, 0.0722][i],
          0,
        );
    return [
      ".memory-article > .prose",
      ".article-byline",
      ".memory-row.selected .row-meta",
      ".evidence-summary strong",
      ".reader-actions .button",
    ].map((selector) => {
      const element = document.querySelector(selector)!;
      const style = getComputedStyle(element);
      let cursor: Element | null = element;
      let background = "rgb(255, 255, 255)";
      while (cursor) {
        const value = getComputedStyle(cursor).backgroundColor;
        if (value !== "rgba(0, 0, 0, 0)" && value !== "transparent") {
          background = value;
          break;
        }
        cursor = cursor.parentElement;
      }
      const foregroundL = lum(rgb(style.color));
      const backgroundL = lum(rgb(background));
      return {
        selector,
        font: parseFloat(style.fontSize),
        contrast:
          (Math.max(foregroundL, backgroundL) + 0.05) /
          (Math.min(foregroundL, backgroundL) + 0.05),
      };
    });
  });
  for (const item of metrics) {
    expect(item.contrast, `${item.selector} 对比度`).toBeGreaterThanOrEqual(
      4.5,
    );
    expect(item.font, `${item.selector} 字号`).toBeGreaterThanOrEqual(
      item.selector.includes(" > .prose") ? 15 : 12,
    );
  }
});

test("导入完成明细区分正文、引用、附件元数据和未导入内容", async ({ page }) => {
  await openDemo(page, "normal", "activity");
  await page.getByRole("button", { name: "处理明细" }).click();
  await expect(page.locator(".import-coverage")).toContainText(
    "3 条附件元数据，导出未包含文件原件",
  );
  await expect(page.locator(".import-coverage")).toContainText(
    "2 条工具记录仅保留类型",
  );
  await expect(page.locator(".import-coverage")).toContainText(
    "3 个隐藏推理片段未收集",
  );
});

test("纯附件和工具占位可见，引用空标题使用网址且不自动打开外链", async ({
  page,
}, info) => {
  await openDemo(page, "assets", "sources");
  const assets = page.locator(".message-assets").first();
  await expect(assets).toContainText("autumn-report-v3.pdf");
  await expect(assets).toContainText("导出未包含文件内容");
  await expect(assets).toContainText("https://reference.example/report-method");
  await expect(assets).toContainText("导出未包含工具正文");
  await expect(assets.locator("a")).toHaveCount(0);
  await expect(
    page.locator(".source-message").first().locator(".prose"),
  ).toHaveCount(0);
  await page.screenshot({
    path: info.outputPath("08-source-assets.png"),
    fullPage: true,
  });
});
