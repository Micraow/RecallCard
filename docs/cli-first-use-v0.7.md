# 从 CLI 导入后，让 AI 知道原文已经可查

CLI 可独立使用，不需要先打开桌面或配置整理模型。以下命令使用同一个目录；已有资料库请改成其实际路径，不要另建一个空库后误以为资料丢失。

```sh
./recallcard --vault ./my-vault setup
./recallcard --vault ./my-vault import ./official-export.zip
./recallcard --vault ./my-vault search '项目关键词' --scope personal
./recallcard --vault ./my-vault bootstrap --scope personal
./recallcard --vault ./my-vault status
```

首次读取不存在的目录会明确提示用相同 `--vault` 参数执行 `setup`，不会自动创建或替换资料。`setup` 对已有非空普通目录会停止。

导入默认 `--format auto`，自动识别受支持的官方 ZIP / JSON。`--format deepseek` 与 `deepseek-export` 等价，`chatgpt` 与 `chatgpt-export` 等价。显式指定平台后遇到另一平台的文件会报错，不会偷偷改成自动识别。旧 `manual-jsonl` 只保留在 `--file` 兼容入口。完整格式提示见 `import --help`。

`bootstrap` 的 `stable_text` 仍是受保护记忆前缀与记忆分类，不复制整个原文库。新增的 `activity_text` 单独说明当前授权范围内可检索的原文条数、来源会话数和当前记忆数；Claude Code SessionStart Hook 会在稳定前缀后附上这一段。没有整理记忆时，原文仍能经 `search` 找到，再用 `read` / `sources` 核对。数字来自当前范围、修订与抑制过滤后的正本，不是全库统计，也不代表模型已阅读这些原文。

普通导入只改变动态覆盖量，不因此改变稳定前缀的 `bootstrap_version`。输出仍计入同一个字节预算；小预算可能使稳定前缀截断，`truncated` 会明确标示。已连接宿主需要开启新会话或触发其恢复入口，才能取得新的启动输出；按需 MCP 查询每次读取当前状态。

本次修复有独立的合成 CLI、范围过滤和 Hook 回归。旧候选上的真实测评保留原结果，新构建的实际宿主体验另行记录，不用源代码回归替代实际验收。
