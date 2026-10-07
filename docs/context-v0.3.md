# 当前召回补强：实体、标签视图与有效时间

这轮沿用四个只读能力 `bootstrap / search / read / sources`，不增加事实类型或写入权限。改动在 `Context` 投影层；Event 和 Memory 仍是正本，生成的 Markdown 不是模型读取接口的权威来源。

## 实体与标签进入文本检索

`Document` 保留 Memory 的 `entities` 和 `labels`，BM25 对内容、实体、标签一并分词。完整实体或标签匹配有额外分数；返回的 `text` 仍是原有 Memory 内容，元数据独立返回，不把标签伪造成用户说过的话。Event 的实体数组为空，不从正文猜测所属机器。

例如两条同属 `personal` 的 Memory：

- 内容“操作系统为 Arch Linux。”，entities 为“笔记本”和“laptop-synthetic”
- 内容“操作系统为 Debian。”，entities 为“服务器”和“server-synthetic”

查询这些实体可分别定位对应记忆；查询“操作系统”可同时取得两条。相同 scope 或相近文字不会自动合并、替换或覆盖记录。此功能是词法检索，不是已经验证的自然语言实体消歧或语义理解。

所有授权、来源 scope 和 suppression 过滤发生在构造候选语料之前，私有记录不会进入 BM25 的词频统计。返回字段仍明确包括来源、范围、状态和时间。实体/标签发生变化时，文档摘要及旧搜索游标随之失效；添加不可访问的资料不会改变该客户端的候选摘要。

## 具名 View

`read` 现在接受 `view:<label>`，例如：

```json
{"refs":["view:computing","view:计算环境"],"budget_tokens":4096}
```

视图在每次调用时从正本动态构建，只包括：

1. 当前客户端被允许读取的 Memory 与其原始证据范围
2. Memory 及来源均未被抑制
3. 状态为 `active` 或 `tentative`
4. 当前时刻位于有效区间内
5. `labels` 与请求标签逐字匹配

不读取 `generated/views/<label>.md`，不把 ref 拼接成文件路径，也不把模型传来的字符串变成本机文件读取权限。已有生成文件即使陈旧或被改写，也不影响这个结果。

标签引用允许 1–128 个 UTF-8 字节，字符限 Unicode 字母/数字及 `-`、`_`、`.`；拒绝单独的 `.`、连续 `..`、斜杠、反斜杠、百分号编码、冒号、空白、控制符和 `@revision`。标签元数据本身不会被重写；不适合 View 引用的标签仍可在授权检索中查询，但不会列入 Bootstrap 的可访问目录。

`view:profile` 保留原有 Bootstrap 别名，因此 `profile` 是保留名称。普通标签视图返回 `records`，每项包含版本化 Memory ref、文本、entities、labels、来源和时间。标签不存在与该标签全部属于未授权范围都返回空 records，不额外暴露隐藏目录是否存在。

`sources` 传入具名 View 时同样返回当前过滤后的视图与证据 refs；需要原始 Event 时，再对其中的 Memory ref 调用 `sources`。不会以视图名绕过单条来源权限。

### 预算与继续读取

`budget_tokens` 仍沿用现有保守 UTF-8 字节上限，而不是声称精确的供应商 token 数。包括外层包装、多个结果、元数据及待处理引用的完整 JSON 均受同一预算约束。

标签视图按稳定 ref 顺序返回能放入预算的完整记录。超出预算的记录列入视图自己的 `pending_refs`，同时标记 `truncated`；待处理列表本身放不下时，再标记 `pending_list_truncated`。外层批量读取被裁减的结果也保留对应 pending ref，避免无提示丢失续读入口。

可以单独 `read` 尚未返回的 Memory ref，或用标签 `search` 并沿搜索游标继续。长内容不会在标签视图中被悄悄截成完整事实。预算不足不授权扩大范围，也不跳过后续引用的权限检查。

## 默认当前时间与历史查询

默认的 Memory 搜索、Bootstrap 目录、标签视图及 embedding 语料导出采用当前有效集合：

```text
状态为 active 或 tentative
并且 valid_from 为空或 valid_from <= 当前时刻
并且 valid_to 为空或 当前时刻 < valid_to
```

Bootstrap 的个人资料正文还保持原有更严格条件：必须是 `active`、受保护且显式带 `bootstrap` 标签。tentative 可以有目录入口，但不会因此进入受保护个人资料正文。

每次读取只用当前时间决定哪些记录可见，不把 now、时间戳或随机数据写进稳定 Bootstrap 文本和摘要。记录没有跨越有效边界且输入不变时，重复输出保持同字节；到达真实有效边界后，内容集合和版本允许改变，不能为了前缀缓存继续提供失效资料。

显式 `search.as_of` 继续提供历史查询，按 `[valid_from, valid_to)` 半开区间筛选；仍可检索相应时间内的 `superseded` 记忆。授权、suppression 和已撤回记录的限制不因历史查询而放宽。未知日期保持 null，不拿记录时间、导入时间或当前时间补造生效日期。

默认筛选不删除历史。已授权的显式 `read memory:...@revision` 仍可以查看未来或过期记录；版本已更新时仍拒绝旧 ref，不用最新内容冒充旧版本。

Event 是已捕获的原始证据，没有 Memory 的有效区间。默认 `target=all` 仍可搜到相关原文；排除一条过期 Memory 不会反向删除其 Event。需要只查询当前整理记忆时使用 `target=memories`，需要特定历史时再明确提供 `as_of`。

## 验证

`crates/recallcard/tests/context.rs` 新增 13 项合成回归，覆盖：

- 同 scope 下 Arch 笔记本与 Debian 服务器的实体区分和不相互覆盖
- 仅出现在 metadata 中的实体/标签参与检索
- 动态 View 不读取生成文件，正本更新即时生效
- 标签路径穿越、编码路径、版本与控制字符拒绝
- 不同 scope、Memory/来源 suppression、未知标签的安全边界
- 默认未来/过期过滤，以及半开历史区间和 superseded 查询
- 未知日期不改写、历史查询不绕过撤权
- Bootstrap 同字节稳定、受保护资料和 tentative 目录分离
- View 返回的旧版本 ref 不冒充新版本
- 批量 read/sources 与嵌套 View 的总字节预算
- 元数据变化导致游标失效、隐藏 scope 不影响公开游标

```sh
RECALLCARD_STATE_DIR=/tmp/recallcard-context-test-state \
  cargo test --locked -p recallcard --test context
cargo clippy --workspace --all-targets -- -D warnings
```

这里只验证本机正本到只读上下文的确定性行为；没有调用模型、验证向量语义质量或宣称可以准确理解任意机器/项目的自然语言关系。
