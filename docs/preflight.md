# 固定本地预检门禁

每次准备提交或启动安装包 CI 前，在仓库根目录运行：

```sh
python3 scripts/preflight.py --report /tmp/recallcard-preflight.json
```

stdout 始终输出一份 JSON，stderr 输出中文逐项摘要。任一检查失败都返回非零，
独立检查仍继续运行，方便一次看到问题。`local_gate_passed: true` 仅表示完整本地
门禁通过；`full_acceptance_passed` 始终为 false。报告的 `not_run` 明确列出真实
Chromium、Tauri/WebKit 原生闭环、Rust 测试和安装包阶段。

预检不自动安装依赖、不启动浏览器、HTTP 服务或 socket，不重新编译，不访问外部
API，也不推送 Git 或触发 GitHub Actions。只读取仓库，运行下列已审阅的本地
验证器，在系统临时目录保存语法检查副本，按 `--report` 指定位置写报告。临时副本
会自动删除；报告不会自动创建父目录。不要把真实资料库、个人数据或密钥放入测试。

## 一次性准备开发工具

Node 必须和 `.github/workflows/package-desktop.yml` 的精确版本一致，目前为
24.19.0。`local-dom/package.json` 和锁文件要求 `^24.19.0 || >=26.0.0`，但本门禁
为了复现 CI，额外要求实际 Node 与安装包工作流相同。Node 22 不接受这里使用的
`--test-isolation=none`；不能仅根据 jsdom 的最低版本推断测试参数兼容。

安装阶段需要访问官方 npm/PyPI，和之后的离线预检分开：

```sh
npm --prefix extension ci --ignore-scripts --no-audit --no-fund
npm --prefix desktop/tests/local-dom ci --ignore-scripts --no-audit --no-fund
python3 -m venv /tmp/recallcard-preflight-venv
/tmp/recallcard-preflight-venv/bin/python -m pip install -r scripts/preflight-requirements.txt
```

Python 3.10 或更新版本和 bash 为必需工具；当前验证使用 Python 3.12。
`scripts/preflight-requirements.txt` 只固定开发用的 PyYAML 6.0.3，不影响应用依赖。
YAML 由该解析器读取，并拒绝重复键；GitHub Actions 语义由官方 actionlint 校验。

actionlint 使用[官方安装说明](https://github.com/rhysd/actionlint/blob/main/docs/install.md)
提供的发行版。以下是 Linux x86_64 的固定安装例子；其他平台应按官方文档选择对应
发行版及校验和。不要为预检绕过下载、执行或安全限制。

```sh
mkdir -p /tmp/recallcard-preflight-tools
curl --fail --location https://github.com/rhysd/actionlint/releases/download/v1.7.11/actionlint_1.7.11_linux_amd64.tar.gz \
  --output /tmp/recallcard-preflight-tools/actionlint.tar.gz
printf '%s  %s\n' 900919a84f2229bac68ca9cd4103ea297abc35e9689ebb842c6e34a3d1b01b0a \
  /tmp/recallcard-preflight-tools/actionlint.tar.gz | sha256sum --check
tar -xzf /tmp/recallcard-preflight-tools/actionlint.tar.gz -C /tmp/recallcard-preflight-tools actionlint
/tmp/recallcard-preflight-venv/bin/python scripts/preflight.py \
  --actionlint /tmp/recallcard-preflight-tools/actionlint \
  --report /tmp/recallcard-preflight.json
```

上述 SHA256 已和该版本的[官方校验和文件](https://github.com/rhysd/actionlint/releases/download/v1.7.11/actionlint_1.7.11_checksums.txt)
核对。预检也接受 PATH 中已有的 actionlint，并把其版本记入报告。缺少工具或依赖
会明确失败，不会联网补装，也不会静默跳过。

## 门禁实际验证什么

1. 实际 Node 与安装包 CI 版本一致；用 npm 自带的 semver 检查两个 npm 锁文件内
   所有适用依赖的 Node engines。package.json 与锁文件的根依赖、engines 必须
   一致，已安装依赖版本须符合锁文件；npm 未安装的可选平台依赖单独记录。
   所有执行 local-dom 的工作流步骤之前，必须设置同一 Node 版本。最后让实际
   Node 解析测试参数，避免仅检查版本字符串。
2. actionlint 校验所有工作流的 YAML、Actions 表达式和上下文、job/step/action
   引用。这会拦住把 `runner.temp` 放到不支持该上下文的位置等错误。预检明确
   关闭其可选 shellcheck/pyflakes，避免不同机器上偶然安装的工具改变门禁结果。
3. 工作流的现有源码路径、通配测试路径、npm cache 锁文件、本地 action 和
   working-directory 必须存在且不能越出仓库。已知构建输出目录如 target、
   node_modules、release-artifacts、distribution 不要求预先生成。运行时动态
   路径与生成产物不作“已存在”声明，actionlint 仍检查其中的表达式。
4. `bash -n`/`sh -n` 验证内联 shell，Python AST 验证引用到的 Python 源码、
   Python stdin heredoc 或 Python shell，`node --check` 验证 github-script。只把有限、单行、合法字符
   的 Actions 表达式替换为 `0` 后检查临时副本，绝不执行这些步骤。未知 shell、
   未支持的 Python heredoc 或动态 working-directory 明确失败。默认 shell
   仅按 Linux/bash 语法验证；matrix 中 Windows 的默认 PowerShell、各平台命令
   能否实际运行、权限和外部服务状态都未验证。
5. 用 Python AST 核对 NativeSmoke.exercise 中关键原生流程都唯一、无条件且
   直接调用；密度验收 exercise_workspace_usability 必须位于所有 exercise_*
   流程末尾。每个工作流 job 也只能调用一次完整 native_smoke.py，避免插入重复
   验收后仍误以为通过。
6. 实际调用 workspaceAssets()，核对它返回的每份资源和 desktop/ui 真实文件
   字节相同、资源表无遗漏或陈旧路径。V8 SourceTextModule 解析并链接所有真实
   静态 import/export，HTMLParser 核对 index.html 的脚本、样式与图片引用。
   此阶段不执行应用代码、不启动浏览器。
7. 用 Node 检查仓库应用、扩展和测试 JavaScript 语法；运行现有 test_native_*.py
   驱动合同、extension/tests 与 desktop/tests 的本地 .test.js，以及加载实际
   应用模块的 local-dom DOM/事件合同。只调用已审阅的固定 Node 命令，不执行
   npm lifecycle hooks。driver 合同会模拟驱动，不启动 WebKit 或文件选择器。
8. 执行 scripts/tests/test_preflight.py 的固定负向回归，覆盖错误 Node、锁文件
   engines、遗漏资产、模块/HTML引用缺失、路径越界、重复工作流调用、重复或提早
   的原生密度流程、非法 shell/Python/JavaScript、错误 runner 上下文及 JSON
   秘密值遮盖。负向样本只写临时副本；还核对 Python heredoc 不会被执行。

环境报告仅记录 CI、GITHUB_ACTIONS、GITHUB_WORKSPACE、RUNNER_TEMP 和相关
RecallCard 变量是否存在，不打印值。在真实 GitHub Actions 环境额外核对
RUNNER_TEMP 目录可用、GITHUB_WORKSPACE 与仓库一致。诊断中的秘密环境值按字符串
遮盖，保留 JSON 布尔值和数字类型；报告不包含环境变量清单或认证信息。

## 单独定位失败

```sh
python3 scripts/preflight.py --checks native_order,browser_assets
RECALLCARD_PREFLIGHT_ACTIONLINT=/tmp/recallcard-preflight-tools/actionlint \
  python3 -m unittest discover -s scripts/tests -p 'test_preflight.py' -v
```

`--checks` 只用于定位问题。即使所有所选检查通过，报告也会标为 partial，
`local_gate_passed` 为 false，并列出没选的检查。完整 CI 应运行不带 `--checks`
的命令。单独跑负向测试时若未安装 actionlint，其专项负向用例会显式跳过；完整
预检仍会因 actionlint 缺失失败。

## 必须继续保留的真实验收

DOM 合同不会检查 CSS 布局、元素几何、toast 是否遮挡复制按钮、真实鼠标命中、
键盘/输入法、系统剪贴板、原生模态焦点、Tauri IPC 或真实写盘。这些属于既有
Chromium、Rust 和原生验收，须在门禁后独立运行。预检通过不能把那些阶段标绿，
也不能作为发布或安装包验收通过的依据。新增的 1180/860 宽度通知条与复制按钮
真实几何用例属于 Chromium 阶段，本地预检不会运行它们。

测试门槛不仅检查退出码，还要求Node与Python报告实际执行了至少一项测试；缺失摘要或0项不计为通过。浏览器失败时的PNG、DOM与矩形诊断由正式界面步骤保存，仍不计入本地预检的已执行能力。
