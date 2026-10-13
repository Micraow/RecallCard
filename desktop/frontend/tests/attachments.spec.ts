import { test, expect } from "@playwright/test";
import type { SourceAssets } from "../src/service/types";

const files: SourceAssets["files"] = [
  {
    source_file_id: "file-synthetic", name: '<img src=x onerror="window.syntheticExecuted=true">.pdf',
    byte_count: 512, payload_status: "retained", archive_name_alias: '<script>synthetic()</script>.dat',
    declared_media_type: "application/pdf", library_media_type: "application/octet-stream",
    detected_media_type: "application/zip", declared_media_type_conflict: true,
    object_sha256: "a".repeat(64),
  },
  { source_file_id: "file-missing", name: "synthetic-missing.pdf", byte_count: null, payload_status: "missing" },
  { source_file_id: "file-read-only", name: "synthetic-read-only.pdf", byte_count: 32, payload_status: "available_not_retained" },
  { source_file_id: "file-legacy", name: null, byte_count: null, payload_status: "not_in_export" },
];
const assets = (rows = files): SourceAssets => ({ files: rows, files_total: rows.length,
  citations: [], citations_total: 0, tool_trace: [], trace_total: 0, unsupported_total: 0, truncated: false });

test("附件真实组件安全呈现四种状态、独立别名和类型差异，窄屏与反复更新无残留", async ({ page }, info) => {
  await page.route("**/attachment-projection-test", route => route.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html lang="zh-CN"><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/styles.css"></head>
      <body><main id="attachment-fixture" style="max-width:900px;margin:16px"></main>
      <script type="module">import {renderAttachments} from '/tests/attachment-fixture.tsx';window.renderAttachments=renderAttachments;</script></body></html>`,
  }));
  await page.goto("/attachment-projection-test");
  await page.waitForFunction(() => typeof (window as any).renderAttachments === "function");
  await page.evaluate(value => (window as any).renderAttachments(value), assets());
  const view = page.locator(".message-assets");
  await expect(view.locator(".asset-row")).toHaveCount(4);
  await expect(view.locator("strong").first()).toHaveText(files[0].name!);
  await expect(view).toContainText(`归档文件名：${files[0].archive_name_alias}`);
  await expect(view).toContainText("原件已保留");
  await expect(view).toContainText("所提供归档中未找到原件");
  await expect(view).toContainText("只读解析已发现原始字节，本次未保留");
  await expect(view).toContainText("旧记录仅有元数据，可重新导入所提供归档以核对原件");
  await expect(view).toContainText("字节头识别类型：application/zip（未解析正文）");
  await expect(view).toContainText("消息与文件库的类型声明不一致");
  await expect(view).toContainText(`原件 SHA-256：${"a".repeat(64)}`);
  await expect(view).toContainText("以下附件关联此条原始消息");
  await expect(view).toContainText("未提取、索引或发送附件正文");
  await expect(view.locator("a,button,img,script,iframe,object,embed")).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).syntheticExecuted)).toBeUndefined();
  await page.screenshot({ path: info.outputPath("attachment-status-wide.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.screenshot({ path: info.outputPath("attachment-status-narrow.png"), fullPage: true });

  for (let i = 0; i < 2; i++) {
    await page.evaluate(value => (window as any).renderAttachments(value), assets([files[1]]));
    await expect(view.locator(".asset-row")).toHaveCount(1);
    await expect(view).not.toContainText("原件已保留");
    await expect(view).not.toContainText("原件 SHA-256");
    await page.evaluate(value => (window as any).renderAttachments(value), assets([]));
    await expect(view.locator(".asset-row")).toHaveCount(0);
    await expect(view).not.toContainText("以下附件关联此条原始消息");
    await page.evaluate(value => (window as any).renderAttachments(value), assets());
    await expect(view.locator(".asset-row")).toHaveCount(4);
  }
});
