# RecallCard

把不同 AI 的对话和重要记忆收进自己的资料库，带着原话和背景继续任务。

## 三步开始

1. **打开桌面版，创建资料库**：选择一个空文件夹
2. **接入一段对话**：直接选择备份 ZIP、勾选会话并预览导入，或用扩展保存 DeepSeek / ChatGPT 当前可见消息
3. **换一个 AI 继续**：选择对话后点击「带到另一个AI」，填写下一步目标，检查并复制交接内容，自己发送

默认即可使用本地搜索。整理记忆、浏览器扩展和同步等功能，可以用到时再设置。

桌面0.4已通过标准Linux的28步真实窗口验收：[操作指南](docs/desktop-quickstart-v0.4.md)。常用界面集中为会话和记忆两个工作区。

## 下载与打开

- Linux 安装包：[下载 0.4.0 安装包](https://github.com/Micraow/RecallCard/actions/runs/37623800674/artifacts/11482734288)（登录 GitHub 后下载 ZIP）；Arch 优先使用 AppImage，Ubuntu 可使用 deb
- 已拿到运行 ZIP：解压后运行 `recallcard-desktop` 或 `运行桌面版.sh`
- [三步指南与验收范围](docs/desktop-quickstart-v0.4.md) · [遇到问题](docs/desktop-v0.3.md#当前验收边界)

运行ZIP内含CLI与扩展0.3.1。程序提交41e70dd；安装包保留版本、提交号和校验清单。用户自己的Arch/KDE Wayland和登录态网站尚未实测；[0.3旧版](https://github.com/Micraow/RecallCard/actions/runs/37603515640/artifacts/11473229342)继续保留。

## 下一步

- [导入与资料格式](docs/usage.md)
- [生成整理任务与审阅结果](docs/manual-web-dream-task.md)
- [浏览器保存与导出](docs/browser-conversations-v0.3.md)
- [命令行与高级功能](docs/project-overview-v0.3.md)
- [整体设计](docs/RecallCard-DESIGN-v0.2.md) · [实施与验收状态](docs/progress.md)

开发者先运行 [固定提交前检查](docs/preflight.md)，完整能力见 [开发说明](docs/project-overview-v0.3.md)；普通使用不需要先读设计文档或修改配置文件。
