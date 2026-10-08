# 跨 AI 上下文与个人记忆产品研究

研究日期：2026-10-08。研究目的：为 RecallCard 的本地 CLI、daemon、GUI、浏览器和 Agent 连续工作流选择参考对象，判断哪些部分值得自建、对接或避免重复建设。

## 主要结论

1. **这一方向已有直接产品，不能只用“跨 AI 记忆”定位。** ArcRift 已公开提供浏览器扩展、桌面托盘、MCP 和共享本地数据库；Personal AI Memory 已提供网页被动捕获和输入框召回。Supermemory、Mem0 也已从开发者 API 扩展到面向终端用户的插件。
2. **本地也不是单独的区分点。** Supermemory 当前有本地引擎；Signet、Context Harness、多个同名 OpenContext 项目都有本地方案。更值得证明的是：原文可核对、当前态可纠正、失败不静默、跨网页与 Agent 真正续接，以及无需维护多个事实源。
3. **团队上下文已经形成独立产品分支。** Memsprout、ContextVault 关注空间、成员、权限和归属；Rulesync 关注规则、skills 与配置的分发。共享个人记忆、发布团队知识、同步工具规则，需要不同权限和生命周期。
4. **“本地”必须拆成数据流检查。** Mem0 扩展宣传文与官方仓库存在明确口径差异；Pieces 的当前文档区分本地捕获和云端推理；Supermemory 的离线文本路线仍有 URL 抓取、模型下载和遥测条件。不能从本地 UI 或本地数据库推出内容从不出机。
5. **现阶段优先参考 ArcRift、Personal AI Memory、Supermemory、Signet、Rulesync。** 前两者用于网页体验对标，中间两者用于后台整理和可观察性对标，Rulesync 用于规则适配。没有发现可直接替换 RecallCard 全部约束且已经过本研究实测的成品。

## 方法与证据边界

- 先检索 Hacker News、Reddit 的发布帖和实践讨论，再检查项目官网、官方仓库、文档、发布记录及问题报告。技术能力只据一手资料判断；论坛用于发现项目和理解取舍。
- 重点列出 11 个产品或项目，均有 HN 或 Reddit 讨论入口。另列同名项目及暂不宜采用的线索，避免名称混淆。
- “已提供”指公开文档、源码或安装发行物可见，**不表示安装验证通过**。本次未注册、登录、安装、付费、发送信息或运行第三方项目；未独立复现性能、安全或质量声明。
- 作者发布帖属于供给证据，不能当独立需求或采用证据。票数、star、提交次数不用于推导用户量、成熟度、市场规模或付费意愿。
- 活动日期仅记录本次可见的一手更新时间，不把搜索引擎抓取时间当产品更新。没有取得绝对日期时不由“几个月前”倒算。
- 价格是研究日公开页面显示的美元标价，用于理解商业模式，不是报价或购买建议；配额、税费、模型费用及结账条件可能另计。
- “未核实”表示本次材料不足，不等于产品一定没有。尤其不能把某个 MCP 客户端可连接，等同于每轮自动捕获、自动调用、完整历史覆盖或多人权限正确。

## 产品地图

| 项目 | 类型与交付状态 | 捕获与使用入口 | 存储及控制 | 团队方向 | 对 RecallCard 的关系 |
|---|---|---|---|---|---|
| ArcRift | 个人/社区 OSS；有桌面发行物 | 网页扩展 + IDE MCP；自动注入 | 本地 SQLite、Ollama；JSON 导入导出 | 项目隔离、文件分享；组织治理未核实 | 最直接的全链路体验对手 |
| Personal AI Memory | 作者称 side project；OSS beta 与商店入口 | 五类 AI 网页被动捕获；点击 Recall | IndexedDB、浏览器内嵌入；JSON 备份 | 多人治理未核实 | 最贴近“不再造聊天客户端”的网页参考 |
| Supermemory | 商业公司；托管产品、插件、MIT 本地引擎 | 多 Agent 插件、MCP、网页导入、连接器 | 托管或本地引擎；开放 API | Spaces、OAuth 范围、成员权限 | 自动维护与可观察性基线；可选引擎候选 |
| Mem0 / OpenMemory | 商业公司；OSS SDK、自托管服务、托管产品及扩展 | SDK、MCP、IDE 插件、网页扩展 | 需分别判断各发行形态；扩展会调用 Mem0 API | 用户/项目范围与企业功能 | 可选引擎候选；不能整体标为离线 |
| Pieces | 商业桌面产品 | 被动桌面工作流捕获、CLI、MCP | 本地捕获和存储；AI 请求可发云端 | 有组织付费方案；具体共享记忆边界未核实 | 低负担时间线与来源解释参考 |
| Letta | 商业公司与 OSS；完整有状态 Agent 平台 | 自有 harness、CLI/App/API、记忆文件 | MemFS、记忆块、历史、Git 共享仓库 | 共享 Agent、组织权限 | 生命周期和并发参考；替换成本较高 |
| Signet | OSS 项目；有稳定/预发布渠道 | 多 harness、transcript、文件导入、daemon | SQLite 权威状态 + 原始 transcript/导入文件 | 自托管 token、role、scope | 后台维护与诊断强参考；不是文件正本同构 |
| Context Harness | Parallax Labs OSS；有二进制发行物 | 文件/Git/S3/脚本同步，CLI/MCP | Rust + SQLite；本地/外部嵌入 | Git-backed 扩展和共享知识源 | 摄取、检索、适配器参考 |
| Memsprout | 独立商业 SaaS；有文档、价格与更新记录 | 人工/Agent 写入，MCP 读取 | 托管 Spaces；来源、版本、导出 | owner/editor/viewer；面向非工程成员 | 共享权限和新手引导参考 |
| ContextVault | 独立商业 SaaS；已宣布发布，首页仍有 private beta 文案 | MCP 读写，组织知识复用 | 托管数据库；用户/组织/组范围 | 组可见性、角色、审计 | 团队产品对标；开放导出门槛需核实 |
| Rulesync | 社区 OSS 工具；持续发行 | 规则、skills、MCP 等配置生成/导入 | 本地文件；可直接转换或用统一源 | Git 中分发一致配置 | 优先对接候选，不是对话记忆引擎 |

表中能力和限制的依据见下列逐项说明。商业公司、独立商业产品与 OSS 项目仅描述本次可核实的公开形态，不推断融资、收入或维护者职业。

## 1 ArcRift

**社区与活动。** 作者在 2026-05-31 的 [Reddit 发布帖](https://www.reddit.com/r/SelfHostedAI/comments/1tt0j4f/i_built_an_opensource_desktop_app_that_gives_your/)说明，用户反馈促成去 Docker 和桌面安装体验。[发行页](https://github.com/Eshaan-Nair/ArcRift/releases)当前标记 v1.6.3 为最新，显示 6 月 3 日；有 Windows、macOS、Linux 安装包及浏览器扩展说明。安装仍需 Node.js 和 Ollama，不能把“单桌面应用”理解为无依赖。

**能力与限制。** [官方仓库](https://github.com/Eshaan-Nair/ArcRift)描述七类网页 AI 与多个 MCP 客户端共享 SQLite、知识图谱和混合检索，并有 JSON 会话导入导出。文档区分项目内 recall 与跨项目 search；也暴露 store、prune 等模型写工具。项目隔离宣传不代表所有工具都仅能访问当前项目；自动提示增强是否严格保留人工最终发送，未进行行为验证。

**值得借鉴。** 浏览器和 Agent 共用一套服务、托盘常驻、失败队列及崩溃恢复，比增加另一个聊天入口更接近目标。其数据库为核心，不能不经转换就替代 Event/Memory 文件正本。优先做合成任务对照评测，不据 README 的效果数字作选型。

## 2 Personal AI Memory

**社区与身份。** [Reddit beta 帖](https://www.reddit.com/r/alphaandbetausers/comments/1rk90vj/beta_personal_ai_memory_a_localfirst_chrome/)和 [扩展社区发布](https://www.reddit.com/r/chrome_extensions/comments/1rk9m7y/losing_context_between_chatgpt_claude_gemini_or/)来自作者自述。原项目是 [marswangyang/personal-ai-memory](https://github.com/marswangyang/personal-ai-memory)；此前资料里的 pairojvrh/ai-memory 是 fork，不应混为独立产品。原仓库采用 Apache-2.0。

**交付与范围。** [发行页](https://github.com/marswangyang/personal-ai-memory/releases)可见 v0.0.5，显示 3 月 14 日；有 [Chrome 商店页](https://chromewebstore.google.com/detail/personal-ai-memory/cjkjgbddkaoogdbfffiooeppnmbplpnh)。README 提供 ChatGPT、Claude、Gemini、Perplexity、Grok 捕获，浏览器内混合检索、点击 Recall、原平台导出导入及 JSON 备份。未核实 CLI/daemon/MCP 或团队权限。

**值得借鉴。** 用户继续在原站点聊天，扩展负责捕获和召回，入门负担小。其主要持久化是浏览器 IndexedDB；嵌入、检索和备份并不自动解决记忆矛盾、当前态整理和跨 Agent 生命周期。文档称可选匿名分析可关闭；“扩展存储隔离”也不代表插入网站输入框后的内容仍不可见。

## 3 Supermemory

**社区与当前形态。** [2024-07-22 HN](https://news.ycombinator.com/item?id=41030219)讨论的是较早的保存内容/第二大脑产品，不能代表今天的能力。[当前官方仓库](https://github.com/supermemoryai/supermemory)已覆盖记忆引擎、profile、混合检索、插件与连接器，并提供 MIT 本地版本。不要沿用“只有云端 SaaS”的旧分类。

**本地和托管需要分开。** 本地引擎支持本地嵌入与 Ollama 文本流程，数据放在引擎目录；URL 摄取使用 hosted reader，关闭遥测需配置。它不是以可编辑 Markdown/Git 文件作为全部正本。[本地更新记录](https://supermemory.ai/changelog/local/)有 2026-06-10 二进制发布与 7 月改进。[MCP 文档](https://supermemory.ai/docs/supermemory-mcp/mcp)已经提供 source document、memory version、profile、Spaces、OAuth 范围和待用户提交的 guided-save。

**真正值得学习的变化。** [2026-08-21 Claude 插件更新](https://supermemory.ai/changelog/plugins/claude/)包括自动召回、后台捕获、状态栏收据、token 可见、限时 fail-open，以及失败保存重试和 API key 实测。它还披露过 8 月的插件自动批准命令问题及更新提醒。应将可观察性与权限隔离作为验证基线，不能把“用了 MCP”当安全证明。

**成本与采用。** [价格页](https://supermemory.ai/pricing/)显示 Free、Pro $19/月、Max $100/月、Scale $399/月及用量计费；企业自托管和免费本地引擎不是同一交付承诺。可评估作为派生整理/检索适配器，先验证离线、原文出处、撤销及重建，再决定是否引入。

## 4 Mem0 和 OpenMemory

**社区与产品分层。** [2024-09-04 Show HN](https://news.ycombinator.com/item?id=41447317)由创始人发布。必须分别看 Mem0 OSS SDK、自托管服务、托管 Platform、OpenMemory MCP 和浏览器扩展；另外 CaviraOSS/OpenMemory 是同名的独立项目。

**关键隐私差异。** [扩展发布文](https://mem0.ai/blog/introducing-the-openmemory-chrome-extension)使用 local、browser-only 等描述，页面标记更新于 2026-08-25；但 [官方扩展仓库](https://github.com/mem0ai/mem0-chrome-extension)明确要求登录，并说明消息会发送给 Mem0 API 以提取和检索记忆。当前实现与宣传应进一步核对，不能向用户保证离线。扩展免费说明也不能套用到 Platform。

**实现与成本。** [当前核心 README](https://github.com/mem0ai/mem0)区分 library、自托管、cloud，默认自托管认证开启；2026 年算法说明强调追加提取、时间检索，并明确托管 benchmark 不等于 OSS 结果。[公开价格](https://mem0.ai/pricing)为免费 Hobby、Starter $19/月、Pro $249/月，Dream consolidation 属较高套餐。旧 [OpenMemory MCP 文章](https://mem0.ai/blog/introducing-openmemory-mcp)还给出 OpenAI key 配置，因此“本地存储”也不等于本地推理。

**对 RecallCard。** 可比较其提取/检索接口和项目范围，不能直接把 SDK 的 memory 表升级为正本。建议把“已确认操作”与“Agent 自称完成”单独分级，避免默认给两者相同事实权重。旧文档中的 openmemory 目录本次直接打开返回 404，部署前应以当前仓库路线重新核验。

## 5 Pieces

**社区入口与活动。** [HN 实践评论](https://news.ycombinator.com/item?id=47798721)有使用者推荐；不是独立可靠性评测。[官方发布记录](https://github.com/pieces-app/pro_tips/blob/main/releases/Whats%20New%20in%20Pieces%206.0.0.md)列出 2026 年 5 月 6.0.0。当前文档还要求旧于 PiecesOS 12.4.0 的版本升级才能使用部分云服务，说明版本生命周期会影响可用性。

**实际形态。** [PiecesOS 文档](https://docs.pieces.app/products/core-dependencies/pieces-os)明确捕获、索引、存储在设备上，需要 LLM 的 AI 功能把该次范围内的上下文发往云端。[MCP 页](https://pieces.app/mcp)提供一键连接多种现有工具，列为 Pro 功能；不能把历史上“免费、完全离线”评论当作当前完整承诺。[付费文档](https://docs.pieces.app/products/paid-plans)要求以账号结账页为价格准，本次不报未经核验的固定月费。

**值得借鉴。** [桌面文档](https://docs.pieces.app/products/desktop)以时间线展示捕获活动和状态；[回答操作文档](https://docs.pieces.app/products/desktop/conversational-search/using-conversational-search)支持回到来源事件、按应用和时间筛选及导出回答。回答导出不是全部原始记忆的开放格式备份，本次未核实完整 round-trip。全桌面捕获增加权限、敏感信息过滤和设备资源成本，不宜直接成为 RecallCard 默认范围。

## 6 Letta

**社区与当前活动。** [HN 的 Letta Code 讨论](https://news.ycombinator.com/item?id=46294274)说明其差异在于 harness 内的有状态记忆。[官方仓库组织页](https://github.com/letta-ai)可见 2026 年 10 月更新；这证明持续开发，不证明所有新功能已稳定发布。

**能力边界。** [官方共享记忆说明](https://github.com/letta-ai/letta-code/blob/main/src/skills/builtin/managing-shared-memory/SKILL.md)定义组织拥有、托管于 Letta Cloud 的 Git 仓库，可挂给多个 Agent，并说明冲突时保留脏状态、提示后续处理。[MemFS 源文档](https://github.com/letta-ai/letta-code/blob/main/src/agent/prompts/letta_root_memfs.md)区分不可修改的经验历史与可维护的记忆文件。文件出现于本地 checkout，不代表服务整体无云依赖；旧 HN 的自托管说法需另按当前部署路线验证。

**成本与取舍。** [当前价格](https://docs.letta.com/pricing)列 Free、Pro $20/月、Teams Pro $20/席/月及另外的开发者用量计费。它是完整 Agent 平台，采用它通常会改变运行时和工作入口。可学习共享文件的并发冲突、历史/当前态分离及记忆预算，不能因为同样使用 Git 就认定满足 RecallCard 的模型只读边界。

## 7 Signet

**社区与活动。** [HN 讨论中的使用者线索](https://news.ycombinator.com/item?id=47887560)将其作为同类项目提出，另有 [GitHub Discussions](https://github.com/Signet-AI/signetai/discussions)。[发行页](https://github.com/Signet-AI/signetai/releases)本次可见 2026-10-04 的 v0.230.9，明确标为 pre-release；[README](https://github.com/Signet-AI/signetai)也警告 nightly 不适合生产。

**实际结构。** 多 harness transcript、导入文件、后台提取/维护、dashboard 和 daemon 已有公开文档。[Workspace v2](https://docs.signetai.sh/workspace-v2/)把 SQLite 定义为权威状态，保留原始导入文件和 JSONL transcript，并明确缓存、运行时文件与派生视图。[快速开始](https://github.com/Signet-AI/signetai/blob/main/docs/QUICKSTART.md)提供 identity-off 模式及本地 loopback/远程鉴权区别。它不是单纯 Markdown 真源工具，也没有在本次证明完整网页聊天被动捕获。

**值得借鉴。** 来源保存、回填一致性、迁移拒绝条件和诊断接口很具体。[issue #1061](https://github.com/Signet-AI/signetai/issues/1061)记录了 2026-08-03 的 hook 孤儿进程回归，读取时已关闭并关联修复；不能称当前版本仍存在。它提示 RecallCard 应测试超时后子进程退出和真实资源健康，而不只检查 daemon 存活。代码可参考，预发布速度不能替代稳定性证据。

## 8 Context Harness

**社区与交付。** [Show HN](https://news.ycombinator.com/item?id=47162581)由作者介绍 Rust 单二进制、本地 SQLite/FTS 和可选嵌入。[发行页](https://github.com/parallax-labs/context-harness/releases)本次最新标记为 v0.8.0，发行物时间为 2026-06-08。

**范围。** [当前仓库](https://github.com/parallax-labs/context-harness)重点是文件、Git、S3 和 Lua 连接器，经规范化与分块后由 CLI/MCP 提供检索；还包括增量 checkpoint、得分解释、profile 和扩展注册表。可无模型做关键词检索，嵌入可本地或外部。它更接近知识摄取/检索基础设施，未证实有完整网页聊天捕获与持续事实整理。

**值得借鉴。** source 状态、可解释检索、连接器独立测试及本地无模型降级与 RecallCard 很匹配。Show HN 明确当时没有内置认证层，因此多人或远程部署必须重新核验当前版本，不能把绑定一个 HTTP 地址当团队产品完成。其 MIT 组件可作为接口设计参考，未建议在研究阶段引入依赖。

## 9 Memsprout

**社区与状态。** [Show HN](https://news.ycombinator.com/item?id=49109106)作者说，仓库知识和内部 MCP 对非技术成员维护负担较大，因此做共享空间。[官网](https://memsprout.com/)已有 Space → Topic → Memory、来源、版本回滚和角色说明；[更新记录](https://memsprout.com/changelog)本次可见到 2026-07-30。它是独立商业服务，不应称“开源团队记忆”。

**最重要的 UX。** 7 月更新明确：只有用户点名共享空间，Agent 才写入该空间；否则默认个人空间，写入收据提示空间和可见人数。连接后的 onboarding 还实际检查 Agent 是否用过，而非仅显示已配置。这里的自动捕获依赖 Agent/MCP 工作流，不能推成网页扩展级全量被动捕获。

**数据与成本。** [隐私说明](https://memsprout.com/privacy)披露 Supabase 托管、记忆文本经 OpenRouter/OpenAI 做嵌入及元数据处理，允许导出；界面删除后内容目前仍在后端保留。[价格](https://memsprout.com/pricing)为个人 $10/月、团队 $15/人/月及 14 天试用。对 RecallCard 最有价值的是共享边界和可见性收据，不是把私人正本搬入 SaaS。

## 10 ContextVault

**社区与活动。** [Show HN](https://news.ycombinator.com/item?id=48900288)作者描述从自己的 MCP 记忆库发展到团队产品。[官方新闻](https://www.contextvault.dev/news)记录 2026-06-01 发布、8 月 27 日新增 Cursor/Windsurf；首页仍有 private beta 文案，因此阶段表述不完全一致，不能据营销页认定广泛正式生产使用。

**能力。** [连接文档](https://www.contextvault.dev/docs)有托管 HTTP MCP、OAuth、当前 workspace 及组作用域；每条记忆归一组，组成员才可检索。归档组与删除记忆分开。HN 介绍包含手动要求 Agent 保存经验的流程，本次未证实跨网页自动全量采集或自动维护矛盾。

**成本与限制。** [官网价格](https://www.contextvault.dev/)列 7 天/50 条试用，Solo $9.99/月，Team $49.99/月含 10 席、2500 条记忆和每月 15000 查询；Data export 只明确列在 Enterprise。不能据“无 AI 客户端锁定”推出普通套餐可完整导出。团队治理是有价值的对标，是否可自托管及完整导出格式仍待核实。

## 11 Rulesync

**社区与活动。** [HN 讨论](https://news.ycombinator.com/item?id=48051242)发帖者明确称非关联者，主要价值是减少工具锁定。[发行页](https://github.com/dyoshikawa/rulesync/releases)本次最新为 2026-10-05 的 v27.0.0，近期修复涉及符号链接、输出目录、配置差异和孤立文件清理。

**真实用途。** [官方仓库](https://github.com/dyoshikawa/rulesync)为 MIT CLI，支持规则、命令、MCP、subagent、skills 等配置的导入与生成；可用统一文件源，也可一次性跨工具转换。不同工具、global/project/simulated 模式的支持不相同，不能把表里一个勾解释成完全语义等价。

**采用建议。** 适合成为 RecallCard 规则/skills 导出的可选适配器或测试参考，不宜再从零维护几十套规则格式。它不负责对话原文、动态记忆、事实替代、团队成员 ACL，也不能保证模型真的遵守规则。对接应保留 diff/预览、版本固定、文件归属、冲突保护，避免将自动提取的个人记忆写进执行规则。

## 同名与暂不推荐直接采用的线索

### OpenContext 不是一个项目

- [0xranx/OpenContext](https://github.com/0xranx/OpenContext)：MIT、本地 contexts 文件库、CLI、MCP、skills、桌面 GUI，重用 Codex/Claude/OpenCode CLI；[发行页](https://github.com/0xranx/OpenContext/releases)最新可见 desktop-v0.2.7，显示 1 月 30 日。其 GUI 还包裹 Agent 会话，与 RecallCard 不新增聊天客户端的方向有差异。本次未取得可靠匹配的 HN/Reddit 原帖，未计入重点 11 项。
- [aviskaar/open-context](https://github.com/aviskaar/open-context)及 [官网迁移说明](https://open-context.dev/blog/migrate-chatgpt-to-claude/)：从 ChatGPT 导出转换、preferences/memory 文档和本地 JSON/MCP 起步，支持多种数据库。迁移入口以导出、分析、粘贴为主，不能等同持续被动同步。二手文章给的 HN 44992081 本次无法打开且未检索到匹配原帖，不作为已验证社区证据。
- [ohmyctx/opencontext](https://github.com/ohmyctx/opencontext)：Go daemon，摄取 shell、Git、Agent 等工作信号，输出 memory.md，按敏感级别和 subscription 控制。最值得学习的是收集范围显式化；不能与上述两个项目合并计算能力。
- [melandlabs/opencontext](https://github.com/melandlabs/opencontext)：可嵌入的 Agent context runtime，包含时序图和记忆 API，是另一套项目。其 README 自评 benchmark 不作为跨产品优劣依据。

### Bindly 的现状改变了采用结论

[Show HN](https://news.ycombinator.com/item?id=46528730)曾展示 Claude Code 保存变化、ChatGPT 继续思考的工作流；但 [当前官网](https://bind.ly/)明确写正在重建、尚待发布。版本化 Binding、Sets 和 token 预算值得观察，不应按当前可直接部署产品推荐。

### 别混淆同名 OpenMemory

[CaviraOSS/OpenMemory 的 HN](https://news.ycombinator.com/item?id=46262294)讨论的是另一个 SQLite 项目，不是 Mem0 OpenMemory。论坛对代码质量的评价不是本研究的审计结论，也不能转嫁给另一项目。

## 对 RecallCard 的具体决策

### 继续自建的核心

以下是基于本次材料的产品判断，不是“其他产品一定做不到”的断言。

1. **Event 与 Memory 文件为唯一正本。** 检索引擎、图、摘要、GUI 和适配器均可重建；不可因为某引擎方便就再引入一份不可恢复的事实源。
2. **完整收据链。** 区分已连接、已捕获、原文已落盘、已整理、可检索、宿主实际读取、已交付。业内已经有不少状态栏，应进一步证明它们反映真实完成而非排队或进程存活。
3. **用户可检查的当前态。** 计划、建议、尝试、完成、撤销和替代不能仅靠语义相似度处理。来源角色、时间和用户纠错必须参与整理；错误记忆撤销后不能被旧任务重新写回。
4. **网页与 Agent 统一但权限不同。** 浏览器只在准确绑定的会话内捕获/预览；模型检索面保持只读，后台整理经受控服务写入。网页最终发送由人点击。不要直接照搬竞品自动注入、工具写入或 prune 权限。
5. **接入可恢复。** 扩展适配器版本化；有导出导入兜底；处理分页、429、UI 改版、流式解析和中断。网页上实际看过哪些历史、未知哪些范围，应能讲清楚。

### 优先对接与不急着自建的部分

| 选择 | 建议 | 接入前必须验证 |
|---|---|---|
| 规则和 skills 格式 | 优先评估 Rulesync，沿用 AGENTS.md 等标准 | 输出 diff、归属/覆盖保护、版本兼容、global/project 范围 |
| 整理与混合检索 | 可评估 Supermemory local、Mem0 或自有实现，保持可替换 | 原文定位、成本、离线数据流、导出、纠错、删除、从正本重建 |
| 网页入口 | 以 ArcRift、Personal AI Memory 为对照样本 | 两端真实续接、完整捕获、重复注入去重、草稿保护、零自动 send |
| 后台服务 | 学习 Signet 的状态分层和恢复方案 | hook 超时退出、队列幂等、崩溃回放、迁移/备份恢复、版本混用 |
| 全桌面感知 | 暂不把 Pieces 式全量捕获设为默认 | 明确需求、应用排除、敏感处理、权限、资源/电量成本 |
| 团队托管 | 个人闭环稳定后再决定是否独立做 | 私人默认、共享确认、受众显示、成员离开撤权、权限过滤后再检索 |
| 新 Agent 运行时 | 不为使用 Letta 的记忆而默认替换所有工作流 | 是否真的接受新客户端/harness、运行成本和权限模型变化 |

### 下一轮可执行对比测试

使用同一组合成数据和任务，不上传真实个人对话，也不根据厂商 benchmark 排名。建议最先对比 ArcRift、Personal AI Memory、Supermemory local、Signet 与 RecallCard；Rulesync 单独测试规则生成。

- 在网页 A 决定方案、拒绝另一方案，然后让 Agent B 找到原因继续工作；再把实际验证结果交回网页 C。
- 先声明“准备实施”，后报告失败，再纠正最终选择，测试当前态和历史来源是否同时正确。
- 中途关机、模型 key 失效、网页解析变化、队列失败，核对最后成功时间、缺口与恢复收据。
- 在两个项目放同名实体和诱导跨域查询，测试没有结果时是否干净返回；不要只测正向命中。
- 从头重建索引、换引擎、导出后导入到空安装，检验原文、版本、删除/撤销状态和引用是否保留。
- 在网页草稿中召回并模拟页面重绘，检测自动路径是否触发 send/submit，是否把注入的旧记忆再次提取为新事实。
- 测试安装和排错的实际用时、每周需要人工维护的次数、设备资源与模型账单；这些比支持平台数量更接近实际价值。

停止条件是能够给出每种方案的真实续接结果、失败边界和总维护成本，而不是完成安装或看到绿色连接图标。本附录只完成公开资料研究，没有宣称上述实测已经通过。
