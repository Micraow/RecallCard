# RecallCard 0.7：让 AI 从你的资料继续工作

此版本为 Linux x86_64 开发候选版。桌面界面和 CLI 使用同一套本地资料、权限与检索服务。应用构建为 `9603053`；项目接入原生验收为 `0af9d0e`。包内另有 `build-info.json` 和 `SHA256SUMS`，可核对来源与实际程序字节。

## 打开与导入

1. 将整个运行 ZIP 解压到长期保留的目录，运行 `./运行桌面版.sh`。不要只拷贝 GUI 二进制，旁边的 CLI、Python 资源和扩展也有用途。
2. 点击「导入聊天记录」，选择 ChatGPT 或 DeepSeek 官方导出的 ZIP / JSON。导入后可以立即在「原始资料」阅读和查找，不必先配置模型。
3. 资料没有经过整理时只显示原话和来源目录，不会把原文自动冒充已确认的长期记忆。

命令行同样可独立使用：`./recallcard --help`、`./recallcard --vault /你的资料库 import --help`。用 `--json` 获取机器可读输出；`search`、`read`、`sources`、`status`、`mcp` 都是正常入口。

## 一次授权，连接一个 Agent 项目

打开「连接」，选择 Codex 或 Claude Code，然后选择实际项目目录。先检查将合并的文件、读取范围和接收方，再勾选授权并安装。只选择目录和浏览预览不会写配置或授予读取权限。

- Codex：合并项目的 `.codex/config.toml` 和 `AGENTS.md` 托管区域。
- Claude Code：合并项目的 `.mcp.json`、`CLAUDE.md` 及会话入口配置。
- 保留用户原有规则和不相关设置。配置在预览后有变化时会停止，不应覆盖你的编辑。
- 这是项目级连接。其他项目不会自动获得这份授权；不修改你的全局 Agent 配置。

完成后「本机试读通过」表示随包 CLI 已按该连接实际读取授权范围。它不表示已启动外部 Agent，也不表示模型已经理解或使用了内容。打开相应项目的 Agent 后，再看真实客户端读取记录；宿主仍可能要求批准项目或 MCP 工具。

自动入口会先调用受管理的 `connection-context`，然后读取该连接的 `generated/bootstrap/<连接ID>/context.json`。这个文件包含经过范围、隐藏与来源筛选的背景和来源目录。按需查询仍使用只读 MCP 的 `search`、`read`、`sources`，长文可续读。入口指令不包含私人正文，也不要求每轮手动选择 context。

你可以暂停、恢复或撤销连接。正本变化使旧文件快照失效，下次读取重新生成；撤权会删除受管理快照并拒绝后续查询。已被宿主读走的内容无法收回。同一操作系统账户直接读取原始资料的能力不由此权限层隔离。

安装的事务备份保存在系统状态目录，不放进项目或 Git。当前要求项目配置与系统状态目录位于同一文件系统，才能保留原文件并原子提交；跨文件系统会明确拒绝，请使用同一磁盘分区中的项目。不要手工删除事务恢复文件来绕过拒绝。

## 网页版连接的真实边界

随包 `extension/` 是需要手动加载的浏览器开发测试版，尚无商店安装链接。按连接页的测试扩展流程安装，正常配对需你确认范围；手工扩展编号属于高级故障恢复。

ChatGPT / DeepSeek 的悬浮接力可准备资料、保留当前轮次、续读长文、暂停和显式复制／粘贴。最后发送始终由你点击。当前没有自动监听系统剪贴板，也没有验证真实登录账号的所有页面变化。Qwen Studio 尚未接通；不要把实验性 Qwen 适配理解成 Studio 已支持。详见随包 [接力说明](https://github.com/Micraow/RecallCard/blob/work/connected-context-v0.7/extension/RELAY.md)。

## 可选后台整理

导入和读取不需要模型密钥。启用后台整理时应在设置中明确选择模型、范围和预算，并按实际返回状态判断凭据是否可用。可以暂停。此候选版没有证明所有真实模型的整理质量或费用准确性；不要把「服务运行」当成已经得到可信的新记忆。旧记忆也不会仅因一条新近况出现就保证自动过时。

## Linux 依赖

需要 Git、GLIBC 2.35 或更新、GTK 3、WebKitGTK 4.1。可选后台模型工作器需要 Python 3.10 或更新。系统运行库通过发行版官方包管理器安装；Ubuntu 22.04 示例：

```sh
sudo apt-get install git libgtk-3-0 libwebkit2gtk-4.1-0
./运行桌面版.sh
```

新版运行包入口会先检查动态库；在 Debian / Ubuntu x86_64 上还会检查 WebKit 的固定辅助程序及其依赖。缺少时会停止并给出具体文件与官方包安装提示；有可用的系统对话框工具时也会弹窗。不会自动安装软件或关闭沙箱。可运行 `./运行桌面版.sh --check-runtime` 只检查环境，不打开窗口；它不是完整功能验收。

仅解压 `.so` 再设置库搜索路径，仍可能缺少 `/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess` 等固定位置的辅助程序。应安装完整的发行版包，已安装但文件缺失时可使用 `sudo apt-get install --reinstall libwebkit2gtk-4.1-0`。官方文件清单：[Debian](https://packages.debian.org/trixie/amd64/libwebkit2gtk-4.1-0/filelist)、[Ubuntu](https://packages.ubuntu.com/jammy-updates/amd64/libwebkit2gtk-4.1-0/filelist)。没有管理员安装能力的环境可先使用 CLI；不要把库检查通过当作真实图形界面已验收。

仅 Linux x86_64 的合成归档、临时项目和真实原生窗口路径已验证。没有将本次验收扩写为 macOS、Windows、Arch/Wayland、用户机器或已登录 AI 网站全部通过。

## 旧资料与回退

升级前关闭旧 GUI、停止旧服务并备份完整 Vault，包括隐藏的 `.git`、`events/`、`memories/`、`control/`、`objects/`。另备份应用设置和系统状态；不能只备份生成视图。

- 默认资料库：`${XDG_DATA_HOME:-$HOME/.local/share}/app.recallcard.desktop/vault`
- 应用设置：同一应用数据目录内的 `recent-workspace.json` 等文件
- 作业、连接及事务状态：`RECALLCARD_STATE_DIR`，未设置时为 `${XDG_STATE_HOME:-$HOME/.local/state}/recallcard`

已有 Event / Memory 文件保留。新版读取旧资料有兼容路径，但没有证明任意新状态都能由旧版安全处理；回退不能只换回旧程序，需连同对应备份一起恢复。系统安全存储中的凭据另按系统提供的方法管理，不应复制进 Vault 或项目。

程序升级不会自动更新已连接客户端保存的旧路径。保留安装目录，必要时重新预览并确认项目配置。不要把旧的全库 `generated/views/memories.md` 当作新连接入口。

## 怎样判断是否真的可用

[原生项目接入验收](https://github.com/Micraow/RecallCard/actions/runs/37779993247)验证了：导入、原话出处、预览不提前授权、真实项目配置、随包 CLI 试读、补充后文件更新、同库 MCP 读取、重启和撤权。它没有执行外部 Codex / Claude 程序，也没有真实 ChatGPT 账号验收。最终 ZIP 的隔离复验以 `build-info.json` 中的打包运行结果为准。
