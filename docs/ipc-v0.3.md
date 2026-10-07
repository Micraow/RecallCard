# RecallCard v0.3 本机 daemon 与 IPC

此阶段提供一个固定 Vault、固定 scope 集合的只读核心进程。MCP stdio 与浏览器 Native Messaging 可以把四个查询转交这个进程；不需要 HTTP、监听 TCP 端口或公网服务。

这不是中心写入服务。Capture、Dream 审查/应用与 Git 同步仍使用原有 CLI、Vault 文件锁与事务恢复。daemon 不提供写入、文件读取路径、shell、插件加载或模型指定的 worker 配置。

## 启动与桥接

Linux 示例使用一个短、私有的绝对路径。首次启动可自动创建 `ipc` 目录，已有目录必须归当前用户拥有、权限为 `0700`：

```bash
recallcard --vault /home/alice/RecallCard-vault daemon \
  --scope personal \
  --endpoint /home/alice/.local/state/recallcard/ipc/core.sock
```

另一终端启动 MCP bridge：

```bash
recallcard --vault /home/alice/RecallCard-vault mcp \
  --scope personal \
  --ipc-endpoint /home/alice/.local/state/recallcard/ipc/core.sock
```

浏览器 Native Host 可在人工安装准备阶段固化同一端点：

```bash
recallcard --vault /home/alice/RecallCard-vault native-install \
  --scope personal \
  --extension-id abcdefghijklmnopabcdefghijklmnop \
  --output-dir /home/alice/.local/share/recallcard/native \
  --ipc-endpoint /home/alice/.local/state/recallcard/ipc/core.sock
```

扩展 ID 是示例，必须替换为实际已授权的扩展 ID。浏览器注册仍按 [Native 安装说明](browser.md) 人工完成；端点来自相邻本机启动配置，网页、扩展 action 和模型参数不能覆盖它。直接使用 `native-host` 时也支持本机 `--ipc-endpoint`。

daemon、MCP 和 Native 的 Vault 与完整 scope 集合必须一致。若一个客户端只能读 `personal`，另一个还能读 `project:work`，应启动两个不同端点，各自绑定所需集合；不能把一个宽权限 daemon 当作按请求任意缩权的共享服务。

未指定 `--endpoint` 时，daemon 会在 stderr 打印所选端点：

- Unix：优先使用 `RECALLCARD_STATE_DIR`，否则选择系统 runtime/state/local-data 目录；文件名由规范 Vault 路径和排序后的 scope 集合摘要导出
- Windows：使用 `\\.\pipe\recallcard-…` 本机命名管道，名称包含当前用户与 Vault/scope 摘要
- Unix socket 路径保守限制为 100 字节；过长时显式指定较短的私有目录。默认目录先解析系统 `/tmp`、`/var` 别名，再绑定真实路径；显式端点及其祖先不得包含符号链接

没有端点参数的旧 MCP/Native 入口继续直接调用本机库。显式选择 IPC 后，连接失败会报告错误，不会悄悄改读另一个 Vault 或绕回直接读盘。

Windows 本机端点示例：

```powershell
recallcard --vault C:\RecallCard-vault daemon --scope personal --endpoint '\\.\pipe\recallcard-personal'
recallcard --vault C:\RecallCard-vault mcp --scope personal --ipc-endpoint '\\.\pipe\recallcard-personal'
```

可选语义后端只能在 daemon 启动时由维护者用 `--semantic-config` 指定；worker 故障与授权过滤沿用 Context 的规则。客户端请求没有程序、参数、API 地址、scope 或本地配置字段。

## 权限与身份

Unix 会创建/验证私有目录，socket 权限设为 `0600`，连接两端检查对端 UID。目录祖先不能允许其他用户替换路径，系统拥有的 sticky 临时目录除外。预存符号链接、普通文件、共享权限 socket 和非本用户端点都被拒绝，不会被覆盖。

Windows 管道显式拒绝远端连接，DACL 仅允许当前用户 SID。客户端还校验管道服务进程属于同一用户，避免向其他用户抢占的同名端点发送查询。保留 Tokio 默认的 identification 安全级别，不允许服务端借客户端身份做 impersonation。

本阶段的信任边界是同一操作系统用户。摘要绑定用于防止误接不同 Vault/scope，不是密码或应用身份凭据；同一用户下恶意进程通常也能直接读取 Vault。管理员/root 不在此隔离保证内。本阶段没有多应用凭据、每客户端 scope 动态缩权、热更新 scope 或跨用户共享。

修改读取范围需要重启相应 daemon/bridge。Vault 中的当前 suppression 与记录更新在每次查询时重新读取；停止响应的旧客户端不能靠请求参数解除遗忘。

## 单实例、退出与恢复

- Unix 每个端点持有一个独占文件锁。锁文件保持不删除，避免两代进程持有不同 inode 的“同名锁”
- 新进程只有在取得锁、确认原端点是本用户私有 socket、连接明确被拒绝、再次核对 inode 后，才清理失效 socket
- 对可连接、超时、状态不确定或类型不符的端点一律报错，不强制抢占
- 正常关闭只删除自己创建的同一 inode socket；路径被替换时保留替换对象
- Windows 使用 first-pipe-instance 防止双实例；始终先建立下一管道实例再交出当前连接，进程退出由系统回收管道

daemon 在前台运行，可由用户用 Ctrl-C 停止，或交给已有的进程管理器。强制终止后，下次启动走上述失效端点检查。本阶段不安装 systemd/launchd/Windows Service、不自动开机启动，也没有模型可调用的 stop/restart 命令。

## 协议与资源界限

每个连接处理一条请求和一条响应，然后关闭：

1. 4 字节小端无符号长度，加 UTF-8 JSON 正文
2. 请求与响应正文均为 1 至 262144 字节；先检查长度，再分配正文内存
3. 客户端完整读取响应后发送一个 `0x06` 确认字节；服务端随后关闭实例。此确认避免 Windows 关闭管道时丢弃尚未读出的响应

请求只允许以下字段，多余字段直接拒绝：

```json
{
  "protocol": "recallcard.ipc/1",
  "binding": "本机客户端由 Vault 和 scope 算出的完整 SHA-256",
  "request_id": "r_example_01",
  "method": "search",
  "arguments": {"query": "之前的决定", "budget_tokens": 1500}
}
```

method 仅支持 `bootstrap`、`search`、`read`、`sources`，arguments 再由现有 Context schema 校验。`read` 使用不透明引用，不能读取任意路径。响应回传协议、绑定和 request_id，并且只能包含 `result` 或 `error` 之一；客户端校验全部关联字段。

默认整次请求期限为 5000 毫秒，包含客户端连接、完整帧读取、核心执行、响应写回与确认；慢速逐字节发送不会重置期限。`--timeout-ms` 接受 10 至 120000。默认最多 16 个并发请求，`--max-connections` 接受 1 至 64；没有空位时关闭新连接，客户端可在用户控制下重试。

同步 Vault/Context 工作放在有界后台线程上，避免阻塞接受连接。超时关闭响应，但已经执行的本机只读磁盘/CPU 工作不能被安全强杀；它继续占据并发名额直到结束，不会因超时无限累积后台任务。不存在写入中断或自动重放的 IPC 路径。停止服务等待后台工作的时间有界，不能据此承诺对卡死文件系统调用的强制取消。

服务不记录请求正文和召回内容；启动消息只输出本机端点。消息预算仍由 Context 独立执行，IPC 帧上限不能扩大读取权限或输出预算。

## 验证与当前边界

`tests/ipc.rs` 包含真实子进程 daemon，覆盖即时导入可见性、scope/Vault 误绑定、当前 suppression、单实例、强制结束与重启、受控退出、格式错误、超长帧、断帧、慢速客户端、目录/socket 权限、链接拒绝以及保留非 socket 文件与被替换路径。`ipc::tests` 另覆盖内存双工流的帧与确认协议。

本轮本机已通过内存协议 5 项回归，以及 Windows GNU 目标全部 Rust target 的交叉编译检查。本机环境在 socket bind 层返回 EPERM，即使测试命令获得升级执行批准仍如此，因此不能把本机真实 IPC 场景记作通过。全部真实进程回归保持启用，必须以当前提交的 Linux CI 结果验收。Windows/macOS 的进程与权限行为同样以各平台当前提交 CI 为准；交叉编译不等于 Windows 实际运行成功。

这是可测试的 IPC/桥接阶段，不代表设计中的真实浏览器登录场景、真实 Agent 对话、完整后台宿主生命周期或中心 writer 已全部完成。

参考实现依据：[Tokio UnixListener](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html)、[Tokio Named Pipe ServerOptions](https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/struct.ServerOptions.html)、[Windows 管道权限](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)、[Tokio 阻塞工作的取消边界](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)。
