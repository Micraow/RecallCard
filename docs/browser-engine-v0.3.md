# Chromium 合成页面回归 v0.3

原生 DOM 回归加载本仓库的输入框适配器和完全自建的最小 HTML，检查真实 textarea、InputEvent、contenteditable、TreeWalker 与 Range。测试页面没有厂商内容、账号或个人对话；所有网络请求被测试框架拒绝。测试中显式传入的平台 URL 仅用于选取适配规则，不表示访问过或验收过对应真实网站。

## 本次修复

编辑器重新建立注入段落后，用户可能在其中新增粗体或链接等格式。如果 Range 的起止点都在同一格式元素内部，`cloneContents()` 可能只返回文字；仅检查它是否包含元素不能证明可以安全删除。

新增回归先复现了原逻辑未拒绝该情况。修复同时检查起止文字节点到编辑器之间的祖先：只允许无属性的普通段落/容器，或仅保留原始 `white-space: pre-wrap`。遇到新格式、链接、属性或不确定结构就保留草稿并要求用户手动处理。原本完整的自有段落仍按所有权撤销；纯文本段落归一化继续支持，不重建整个编辑器。

## 验证方式

Node 协议与合成对象回归：

```sh
node --test extension/tests/*.test.js
```

真实 Chromium 引擎测试（需要 Node 22+）：

```sh
cd extension
npm ci --ignore-scripts --no-audit --no-fund
npx playwright install --with-deps chromium
npm run test:browser
```

如机器已安装兼容 Chromium，可通过 `RECALLCARD_CHROMIUM_PATH` 指定其可执行文件，避免再次下载。Playwright 是测试期开发依赖，扩展运行时没有该依赖。框架按 [Playwright 官方安装说明](https://playwright.dev/docs/intro) 配置，版本锁定在 package-lock 中。

独立 Actions 工作流按仓库 CI 文档的显式验收条件运行，没有部署、登录或向模型发送内容。覆盖四种 textarea 结构、重复追加、零发送事件、富文本节点身份、用户新增格式保护、纯文本归一化，以及真实 CSS/只读拒绝。

本机 Node 76 项回归通过。dot 云端终端的 socket 策略阻止了独立 Chromium 启动，云浏览器也不允许 data 协议页面；没有尝试绕过这些限制。Chromium 的八项测试保持启用，结果须以该提交的实际 Actions 为准，不能用 Node mock 通过替代浏览器引擎证据。引擎回归也不等于登录态厂商网页或已安装扩展的端到端验收。


## 会话捕获与新界面补充

扩展 0.3.0 增加 DeepSeek textarea 与独立 conversation.test.mjs：当前可见普通文字、隐藏/推理排除、未知角色人工确认、流式与空数据拒绝、同 URL 身份变化、全快照复核、下载取消和保存失败。测试 HTML 与文字完全合成；测试脚本以拦截方式提供本地资源，不访问真实网站。Node 回归现为 90/90；新增 Chromium 用例待实际引擎执行，不以准备脚本冒充通过。命令已限制 test-concurrency=1。
