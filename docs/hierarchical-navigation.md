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
- 客户端每次从当前授权正本重算投影，不把磁盘 JSON 当授权依据。旧快照即使存在，隐藏来源后其目录名、别名和简介也不会通过工具继续暴露。启动背景及导航现逐条验证 Event 正本，只保留所需 Memory 来源的 scope，不再构造全部 Event 正文投影；仍须扫描正本，尚非持久化低延迟索引优化。

## 搜索与预算

旧请求的默认搜索保持 Event/Memory。显式 `target=views` 搜索当前授权目录的标题、来源简介、关键词和别名；`include_navigation=true,target=all` 增加独立 View 候选通道，按两条事实候选、一条 View 交错，不删除剩余候选。小预算不保证一屏同时容纳所有通道，结果保留游标、计数与预算不足状态。

View 仍使用现有词法规则，尚未进入 semantic Memory 索引。此批没有修复自然中文整句词法召回，也没有新训练/embedding 调用。跨语言/长自然问句的发现能力需独立验证。查询目录不能使用 Event 专属角色条件；查看 Memory 的证据角色应 sources 后读原文。

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
