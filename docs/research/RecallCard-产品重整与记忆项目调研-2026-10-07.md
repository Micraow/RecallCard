# RecallCard 的真正任务与产品重整方案

日期：2026-10-07。本文是产品诊断与下一阶段建议；尚未实现的路径不写成当前功能。当前交付基线为桌面0.4，程序41e70dd，主线499c98c。

## 先说结论

RecallCard要解决的是：换一个AI后，不必重新解释自己和项目；能找回过去的决定、原话及变化。它不应要求用户先经营一座“记忆库”，才能开始使用AI。

**保留原话、出处、版本与可撤销的核心，重做接入和使用路径。** 成熟记忆项目值得借鉴的是它们如何把捕获、整理和召回接到日常对话中。仅替换成某个记忆SDK，不能解决网站历史拿不到、连接复杂、反复确认、模型没有实际读取背景的问题。

原设计已把“刚在Web讨论，换到Agent能继续”放在第一个验收里，并要求未整理的原话即时可查；四个读取能力不是让用户手工完成的四步流程。0.4改善了阅读结构和出处核对，但主要体验仍是“导出文件、导入、人工挑背景、复制交接”，离这个目标有明显距离。[原设计](../RecallCard-DESIGN-v0.2.md)

## 用真实任务判断是否可用

| 用户要做什么 | 0.4真实状态 | 下一步必须改变什么 |
|---|---|---|
| 把过去的对话带进来 | 文件导入可用；扩展只有主动读取当前可见会话 | 一个平台的完整历史导入，逐项记录进度，失败后可继续 |
| 换AI继续昨天的决定 | 已保存原话可查、可生成交接；用户仍需找到会话并操作 | 从目标任务出发准备相关背景，直接在目标端使用 |
| 让AI一直知道稳定偏好 | 可以手工选择记忆，MCP/交接使用同一投影 | 初次接入和恢复会话实际加载；普通用户不用认识内部标签 |
| 整理长期信息 | 手动整理任务与审查完整；API执行器仍是高级CLI | 安静时有界处理新内容；不让整理成为导入和召回前提 |
| 更正错误、不再使用旧资料 | 有出处、修订、保护、遗忘和恢复 | 保留这些能力，但只在纠错时出现；不让每次普通导入像高风险发布 |

下一次验收的主问题是“用另一个AI继续一段刚保存的真实任务，少做了哪些操作”，不是累计了多少测试。技术回归仍然必要，但只能证明它所覆盖的层。

## 默认路径的最少步骤

### 第一次

1. 打开RecallCard，使用默认的本机个人资料位置。已有0.4资料库可直接沿用；换位置放到设置中，不要求先理解Vault或scope。
2. 选择“导入DeepSeek历史”。已登录的网站仍使用原浏览器会话；一次明确开始导入，即批准本次整批保存到所显示的本机位置。安装扩展或注册本机连接仍需要浏览器/系统正常流程，不能伪装成已经连接。
3. 导入过程中即可查找已保存内容。下一步入口是“在另一个AI继续”，而非去配置向量、运行整理任务或逐条勾选全部历史。

这是目标操作路径，不是当前已交付承诺。第2步的全历史适配和初次连接尚未完成。先把一个平台从头走通，不同时堆多个半成品适配器。

### 之后每天

用户在目标对话准备继续一个任务，点击“带上相关背景”；软件准备已确认的稳定背景、与本次任务相关的原话及出处。目标输入框已有内容必须保留，最终发送仍由用户完成。允许展开检查实际文字，但不用把展开检查、复制内部请求代码块、再找工具变成每次必经操作。

本地Agent应在首次、恢复与压缩后实际取得背景，并可继续按需读取。提供MCP工具和配置文件本身不代表宿主已调用，更不代表模型使用正确；首个真实宿主的生命周期验收是交付条件。

### 文件导入保留为便捷备用

ZIP清单提供清楚的“导入全部”主按钮，单会话挑选和原文预览作为可选项。普通导入在唯一一次明确开始后处理整批，不在末尾再弹一次同义确认。批量任务内部可以分段、脱敏和去重，不能要求用户自己拆成多轮5000条。

受保护记忆的覆盖、外发到新的供应商、不可逆删除、团队分享仍是不同性质的决定，不与普通导入合并。撤回未开始的导入无需额外确认；已经保存的部分必须清楚显示，不能把取消说成全量回滚。

## 批量历史导入应借鉴什么

已核对[自己的nexus扩展](https://github.com/Micraow/nexus/tree/fd098bff2288d4d78bccb495cdf77b28d9af5bb8/extension)和[chatgpt-multimodal-exporter](https://github.com/ha0xin/chatgpt-multimodal-exporter/tree/9e41569301f8f4897c1a140a7ab04b4be7997351)，两者都是MIT。借鉴任务结构与数据约束，不直接运行其脚本。

- nexus：普通页面驱动、会话清单、顺序队列、暂停/继续、失败重试、部分成功可导出。它的“滚动三次无增长”等判定不能证明全历史完整；全部同源JSON监听和全库IndexedDB扫描也不适合直接照搬。
- ChatGPT导出器：个人与项目分别分页，再逐会话取得消息树。代码存在请求失败转为空结果后结束扫描的路径，不能把这种结束当完整成功；它会读取并使用登录令牌，不作为本次无凭据处理的实现路径。

RecallCard需有持久导入任务：平台与账号边界、目标位置、目录分页游标、会话版本/已完成项、失败原因、取消状态。账号切换、权限变化和分页异常必须停止并解释。只重试尚未成功的项；已存消息以来源ID和版本去重。

完整性必须分层：发现的会话清单是否穷尽；每段会话当前分支是否完整；哪些消息/附件未包含。没有明确终点、父消息链缺口或失败项时，显示“仍有未完成内容”，绝不以短暂不增长或页面没有更多DOM宣称“全部导入”。首版只存普通可见角色的文本，不采集隐藏推理、凭据或后台无关响应。

## 成熟项目的实际取舍

### Graphiti与Zep

[Graphiti固定源码](https://github.com/getzep/graphiti/tree/aa5bb2706929fce502d99d8c7c4ddbb77bff4995)采用Apache-2.0。其强项是带有效时间的事实边、原始episode出处、矛盾后旧事实失效、语义/关键词/图查询组合。`add_episode`包含抽取、去重、embedding和事实失效处理；默认使用OpenAI客户端及Neo4j，支持其他模型和图数据库。[README](https://github.com/getzep/graphiti/blob/aa5bb2706929fce502d99d8c7c4ddbb77bff4995/README.md) · [实际写入/删除实现](https://github.com/getzep/graphiti/blob/aa5bb2706929fce502d99d8c7c4ddbb77bff4995/graphiti_core/graphiti.py)

借鉴时间与出处传播，不把整套图数据库搬成个人桌面首次使用的前提。`group_id`是分区，不能单独当成已认证的团队授权。删除原始数据时也要检查衍生事实与其他证据的关系；“事实失效”和“物理删除”不是同一语义。

Zep当前开源仓库是Cloud示例/集成，旧Community Edition已停止支持；不能把该仓库的Apache许可理解为当前云端产品全部可本地部署。[官方仓库说明](https://github.com/getzep/zep/blob/b6b129bf70541945ba2c7502b656b3f32755d707/README.md)

Zep的统一context入口和来源元信息向衍生记录传播值得参考；其按主体、动作和数据属性执行的ABAC为Enterprise功能，区别于给记录加一个项目标签。RecallCard暂不引入这项云依赖。[官方访问控制](https://help.getzep.com/policy-based-access-control)

### MemOS

[MemOS固定源码](https://github.com/MemTensor/MemOS/tree/a7367d07e55db61099f7b4e2c1108bc5831a24f3)采用Apache-2.0，必须分开看Python核心Dream和local-plugin。

核心[Dream说明](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/src/memos/dream/README.md)明确是beta、默认关闭。它根据新增记忆信号异步形成动机、召回相关旧内容、调用模型生成洞察并写日记；[信号存储](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/src/memos/dream/signal_store.py)是内存字典，默认触发阈值100，重启不能保留这些信号。候选理由和模型信心不能代替用户事实证据。适合借鉴前后台分离，不宜作为已成熟持久整理任务直接替换。

[本地插件](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/apps/memos-local-plugin/README.md)使用SQLite与具体Agent适配器，捕获与整理在后台；没有模型配置时基本记录仍可工作。它的trace→policy→world model→skill面向Agent执行经验，不是RecallCard必须新增的四层对象。默认本地embedding、宿主模型回退及可选LLM过滤仍有下载、算力和调用成本；“local-first”不等于无模型调用或无配置。[默认配置](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/apps/memos-local-plugin/core/config/defaults.ts) · [实际召回实现](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/apps/memos-local-plugin/core/retrieval/retrieve.ts)

其可选[hub](https://github.com/MemTensor/MemOS/blob/a7367d07e55db61099f7b4e2c1108bc5831a24f3/apps/memos-local-plugin/core/hub/server.ts)默认关闭，代码检查成员状态和令牌、区分管理员操作，发布/撤销绑定认证用户。值得借鉴显式发布、取消分享和本地/共享内容分离；当前查询接口面向已发布的团队公共池，不应宣传成任意项目级细粒度ACL。本文不启用或创建团队共享服务。

### LangMem

[LangMem固定源码](https://github.com/langchain-ai/langmem/tree/48e3c11f5bb527282c7d5339c6a87a0b35abccfc)为MIT。其[函数式记忆候选生成](https://github.com/langchain-ai/langmem/blob/48e3c11f5bb527282c7d5339c6a87a0b35abccfc/src/langmem/knowledge/extraction.py)可以接受消息和既有记忆、返回候选，不要求把正本交给LangGraph Store；带持久状态的集成层则另有存储依赖。[延后处理](https://github.com/langchain-ai/langmem/blob/48e3c11f5bb527282c7d5339c6a87a0b35abccfc/docs/docs/guides/delayed_processing.md)按会话安静期合并任务，减少中途整理和重复模型调用。

这是比整体换核心更小的复用候选：只接到可选Python执行器，输出仍经过RecallCard的来源/版本/保护/事务校验。是否采用依赖，要先用同一组合成纠正与矛盾样本比较当前执行器的漏提、误提、成本；本轮没有安装或调用其模型。

### Mem0与OpenMemory

不能按旧OpenMemory教程选型：[删除前官方README](https://github.com/mem0ai/mem0/blob/540d23d610fcdc9a1f8795875c013af67d35ae93/openmemory/README.md)已标记sunset，2026-07-29的[移除提交](https://github.com/mem0ai/mem0/commit/ea2ee0758635a9230bd60855c3fe339170f6cd18)删除原目录。当前同名入口与托管MCP、编码插件需要分开看；托管MCP将数据保存到Mem0账户，不能等同原来的本地应用。

[Mem0当前插件核心](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/integrations/agent-plugin-core/README.md)更有参考价值：hook先无模型采集到本地SQLite，后台批量flush，记录失败及已注入记忆；第一次检索直接用用户任务，不先额外调模型改写。借鉴持久队列和小型宿主适配器，默认平台发送行为不直接采用。

[OSS原文保留代码](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/mem0/memory/storage.py#L257)仅保留每scope最近十条messages；history记录提炼记忆的修改，不能代替完整原话档案。[当前核心](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/mem0/memory/main.py)默认模型提取与embedding，“单次LLM”并不等于单次网络调用；infer=false也不等于无需embedding。中文效果和成本必须另测，不能采用其托管平台成绩作为RecallCard或OSS成绩。

[云Dream文档](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/docs/platform/features/dream.mdx)区分Supersede/Merge与Synthesis：后者属于Pro起的功能，并受数量、时间和纯user_id范围限制，带app_id/run_id等字段的编码插件记忆不能自动等同于得到同样整理。这是公开服务契约，未核实服务端算法。

代码为[Apache-2.0](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/LICENSE)。self-hosted有认证，但所检查[路由](https://github.com/mem0ai/mem0/blob/b7ad69afda6b6ed030347c66d48a13e4de9dec08/server/main.py#L443)及共享Memory入口未见按认证主体校验目标记忆所有权；这不是完整安全审计或漏洞判定，只说明不能将user_id筛选直接当团队ACL。

### Letta

[旧Letta服务器说明](https://github.com/letta-ai/letta/blob/5bcdd177d70fa2b31a754cfcd801e77b2e1ab16a/README.md)已把V1 server退休到归档。当前[Letta Code](https://github.com/letta-ai/letta-code/tree/4b028fab07c69edaac2ddb4f7b9a43573ff20d81)以Git-backed MemFS组织可读记忆，根部核心内容进入上下文、索引目录按需读；不是必须先建向量库才可用。

最值得借鉴的是实际执行边界：[worktree](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/src/agent/memory-worktree.ts#L215)中整理，合并成功或无变化才消费来源；父目录脏、冲突或失败不推进处理进度。[整合事务](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/src/cli/helpers/reflection-launcher.ts#L609)在同一lease下完成验证与合并。这与RecallCard已有read-set/receipt/事务边界相容，可改进后台任务，而不用引入整个Agent运行时。

不要照搬默认写入政策：[默认配置](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/src/settings-manager.ts#L182)为每25步触发、auto merge；explicit整合也由另一Agent执行，不能解释成人工批准。[Dream模型选择](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/src/agent/subagents/subagent-model.ts#L239)在本地沿用父模型，云端由auto-memory路由，不能假设后台调用总是低价小模型。

其[本地会话实现](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/src/backend/local/local-backend.ts#L896)固定已编译前缀，用revision更新消息传达记忆变化，符合原设计的前缀稳定方向。Git删除文件不自动清除历史及派生副本，不能用版本控制替代完整遗忘语义。代码[Apache-2.0](https://github.com/letta-ai/letta-code/blob/4b028fab07c69edaac2ddb4f7b9a43573ff20d81/LICENSE)不覆盖其排除的品牌和视觉资产；共享仓库和云端能力也不能由代码许可推定免费可本地使用。

## 保留 借鉴 复用 重做

| 决定 | 具体范围 | 原因 |
|---|---|---|
| 保留 | Event/Memory正本、来源/角色/时间、版本、保护、可撤销遗忘、事务与收据 | 用户要的是能查原话并纠错，提炼记忆不能取代证据 |
| 借鉴 | Mem0持久outbox和宿主hook；Letta隔离整理和成功后watermark；LangMem安静期合并 | 把常规工作留在后台，同时允许恢复和追责 |
| 小范围试用 | LangMem候选生成或成熟分页/重试模块，经过同一组本地测试后再决定 | 减少自造，但不更换正本与授权边界 |
| 重做 | 默认位置与首次连接、单次整批导入、目标端直接使用、最近整理的例外处理 | 当前最大成本来自人为搬运和反复确认 |
| 不作为默认依赖 | 图数据库、另一套Agent runtime、托管记忆服务、实时全量embedding | 会增加配置、运维和数据外发，未证明能改善当前首个任务 |
| 暂只研究 | 团队共享、自动跨项目合并、云端同步 | 必须有真实身份、读写权限、撤销传播，不能只有scope标签 |

## 后台整理和召回的具体边界

先保存和索引原话，即时可用；整理以新增来源版本作为输入，既有收据防止重复处理。后台任务需持久保存pending/running/completed/failed状态，中断可续、显式取消、失败不推进watermark。无需新增Episode正本或复杂认知对象。

无需模型的去重、索引、来源映射自动完成。语义提炼需要本地模型或已授权提供方；没有配置时原话召回照常工作，不要求用户复制整理任务才能继续。以后启用自动整理应一次说明提供方、允许的资料范围、预算和运行时机，而不是每次重复相同表单。当前手动任务与可选API执行器保留为高级路径。

模型产生的候选须保留出处和不确定性。清楚的用户陈述与助手建议分别处理，语义置信分不能替代授权或真伪证明。受保护记录、矛盾改口与高影响更改进入例外审查，其余自动处理范围要在用户设置中明确，而不是把助手曾经说过“已批准”当事实。

召回默认先查可重建的本地文本索引，按需要补充向量。原话、长期记忆和稳定背景使用同一权限过滤，检索结果带可打开的证据入口。第一次不应要求用户解释“该用哪一种记忆”。更相关的候选不够时允许空结果，不能为凑数强行注入。

## 缓存与费用

一段会话固定稳定背景快照与指引；动态任务资料放到后部普通参考内容或真实工具结果中。后台整理改变版本时通过增量提示处理，新会话或压缩时再生成快照。纠错和遗忘立即生效，不能为了命中缓存继续提供过期内容。

Zep当前[安全文档](https://help.getzep.com/memory-security)明确把检索内容视为不可信数据。即便某个示例把记忆放进高优先级消息，RecallCard也不应把原话、网页或模型总结升级为系统指令。固定前缀是工程属性；实际缓存命中与节省还取决于目标宿主和模型API，没有计费指标时不声称节省比例。

成本要分别记录原文归档、候选提取、合并、embedding、检索重排、重试和后台调用；不因“在后台”就忽略成本。不处理未变来源，不重做未变embedding，失败只补失败步骤；先测中文实际任务，不照搬厂商benchmark。

## 从0.4迁移

1. 先定位并修复切页显示异常，完成唯一一次导入确认。本次异常尚未取得可读画面，未归因；这项修复不等架构调研结束才处理。
2. 保留现有资料库的Event/Memory编号、来源链和Git历史。默认位置只是新用户体验，不自动迁移或复制既有私有资料；已有用户打开原资料即可继续。
3. 新增可恢复导入任务状态，独立于知识正本；源消息与版本继续进入既有捕获/去重接口。历史导入完成后，不要求重新Dream才能检索。
4. 只把一个平台的全历史读取与一个目标端接续接通，真实验证分页、角色、时间、故障恢复和最终发送边界。不能以合成页面或组件测试替代登录态链路声明。
5. 再增加按授权和预算运行的后台候选整理，把逐条管理改为“最近整理了什么 / 哪几项需我判断”。现有受保护修改校验和原始出处读取继续使用。
6. 最后才评估语义索引和团队共享。可重建索引可以升级；正本格式改变必须另有迁移、备份和回退验证。

## 下一阶段的完成标准

- 不懂内部数据模型的用户可完成首次导入；普通导入只有一次最终开始，不重复确认
- 同一历史再次导入不重复保存；中断后继续只处理未完成项，失败不会被统计为完成
- 新保存的决定无需等待整理即可在另一个端找到原话；回答采用了哪个来源可以核对
- 来源改口、遗忘和项目隔离在实际目标端生效，旧注入不被再次采集为新用户事实
- 日常不强迫查看后台队列、模型参数、JSON或Git；需要排错时能看到具体失败与覆盖
- 同时报告操作步数、首次可用时间、来源正确性、遗漏、无答案和费用；技术测试通过不能替代这些结果

本报告未安装、运行这些第三方项目，也未把私人历史发送给外部模型。上述源码审查不是其完整安全审计；可复用模块仍需在RecallCard接口内验证。网页能力和价格会变化，本文结论绑定所列日期与提交。
