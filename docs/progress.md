# 实施进度

依据 `RecallCard-DESIGN-v0.2.md`，保留 v0.1 作为历史，不引入第三种知识对象。

## 已实现：A 的首个离线纵切

- Rust 文件 Vault、JSONL 不可变事件段、Markdown Memory、Git 初始化
- 原始事件去重/修订、来源校验、未知时间、版本保护、确定性视图
- 中英文 BM25、按范围/会话/时间过滤、四个只读 MCP 工具
- 手动 Web JSONL、ChatGPT 官方导出当前分支、Claude Code 日志导入
- 秘密模式脱敏、注入证据拒绝、抑制/恢复
- 合成 fixtures 的 CLI/MCP 跨来源查询，不依赖 Dream 或云 API

## 后续分段

1. B：有界手动 Dream，来源哈希、人工预览、保护记录与事务收据
2. C：Native Messaging、浏览器可见草稿、手动发送与重复请求保护
3. D：可选云 Embedding/Python worker、向量空间签名、增量缓存与降级
4. E：安全 Git 同步、冲突恢复、跨平台 CI/安装测试

## 验证边界

- 2026-10-06 第一段：Linux 53 项测试通过（38 项 Vault、12 项 Context/MCP、3 项导入），格式检查与 clippy 严格告警检查通过
- 已加入 Linux/Windows/macOS CI；远端运行结果需另行确认
- 测试编号、结果和检查命令随每段提交更新；不能将 fixture 成功等同于真实厂商网页或模型记忆准确率
- 真实已登录 ChatGPT 页面、真实 Claude Code 宿主启动/压缩后钩子尚未验收
- Windows/macOS 原生运行和安装尚未验收
- 没有真实云 API 费用、embedding 效果或前缀缓存命中数据
- 当前检索每次从正本构建内存文本索引，适用于小规模验证；尚未宣称大型库增量索引性能
