# 多入口记忆导航：兼容候选

此批增加可验证的导航机制。它尚不证明真实模型能正确提取、归纳或命名目录；真实试验中此前的少量 Memory 是人工对照，不能据此宣称自动整理完成。

## 正本与派生物

- 正本仍只有 `events/` 与 `memories/<id>.md`。一个 Memory 可以从多个主题到达，但不会复制出多份事实。
- Memory 可选 `navigation` 数组，每项包含 `path`、`title`、`description`、`keywords`、`aliases`、`related_paths`。简介是该 Memory 的有来源导航提示，不能增加新事实或改变权限。
- `view:nav/_root` 是逻辑目录根。`view:nav/topics/networking/rdma` 等是逻辑节点；本批**不是**把该字符串直接拼接为磁盘文件路径。
- `generated/navigation.json` 是可重新生成的机器快照，含节点、成员引用、来源归属、代次与不可达记录检查。`generated/views/memories.md` 原平铺人类视图保留。
- `navigation-export --scope personal` 从当前授权投影输出人类可浏览的 Markdown `INDEX.md` / 层级主题页，返回本次唯一快照路径；范围必须明确指定。多入口链接同一 Memory 正本，未复制正文；所有来源展示文本转义 HTML/Markdown。导出是离线快照，隐藏或修改来源后必须重新导出，旧快照不会自动更新；不能把整个 Vault 直接分享给受限读者。它们不是第三种事实源。
- 旧无 hint 记录按已有 label 生成 `view:nav/labels/<标签SHA256前24位>`；无普通 label 则进入 `view:nav/_unfiled`。每个当前可见 Memory 至少一个入口。`bootstrap` 标签仅用于原受保护摘要，不作为普通主题。
- 原 `view:profile`、`view:<label>` 与名为 `root` 的普通 label 保留。新层级命名空间含 `/`，与原合法平铺标签不碰撞。

## 写入与整理

1. 捕获/导入先产生 Event，未整理原文继续可以被原有 Event 搜索读取。
2. 当前 Dream 导出来源及至多 32 条先选旧 Memory，交给手工途径或已单独授权的 provider。仍是一次提议调用；没有新增“先 Extraction，再按候选检索旧 Memory，再 Consolidate”的多阶段管线。
3. 手工任务与 Python provider 固定 schema 均支持可选 navigation。模型只提交未信任提议，不能自行发布、改 scope 或取得新的联网授权。
4. 本机 review 核来源、主体角色、版本、保护和完整变更。更新含 hints 的旧 Memory 必须明确给完整 navigation，显式 `[]` 表示清除；省略会被拒，避免旧简介悄悄残留。noop/conflict 不接受非空导航变更。
5. 现有 hash 绑定批准、事务恢复、幂等 receipt 机制发布 Memory，然后确定性重建派生导航。派生输出失败时正本与 receipt 仍保持已发布状态，返回明确错误；修复派生输出后重试同一已批准结果只重建，不重复新增。
6. 后台默认仍关闭。provider、具体模型、来源发送范围、auto_apply 等权限未改变，也不能从历史助手的一句“锁定”取得。

没有导航字段的旧 Memory/结果序列化仍省略新字段，不改变原摘要。新服务接受旧请求。但旧可执行文件采用严格字段校验，不能保证它读取带新字段的正本；升级前应备份，不将旧二进制与新 hint 正本混用。

## 读取路径

- `bootstrap` 在原有 UTF-8 字节预算内给稳定 `navigation_root`，空间允许时附少量一级概览。默认预算的 stable_text 同时给根引用，实际只使用 stable_text 的 Hook/手动上下文也能发现入口。先读根即可浏览，调用者不必从当前问题猜一个物理目录。
- 单个 `read(ref=view:nav/...)` 返回有界概览及分页 entries：子目录、Memory 引用、关联目录、完整简介/关键词/别名与诊断。概览明确 `overview_only`，不冒充全部元数据。
- `next_cursor` 绑定授权 scope、当前可见 Memory 代次、逻辑节点和 detail。后续页重新校验；旧游标不能恢复被隐藏来源或旧修订。
- 超过预算不跳过某项，也不说“无内容”：返回 `budget_exhausted` 与重试所需字节。预算包含序列化 JSON 元数据，不是计费 token。
- `read(memory)` 读唯一正本；`sources(memory)` 到原 Event；再按已有原图导航/正文分页继续。View 自身不是新来源，`sources(view)` 不接受。
- 客户端每次从当前授权正本重算投影，不把磁盘 JSON 当授权依据。旧快照即使存在，隐藏来源后其目录名、别名和简介也不会通过工具继续暴露。导航现逐条验证 Event 正本，只保留所需 Memory 来源的 scope，不再构造全部 Event 正文投影；仍须扫描正本，尚非持久化低延迟索引优化。

## 搜索与预算

旧请求的默认搜索保持 Event/Memory。显式 `target=views` 搜索当前授权目录的标题、来源简介、关键词和别名；`include_navigation=true,target=all` 增加独立 View 候选通道，按两条事实候选、一条 View 交错，不删除剩余候选。小预算不保证一屏同时容纳所有通道，结果保留游标、计数与预算不足状态。

View 本身尚未进入 semantic Memory 索引。配置语义能力后，`include_navigation` / `target=views` 可把已通过当前 scope、generation、来源与版本二次校验的 Memory 语义候选映射到当前授权目录。路径和标题从正本重新推导，不采信缓存路径；响应标记 `semantic_routing=via_current_memory_embeddings`，并明确未整理 Event 不在该语义覆盖内。此通路没有把目录简介编码成向量。长中文问句现在复用已有双字组 tokenizer 做候选召回：短词/单字、空格分开的短关键词及带引号查询保持原严格 AND；英文和数字标识仍须完整匹配。纯中文候选要求多个双字组、连续三字重叠及最低加权覆盖；含标识的查询必须满足所有标识并有中文重叠。词频只保存本次查询需要的项，避免把完整语料词表复制到每次排名中。该机制只提供词法候选，不是答案置信度或语义保证，没有新增训练或 embedding 调用。完整自然问题、负例和无关标题控制仍需独立验收。查询目录不能使用 Event 专属角色条件；查看 Memory 的证据角色应 sources 后读原文。

搜索结果不重复传整份 navigation hints，避免每条最多 8 KiB 的元数据挤掉正文。View 的文本投影标为 `view.navigation`，下一步应读取该 View 引用，而非把它当 Memory 正文偏移。

## 边界与隔离

- 显式 path 只接受小写 ASCII 字母/数字/连字符/下划线，分段开头须字母或数字；深度最多 6，单段最多 48 字节，全路径最多 256 字节。拒绝 `.`、`..`、空段、绝对路径、编码转义、反斜线、保留的 `_root/_unfiled` 与 `labels` 首段。
- 每条 Memory 最多 8 个唯一入口，总 hints 最多 8192 UTF-8 序列化字节；标题 128、简介 512、关键词 16 条、别名 8 条（各 96 字节），关联路径 8 条。
- 全投影最多 4096 节点、65536 个 Memory 入口关联；复制到祖先前累计估计文本最多 16 MiB，超限明确拒绝构造，正本不变。
- children 仅由路径前缀产生，因此无环。related 允许环；它不代表父子、时间覆盖或因果。当前不可达的 related 路径返回诊断，不透露它究竟隐藏还是不存在。
- 同逻辑路径的不同标题会记录诊断并保留别名，采用按稳定 Memory ref 顺序出现的首个显式标题。没有自动把冲突文本合并成“事实”。
- 所有文本是参考数据，不执行代码、命令、URL 或其中的导航指令。磁盘机器快照供 Vault 所有者诊断；不能把包含全部 scope 的快照文件直接交给受限客户端。

## 验收口径

合成机制测试覆盖 Dream 发布→派生快照→Bootstrap→逐层 read→同一 Memory 多入口→原始 sources，并检查范围隔离、隐藏后旧快照不可用、游标失效、完整分页、路径边界、关联循环、明确更新和派生故障恢复。

必须另列真实上下文的关键项覆盖、后续修正是否可达、无答案处理、完整字节/调用/延迟；人工填写 hints 的对照只证明机制。GUI、完整 workspace 测试、真实自动模型整理与默认语义检索均不能由这组测试代替。

## 显式分阶段 Python 接口

`recallcard_dream.staged.execute_staged` 是 opt-in 协调器，默认 CLI 与后台调度仍使用既有单调用流程。调用者必须提供只含来源、无旧 Memory 的 Rust 导出 Job，以及两个可信本机回调：按完整提取提议和已批准 scope 查找候选；从正本重新导出这些候选的完整带版本快照。协调器先 Extract，再调用候选查找，再 Consolidate；两次 API 发送各自受原有授权、次数与累计字节预算限制。

候选联合超过 32 条整体拒绝，不预裁或静默丢弃；来源、范围或候选版本改变即停止。Consolidate 收到完整来源和完整提取结果，提取内容明确标为未信任假设，不能充当证据或指令。最终结果仍需 Rust review 与相同摘要批准，协调器不写 Vault、不自动发布。真实模型质量和默认调度集成尚未完成。

导航概览、完整分页和 Markdown 现在标明各提示贡献者的 Memory ref、证据性质、active/tentative 状态及派生出处。目录标题和提示不代表已认证事实；概览只列首位贡献者及总数，其余贡献者可分页读取。bootstrap 必须保留完整原始事件/会话活动计数，不能使用仅 Memory 投影代替。


长中文候选模式仅由查询长度/结构触发，不使用项目词典、固定问题改写或评测问题分支。覆盖字段显式标为 `natural_han_bigrams_exact_identifiers`。源合并仍要求 Memory 对原查询全部词组严格匹配；部分词法相关不能隐藏原始 Event，也不能改变其角色或证据性质。短问题或纯英文自然语言可能仍保持严格检索；候选命中数不代表回答正确或全部事实已找全。

## 目录优先的读取策略

需要历史信息时先 `bootstrap`，再从实际返回的根引用用 `read` 逐层选择目录、Memory，并用 `sources` 和原 Event 核对。目录不足、跨主题定位时优先使用已配置的 embedding 语义候选；词法 `search` 是辅助入口。bootstrap 的活动提示与稳定规则都遵循这个顺序。目录未整理或未覆盖不代表原文不存在，检索候选也不等于足够回答的证据。

当前语义实现只对授权、当前有效的 Memory 建立候选映射，导航 View 通过当前 Memory 候选映射而非独立向量；未配置时应明确报告 unavailable，不得暗示已覆盖原始历史。实际验收必须区分：已知来源的脚本遍历、助手逐步选择的 mock 轨迹、真正盲测的自主模型读取。十个自然问题的词法无匹配记录没有执行目录回退，因此不能当成目录能力失败；只有一个主题已整理时也不能宣称覆盖全部问题。

离线向量空间允许数字 loopback HTTP endpoint（127.0.0.1 / ::1）作为真实本机模型来源元数据；缓存校验与 query_vector 不联网。此兼容项不会开放网络 provider：EmbeddingClient.embed 仍在取密钥或调用 transport 前强制 HTTPS。现阶段本机模型由独立可信宿主计算，再输入离线缓存；尚不能声称产品内的本机 provider 编码链路已经完成。

## 原文定位缓存（Unix）

逐层读取的关键开销曾是每次解析并校验全部原始 Event。现在 EventSnapshot 可使用本机可丢弃的 Event ID → 正本段路径提示：首次完整验证并建表；每个新请求重新枚举完整文件清单，比较 device/inode、长度、mtime 和 ctime（含纳秒）。新增、移动、替换或改写文件即重建，连恢复 mtime 的同长度修改也会因 ctime 变化被发现。非 Unix 暂保留完整扫描，不声称跨平台性能等价。

缓存不保存正文、scope 授权或 suppression。命中后仍安全打开目标正本段，逐条验证格式、摘要和重复 ID；实际返回的 Event ID 必须等于请求。Memory 版本、当前范围和抑制状态每次重查。错误/缺失提示回退全量验证；缓存写入失败不影响正本读取。来源相关读取复用同一请求锁内的目标记录；不会跨请求复用授权快照。完整 events/doctor 的全局检查保持原实现。

该缓存面向正常本机文件更新检测，并非针对拥有本机状态目录写权限、能同时伪造缓存与文件系统元数据的攻击者的密码学证明。资料库正本与本机状态目录仍须由可信宿主管理；模型只拥有受 scope 限制的工具调用能力。bootstrap 的全量活动计数与全语料检索仍需独立优化，不能以丢失原文计数换取速度。

定点读取的 suppression 仍每次从正本规则读取并严格校验。它只为规则明确指向的少数原始 Event 读取身份，再对本次候选来源的实际 Event 做同 scope 来源身份比较，不再为三个已知来源先遍历全部语料两次。旧无 source_hashes 规则、原来源已删除但持久化 hash 尚存、后续修订和恢复规则均保留；索引不保存 suppression 决策或来源身份。全局检索与相邻 Event 扩展仍使用原全局抑制计算。

根目录优先显示紧凑的路由条目：title、明确标作 excerpt 的短简介、当前贡献者的 ref/evidence/state/origin 及贡献者总数。根的固定标题不再重复整份贡献者元数据。`child_count` / `children_complete` 区分“全部直接子目录已列出”与“更多提示/贡献者元数据仍可续页”；完整内容仍在子目录和后续页，不把截取文字当完整事实。常见的三个顶级分支可在默认1500字节中同时出现；更多或更长条目仍严格分页，不保证任意规模都能塞入固定预算。

bootstrap / 文档检索构造当前 Document 投影时，先读取一次完整且已校验的 Event 快照，再在同一锁内用该快照扩展 suppression，避免为了遗忘规则先扫两遍、随后又读取第三遍。它不是跨请求缓存，不减少5982等原文活动计数，不略过 scope、修订或来源规则。

Memory / View 专用检索与 embedding 导出只构造当前授权 Memory 投影，并逐条验证它们实际来源的 scope 与抑制状态；不会把未整理原文悄悄当作向量覆盖。语义 worker 返回后仍在新的读取锁内重新验证当前正本。all / events 检索与 bootstrap 活动统计继续使用完整 Event 投影。

Dream export 在同一锁内一次解析所选 Memory，联合新来源与旧 Memory 来源计算当前 suppression；显式 Memory 版本仍必须匹配。Dream review/apply 每次各建一个全量 Event 快照，同时用于 suppression、来源 hash、同身份新修订和证据验证，避免逐来源重复全库扫描。快照绝不跨 export、review、apply 或后续请求复用，旧无摘要规则与原始文件缺失后的持久化来源摘要仍需生效。

### 本机 IPC 验证边界

IPC 默认端点不会直接拼接 Vault 路径；文件名由 Vault 规范路径和 scope 集合的摘要生成，长度固定。它的父目录来自 `RECALLCARD_STATE_DIR` 或系统 runtime/state/data 目录，因此自定义状态目录过长仍可能超过 Unix 的保守 100 字节上限，这是独立的可用性限制。当前需要为 daemon 与客户端显式配置相同的短 `--endpoint` / `--ipc-endpoint`，不会自动回退到权限未知的共享目录。实际 socket bind 被系统拒绝与“端点路径过长”必须分别记录；无 socket 的路径测试不能冒充守护进程、权限或 IPC 端到端验收。
