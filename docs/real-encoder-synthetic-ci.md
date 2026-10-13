# 独立真实小编码器合成验收

此目标新增工程验收与质量观测，不修改核心、扩展、提供器默认设置或发布流程。实现代码不代表已下载、已运行或已达到语义质量门槛。必须查看指定源码 SHA 的 CI 和 `summary.json`。

## 触发与边界

- workflow：`.github/workflows/real-encoder-synthetic.yml`，仅 `workflow_dispatch` 或 `.github/real-encoder-ci-request.json` 这个显式 marker 变化触发；普通 push 不下载模型。
- marker 可写一个新的合成请求编号；此变更必须确实是一次明确请求，不要把 marker 纳入自动版本更新。脚本拒绝非 GitHub-hosted CI；不在用户机器或本地助手环境下载模型、启动回环服务。
- 不上传聊天、真实事件、Vault、索引、运行配置、凭据、日志或机器路径。产物白名单仅两个 JSON：合成指标 summary 与包名/版本/包摘要 dependency lock。
- 只需 `contents: read`。不改变任何仓库/账户/安全配置，没有在线推理和自动云端回退。GitHub Actions 的实际计费未读取，不能声称运行免费。

## 固定模型和可复现来源

- FastEmbed 0.9.0，BAAI/bge-small-zh-v1.5，512 维，CLS pooling，L2 normalization。
- 官方 ONNX 仓库 Qdrant/bge-small-zh-v1.5 revision `46fbe35fd4374a00fee7de77dfddaeb6dd6a2c59`；ONNX 文件 94,781,076 bytes，SHA256 `1294ea4b6331115a353d81f96b85e8c8d7fdcc284453d5b2fab5b016230aad38`。
- `model-lock.json` 的 6 个小文件使用该固定 revision 官方 tree API 的原始长度和 Git blob SHA-1，算法精确为 SHA1(`blob ` + ASCII 十进制原始字节长度 + NUL + 原始文件字节)。它们不是 SHA256；CI 验证 blob identity 后另外计算全部文件 SHA256 写入 summary。不要把 Git blob OID 错写成文件 SHA256。
- 固定关键编码运行时版本在 `requirements.txt`。其他依赖由官方 PyPI 解析，本次实际所有安装包版本和分发文件 hash 写入 `dependency-lock.json`；再次复现应在相同 Linux x86_64/Python 版本下按该产物锁定完整依赖，不把未固定的传递依赖声称为跨时间 bit-for-bit 固定。
- workflow 用根 Cargo.lock 编译已有库。验收驱动从 `scripts/real_encoder/harness.rs` 临时复制为 Cargo example；这是显式 CI-only overlay，不提交核心代码。summary 记录仓库实际 SHA、模型锁、驱动、二进制和输入 fixture SHA256；已跟踪文件有变更就失败。

官方依据：[固定模型文件目录](https://huggingface.co/Qdrant/bge-small-zh-v1.5/tree/46fbe35fd4374a00fee7de77dfddaeb6dd6a2c59)、[FastEmbed 0.9.0 ONNX 编码实现](https://github.com/qdrant/fastembed/blob/v0.9.0/fastembed/text/onnx_embedding.py)、[tokenizer 配置实现](https://github.com/qdrant/fastembed/blob/v0.9.0/fastembed/common/preprocessor_utils.py)。pooling 依据实现，不能把通用 docstring 的 mean pooling 当成本模型实际行为。

## 真实链路和防静默截断

独立 Rust 驱动导入原始 `short-question/corpus.json` 的 12 个来源，读取原 `queries.json` 的 52 问。不另编 gold 冒充原集。先通过现有 search 记录 exact 和 question_intent 词法基线，再配置现有 LocalEvents，经现有 Python worker 和 SQLite 建索引，调用实际 `transport::invoke(..., "recall_search", ...)` 与 `recall_fetch`。每个返回来源均核验原文和快照；再次查询核验缓存复用。没有写 Memory。

服务只接受固定合成文档和固定查询 allowlist。生产 `EmbeddingSpace.prepare` 加查询指令“为这个句子生成表示以用于检索相关文章：”、换行、原查询；文档不加指令。适配器只调用 `model.embed`，不调用 query_embed，不二次加 prefix。

从同一固定文件独立加载 tokenizer，注册相同特殊符号，调用 `no_truncation()`、`no_padding()`，对全部实际输入含指令及特殊符号计数；超过 512 就在模型执行前拒绝。模型 tokenizer 同样关闭截断。CI 还以额外纯合成超长输入验证拒绝，不将超长输入交给模型。返回维度、数目、有限值、L2 范数和有序 index 都有校验。

下载在独立阶段结束；编码阶段使用 `specific_model_path`、`local_files_only=True`、离线环境变量，并禁止编码 Python 进程的外部 socket/DNS。此 audit 防线不是整个 runner 的网络沙箱；Rust/现有 worker 使用既有严格数字回环端点。没有 firewall、权限或网络设置变更。

## 指标含义

- 原集总数与 9 个 synonym_question 单独报告 source-hit@1/@3/@5，并与两个词法基线同集比较。
- 同时列出非预期来源候选数、无关/负例问题的非空候选数和全部固定查询的返回来源顺序。原生 facade 自己的固定结果/字节预算可能截断候选，原样记录，不偷偷扩大预算。
- 小集新增 `edge-cases.json` 是单独的 4 源/3 问否定与更正诊断；显式报告过时/冲突来源出现在前 k 的情况。它不能证明模型理解了否定、能自动挑出当前事实或答案正确。
- 此轮不事后选择一个好看的阈值；第一次 baseline 不设生产召回率验收门槛。工作流绿色表示固定模型和工程链通过，质量数据须另行解释。相似度候选检索没有拒答阈值，负例返回候选可能是预期局限。

## 预算和实测

单 job 最多 30 分钟；编译 15 分钟、依赖 5 分钟、下载 4 分钟、运行 5 分钟。模型下载总量不超过 100 MiB；下载脚本内部 180 秒，单请求 30 秒，下载前要求 2 GiB 余量。CPU 2 线程、batch ≤8；运行总时限 240 秒，外层 timeout 260 秒；最多 128 次编码请求/256 个输入；每个派生索引上限 16 MiB。

监控每 50ms 采样进程树 RSS，总和超过 1280 MiB 就停止验收；该值存在共享页重复，可能遗漏短峰。另报告编码进程实际 ru_maxrss；不是硬 cgroup 隔离。报告模型加载、首次推理、累计推理、索引构建、查询首个/中位/最大、运行总耗时、模型字节、环境安装大小和实际 SQLite 文件大小。冷启动专指本进程首次加载固定本地文件，不包含从网络下载或宿主页缓存保证。

## 不用模型的测试

`python3 -m unittest discover -s scripts/real_encoder -p 'test_*.py' -v`

这些单测不加载 FastEmbed、不下载模型、不监听端口。它们验证 hash、门禁、prefix、防截断计数、维度/范数、合成白名单、指标口径等纯代码合同；不能替代真实 CI 模型质量结果。
