# 实施进度

依据 `RecallCard-DESIGN-v0.2.md`，保留 v0.1 作为历史，不引入第三种知识对象。

## 已实现：A 的首个离线纵切

- Rust 文件 Vault、JSONL 不可变事件段、Markdown Memory、Git 初始化
- 原始事件去重/修订、来源校验、未知时间、版本保护、确定性视图
- 中英文 BM25、按范围/会话/时间过滤、四个只读 MCP 工具
- 手动 Web JSONL、ChatGPT 官方导出当前分支、Claude Code 日志导入
- 秘密模式脱敏、注入证据拒绝、抑制/恢复
- 合成 fixtures 的 CLI/MCP 跨来源查询，不依赖 Dream 或云 API

## 已实现：B 与 C 的可测试本地闭环

- B：有界手动 Dream、来源/旧记忆哈希、完整差异预览、摘要绑定批准、protected 保护、事务日志、持久收据、幂等重放与恢复
- C：四只读 Native Messaging、精确扩展来源名单、长度边界；Chrome MV3 预览/草稿追加/安全移除/人工发送
- 扩展依据平台条款采用用户主动粘贴与官方导出，不自动抓取回复
- 默认召回拒绝合成 Context/Dream 回声；输出整体实行服务器字节预算
- 浏览器重用启动快照/插入草稿前重新核验，资料变化或撤权会废弃旧预览
- 只生成安装包装器与 manifest，不自动注册或修改用户电脑

## 已实现：D/E 的可运行子集（2026-10-07 恢复并重新验证）

- Python 向量 worker：精确授权端点/范围/数据，默认离线，空间签名、输入哈希增量复用、批次/重试预算、原子检查点、离线 query_vector 与排序融合
- API Dream：稳定中文前缀，单次有界请求，usage/cache 字段仅据服务返回记录，输出只作提议，经 Rust review 后仍需人工批准
- Git 同步：先提交本地，再 fetch/整合/push；严格正本白名单、不运行 hooks/filter、不强推；文本/语义冲突保留待人工处理
- Native 共享 executable launcher：固定相邻配置、复制二进制、Unix 包装器与 Windows manifest；不自动注册系统配置
- 浏览器精确增加 ChatGPT guest 的 `textarea#mobile-composer-prompt`，继续要求唯一可见可写编辑器，不自动发送

## 已实现：召回语义与持续遗忘修复

- 标签与实体进入文本排序，具名 `view:<label>` 从授权正本动态产生，拒绝路径与旧版本引用
- 默认检索、Bootstrap、View 与 Embedding 导出只用当前有效 Memory，历史查询保留显式 `as_of`
- CLI 接受工具引用，开放会话/时间/详情/游标查询参数；导入支持有界 stdin
- 遗忘按 scope + 来源身份摘要继承到现有/未来修订；保留旧规则兼容和显式恢复，不混同独立消息
- [T01–T12 验收差距 v0.3](acceptance-v0.3.md) 逐条区分实现、自动回归和未实机验证

## 已实现：更多手动网页与 Agent 生命周期

- Qwen/Z.ai 实验性 composer 适配：固定主机/路由、显式重置绑定、输入框变化失效、保留草稿、人工最终发送；DeepSeek 因会话身份不确定仍停用
- Claude Code `SessionStart` 的 startup/resume/compact/clear 只读 Hook：有界输入、固定 scope、稳定 Bootstrap、官方输出结构及合成配置示例
- Hook 回归既调用库，也直接启动 RecallCard CLI；不运行外部 Agent，不自动安装配置
- 对应 [浏览器适配](browser-adapters-v0.3.md) 与 [生命周期 Hook](agent-hooks-v0.3.md) 文档说明实机验证边界

## 已实现并通过三平台 CI：本机 IPC 与语义检索

- Unix socket / Windows named pipe 只读 daemon，Vault/scope 固定绑定、有界帧、超时、私有端点与单实例保护
- MCP、Native host 与生成的固定 Native 配置可选择 IPC；服务不可用时返回错误，不暗自换回直接读盘
- Rust 监督 Python worker，兼容空间/当前 generation 校验、融合排序、等待后再检查权限/有效期；错误或超时退回文本检索
- `search`、本地 `mcp`、`daemon` 的语义配置由可信启动参数指定；默认离线，云 query 需单独明确配置授权
- 本机云终端底层拒绝 Unix socket bind（EPERM），包括标准库最小 bind；授权后的相同测试仍受限，真实 IPC 测试保留在 CI 中，不跳过也不记为通过
- 该段验证分支和 main 的 Linux/macOS/Windows、Python、Node CI 全部成功，main=9061a9d；Linux完整208项Rust实跑通过，平台专属数量以各runner日志为准。详见 [IPC](ipc-v0.3.md) 和 [语义检索](semantic-search-v0.3.md)

## 已交付：中文 Tauri 桌面版与免配置首条记录

- 桌面窗口复用 Rust 核心，支持创建/选择资料库、粘贴文字、文件导入预览与确认、关键词查找、原文与出处阅读、整理结果审阅和保存
- 默认分类可直接开始，不要求先配置 API、向量、Git 同步或浏览器扩展；README 与三步指南已缩短，复杂功能在需要时再配置
- `9d76621` 正式程序在 Ubuntu 22.04 标准 WebKitGTK 环境通过 19 项界面/状态用例和 9 步真实原生操作，AppImage/deb 构建成功：[验收与安装包](https://github.com/Micraow/RecallCard/actions/runs/37579114793)
- `6581e52` 只读取已成功验收的成品，核对原 SHA-256 后按完整 deb、运行 ZIP、校验清单分发；没有重新编译：[分发记录](https://github.com/Micraow/RecallCard/actions/runs/37581139328)
- 安装包、截图和版本化三步指南已经交付；详情见 [桌面说明](desktop-v0.1.md) 和 [三步开始](desktop-quickstart-v0.1.md)
- 用户自己的 Arch/KDE Wayland 电脑未实测；云端受管理 Chromium 明确阻止加载扩展，因此真实已安装扩展链路仍未验收，不把桌面或合成浏览器通过算作扩展安装成功

## 下一步与未完成范围

1. 本机 IPC 与语义后端的真实安装验证
2. 实际 Agent 首次/恢复/compact 后加载，以及扩展安装后的真实网页端到端验证
3. 大型资料库持久增量索引与性能验证；当前每次读取正本构建文本快照

## 验证边界

- 2026-10-07 本段 Linux 工作区：190 项不需要 socket 的 Rust 测试、105 项 Python（包括 Rust→fake API→Rust review 合同）、74 项 Node 全部通过，格式与严格 Clippy 通过；另有真实 IPC 端点/桥接测试被本机 EPERM 阻挡，未计入通过数，须由本段 CI 实跑
- Rust→Python embedding 导出/哈希/增量缓存/撤权过滤合同独立通过
- 已修复工作流 runner 上下文、Windows 严格告警、macOS 父目录别名；Windows 原子替换短暂占用修复及恢复阶段 d3105ef 已在三平台 Actions 全部通过；本段新改动另跑独立 CI，详见 [CI 修复记录](ci.md)
- Linux/macOS/Windows CI 的每次结果绑定具体提交；本机通过不等于远端通过，也不等于浏览器安装验收
- Native 安装链路已做合成进程测试，Windows/macOS 真实浏览器注册与运行尚未验收
- 没有真实云 API 费用、embedding 效果、记忆准确率或前缀缓存命中数据
- Hook 适配代码及真实 RecallCard 子进程合同已验证；尚未运行实际 Claude Code 宿主启动/压缩，不能把 fixture 成功写成真实宿主验收

## 富文本撤销后续修复

已复现并修复编辑器重建注入段后遗漏用户新增格式的撤销判断。Node从74增至76项全过，新增8项真实Chromium合成页面回归，先走独立验证分支；见 [原生 DOM 回归](browser-engine-v0.3.md)。合成浏览器测试不替代登录态网页验收。
