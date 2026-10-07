# RecallCard 桌面版 0.1：从窗口开始

桌面版使用 Tauri 2，直接复用 RecallCard 的本地 Rust 核心。它和 CLI 共用同一个 Vault，无须迁移已有资料。首版优先 Linux x86_64，界面全中文。

## 运行 Linux 成品包

解压后双击 `recallcard-desktop`，或在解压目录运行 `./运行桌面版.sh`。包内带现有 CLI 与扩展目录；不需要 Rust、Node 或 Python。需要 Git、GLIBC 2.39+、GTK 3 和系统正常安装的 WebKitGTK 4.1 运行库。在 Arch Linux 上请使用系统包管理器安装 `webkit2gtk-4.1` 和 `git`，由它解析 GTK 等依赖。

这是桌面预览构建（dev profile，已去除调试信息、未做发布优化）。可运行包不捆绑系统动态库，不安装后台服务。

## 第一条使用路径

1. 启动 `recallcard-desktop`，点「创建新资料库」并在原生窗口选择一个空文件夹；已有 Vault 点「打开已有资料库」。再次确认后，应用初始化所选目录
2. 点「导入资料」。选择文件格式和范围，点「选择文件并预览」。ChatGPT 官方导出 ZIP 需要先解压，再选择 `conversations.json`；也支持 Claude Code JSONL 和 RecallCard 标准 JSONL
3. 检查文件名、范围、记录数和正文样本，再点「确认导入」。取消预览不会导入；重复导入同一条消息会去重
4. 打开「查找与阅读」，输入中文或英文关键词。左侧结果可以点击，右侧显示正文、状态、时间和来源。`Ctrl+K` 可前往检索
5. 需要长期整理时，在阅读区选择来源，再去「整理记忆」。导出本次来源包，将自己工作流产生的 Dream JSON 结果导入审阅。核对前后差异、必要时单独批准受保护记忆，然后确认发布

每个操作都针对窗口当前选中的 Vault 和范围。切换资料库会清除旧结果、来源选择和待确认的预览；切换范围、取消预览也会废弃服务端待确认编号。文件、来源或旧 Memory 在预览后改变时，需要重新审查。

## 可以直接完成的事

- 新建、打开和切换本地资料库，检查原始记录和长期记忆数量
- 三种现有文件格式的只读预览、按范围导入与去重
- 在当前范围浏览最多 30 条记录、中英文关键词检索、读取有界正文与证据
- 选择原始来源和旧 Memory，导出本地 Dream job，审阅结果、确认发布与受保护记忆额外批准
- 查看资料库健康状态、了解 CLI 与浏览器连接步骤

界面保留 loading、空列表、错误、取消、二次确认和过期操作提示。导入按既有追加语义写入，进程中断后可重复导入去重；它不会回滚删除已成功写入的记录。

## 当前边界

- 不自动扫描电脑、浏览器或聊天窗口，不联网调用模型，不保存 API key，不自动发送网页输入框
- 桌面 UI 暂不管理 Git 远端同步、语义 worker 配置、云端配额、后台服务或开机启动；这些已有高级能力仍使用 CLI
- 浏览器扩展仍按浏览器正常安装和 Native Messaging 注册流程连接。桌面窗口不会改变浏览器策略，也不能代表扩展已经连接成功
- 文件预览最多 16 MiB / 5000 条事件；Dream 结果最多 1 MiB。正文/检索单次输出最多 32 KiB；超出会提示片段或未完全显示，不暗示完整展示了长对话
- 首版每次启动通过原生选择器打开资料库；不在本地浏览器存储持久化私人正文

## 从源码构建

桌面应用是独立 Cargo 工程，现有 CLI 的默认 workspace 不需要 GTK/WebKit 依赖。

```sh
cargo test --locked -p recallcard --test desktop --test import
node --test desktop/tests/*.test.js
cargo build --locked --manifest-path desktop/src-tauri/Cargo.toml
./desktop/src-tauri/target/debug/recallcard-desktop
```

需要 Rust 1.90+。Linux 需要 Tauri 官方 GTK3、WebKitGTK 4.1 等开发依赖；用户运行成品包只需要相应运行库。Arch Linux 的包名为 `webkit2gtk-4.1`、`gtk3` 等；以系统包管理器实际依赖解析为准。构建前提见 [Tauri 官方说明](https://v2.tauri.app/start/prerequisites/)。没有 Node 运行依赖，前端是随二进制嵌入的本地 HTML/CSS/JavaScript。

桌面配置不启用文件系统、shell 或 HTTP 插件的前端权限；受限 Rust 命令使用原生文件选择器取得路径。参考资料用纯文本渲染，CSP 禁止外部脚本和框架页面；文件中的 HTML 或指令不会被执行。

## 验证记录

服务层测试覆盖取消、改范围、换资料库、文件替换、过期 Dream read-set、受保护批准和正常导入/读取/发布。macOS 系统目录别名与 Windows 原生文件身份检查已有专门回归。Linux Tauri 2.12.1 程序在本机构建成功，桌面 Clippy 通过。

提交 `88461b8` 的全部现有 CI 已通过：三平台 Rust、Python、Node、Linux Tauri 构建，以及 18 项真实 Chromium 的界面/状态回归。Chromium 用例使用合成原生响应，不等于 Tauri/Rust 完整链路。

dot 云端桌面已通过正常启动器实际尝试运行，系统缺少标准路径的 `WebKitNetworkProcess`，因此没有完成原生窗口验收。仅解压官方系统库足以构建，但不足以运行该发行版写死辅助程序路径的 WebKit；没有修改系统策略或动态库。已增加 GitHub Linux runner 上使用正常系统依赖、官方 WebDriver 和 Xvfb 的真实 Tauri 验收，结果以对应 CI 为准。
