# RecallCard 桌面版 0.1 · 三步开始

版本：0.1.0，程序提交 `9d76621`，Linux x86_64。

## 下载和打开

[下载已通过验收的 Linux 安装包](https://github.com/Micraow/RecallCard/actions/runs/37579114793/artifacts/11463936600)（GitHub 登录后下载 ZIP，约 93.4 MB）。解压后按系统选择一个文件：

- **Arch Linux / KDE：优先 AppImage**。在文件属性中允许作为程序执行，然后双击。此版本还没有在你的 KDE Wayland 电脑上实测
- **Ubuntu / Debian：使用 deb**。通过系统的软件安装器打开，安装后从应用菜单启动 RecallCard
- **运行 ZIP：给已装好运行库的系统**。解压后双击 `运行桌面版.sh`，或运行 `recallcard-desktop`。需要 Git、GTK 3、WebKitGTK 4.1；不需要 Rust、Node 或 API 密钥

AppImage 和 deb 使用 Ubuntu 22.04 标准环境构建。校验清单随包提供；正式程序的原生验收记录见[本次成功运行](https://github.com/Micraow/RecallCard/actions/runs/37579114793)。运行 ZIP 中的早期构建信息仍保守标注 GLIBC 2.39，实际构建基线以上述运行记录为准。

## 保存并找到第一条资料

1. 打开 RecallCard，点 **创建新资料库**，选择一个空文件夹，并确认。已有资料库则点 **打开已有资料库**
2. 点 **添加资料**，粘贴一段想保留的文字；点 **预览并保存**，检查内容后点 **确认保存记录**
3. 在 **查找与阅读** 输入关键词，点击结果查看原文和出处

到这里就能使用。默认分类可直接保留，向量检索、模型 API、Git 同步和浏览器扩展都可以以后再设置。

## 已有聊天文件

在「添加资料」展开 **导入对话文件**。ChatGPT 官方导出 ZIP 先解压，再选 `conversations.json`；检查预览后确认导入。相同消息再次导入会去重。

## 以后再用

- **整理记忆**：先在阅读区选择来源，导出整理包，再导入你现有工作流生成的整理结果 JSON，逐条审阅并保存
- **浏览器连接**：在「连接与状态」看正常安装步骤；资料加入网页草稿前需要确认，最后发送仍由你点击
- [完整桌面说明](https://github.com/Micraow/RecallCard/blob/main/docs/desktop-v0.1.md)

## 验证范围

此版本通过 19 项界面/状态用例，以及 Ubuntu 22.04 标准 WebKitGTK 下真实 Tauri 程序的 9 步操作：原生文件选择与取消、粘贴后立即查找、文件导入与去重、中文检索、整理结果审阅/保存、来源追溯与防重复保存。测试使用合成资料。

云桌面受管理浏览器不允许加载扩展，所以「真实安装扩展＋Native Messaging」这条链路尚未验收；它不影响桌面里添加和查找资料。macOS、Windows 的桌面安装包本轮未交付。
