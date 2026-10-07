# 跨平台验证与已定位问题

CI 对 Linux、Windows、macOS 分别执行 Rust 格式、严格 Clippy 与完整测试，另执行浏览器协议测试。成功只代表这些自动测试通过，不代表浏览器安装、真实模型网页或云端模型效果已验收。

## 2026-10-07 修复记录

1. 工作流原来在 job 级环境变量引用 `runner.temp`，导致任务未能开始。已将变量移到测试步骤。
2. Windows 条件编译不执行目录同步时留下未使用参数；已按平台消除严格告警。
3. macOS `/var` 是系统路径别名。Native 安装器先拒绝用户选择的根目录链接，再规范化父路径；目标根目录及内部目标仍拒绝链接。
4. Windows 的原子替换在并发读句柄或短暂占用期间可能返回访问/共享拒绝。对系统错误 5、32、33 增加约一秒上限的退避重试；保留同一份已同步临时文件，始终执行替换，不先删除旧文件、不改 ACL。持续占用仍返回错误并保留旧记录。

第 4 项新增 Windows 专用回归：读句柄释放后成功、持续占用后旧记录完整。原有并发读写测试继续启用，没有降低线程数或移除覆盖。Linux 本机无法运行 Windows 专用测试，须以该提交的 Windows Actions 结果为准。

参考：[Rust 文件共享默认行为](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html#tymethod.share_mode)、[Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)。

## Windows worker fixture 的启动竞争

文档提交的复跑曾在 Windows 触发多个 Python 冷启动同时争抢资源，使语义用例的 3 秒成功窗口误触发既有的正常降级。测试现串行安排独立进程 fixture，并给成功/握手场景 10 秒准备窗口；同一个用例里的并发查询、撤权/修订、100 毫秒超时、阻塞 stdin 与快速忙碌降级检查仍保持。没有修改生产默认时限，没有跳过用例，也没有把运行失败当作通过。


## 桌面验收与 Actions 用量

日常开发先在 dot 云电脑运行编译、Clippy、相关 Rust/Python/Node 测试。普通 main 提交和文档修改不自动启动全平台矩阵。核心、独立浏览器和独立桌面工作流可以手动启动；只有相应 `validate/core-*`、`validate/browser-*`、`validate/gui-*` 验收分支自动触发，PR 仍保留相关检查。

Linux 安装包工作流只在手动启动或 `validate/desktop-*` 分支上执行，一次完成正式构建、界面回归、真实 Tauri/原生文件选择器操作和 AppImage/deb 打包。它采用 Ubuntu 22.04 基线，收集非空文件、版本/提交号和校验清单，保存 Actions artifacts，不创建 Release、不推送别的仓库。Cargo 缓存复用依赖；同工作流同分支的新运行取消旧运行。

云电脑已经能编译 Tauri，但其系统未安装标准路径 WebKit 辅助进程且没有 sudo；因此这一次标准 Linux 运行环境与安装包验收保留在 Actions。不能把本地未执行的原生检查标成通过，也不会为每次细小文案改动重新跑全矩阵。

打包安排参考用户提供的 [Nexus package.yml](https://github.com/Micraow/nexus/blob/main/.github/workflows/package.yml) 和 [MoonBridge-GUI release.yml](https://github.com/Micraow/MoonBridge-GUI/blob/main/.github/workflows/release.yml) 的系统基线与产物组织，没有复制发布、跨仓库同步、更新服务或 AUR 步骤。
