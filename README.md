# RecallCard

把不同 AI 的对话和重要记忆收进自己的资料库，带着原话和背景继续任务。

## 三步开始

1. **打开桌面版，创建资料库**：选择一个空文件夹
2. **接入一段对话**：直接选择备份 ZIP、勾选会话并预览导入，或用扩展保存 DeepSeek / ChatGPT 当前可见消息
3. **换一个 AI 继续**：在「会话与接续」选择对话，填写下一步目标，检查并复制交接内容，自己发送

默认即可使用本地搜索。整理记忆、浏览器扩展和同步等功能，可以用到时再设置。

## 下载与打开

- Linux 安装包：[下载已验收的 0.3.0 安装包](https://github.com/Micraow/RecallCard/actions/runs/37603515640/artifacts/11473229342)（登录 GitHub 后下载 ZIP）；Arch 优先使用 AppImage，Ubuntu 可使用 deb
- 已拿到运行 ZIP：解压后运行 `recallcard-desktop` 或 `运行桌面版.sh`
- [三步使用指南](docs/desktop-quickstart-v0.3.md) · [遇到问题](docs/desktop-v0.3.md#当前验收边界)

安装包工作流会保留带版本和提交号的文件及校验清单。首版优先 Linux；其他平台的核心测试不代表桌面安装包已经实机验收。

## 下一步

- [导入与资料格式](docs/usage.md)
- [生成整理任务与审阅结果](docs/manual-web-dream-task.md)
- [浏览器保存与导出](docs/browser-conversations-v0.3.md)
- [命令行与高级功能](docs/project-overview-v0.3.md)
- [整体设计](docs/RecallCard-DESIGN-v0.2.md) · [实施与验收状态](docs/progress.md)

开发者从 [开发与完整功能说明](docs/project-overview-v0.3.md) 开始；普通使用不需要先读设计文档或修改配置文件。
