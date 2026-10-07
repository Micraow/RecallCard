#!/usr/bin/env python3
"""离线、本地的提交前门禁。stdout 为 JSON，stderr 为中文摘要。"""
from __future__ import annotations

import argparse
import ast
from collections import Counter
from datetime import datetime, timezone
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


class PreflightError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise PreflightError(message)


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def sanitized(text):
    # 不输出环境表；诊断中出现的秘密环境值也遮盖。短值不全局替换，以免破坏错误说明。
    for name, value in os.environ.items():
        if len(value) >= 4 and re.search(r"TOKEN|SECRET|PASSWORD|CREDENTIAL|API_KEY|PRIVATE_KEY", name, re.I):
            text = text.replace(value, "[已隐藏]")
    return text


def sanitized_data(value):
    # 只清理字符串再序列化；不能替换已编码 JSON 的 true/false/null 或数字。
    if isinstance(value, str):
        return sanitized(value)
    if isinstance(value, list):
        return [sanitized_data(item) for item in value]
    if isinstance(value, dict):
        return {key: sanitized_data(item) for key, item in value.items()}
    return value


def command(arguments, cwd, *, input_text=None, timeout=120, include_stderr=False, environment_overrides=None):
    environment = dict(os.environ)
    environment.update(environment_overrides or {})
    # 验证命令不继承 Node 启动注入，也不让 bash -n 读取用户启动脚本。
    for name in ("NODE_OPTIONS", "BASH_ENV", "ENV"):
        environment.pop(name, None)
    try:
        result = subprocess.run(arguments, cwd=cwd, env=environment, input=input_text,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise PreflightError(f"无法完成本地检查 {Path(arguments[0]).name}：{error}") from error
    require(result.returncode == 0,
            f"{Path(arguments[0]).name} 退出码 {result.returncode}\n{result.stdout[-16000:]}{result.stderr[-16000:]}")
    return (result.stdout + (result.stderr if include_stderr else "")).strip()


def workflows(root):
    try:
        import yaml
    except ImportError as error:
        raise PreflightError("缺少 PyYAML；按 docs/preflight.md 先安装锁定的开发工具，预检不会自动下载") from error

    class UniqueLoader(yaml.SafeLoader):
        pass

    def mapping(loader, node, deep=False):
        result = {}
        for key_node, value_node in node.value:
            key = loader.construct_object(key_node, deep=deep)
            require(key not in result, f"工作流 YAML 有重复键：{key}")
            result[key] = loader.construct_object(value_node, deep=deep)
        return result

    UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, mapping)
    paths = sorted((root / ".github/workflows").glob("*.y*ml"))
    require(paths, "找不到 GitHub 工作流")
    return [(path, yaml.load(path.read_text(encoding="utf-8"), Loader=UniqueLoader)) for path in paths]


def setup_node_versions(data):
    return [str(step.get("with", {}).get("node-version", ""))
            for job in data.get("jobs", {}).values() for step in job.get("steps", [])
            if str(step.get("uses", "")).startswith("actions/setup-node@")]


def check_node(root):
    actual = command(["node", "--version"], root).removeprefix("v")
    package_workflow = dict((p.name, data) for p, data in workflows(root))["package-desktop.yml"]
    pins = setup_node_versions(package_workflow)
    require(len(pins) == 1 and re.fullmatch(r"\d+\.\d+\.\d+", pins[0]),
            "安装包工作流必须有唯一的精确 Node 版本")
    require(actual == pins[0], f"本地 Node {actual} 与安装包 CI {pins[0]} 不一致")
    npm = shutil.which("npm")
    require(npm, "缺少 npm，无法复用 npm 的 semver 校验器")
    checks, absent_optional = [], []
    packages = {}
    for directory in ("extension", "desktop/tests/local-dom"):
        package = read_json(root / directory / "package.json")
        lock = read_json(root / directory / "package-lock.json")
        require(lock.get("lockfileVersion") == 3, f"{directory} 必须使用已支持的 lockfile v3")
        locked_root = lock["packages"][""]
        for field in ("dependencies", "devDependencies", "optionalDependencies", "engines"):
            require(package.get(field, {}) == locked_root.get(field, {}),
                    f"{directory} 的 package.json 与锁文件 {field} 不一致")
        packages[directory] = package
        for name, entry in lock["packages"].items():
            installed = root / directory / name / "package.json"
            if name and entry.get("optional") and not installed.is_file():
                absent_optional.append(f"{directory}/{name}")
                continue  # npm 可按平台跳过 fsevents 等可选依赖。
            if entry.get("engines", {}).get("node"):
                checks.append({"version": actual, "range": entry["engines"]["node"],
                               "label": f"{directory}/{name or 'package.json'}"})
            if name:
                require(installed.is_file(), f"未安装锁定依赖：{directory}/{name}；请先执行文档中的 npm ci")
                require(read_json(installed).get("version") == entry["version"], f"已安装依赖与锁文件不同：{directory}/{name}")
    relevant_jobs = []
    for path, data in workflows(root):
        for job_name, job in data.get("jobs", {}).items():
            selected = None
            for step in job.get("steps", []):
                if str(step.get("uses", "")).startswith("actions/setup-node@"):
                    selected = str(step.get("with", {}).get("node-version", ""))
                if "local-dom" in str(step.get("run", "")):
                    require(selected == actual, f"{path.name}/{job_name} 运行 local-dom 前必须固定 Node {actual}，实际 {selected or '未设置'}")
                    relevant_jobs.append(f"{path.name}/{job_name}")
    # 使用 npm 自己随附的 semver，避免自行近似实现 engines 语义。
    source = """
const fs = require('node:fs');
const {createRequire} = require('node:module');
const semver = createRequire(fs.realpathSync(process.argv[1]))('semver');
const checks = JSON.parse(fs.readFileSync(0, 'utf8'));
const bad = checks.filter(c => !semver.validRange(c.range) || !semver.satisfies(c.version, c.range));
if (bad.length) { console.error(JSON.stringify(bad)); process.exit(1); }
console.log(JSON.stringify({checked: checks.length}));
"""
    command(["node", "-e", source, npm], root, input_text=json.dumps(checks))
    # 真正让当前 Node 解析参数，而不是仅依赖版本表。
    command(["node", "--test-isolation=none", "--experimental-vm-modules", "-e", "void 0"], root)
    return {"node": actual, "ci_node": pins[0], "engine_constraints": len(checks),
            "local_dom_jobs": sorted(set(relevant_jobs)), "locked_packages": sorted(packages),
            "absent_optional_dependencies": absent_optional}


EXPRESSION = re.compile(r"\$\{\{(.*?)\}\}", re.S)


def syntax_placeholder(source):
    def replace(match):
        require("\n" not in match.group(1), "工作流含多行表达式，不能安全地做语法占位")
        expression = match.group(1).strip()
        require(expression and re.fullmatch(r"[\w.\s()!<>=&|'\"/:+*,\[\]-]+", expression, re.ASCII)
                and "\n" not in expression, "工作流含不支持的多行或特殊表达式，不能安全地做语法占位")
        return "0"
    replaced = EXPRESSION.sub(replace, source)
    require("${{" not in replaced, "工作流表达式未闭合")
    return replaced


def python_heredocs(source):
    """仅识别本仓库支持的 Python stdin heredoc；源码只 compile，从不执行。"""
    lines = source.splitlines()
    bodies, shell_lines = [], []
    index = 0
    while index < len(lines):
        line = lines[index]
        match = re.search(r"\bpython(?:3(?:\.\d+)?)?\s+-\s+<<(-?)\s*(['\"]?)([A-Za-z_]\w*)\2\s*$", line)
        if not match:
            require(not (re.search(r"\bpython[\d.]*\b", line) and "<<" in line),
                    "遇到未支持的 Python heredoc 写法，须扩展预检后再使用")
            shell_lines.append(line)
            index += 1
            continue
        shell_lines.append(line.split("<<", 1)[0])
        delimiter = match.group(3)
        body = []
        index += 1
        while index < len(lines) and lines[index].lstrip("\t") != delimiter:
            body.append(lines[index].lstrip("\t") if match.group(1) else lines[index])
            index += 1
        require(index < len(lines), f"Python heredoc 未闭合：{delimiter}")
        bodies.append("\n".join(body) + "\n")
        index += 1
    return bodies, "\n".join(shell_lines)


SOURCE_ROOTS = ("scripts/", "desktop/", "extension/", "python/", "crates/", "docs/", ".github/")
GENERATED_PARTS = {"target", "node_modules", "release-artifacts", "distribution", "gen"}


def source_path(root, cwd, token):
    token = token.removeprefix("./")
    if "=" in token and token.startswith("--"):
        token = token.split("=", 1)[1]
    if any(marker in token for marker in ("$", "{{", "\n")) or "://" in token:
        return
    path = Path(token)
    if GENERATED_PARTS.intersection(path.parts):
        return
    candidate = (cwd / path).resolve()
    relevant = token.startswith(SOURCE_ROOTS) or token in ("Cargo.toml", "Cargo.lock", "package.json", "package-lock.json")
    relevant = relevant or token.endswith((".py", ".js", ".mjs", ".toml")) and "/" in token
    if not relevant or path.is_absolute():
        return
    require(candidate.is_relative_to(root), f"源码引用超出仓库：{token}")
    if any(character in token for character in "*?["):
        candidates = list(cwd.glob(token))
        require(candidates, f"工作流源码通配路径没有匹配：{token}")
    else:
        require(candidate.exists(), f"工作流引用的源码路径不存在：{token}")
        candidates = [candidate]
    for referenced in candidates:
        require(referenced.resolve().is_relative_to(root), f"源码引用超出仓库：{token}")
        if referenced.suffix == ".py" and referenced.is_file():
            ast.parse(referenced.read_text(encoding="utf-8"), filename=str(referenced.relative_to(root)))


def check_workflow_sources(root):
    scripts, embedded_python, embedded_js, paths = 0, 0, 0, []
    with tempfile.TemporaryDirectory(prefix="recallcard-preflight-") as temporary:
        for path, data in workflows(root):
            paths.append(path.name)
            for job_name, job in data.get("jobs", {}).items():
                defaults = dict(data.get("defaults", {}).get("run", {}))
                defaults.update(job.get("defaults", {}).get("run", {}))
                native_calls = 0
                for number, step in enumerate(job.get("steps", []), 1):
                    label = f"{path.name}/{job_name}/步骤{number}"
                    directory = step.get("working-directory", defaults.get("working-directory", "."))
                    require("${{" not in str(directory), f"{label} 动态 working-directory 尚未支持本地路径校验")
                    cwd = (root / directory).resolve()
                    require(cwd.is_relative_to(root) and cwd.is_dir(), f"{label} working-directory 不存在或超出仓库：{directory}")
                    uses = str(step.get("uses", ""))
                    if uses.startswith("./"):
                        action_path = (root / uses).resolve()
                        require(action_path.is_relative_to(root) and action_path.exists(), f"{label} 本地 action 不存在或超出仓库：{uses}")
                    for cache in str(step.get("with", {}).get("cache-dependency-path", "")).splitlines():
                        source_path(root, root, cache)
                    if uses.startswith("actions/github-script@"):
                        code = syntax_placeholder(step.get("with", {}).get("script", ""))
                        target = Path(temporary) / "github-script.cjs"
                        target.write_text("async function preflightSyntaxOnly() {\n" + code + "\n}\n", encoding="utf-8")
                        command(["node", "--check", str(target)], root)
                        embedded_js += 1
                    if "run" not in step:
                        continue
                    source = syntax_placeholder(str(step["run"]))
                    shell = step.get("shell", defaults.get("shell", "bash"))
                    require(shell in ("bash", "sh", "python"), f"{label} shell={shell} 尚未验证，不能标记通过")
                    if shell == "python":
                        ast.parse(source, filename=label)
                        embedded_python += 1
                        continue
                    command([shell, "-n"], root, input_text=source)
                    bodies, shell_source = python_heredocs(source)
                    for body in bodies:
                        tree = ast.parse(body, filename=label)
                        for node in ast.walk(tree):
                            if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.value.startswith(SOURCE_ROOTS):
                                source_path(root, cwd, node.value)
                        embedded_python += 1
                    lexer = shlex.shlex(shell_source, posix=True, punctuation_chars=";&|()")
                    lexer.whitespace_split = True
                    for token in lexer:
                        source_path(root, cwd, token)
                        if Path(token).name == "native_smoke.py":
                            native_calls += 1
                    scripts += 1
                require(native_calls <= 1, f"{path.name}/{job_name} 重复调用原生完整验收 {native_calls} 次")
    return {"workflows": paths, "shell_syntax_only": scripts, "python_syntax_only": embedded_python,
            "github_script_syntax_only": embedded_js, "expressions": "仅用合法的 0 做语法占位；没有执行工作流",
            "platform_scope": "默认 shell 只检查 Linux/bash 语法；matrix Windows 的默认 PowerShell 及各平台运行语义未验证"}


def check_actionlint(root, executable):
    resolved = shutil.which(executable)
    require(resolved, "缺少 actionlint；见 docs/preflight.md 的官方安装方法。其余检查继续，但预检退出失败")
    version = command([resolved, "-version"], root)
    # 独立使用 bash/Python 编译器做语法检查；避免因机器是否装 shellcheck/pyflakes 而改变门禁。
    files = [str(path.relative_to(root)) for path, _ in workflows(root)]
    output = command([resolved, "-shellcheck=", "-pyflakes=", *files], root)
    return {"version": version.splitlines()[0], "output": output,
            "scope": "官方 YAML、Actions 表达式与上下文、job/step/action 引用校验；shellcheck/pyflakes 未启用"}


def check_native_order(root):
    tree = ast.parse((root / "desktop/tests/native_smoke.py").read_text(encoding="utf-8"))
    classes = [node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == "NativeSmoke"]
    require(len(classes) == 1, "NativeSmoke 类必须唯一")
    methods = [node for node in classes[0].body if isinstance(node, ast.FunctionDef) and node.name == "exercise"]
    require(len(methods) == 1, "NativeSmoke.exercise 入口必须唯一")
    method = methods[0]
    calls = sorted((node for node in ast.walk(method) if isinstance(node, ast.Call)
                    and isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name)
                    and node.func.value.id == "self" and node.func.attr.startswith("exercise_")), key=lambda node: node.lineno)
    counts = Counter(node.func.attr for node in calls)
    expected = {"exercise_zip_import", "exercise_memory_management", "exercise_scope_controls",
                "exercise_background_selection", "exercise_workspace_usability"}
    require(expected <= counts.keys(), f"原生入口缺少关键流程：{sorted(expected - counts.keys())}")
    require(all(value == 1 for value in counts.values()), f"原生入口重复调用流程：{dict(counts)}")
    require(calls[-1].func.attr == "exercise_workspace_usability", "大数据密度验收必须是最后一个 exercise_* 流程")
    # 必须是入口的无条件直接语句，避免藏在条件/循环/内嵌函数里却看似唯一。
    direct = {id(node.value) for node in method.body if isinstance(node, (ast.Expr, ast.Assign, ast.AnnAssign)) and isinstance(node.value, ast.Call)}
    require(all(id(node) in direct for node in calls), "关键原生流程必须是入口的直接、无条件调用")
    return {"calls": [node.func.attr for node in calls], "driver_started": False}


class AssetReferences(HTMLParser):
    def __init__(self):
        super().__init__()
        self.references = []

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if tag in ("script", "img") and attrs.get("src"):
            self.references.append(attrs["src"])
        if tag == "link" and attrs.get("href"):
            self.references.append(attrs["href"])


def check_assets(root):
    output = command(["node", "--experimental-vm-modules", "scripts/check_browser_assets.mjs", str(root)], root)
    evidence = json.loads(output)
    parser = AssetReferences()
    parser.feed((root / "desktop/ui/index.html").read_text(encoding="utf-8"))
    for reference in parser.references:
        require("/" + reference.removeprefix("./").lstrip("/") in evidence["assets"],
                f"index.html 引用的资源不在真实浏览器白名单：{reference}")
    evidence["html_references"] = parser.references
    return evidence


def check_javascript(root):
    files = sorted(path for directory in ("desktop/ui", "desktop/tests", "extension", "scripts")
                   for path in (root / directory).rglob("*")
                   if path.suffix in (".js", ".mjs", ".cjs") and "node_modules" not in path.parts)
    require(files, "没有找到 JavaScript 文件")
    for path in files:
        command(["node", "--check", str(path)], root)
    return {"files_checked": len(files)}


def check_driver_contracts(root):
    return {"output": command([sys.executable, "-m", "unittest", "discover", "-s", "desktop/tests", "-p", "test_native_*.py", "-v"], root, include_stderr=True)}


def check_preflight_contracts(root, actionlint):
    executable = shutil.which(actionlint) or ""
    return {"output": command([sys.executable, "-m", "unittest", "discover", "-s", "scripts/tests", "-p", "test_preflight.py", "-v"],
                              root, include_stderr=True, environment_overrides={"RECALLCARD_PREFLIGHT_ACTIONLINT": executable})}


def check_local_js(root):
    files = sorted(str(path.relative_to(root)) for folder in ("extension/tests", "desktop/tests") for path in (root / folder).glob("*.test.js"))
    require(files, "没有找到本地 JavaScript 合同")
    return {"output": command(["node", "--test", "--test-isolation=none", "--test-concurrency=1", *files], root)}


def check_local_dom(root):
    # 只执行审阅过的 Node 命令，不让 npm lifecycle hooks 越过离线门禁。
    directory = root / "desktop/tests/local-dom"
    script = read_json(directory / "package.json")["scripts"]["test"]
    expected = "node --experimental-vm-modules --test --test-isolation=none --test-concurrency=1 *.test.mjs"
    require(script == expected, "local-dom 测试命令已变化；须审阅执行边界并同步预检")
    files = sorted(path.name for path in directory.glob("*.test.mjs"))
    require(files, "没有找到真实模块 DOM 合同")
    return {"output": command(["node", "--experimental-vm-modules", "--test", "--test-isolation=none", "--test-concurrency=1", *files], directory),
            "scope": "真实应用模块的 DOM/事件合同；不验证 CSS、几何、真实点击、模态焦点或原生 IPC"}


def check_environment(root):
    names = ("CI", "GITHUB_ACTIONS", "GITHUB_WORKSPACE", "RUNNER_TEMP", "RECALLCARD_STATE_DIR", "RECALLCARD_CHROMIUM_PATH")
    present = {name: bool(os.environ.get(name)) for name in names}
    if os.environ.get("GITHUB_ACTIONS") == "true":
        require(present["RUNNER_TEMP"], "GitHub Actions 环境缺少 RUNNER_TEMP")
        require(Path(os.environ["RUNNER_TEMP"]).is_dir(), "RUNNER_TEMP 不是可用目录")
        require(present["GITHUB_WORKSPACE"] and Path(os.environ["GITHUB_WORKSPACE"]).resolve() == root,
                "GITHUB_WORKSPACE 与预检仓库不一致")
    return {"presence_only": present, "values_logged": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--actionlint", default="actionlint", help="已安装的官方 actionlint 路径")
    parser.add_argument("--report", type=Path, help="额外保存同一份 JSON；不会创建父目录")
    parser.add_argument("--checks", help="仅诊断指定逗号分隔检查；报告标明部分检查，不能当作完整门禁")
    arguments = parser.parse_args(argv)
    root = arguments.root.resolve()
    checks = {"environment": check_environment, "node_and_lockfiles": check_node,
              "actionlint": lambda path: check_actionlint(path, arguments.actionlint),
              "workflow_sources": check_workflow_sources, "native_order": check_native_order,
              "browser_assets": check_assets, "javascript_syntax": check_javascript,
              "driver_contracts": check_driver_contracts, "local_js": check_local_js, "local_dom": check_local_dom,
              "preflight_contracts": lambda path: check_preflight_contracts(path, arguments.actionlint)}
    selected = arguments.checks.split(",") if arguments.checks else list(checks)
    require(set(selected) <= checks.keys() and selected, "未知或空的检查名称")
    results = []
    for name in selected:
        started = time.monotonic()
        try:
            detail = checks[name](root)
            status = "passed"
        except Exception as error:  # 单项失败仍运行独立检查，最终始终非零退出。
            detail, status = {"error": sanitized(str(error))}, "failed"
        results.append({"name": name, "status": status, "seconds": round(time.monotonic() - started, 3), "detail": detail})
        print(f"{'通过' if status == 'passed' else '失败'}：{name}", file=sys.stderr, flush=True)
    failures = [item["name"] for item in results if item["status"] == "failed"]
    full = set(selected) == checks.keys()
    report = {"schema_version": 1, "created_at": datetime.now(timezone.utc).isoformat(),
              "status": "failed" if failures else "passed" if full else "partial",
              "local_gate_passed": not failures and full, "full_acceptance_passed": False,
              "checks": results, "not_run": [
                  {"name": "chromium", "reason": "不启动浏览器/socket；真实布局、toast遮挡、鼠标命中、CSS与模态焦点待独立浏览器验收"},
                  {"name": "native", "reason": "不启动 WebKit/Tauri/文件选择/剪贴板；真实原生闭环待独立验收"},
                  {"name": "rust_and_packaging", "reason": "本门禁不编译、不运行 Rust 测试、不制作安装包"}],
              "not_selected": sorted(checks.keys() - set(selected))}
    encoded = json.dumps(sanitized_data(report), ensure_ascii=False, indent=2) + "\n"
    if arguments.report:
        arguments.report.write_text(encoded, encoding="utf-8")
    sys.stdout.write(encoded)
    print(f"本地预检：{len(results) - len(failures)} 项通过，{len(failures)} 项失败；"
          + ("完整门禁" if full else "仅部分检查") + "。Chromium、原生与 Rust/打包未运行。", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (PreflightError, OSError) as error:
        print(json.dumps({"status": "failed", "error": sanitized(str(error))}, ensure_ascii=False))
        raise SystemExit(1)
