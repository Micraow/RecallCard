# 真实桌面产品闭环验收

`native_smoke.py` 使用编译后的 Tauri 程序、真实 WebKitGTK、系统文件选择器和桌面剪贴板。所有资料都在临时目录中合成；不读取真实聊天，不调用模型，也不替换前端 `invoke`。

## 运行

Linux 需要标准系统包提供的 WebKitWebDriver、Xvfb、AT-SPI、xdotool、xclip、scrot、openbox，无需额外的驱动代理。Python 使用安装了 `python3-pyatspi` 的 `/usr/bin/python3`。

```sh
dbus-run-session -- xvfb-run -a -s "-screen 0 1440x1100x24" \
  /usr/bin/python3 desktop/tests/native_smoke.py \
  --application desktop/src-tauri/target/release/recallcard-desktop \
  --cli desktop/src-tauri/target/release/recallcard \
  --artifacts /tmp/recallcard-native-acceptance
```

桌面按钮、输入框、选择框和确认窗口由 WebDriver 操作；文件路径通过原生选择器输入。DOM 脚本仅观察状态、读取可见控件值或滚动，不执行产品命令。CLI 用于读取 canonical 结果和原有 Native Messaging 安装测试；新增的导入、整理、记忆编辑及范围切换全部经过窗口。

Linux 直接连接系统官方 WebKitWebDriver。能力字段 `webkitgtk:browserOptions` 与 `TAURI_AUTOMATION` / `TAURI_WEBVIEW_AUTOMATION` 环境来自 [Tauri 官方 Linux 映射](https://github.com/tauri-apps/tauri/blob/tauri-driver-v2.1.0/crates/tauri-driver/src/server.rs) 和 [原生驱动启动方式](https://github.com/tauri-apps/tauri/blob/tauri-driver-v2.1.0/crates/tauri-driver/src/webdriver.rs)。没有更换 WebKit、修改应用二进制或降低浏览器安全设置；移除的是曾多次在点击回执处断连的中间 HTTP 代理。

启动时必须先取得原生 `/status` 的 ready 响应，再创建唯一会话。标准输出和错误输出保存在 `webkit-webdriver.log`；结束时关闭会话并回收驱动进程。创建会话、点击、键入和确认均不因断连自动重发。

## 覆盖的 20 个检查点

1. 真实桌面首页与 Tauri 环境
2. 原生资料库创建、打开与取消
3. 保存首条个人补充并立即查找
4. 对话预览及取消均不写入 Event
5. JSON 导入及重复去重
6. 中文搜索与完整原文
7. ZIP 包含两个单会话 JSON，仅勾选其中一个；核对角色、原始时间、隐藏推理与跳过文件统计
8. ZIP 重复导入不改变 canonical Event；从已保存会话查看两条原始消息
9. 从会话选择来源，生成完整整理任务，并核对真实剪贴板包含规则、输出格式和完整 DreamJob
10. 根据复制出的真实 job 编号、摘要和来源构造合成 DreamResult，经普通输入框粘贴、逐条审阅及取消
11. 明确确认后实际保存 Memory 与 Dream 凭证
12. 追溯原始出处，重复带回相同结果不能再次发布
13. 记忆管理读取完整正文、证据性质和原始来源
14. 修改正文、标签与保护设置；取消不落盘，确认产生新版本
15. 解保护要求额外勾选，未勾选不能打开最终确认窗口
16. 遗忘规则实际隐藏记忆及其原始来源，搜索不再返回；资料文件保持不变
17. 主动查看隐藏内容、撤销规则；真实搜索重新返回记忆和来源
18. 通过 GUI 创建工作范围资料，切换个人/工作范围，核对记忆列表、搜索和整理来源隔离
19. 原有 Native Messaging 交换格式保存会话、重复去重与跨 AI 接续；复制内容核对真实剪贴板
20. GUI 生成的持久 MCP 配置可检索刚保存的会话

生成的 `summary.json` 只有在全部检查点成功后才写入 `success: true`。每个检查点保留桌面及 WebKit 截图，原生文件窗口另留截图。失败保存错误栈及页面源码；成功另存 canonical Event、编辑前后 Memory、遗忘/恢复规则、Dream 凭证与复制出的完整任务。

## 结果边界

编译、Python 语法检查、传输层单元测试、服务层测试和使用模拟响应的浏览器测试都不能替代本脚本的真实运行。只有对应提交的标准 Linux CI 运行成功，才可声明这些原生产品步骤通过。其他桌面平台及用户自己的机器需要分别验证。

读取状态与定位元素允许有界传输恢复；点击、输入、创建会话或写入操作不会自动重发。系统缺依赖、控件不可用或数据断言失败都会使验收失败，不静默跳过。
