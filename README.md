# RecallCard

把对话、想法和重要决定收在一起，随时查找原文和出处。

## 三步开始

1. **打开桌面版，创建资料库**：选择一个空文件夹
2. **添加第一条资料**：粘贴一段文字，预览后确认保存；已有对话文件可展开「导入对话文件」
3. **开始查找**：输入关键词，打开结果查看原文和出处

默认即可使用本地搜索。整理记忆、浏览器扩展和同步等功能，可以用到时再设置。

## 下载与打开

- Linux 安装包：[GitHub Actions 构建入口](https://github.com/Micraow/RecallCard/actions/workflows/package-desktop.yml)。选择成功构建的 Linux 产物；Arch 优先使用 AppImage，Ubuntu 可使用 deb
- 已拿到运行 ZIP：解压后运行 `recallcard-desktop` 或 `运行桌面版.sh`
- [桌面版简明说明](docs/desktop-v0.1.md) · [遇到问题](docs/desktop-v0.1.md#当前边界)

安装包工作流会保留带版本和提交号的文件及校验清单。首版优先 Linux；其他平台的核心测试不代表桌面安装包已经实机验收。

## 下一步

- [导入与资料格式](docs/usage.md)
- [整理长期记忆](docs/dream.md)
- [连接浏览器扩展](docs/browser.md)
- [命令行与高级功能](docs/project-overview-v0.3.md)
- [整体设计](docs/RecallCard-DESIGN-v0.2.md) · [实施与验收状态](docs/progress.md)

开发者从 [开发与完整功能说明](docs/project-overview-v0.3.md) 开始；普通使用不需要先读设计文档或修改配置文件。
