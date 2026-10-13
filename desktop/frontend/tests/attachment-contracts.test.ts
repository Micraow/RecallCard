import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { coverageNotes, type ImportCoverage } from "../src/service/contracts.ts";
import { attachmentStatus, type SourceAssets } from "../src/service/types.ts";

const assets = (files: SourceAssets["files"]): SourceAssets => ({
  files, files_total: files.length, citations: [], citations_total: 0,
  tool_trace: [], trace_total: 0, unsupported_total: 0, truncated: false,
});

test("附件状态明确区分本机保留、所给归档缺失、只读解析与旧版记录", () => {
  assert.deepEqual(attachmentStatus("retained"), {
    label: "原件已保留", detail: "已在本机保留原始字节",
  });
  assert.match(attachmentStatus("missing").detail, /所提供归档中未找到原件/);
  assert.match(attachmentStatus("available_not_retained").detail, /本次未保留/);
  assert.match(attachmentStatus("not_in_export").detail, /旧记录仅有元数据/);
  assert.equal(attachmentStatus("future_status" as any).label, "状态未知");
});

test("导入覆盖以引用次数报告保留和缺失，旧服务默认零且不误称唯一文件数", () => {
  const coverage = { file_references: 5, attachment_payloads_retained: 3,
    attachment_payloads_missing: 2 } as ImportCoverage;
  assert.deepEqual(coverageNotes(coverage), [
    "5 条附件引用（按消息计数，非独立原件数）",
    "3 条附件引用已保留原始字节，未提取或索引正文",
    "2 条附件引用在所提供归档中未找到原件",
  ]);
  assert.deepEqual(coverageNotes({ file_references: 1 } as ImportCoverage), [
    "1 条附件引用（按消息计数，非独立原件数）",
  ]);
  assert.deepEqual(coverageNotes({ file_references: 0, attachment_payloads_retained: 0,
    attachment_payloads_missing: 0 } as ImportCoverage), []);
});

test("群聊与 Pages 范围明确未导入，不把归档来源数量当成消息数", () => {
  assert.deepEqual(coverageNotes({ unsupported_archive_sources: ["group_chats", "pages", "pages"] } as ImportCoverage), [
    "群聊记录在当前支持范围外，已跳过", "Pages 在当前支持范围外，已跳过",
  ]);
  assert.deepEqual(coverageNotes({ unsupported_archive_sources: [] } as ImportCoverage), []);
  assert.deepEqual(coverageNotes({ unsupported_archive_sources: ["future_source"] } as ImportCoverage), [
    "另有未支持的归档来源，本轮未导入",
  ]);
});

test("真实附件组件保留消息文件名与独立归档别名，安全显示类型和哈希且没有打开或发送操作", async () => {
  const bundle = await build({
    stdin: {
      contents: `import {createElement} from 'react';
        import {renderToStaticMarkup} from 'react-dom/server';
        import {MessageAssets} from './src/pages/Sources.tsx';
        export const render = assets => renderToStaticMarkup(createElement(MessageAssets, {assets}));`,
      resolveDir: new URL("..", import.meta.url).pathname,
      loader: "tsx",
    },
    bundle: true, platform: "node", format: "esm", write: false,
    loader: { ".css": "empty" },
    define: { "process.env.NODE_ENV": '"production"' },
    banner: { js: `import {createRequire} from 'node:module'; const require = createRequire(${JSON.stringify(import.meta.url)});` },
  });
  const { render } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
  const name = '<img src=x onerror="window.syntheticExecuted=true">.pdf';
  const hash = "a".repeat(64);
  const html = render(assets([{
    source_file_id: "file-synthetic", name, byte_count: 512, payload_status: "retained",
    archive_name_alias: '<script>synthetic()</script>.dat', declared_media_type: "application/pdf",
    library_media_type: "application/octet-stream", detected_media_type: "application/zip",
    declared_media_type_conflict: true, object_sha256: hash,
  }]));
  assert.match(html, /以下附件关联此条原始消息/);
  assert.match(html, /未提取、索引或发送附件正文/);
  assert.match(html, /<strong>&lt;img src=x onerror=&quot;window.syntheticExecuted=true&quot;&gt;\.pdf<\/strong>/);
  assert.match(html, /归档文件名：&lt;script&gt;synthetic\(\)&lt;\/script&gt;\.dat/);
  assert.match(html, /消息附件 ID：file-synthetic/);
  assert.match(html, /字节头识别类型：application\/zip（未解析正文）/);
  assert.match(html, /消息与文件库的类型声明不一致/);
  assert.match(html, new RegExp(`原件 SHA-256：${hash}`));
  assert.match(html, /原件已保留/);
  assert.doesNotMatch(html, /<(?:a|button|img|script|iframe|object|embed)\b|\b(?:href|download)=/);
  for (const status of ["missing", "available_not_retained", "not_in_export"] as const) {
    const text = render(assets([{ source_file_id: null, name: null, byte_count: null, payload_status: status }]));
    assert.ok(text.includes(attachmentStatus(status).detail));
    assert.match(text, /未命名附件/);
    assert.match(text, /大小未提供/);
    assert.doesNotMatch(text, /原件已保留|导出未包含文件/);
  }
});
