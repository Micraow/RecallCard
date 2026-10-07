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

提交前先在本机完成 Rust 格式、严格 Clippy、相关 Rust 回归，以及 Node 单元/状态测试。桌面界面另有无需浏览器进程的检查：

```sh
npm ci --prefix desktop/tests/local-dom --ignore-scripts --no-audit --no-fund
npm --prefix desktop/tests/local-dom test
python3 -m unittest discover -s desktop/tests -p 'test_native_*.py' -v
```

本地 DOM 检查直接执行实际应用模块和事件处理器，验证来源/时间与正文节点、ZIP 选择、受保护确认和旧批准失效；不是重新写一套界面实现。它通过网络失败护栏禁止测试期连接或监听，独立开发依赖不进入浏览器扩展或桌面运行包。Node 版本要求与模拟边界见 [本地 DOM 说明](../desktop/tests/local-dom/README.md)。不能由这些测试推断真实布局、WebKit 或剪贴板已通过。

原生驱动检查在本地用合成响应验证：先定位/滚动再点击、多行内容真实粘贴后逐字读回、取消和保护确认、逐条打开搜索结果核对原文及完整引用。断连后不重复点击、输入或写入。WebDriver 的整块可见文本可能含布局空白，因此正文比较使用具体文本节点，同时检查卡片可见并实际打开记录。

标准环境发现仅验收脚本问题时，`recheck-native.yml` 可复用已经通过构建的原始成品；先核对来源运行、程序源码无变化、包内提交和二进制摘要，再执行真实操作。程序源码有变化则必须重新构建。每次通过结论保留原构建提交与验收提交，不删除历史失败记录。

日常开发先在 dot 云电脑运行编译、Clippy、相关 Rust/Python/Node 测试。普通 main 提交和文档修改不自动启动全平台矩阵。核心、独立浏览器和独立桌面工作流可以手动启动；只有相应 `validate/core-*`、`validate/browser-*`、`validate/gui-*` 验收分支自动触发，PR 仍保留相关检查。

Linux 安装包工作流只在手动启动或 `validate/desktop-*` 分支上执行，一次完成正式构建、界面回归、真实 Tauri/原生文件选择器操作和 AppImage/deb 打包。它采用 Ubuntu 22.04 基线，收集非空文件、版本/提交号和校验清单，保存 Actions artifacts，不创建 Release、不推送别的仓库。Cargo 缓存复用依赖；同工作流同分支的新运行取消旧运行。

云电脑已经能编译 Tauri，但其系统未安装标准路径 WebKit 辅助进程且没有 sudo；因此这一次标准 Linux 运行环境与安装包验收保留在 Actions。不能把本地未执行的原生检查标成通过，也不会为每次细小文案改动重新跑全矩阵。

打包安排参考用户提供的 [Nexus package.yml](https://github.com/Micraow/nexus/blob/main/.github/workflows/package.yml) 和 [MoonBridge-GUI release.yml](https://github.com/Micraow/MoonBridge-GUI/blob/main/.github/workflows/release.yml) 的系统基线与产物组织，没有复制发布、跨仓库同步、更新服务或 AUR 步骤。

## 0.4 首轮测试环境修正

验证提交 `1baa4dc` 的运行 [37613355691](https://github.com/Micraow/RecallCard/actions/runs/37613355691) 在进入应用编译前失败：Node 22 报 `bad option: --test-isolation=none`。本地 Node 24.19.0 的55项DOM已通过，但新增工作流步骤没有同步运行版本。这是测试环境声明遗漏。安装包工作流现固定24.19.0，DOM包根级engines与说明一起收紧；依赖本身的引擎声明保持原样。

参数依据：[Node 24.19.0 官方命令行说明](https://nodejs.org/download/release/v24.19.0/docs/api/cli.html#--test-isolationmode)。本次不重新编译已经通过的程序，因为该轮尚未构建应用；后续仍需完整执行产品验证，不能把版本修正本身算成验收通过。

### 0.4 原生标识核对与成品复用

`76ed836` 的 [37613963636](https://github.com/Micraow/RecallCard/actions/runs/37613963636) 已通过DOM、真实Chromium、完整Rust、正式GUI/CLI和安装包构建。原生第2步在取消文件选择后的资料库标识严格文本断言停止；失败截图与保存的HTML均显示“尚未打开资料库”，原脚本未记录WebDriver当时返回值。

后续核对增加有界等待、实际DOM文字、屏幕内可见矩形及WebDriver原返回值的并列诊断。错误文字或不可见标识仍失败；不重发文件选择和写入操作。已补21项本地驱动合同。使用复用工作流核对程序源码、构建步骤、包内提交与二进制摘要后，重验同一0.4程序，不重新编译。该条记录不能提前表示原生闭环已经通过。

### 真实提示遮挡与固定提交前门禁

复验 [37615796639](https://github.com/Micraow/RecallCard/actions/runs/37615796639) 记录了标识DOM可见且正确、WebDriver rendered text为空的差异。其后原有前11步和53会话列表检查通过，复制交接按钮被“导入完成”提示遮挡，截图与 `element click intercepted` 一致。这是实际界面缺陷，提示现改成占据自身布局空间的状态条，并增加宽/窄窗口的矩形不相交及真实点击回归。

同次发现密度流程被测试脚本提前调用两次，已收为AST唯一末尾调用门禁。集中审阅补齐不同记录的阅读位置、本批导入分页回读、背景正文展开；详细本地门禁见 [固定preflight](preflight.md)。它会校验Node与CI声明、锁文件、官方actionlint、内联脚本语法、资源白名单、原生调用顺序及现有合同；未执行的Chromium几何、原生窗口和编译阶段在报告中明确列出。

不会为了通过而删除失败历史、弱化可见性或绕过点击命中。应用源码改变后，必须用新程序做正式构建与原生验收，不能继续声称旧成品已包含界面修正。

### 自动选择后的窄窗接续

[37618878056](https://github.com/Micraow/RecallCard/actions/runs/37618878056) 的统一preflight通过，真实浏览器45/46项通过；新增宽/窄提示几何用例在切到860像素后找不到复制按钮。导入自动选中会话后，打开接续未进入详情状态，窄窗CSS因此隐藏面板。已先在本地真实模块复现状态断言失败，再修复打开接续时的详情状态，59项DOM通过。浏览器用例保留，另外保存逐宽度截图、DOM与关键控件矩形，失败时自动保留证据。应用编译尚未开始，不能把这次运行说成新包构建成功。

预检也拒绝“进程退出0但执行0项测试/没有执行摘要”的空通过，作为固定负向合同。

### 0.4 第16步文案漂移与精确来源定位

[37620418524](https://github.com/Micraow/RecallCard/actions/runs/37620418524) 的统一预检、46项真实Chromium界面用例、16项扩展页面用例、324项Rust测试及GUI/CLI/AppImage/deb构建已通过。原生前15步完成，第16步已核对遗忘后结果卡片数为零，但脚本仍期待旧空状态“暂时没有找到匹配资料”；实际截图、HTML均显示“没有找到匹配资料”。这次失败属于验收文案漂移，尚未执行的后续步骤不记为通过。

空结果文案和定位器移至原生/实际模块DOM共同使用的验收合同，DOM用例同时检查旧结果与旧正文清空。整批检查其余原生按钮、提示、折叠区和后续流程；密度检索还用真实CLI合成53会话验证BM25返回相关记录，改按确切Event引用与完整正文核对目标，而非错误要求总共只返回一条。下一次仅复用41e70dd成品，校验程序源码、构建来源与二进制摘要；不会为验收脚本再次编译。

[成品复验37623130672](https://github.com/Micraow/RecallCard/actions/runs/37623130672) 前22步通过，包括背景保存、真实剪贴板、MCP和接续内容一致。第23步脚本仍把切范围后的页面当成背景页，但实屏和既有DOM合同确认产品会清空旧选择并回到“全部记忆”。原生脚本现明确检查该重置，再点击新范围的“选择背景”；往返范围同一路径加入本地实际模块合同。没有修改产品数据行为，也未把后续未跑步骤记为通过。

### 0.4最终实际验收

[37623800674](https://github.com/Micraow/RecallCard/actions/runs/37623800674)在6dd1ba8完成全部28个原生检查点，包括53会话/106消息、默认首屏8行、真实复制、背景/MCP/接续一致及860px操作。使用41e70dd已构建程序，源码相同和二进制摘要门禁通过。导出每步截图、canonical结果和工作区几何；已查看实际密集列表、背景、交接与窄窗截图。0.3保留，历史失败不记为通过。

交付核验中发现deb与便携GUI摘要不同：逐字节比较仅有官方 `__TAURI_BUNDLE_TYPE_VAR_UNK` 到 `DEB` 三字节标记变更，其余完全相同；该行为与tauri-utils 2.10.1源码一致。deb安装事务、AppImage启动和用户Arch/KDE Wayland尚未单独执行，不能由标准窗口验收推定通过。
