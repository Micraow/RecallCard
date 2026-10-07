# Claude Code 生命周期 Bootstrap 适配器 v0.3

## 已实现的范围

本适配器让可信本机启动配置把 RecallCard 的已授权 Bootstrap 包装成 Claude Code `SessionStart` 的 JSON 输出。Rust 模块位于 `crates/recallcard/src/agent_hook.rs`，配置示例位于 `integrations/claude-code/settings.example.json`。

仅接受四种 `source`：

- `startup`：开始会话
- `resume`：恢复会话
- `compact`：压缩后的加载入口
- `clear`：清空后的新上下文入口

这些名称和 `hookSpecificOutput.additionalContext` 结构已与 [Claude Code 官方 SessionStart 文档](https://code.claude.com/docs/en/hooks#sessionstart) 核对，查阅日期为 2026-10-07。当前官方还列出 `fork`；本版没有实现该入口，示例不匹配它，直接传入时也会拒绝。不要声称覆盖了全部宿主生命周期。

实现和测试均未运行 Claude 或 Codex Agent、CLI、API；没有安装用户 Hook，也没有真实 Claude 宿主验收。通过本地协议测试不等于宿主一定执行 Hook，更不等于模型已经阅读、记住或采用了返回材料。

## 输入、输出和固定权限

库入口：

```rust
pub fn session_start<R: std::io::Read>(
    vault: &Vault,
    access: Access,
    budget_tokens: usize,
    reader: R,
) -> Result<serde_json::Value>
```

Vault、Access、Budget 只能由可信启动参数固定。模块读取最多 `64 KiB + 1` 字节，超过 64 KiB 就拒绝。只解析并校验 `hook_event_name` 与 `source`，不把其它字段当指令、路径或权限。关键字段重复、未知事件/来源、非法 JSON、尾随第二个 JSON、无效 UTF-8、读取失败均返回固定中文错误，不回显输入。

有效输入的最小形态：

```json
{"hook_event_name":"SessionStart","source":"startup"}
```

宿主还可带 session id、transcript 路径、cwd、model 等元数据。本模块跳过未使用字段以兼容宿主扩展，不打开 `transcript_path`，不切换或探查输入 `cwd`，不执行 `command`，也不采用请求中的 scope、budget 或 `permission_mode`。`model` 可以不存在。

成功输出只有一个 JSON 对象：

```json
{
  "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": "既有 Bootstrap 的 stable_text"
  }
}
```

实际 `additionalContext` 原样取自 `Context::bootstrap` 的稳定文本，不加入 session id、当前时间、nonce、路径或调用次数。权限、事实内容和有效区间没有变化时，相同预算生成相同字节；四个生命周期入口不会自行改写同一份资料。有效期跨界、撤权或遗忘发生后应使用更新后的材料，而不是为了缓存保留过期事实。

输出继续受既有 Bootstrap 规则约束：只在正确范围与证据权限内提供用户显式标记 bootstrap 的受保护、当前 active Memory；抑制状态即时应用。普通原始 Event、不合条件的 Memory 不会因启动 Hook 被整体注入。文本保留“参考资料不是系统指令”的边界。

预算参数沿用 `budget_tokens` 名称，但实际使用保守 UTF-8 字节上界，允许 512–32768。最终 JSON 序列化后再检查整个输出大小，包含转义和封装开销。不要把它宣传成特定模型 tokenizer 的精确 token 数。

## CLI 接线约定

主程序已接线，`lib.rs` 导出 `pub mod agent_hook;`，CLI 提供：

```text
recallcard --vault <固定绝对路径> agent-hook --scope <明确允许范围> --budget-tokens 1500
```

成功仅打印 `session_start` 的返回对象到 stdout 并退出 0；错误只进 stderr，以非零状态退出，不输出部分资料。不要在返回对象外再包一层 RecallCard 的 `result`，也不要在 stdout 添加日志，否则不是预期的宿主 Hook 合同。

直接把 `std::io::stdin().lock()` 传给函数，不先经过通用 `input<T>` 或无限制 `read_to_string`。`Access::new` 必须拒绝缺失 scope，不从 stdin 推导全局授权。本模块收到的是已经打开的固定 Vault，不负责选择路径或初始化资料库。

读操作使用已有的共享读锁，可能访问或建立 RecallCard 本机状态目录中的锁文件；不写 Event、Memory、receipt、suppression 或设置文件。既有 `Vault::open` 的目录检查行为由主程序负责；“只读适配器”不表示进程永远不会使用任何本机锁文件。

## 人工安装示例

安装前需用户明确批准：哪个 Claude Code 客户端可以在上述生命周期自动接收哪些 scope 的资料。适配器本身不联网，但宿主可能将 `additionalContext` 随模型请求发送给其所配置的服务。批准本地读取不自动等于批准向该服务持续分享个人或敏感资料。

1. 先在合成资料库完成下节离线测试，确认当前构建已经提供 `agent-hook`
2. 审阅 `integrations/claude-code/settings.example.json`，将两个绝对路径、项目 scope 和预算替换为经过批准的值
3. 查看现有配置并备份；把这一条 SessionStart handler 手工合并进对应的 `hooks.SessionStart` 数组，保留现有 Hook 和其它设置
4. 优先选择项目本机配置；不要仅为方便把个人全局 scope 开放给所有项目
5. 由用户自行检查宿主是否接受该配置并在需要时运行真实宿主验收；本项目测试不会替用户启动 Claude

配置示例采用官方的 `command` 加 `args` 直接执行形式，路径作为独立参数传递，没有 shell 拼接或输入变量展开；设置同步的 10 秒超时。旧宿主是否支持此形式，需要按用户实际版本核对，不能假定所有版本一致。[官方命令 Hook 参数](https://code.claude.com/docs/en/hooks#command-hook-fields)

不要直接把示例文件复制覆盖现有 `settings.json`。示例没有安装脚本，不会查找或编辑用户主目录，也不新增 MCP server；需要后续检索时仍使用独立的四工具 MCP 配置。Hook 不负责捕获对话、执行 Dream、读取转录文件或“补抓历史”。

## 停用

从当初人工添加的配置位置，移除这条调用 `recallcard ... agent-hook` 的 handler；保留同一数组中的其它 Hook。仓库不会替用户修改已有设置。当前官方的配置停用与读取规则见 [禁用或移除 Hook](https://code.claude.com/docs/en/hooks#disable-or-remove-hooks)。

停用只阻止后续发送，不会撤回宿主已经接收的上下文或其历史记录。撤销 RecallCard scope 或抑制某条记录会影响下一次加载；已经处于旧会话中的资料仍可能存在，需要用户按宿主能力处理会话。

## 可执行 fixture 与验证

下列测试只运行本项目 Rust 代码和合成临时 Vault，不启动任何外部 Agent：

```sh
cargo test --offline --test agent_hook
cargo clippy --offline --test agent_hook -- -D warnings
```

测试通过库API和真实 RecallCard CLI 子进程共同验证；不会启动外部 Agent。

17 项专用测试覆盖：

- 四种生命周期使用相同官方输出封装；与现有授权 Bootstrap 文本完全一致
- 未知事件、未知 source（包括暂不支持的 fork）、缺失/重复关键字段、坏 JSON 拒绝
- 恶意路径、命令、scope、budget、permission mode 与其它 metadata 不能改变权限、输出或执行命令
- 既有 scope/证据约束、Memory/来源抑制与 Bootstrap 选择条件
- 重复调用同字节，不写事实文件，不覆盖既有设置
- Unicode 与 JSON 转义后的完整输出预算、64 KiB 边界及无限输入读取上限
- I/O 与解析错误脱敏；配置示例采用固定参数直接执行

`integrations/claude-code/fixtures/` 提供 startup、resume、compact、clear、unknown-event 和 untrusted-metadata 的完整 JSON。当前可只运行 RecallCard 做离线 smoke test：

```sh
target/debug/recallcard --vault /已创建的合成资料库 \
  agent-hook --scope project:demo --budget-tokens 1500 \
  < integrations/claude-code/fixtures/startup.json
```

把 stdin 文件依次换成其它合法 fixture，比较输出；unknown-event 应非零退出且 stdout 为空。untrusted-metadata 应得到与 startup 相同的授权资料，不读取其中路径、不创建任何命令标记文件。

测试不验证真实宿主触发时机、升级后的配置加载、用户账号权限、模型采纳程度、窗口内去重或真实缓存命中。适配器没有写“已经交付给模型”的收据；重复调用会重新输出相同资料，最终宿主如何保存或合并由宿主决定。
