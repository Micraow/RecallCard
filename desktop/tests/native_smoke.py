#!/usr/bin/env python3
"""真实 Tauri/WebKitGTK 冒烟回归：仅使用临时合成数据，不替换 invoke。

在已安装官方 WebKitWebDriver、xdotool、xclip、scrot、openbox、
python3-pyatspi 的 Linux 上运行：
dbus-run-session -- xvfb-run -a /usr/bin/python3 desktop/tests/native_smoke.py
应用按钮使用 W3C WebDriver；系统文件选择窗口使用正常 X11 键盘和 AT-SPI。
"""

import argparse
import base64
from datetime import datetime, timezone
import json
from http.client import RemoteDisconnected
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import traceback
import zipfile
from urllib.error import HTTPError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[2]
ELEMENT = "element-6066-11e4-a52e-4f735466cecf"
UI_CONTRACTS = json.loads((Path(__file__).with_name("native-ui-contracts.json")).read_text())
EVENT_TEXT = "合成资料：native-smoke 水星项目的说明使用简洁中文。"
NOTE_TEXT = "合成首条记录：周末整理书单。"
MEMORY_TEXT = "合成记忆：native-smoke 琥珀计划先核对证据，再用简洁中文说明。"
ZIP_TITLE = "合成 ZIP 琥珀计划"
ZIP_USER_TEXT = "合成用户原话：native-smoke 琥珀计划先核对证据，再用简洁中文说明。"
ZIP_ASSISTANT_TEXT = "合成 AI 回复：建议每周复盘，尚未得到用户确认。"
ZIP_OTHER_TEXT = "合成未选择会话：native-smoke 紫晶计划暂不导入。"
ZIP_HIDDEN_TEXT = "合成隐藏推理占位：此内容绝不能进入资料库或任务。"
ZIP_TIMESTAMP = 1791241200
EDITED_MEMORY_TEXT = "合成编辑记忆：native-smoke 琥珀计划先核对证据，再用简洁中文说明。已手工补充分类标签。"
WORK_NOTE_TEXT = "合成工作范围：native-smoke 范围隔离验收资料。"
DEEPSEEK_DIALOG = "选择导出文件（可多选）"
DEEPSEEK_TITLE = "合成 DeepSeek 分支与独立片段"
DEEPSEEK_TEXTS = {
    "q": "合成 DeepSeek 用户原话：先核对石榴项目证据。",
    "a": "合成 DeepSeek 回答甲：先检查附件清单。",
    "b": "合成 DeepSeek 回答乙：先检查引用，尚未得到用户确认。",
    "separate": "合成 DeepSeek 独立根：这段不属于石榴问答。",
    "second-q": "合成 DeepSeek 第二份用户原话：整理海棠项目。",
    "second-a": "合成 DeepSeek 第二份回答：保留原始出处。",
}
DEEPSEEK_HIDDEN = "合成 DeepSeek THINK：隐藏片段绝不能写入或复制。"
DEEPSEEK_ATTACHMENT = "合成 DeepSeek 附件原件内容绝不能写入或复制。"
DEEPSEEK_ATTACHMENT_URL = "https://example.invalid/synthetic-private-attachment"


class DriverError(RuntimeError):
    pass


def recent_workspace_path(temporary):
    # 宿主 recent_workspace_file 使用 app_data_dir，而非 app_config_dir。
    identifier = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
    directory = temporary / "data"
    path = directory / identifier / "recent-workspace.json"
    assert path.resolve().is_relative_to(directory.resolve())
    return path


def wait_for(check, description, timeout=20):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (DriverError, OSError) as error:
            last_error = error
        time.sleep(0.15)
    raise AssertionError(f"等待超时：{description}；最近错误：{last_error}")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=30).stdout


class WebDriver:
    def __init__(self, application):
        self.session = None
        self.address = "http://127.0.0.1:4444"
        result = self.request("POST", "/session", {
            "capabilities": {"alwaysMatch": {
                "browserName": "wry",
                "webkitgtk:browserOptions": {"binary": str(application), "args": []},
            }},
        }, timeout=60)
        self.session = result["sessionId"]

    def request(self, method, path, data=None, timeout=15):
        body = None if data is None else json.dumps(data).encode()
        request = Request(self.address + path, data=body, method=method,
                          headers={"Content-Type": "application/json"})
        # 直接连接官方 WebKitWebDriver；仅观察/定位和幂等滚动允许有界传输恢复。
        # 点击、输入、创建会话均不自动重发，避免重复写入。
        safe = method == "GET" or path.endswith("/element") or (path.endswith("/execute/sync") and data and (data.get("script", "").startswith("return ") or data.get("script", "").startswith("arguments[0].scrollIntoView")))
        for attempt in range(3 if safe else 1):
            try:
                with urlopen(request, timeout=timeout) as response:
                    result = json.load(response)
                break
            except HTTPError as error:
                raise DriverError(error.read().decode()) from error
            except (RemoteDisconnected, ConnectionResetError, ConnectionAbortedError):
                if not safe or attempt == 2:
                    raise
                time.sleep(0.15 * (attempt + 1))
        # W3C WebDriver 协议错误由 HTTP 4xx/5xx 表达，已在 HTTPError 分支拒绝。
        # execute/sync 成功时的 value 是页面自己的 JSON，允许含 error:null 等字段。
        # 不把页面数据的字段名当作驱动失败，真实应用错误仍由调用方逐项断言。
        return result.get("value")

    def command(self, method, path, data=None):
        return self.request(method, f"/session/{self.session}{path}", data)

    def find(self, selector, using="css selector"):
        return self.command("POST", "/element", {"using": using, "value": selector})[ELEMENT]

    def text(self, selector):
        return self.command("GET", f"/element/{self.find(selector)}/text")

    def assert_vault_badge(self, expected):
        # WebDriver 的 rendered text 可能受单行省略布局影响；同时留下原值，
        # 以实际 DOM 文字和屏幕内可见矩形核对异步恢复后的资料库标识。
        latest = {}
        def observed():
            sample = self.observe("return (()=>{const n=document.querySelector('#vault-badge');if(!n)return {text:null,visible:false};const r=n.getBoundingClientRect();const s=getComputedStyle(n);return {text:n.textContent,visible:r.width>0&&r.height>0&&r.top>=0&&r.bottom<=innerHeight&&s.display!=='none'&&s.visibility!=='hidden'};})()")
            latest.update(sample)
            return sample['text'] == expected and sample['visible']
        try:
            wait_for(observed, "可见资料库标识恢复")
        except AssertionError as error:
            raise AssertionError(f"资料库标识不符：expected={expected!r}, observed={latest!r}") from error
        record = {"check": "vault_badge", "expected": expected, "dom": latest,
                  "webdriver_rendered_text": self.text('#vault-badge')}
        if not hasattr(self, 'observations'):
            self.observations = []
        self.observations.append(record)
        print("标识核验：" + json.dumps(record, ensure_ascii=False), flush=True)

    def click(self, selector, using="css selector"):
        element = wait_for(lambda: self.find(selector, using), f"找到按钮 {selector}")
        # 只把目标滚动到窗口中央；仍由真实 WebDriver 派发点击，不调用业务后端。
        self.command("POST", "/execute/sync", {"script": "arguments[0].scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});", "args": [{ELEMENT: element}]})
        self.command("POST", f"/element/{element}/click", {})

    def button(self, label, container=""):
        if label == "选择文件并预览" and not self.observe("return document.querySelector('#file-import-details').open"):
            self.click("#file-import-details > summary")
        if label in {"选择结果并审阅", "导出本次来源包"} and not self.observe("return document.querySelector('#dream-file-options').open"):
            self.click("#dream-file-options > summary")
        if label.startswith(("查看出处 ", "查看背景出处 ")):
            if self.observe("return Boolean(document.querySelector('details.source-details:not([open])'))"):
                self.click("details.source-details:not([open]) > summary")
        if label == "选择这条资料" or label.startswith("前往整理（"):
            if self.observe("return Boolean(document.querySelector('details.organize-details:not([open])'))"):
                self.click("details.organize-details:not([open]) > summary")
        if label in {"复制当前随身背景", "带上背景继续会话"}:
            if self.observe("return Boolean(document.querySelector('#background-current:not([open])'))"):
                self.click("#background-current > summary")
        # 测试中的中文标签没有引号；由浏览器正常派发点击，不调用业务方法。
        try:
            self.click(f"{container}//button[normalize-space(.)='{label}']", "xpath")
        except (RemoteDisconnected, ConnectionResetError, ConnectionAbortedError):
            titles = {"选择导出文件": DEEPSEEK_DIALOG, "选择文件并预览":"选择要导入的对话文件", "打开已有资料库":"打开已有 RecallCard 资料库", "创建新资料库":"选择用于新资料库的空文件夹", "导出本次来源包":"保存整理包", "选择结果并审阅":"选择整理结果文件"}
            title = titles.get(label)
            if not title:
                raise
            # 不重新点击；只核对不确定请求是否已经产生预期的真实原生窗口。
            def appeared():
                result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", f"^{re.escape(title)}$"], capture_output=True, text=True, timeout=3)
                return result.returncode == 0 and bool(result.stdout.strip())
            if not wait_for(appeared, "断连后核对原生窗口", timeout=3):
                raise
            print(f"点击回执断开，但已观察到原生窗口：{title}；不重发点击", flush=True)

    def type(self, selector, value):
        # 使用真实聚焦、全选和粘贴。WebDriver clear 会提前派发 change，
        # 使受控输入框重建；Send Keys 还可能把换行当控制键吞掉。
        # 不复用旧元素引用，不给 DOM 赋值，也不调用应用业务方法。
        subprocess.run(["xclip", "-selection", "clipboard", "-i"], input=value,
                       text=True, check=True, timeout=5,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.click(selector)
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+a", "ctrl+v"], check=True, timeout=5)
        expected = value.replace("\r\n", "\n").replace("\r", "\n")
        wait_for(lambda: self.observe(f"return document.querySelector({json.dumps(selector)}).value") == expected,
                 "输入框完整保留预期文字与换行", timeout=5)

    def select(self, selector, value):
        # 使用 WebDriver 点击真实 option，让应用收到正常 change 事件。
        self.click(f'{selector} option[value="{value}"]')
        self.idle()
        assert self.observe(f"return document.querySelector({json.dumps(selector)}).value") == value

    def select_keyboard(self, selector, value):
        # 观察实际选项顺序后，打开可见原生选择控件并用键盘选择。
        # 不给 value/selectedIndex 赋值，不点击不可见的 option，也不调用 change。
        options = self.observe(f"return [...document.querySelector({json.dumps(selector)}).options].map(n=>({{value:n.value,disabled:n.disabled}}))")
        choices = [index for index, option in enumerate(options) if option["value"] == value]
        assert len(choices) == 1 and not any(option["disabled"] for option in options), "分支选项缺失、重复或不可用"
        self.click(selector)
        run("xdotool", "key", "--clearmodifiers", "Home", *(["Down"] * choices[0]), "Return")
        wait_for(lambda: self.observe(f"return document.querySelector({json.dumps(selector)}).value") == value,
                 "键盘选择确切分支", timeout=5)
        self.idle()

    def blur(self, selector):
        element = self.find(selector)
        self.command("POST", f"/element/{element}/value", {"text": "\ue004"})
        self.idle()

    def observe(self, script):
        # 仅观察 DOM 状态和错误；不注入后端、设置会话或绕过文件选择窗口。
        return self.command("POST", "/execute/sync", {"script": script, "args": []})

    def idle(self):
        wait_for(lambda: self.text("#operation") == "准备就绪", "本机操作结束")
        error = self.observe("return document.querySelector('#notice.error:not([hidden])')?.textContent")
        if error:
            raise AssertionError(f"应用显示错误：{error}")

    def navigate(self, label):
        # 通过当前产品真正可见的入口进入工作流，不注入路由或调用业务方法。
        if label in {"概览", "会话与接续"}:
            self.click('#navigation [data-workspace="conversations"]')
        elif label == "记忆管理":
            self.click('#navigation [data-workspace="memories"]')
            self.idle()
            self.click('[data-action="memory-all"]')
        elif label == "添加资料":
            self.click('#import-button')
        elif label == "连接与状态":
            self.click('#connect-button')
        elif label == "查找与阅读":
            self.button("查找")
        elif label == "随身背景":
            self.click('#navigation [data-workspace="memories"]')
            self.idle()
            self.click('[data-action="background-select"]')
        elif label == "整理记忆":
            self.click('#navigation [data-workspace="memories"]')
            self.idle()
            self.button("整理记忆")
        else:
            self.button(label)
        self.idle()

    def select_background_scope(self, scope):
        # 切范围清空旧选择并回到全部记忆；通过可见页签重新打开新范围背景。
        contract = UI_CONTRACTS["scope_background"]
        self.select(contract["scope_selector"], scope)
        assert self.observe(f"return document.querySelector({json.dumps(contract['reset_tab'])}).getAttribute('aria-pressed')") == "true"
        self.click(contract["background_tab"])
        self.idle()

    def open_continuation(self):
        self.click('[data-action="open-continuation"]')
        self.idle()
        assert self.observe("return Boolean(document.querySelector('#continuation-panel'))")

    def choose_context_references(self, references):
        assert 1 <= len(references) <= 8 and len(set(references)) == len(references)
        assert all(re.fullmatch(r"event:[A-Za-z0-9_-]+", reference) for reference in references)
        choices = self.observe("return [...document.querySelectorAll('.results .context-check input')].map(n=>({ref:n.dataset.selectionReference,checked:n.checked,disabled:n.disabled}))")
        assert len({row["ref"] for row in choices}) == len(choices)
        assert set(references) <= {row["ref"] for row in choices}, "目标原话必须存在于当前实际搜索结果"
        assert not any(row["disabled"] for row in choices), "选择框必须实际可用"
        # 先移除不需要的勾选，再添加目标，避免暂时触及选择上限。只操作公开控件。
        changes = [row for row in choices if row["checked"] and row["ref"] not in references]
        changes += [row for row in choices if not row["checked"] and row["ref"] in references]
        for row in changes:
            self.click(f'.context-check input[data-selection-reference="{row["ref"]}"]')
            self.idle()
        selected = self.observe("return [...document.querySelectorAll('.results .context-check input:checked')].map(n=>n.dataset.selectionReference)")
        assert selected == [row["ref"] for row in choices if row["ref"] in references]
        return selected

    def close(self):
        if self.session:
            self.command("DELETE", "")


class NativeSmoke:
    def __init__(self, args, temporary):
        self.artifacts = args.artifacts.resolve()
        self.artifacts.mkdir(parents=True, exist_ok=True)
        self.temporary = temporary
        self.vault = temporary / "native-smoke-vault"
        self.cli = args.cli.resolve()
        self.driver = None
        self.processes = []
        self.logs = []
        self.steps = []
        self.dialog_count = 0
        self.application = args.application.resolve()

    def checkpoint(self, name):
        print(f"通过：{name}", flush=True)
        self.steps.append(name)
        self.capture(f"{len(self.steps):02d}-{name}")

    def reopen_application(self):
        previous = self.driver
        observations = list(getattr(previous, "observations", []))
        previous.close()
        self.driver = None

        def ready():
            with urlopen("http://127.0.0.1:4444/status", timeout=2) as response:
                return json.load(response).get("value", {}).get("ready") is True

        wait_for(ready, "应用关闭后原生驱动可建立新的窗口")
        # 明确关闭后的新启动只建立一次会话；不因创建回执不确定而重发。
        self.driver = WebDriver(self.application)
        self.driver.observations = observations
        wait_for(lambda: bool(self.driver.text("h1")), "重新启动后真实窗口显示页面")
        self.driver.idle()
        return self.driver

    def capture(self, name, webview=True):
        # scrot 同时记录原生文件选择窗口；WebDriver 截图记录真实 WebKit 页面。
        for label, action in [
            ("desktop", lambda: run("scrot", str(self.artifacts / f"{name}-desktop.png"))),
            ("webview", lambda: (self.artifacts / f"{name}-webview.png").write_bytes(
                base64.b64decode(self.driver.command("GET", "/screenshot")))),
        ]:
            if label == "webview" and (not self.driver or not webview):
                continue
            try:
                action()
            except Exception as error:
                print(f"截图未完成 ({label})：{error}", flush=True)

    def start(self, executable, logfile, extra_env=None):
        stream = (self.artifacts / logfile).open("w")
        self.logs.append(stream)
        process = subprocess.Popen(executable, stdout=stream, stderr=subprocess.STDOUT,
                                   start_new_session=True, env={**os.environ, **(extra_env or {})})
        self.processes.append(process)
        return process

    def start_native_driver(self):
        # 与官方 tauri-driver 的 Linux 能力映射和自动化环境一致，直接使用其底层驱动。
        return self.start(["WebKitWebDriver", "--host=127.0.0.1", "--port=4444"],
                          "webkit-webdriver.log", {"TAURI_AUTOMATION": "true", "TAURI_WEBVIEW_AUTOMATION": "true"})

    def cli_command(self, *args):
        return json.loads(run(str(self.cli), "--vault", str(self.vault), *args))

    def events(self):
        return [json.loads(line) for path in sorted((self.vault / "events").rglob("*.jsonl"))
                for line in path.read_text().splitlines() if line.strip()]

    def memories(self):
        return list((self.vault / "memories").glob("*.md"))

    def dialog_windows(self, title):
        result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", f"^{re.escape(title)}$"],
                                capture_output=True, text=True, timeout=5)
        return result.stdout.split() if result.returncode == 0 else []

    def accessible_dialog(self, title):
        import pyatspi
        for application in pyatspi.Registry.getDesktop(0):
            for window in application:
                if window.name == title:
                    return window
        return None

    @staticmethod
    def accessible_nodes(root):
        if root is None:
            return
        pending = [(root, 0)]
        while pending:
            node, depth = pending.pop(0)
            yield node, depth
            if depth < 16:
                pending.extend((child, depth + 1) for child in node if child is not None)

    def native_button(self, title, name):
        import pyatspi
        for node, _ in self.accessible_nodes(self.accessible_dialog(title)):
            if node.getRole() == pyatspi.ROLE_PUSH_BUTTON and node.name.replace("_", "") == name:
                state = node.getState()
                if state.contains(pyatspi.STATE_SENSITIVE) and state.contains(pyatspi.STATE_SHOWING):
                    return node
        return None

    def describe_dialog(self, title, phase=None):
        rows = []
        for node, depth in self.accessible_nodes(self.accessible_dialog(title)):
            rows.append(f"{'  ' * depth}{node.getRoleName()}: {node.name} [{node.getState().getStates()}]")
        suffix = f"-{phase}" if phase else ""
        (self.artifacts / f"dialog-{self.dialog_count:02d}{suffix}-accessibility.txt").write_text("\n".join(rows))

    def native_location_entry(self, title):
        import pyatspi
        entries = []
        for node, _ in self.accessible_nodes(self.accessible_dialog(title)):
            if node.getRole() != pyatspi.ROLE_TEXT:
                continue
            attributes = dict(item.split(":", 1) for item in node.getAttributes() if ":" in item)
            if attributes.get("placeholder-text") != "Location":
                continue
            state = node.getState()
            if all(state.contains(value) for value in [pyatspi.STATE_SHOWING, pyatspi.STATE_SENSITIVE,
                                                       pyatspi.STATE_FOCUSED, pyatspi.STATE_EDITABLE]):
                entries.append(node)
        assert len(entries) <= 1, "原生位置输入不唯一，不能确定键盘目标"
        return entries[0] if entries else None

    def navigate_file_folder(self, title, window, folder):
        # 只经真实焦点与粘贴输入目录。逐字键入可能被 GTK 补全改写；
        # Return 前必须逐字读回，不能猜测输入成功或重发确认。
        expected = str(folder) + "/"
        self.capture(f"dialog-{self.dialog_count:02d}-opened", webview=False)
        self.describe_dialog(title, "opened")
        # Ctrl+L 是切换键；已有聚焦输入时再次按会隐藏它。
        if self.native_location_entry(title) is None:
            run("xdotool", "key", "--clearmodifiers", "ctrl+l")
        wait_for(lambda: self.native_location_entry(title), "原生位置输入可见并已获得焦点")
        subprocess.run(["xclip", "-selection", "clipboard", "-i"], input=expected,
                       text=True, check=True, timeout=5,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        run("xdotool", "key", "--clearmodifiers", "ctrl+a", "ctrl+v")

        def exact_location():
            entry = self.native_location_entry(title)
            return entry and entry.queryText().getText(0, -1) == expected

        wait_for(exact_location, "原生位置输入逐字保留完整合成目录")
        self.capture(f"dialog-{self.dialog_count:02d}-location", webview=False)
        self.describe_dialog(title, "location")
        assert run("xdotool", "getactivewindow").strip() == window, "位置输入期间多选窗口失去焦点"
        assert exact_location(), "目录输入或焦点已改变，停止确认"
        (self.artifacts / f"dialog-{self.dialog_count:02d}-location.json").write_text(json.dumps({"expected": expected, "observed": expected}, ensure_ascii=False))
        run("xdotool", "key", "--clearmodifiers", "Return")

    def native_file_cells(self, title, names, selected=False):
        import pyatspi
        cells = {}
        for node, _ in self.accessible_nodes(self.accessible_dialog(title)):
            if node.getRole() != pyatspi.ROLE_TABLE_CELL or node.name not in names:
                continue
            state = node.getState()
            if not state.contains(pyatspi.STATE_SHOWING) or not state.contains(pyatspi.STATE_SENSITIVE):
                continue
            if selected and not state.contains(pyatspi.STATE_SELECTED):
                continue
            assert node.name not in cells, "原生文件列表出现同名可见行，不能确定选择目标"
            cells[node.name] = node
        return cells

    def dialog_files(self, paths):
        # 独立合成目录只放这两份文件，真实文件列表 Ctrl+A 不会包含其他资料。
        paths = [Path(path).resolve() for path in paths]
        assert len(paths) >= 2 and len(set(paths)) == len(paths), "多选验收需要不同文件"
        folder = paths[0].parent
        assert all(path.is_file() and path.parent == folder for path in paths), "合成文件必须在同一目录"
        assert set(folder.iterdir()) == set(paths), "多选目录含未批准的其他文件"
        names = {path.name for path in paths}
        title = DEEPSEEK_DIALOG
        self.dialog_count += 1
        window = wait_for(lambda: self.dialog_windows(title), f"原生窗口：{title}")[0]
        run("xdotool", "windowactivate", "--sync", window)
        wait_for(lambda: self.accessible_dialog(title), "多选窗口辅助功能就绪")
        try:
            self.navigate_file_folder(title, window, folder)

            def visible_files():
                assert self.dialog_windows(title), "选择器在核对文件前已经关闭；不重新打开或确认"
                found = self.native_file_cells(title, names)
                return found if set(found) == names else None

            cells = wait_for(visible_files, "两份合成文件出现在可见原生列表")
            self.capture(f"dialog-{self.dialog_count:02d}-visible-files", webview=False)
            self.describe_dialog(title, "visible-files")
            import pyatspi
            cell = cells[paths[0].name]
            bounds = cell.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            assert bounds.width > 0 and bounds.height > 0, "原生文件行没有可点击区域"
            geometry = dict(line.split("=", 1) for line in run("xdotool", "getwindowgeometry", "--shell", window).splitlines() if "=" in line)
            x, y = bounds.x + bounds.width // 2, bounds.y + bounds.height // 2
            assert int(geometry["X"]) <= x < int(geometry["X"]) + int(geometry["WIDTH"])
            assert int(geometry["Y"]) <= y < int(geometry["Y"]) + int(geometry["HEIGHT"])
            assert run("xdotool", "getactivewindow").strip() == window, "多选窗口失去焦点，停止键盘操作"
            run("xdotool", "mousemove", str(x), str(y))
            run("xdotool", "click", "1")
            run("xdotool", "key", "--clearmodifiers", "ctrl+a")
            wait_for(lambda: set(self.native_file_cells(title, names, selected=True)) == names,
                     "原生列表实际选中两份文件")
            self.capture(f"dialog-{self.dialog_count:02d}-two-files-selected", webview=False)
            approval = wait_for(lambda: self.native_button(title, "Open"), "多文件原生确认按钮可用")
            assert approval.queryAction().doAction(0), "多文件原生确认按钮未接受点击"
            wait_for(lambda: not self.dialog_windows(title), "多文件原生窗口关闭")
        finally:
            if self.dialog_windows(title):
                self.describe_dialog(title)
        self.driver.idle()

    def wait_import_job(self, count):
        # run() 返回后 operation 已经准备就绪，后台任务仍可能正在写入。
        # 只读真实 DOM 完成标题、进度与错误；暂停、失败或读取错误不能冒充成功。
        def completed():
            status = self.driver.observe("return (()=>{const n=document.querySelector('.import-job');const p=n?.querySelector('progress');return {heading:n?.querySelector('h3')?.textContent,processed:p?.value,total:p?.max,error:document.querySelector('#notice.error:not([hidden])')?.textContent||n?.querySelector('.hint.warning')?.textContent,modal:document.querySelector('#modal').open};})()")
            assert not status.get("modal"), "批量导入不得要求第二次确认"
            assert not status.get("error"), f"导入任务显示错误：{status.get('error')}"
            heading = status.get("heading")
            assert heading not in {"已暂停", "正在暂停", "可以继续上次导入", "导入尚未完成"}, f"导入未完成：{heading}"
            if heading != "导入完成":
                return False
            assert status.get("processed") == count and status.get("total") == count, f"完成数量不符：{status}"
            return status
        return wait_for(completed, "后台导入实际完成", timeout=60)

    def confirm_import_job(self, count):
        self.driver.button(f"导入全部 {count} 条消息")
        # 不因 idle、超时或断连重新点击批准；等待只有观察操作。
        self.driver.idle()
        return self.wait_import_job(count)

    def exercise_default_workspace(self):
        # 必须在启动驱动前已隔离的测试目录中验收默认保存位置。
        # 任一环境变量缺失、指向用户目录或经符号链接越界时，连“开始使用”也不能点击。
        temporary = self.temporary.resolve()
        isolated = {}
        for name, suffix in [("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("RECALLCARD_STATE_DIR", "state")]:
            value = os.environ.get(name)
            assert value, f"默认起步验收缺少临时目录隔离：{name}"
            path = Path(value).resolve()
            assert path == (temporary / suffix).resolve() and path.is_relative_to(temporary), f"默认起步验收目录未隔离：{name}"
            isolated[name] = path
        identifier = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
        assert re.fullmatch(r"[A-Za-z0-9._-]+", identifier), "应用标识不是安全的目录名称"
        default_vault = isolated["XDG_DATA_HOME"] / identifier / "vault"
        assert default_vault.resolve().is_relative_to(isolated["XDG_DATA_HOME"])
        assert not default_vault.exists(), "首次起步验收必须使用尚未创建的临时默认资料库"
        browser = self.driver
        browser.button("开始使用")

        def opened():
            for title in ("打开已有 RecallCard 资料库", "选择用于新资料库的空文件夹", "创建资料库", DEEPSEEK_DIALOG):
                assert not self.dialog_windows(title), f"默认起步不应打开原生选择或确认窗口：{title}"
            status = browser.observe("return (()=>{const n=document.querySelector('.import-job');const r=n?.getBoundingClientRect();return {ready:document.querySelector('#operation').textContent==='准备就绪',page:document.querySelector('#location').textContent,importVisible:Boolean(r&&r.width>0&&r.height>0&&getComputedStyle(n).visibility!=='hidden'),modal:document.querySelector('#modal').open,error:document.querySelector('#notice.error:not([hidden])')?.textContent};})()")
            assert not status.get("modal"), "默认起步不应要求第二次确认"
            assert not status.get("error"), f"默认起步显示错误：{status.get('error')}"
            return status if status.get("ready") and status.get("page") == "导入会话" and status.get("importVisible") else False

        screen = wait_for(opened, "默认资料库创建后直接进入导入页")
        marker = json.loads((default_vault / "control/schema-version.json").read_text())
        assert marker == {"schema_version": 1, "application": "RecallCard"}, "默认资料库缺少有效格式标记"
        assert (default_vault / "events").is_dir() and not list((default_vault / "events").rglob("*.jsonl"))
        assert (default_vault / "memories").is_dir() and not list((default_vault / "memories").glob("*.md"))
        browser.assert_vault_badge(default_vault.name)
        self.checkpoint("开始使用直接初始化临时默认资料库并进入导入")
        browser.click("#switch-vault")
        assert browser.observe("return document.querySelector('#modal').open")
        assert browser.text("#modal-title") == "切换资料库"
        return {"relative_path": str(default_vault.relative_to(temporary)), "schema": marker,
                "event_count": 0, "memory_count": 0, "screen": screen}

    def dialog(self, title, path=None, save=False, create=False):
        self.dialog_count += 1
        window = wait_for(lambda: self.dialog_windows(title), f"原生窗口：{title}")[0]
        run("xdotool", "windowactivate", "--sync", window)
        wait_for(lambda: self.accessible_dialog(title), f"原生窗口辅助功能就绪：{title}")
        time.sleep(0.4)
        self.capture(f"dialog-{self.dialog_count:02d}-{title}", webview=False)
        if path is None:
            cancel = wait_for(lambda: self.native_button(title, "Cancel"), "原生取消按钮可用")
            assert cancel.queryAction().doAction(0), "原生取消按钮未接受点击"
        else:
            # 只通过文件选择窗口向应用授予本次合成路径访问权。
            # GTK 的 Recent 视图不是文件目录；先切到 Home，等路径栏完成显示。
            run("xdotool", "key", "--clearmodifiers", "alt+Home")
            time.sleep(0.4)
            run("xdotool", "key", "--clearmodifiers", "ctrl+l")
            time.sleep(0.3)
            run("xdotool", "key", "--clearmodifiers", "ctrl+a")
            run("xdotool", "type", "--clearmodifiers", "--delay", "8", str(path))
            # 给 GTK 文件补全和异步路径校验留下时间，不在未实现控件上连发回车。
            time.sleep(0.7)
            run("xdotool", "key", "--clearmodifiers", "Return")
            time.sleep(0.5)
            try:
                button = wait_for(lambda: True if not self.dialog_windows(title)
                                  else self.native_button(title, "Save" if save else "Open"),
                                  "原生文件选择完成或确认按钮可用")
                if button is not True:
                    assert button.queryAction().doAction(0), "原生确认按钮未接受点击"
            finally:
                if self.dialog_windows(title):
                    self.describe_dialog(title)
        wait_for(lambda: not self.dialog_windows(title), f"关闭原生窗口：{title}")
        if create:
            # rfd 的 GtkMessageDialog 在部分系统未以窗口标题暴露 AT-SPI 对象。
            # 已核对固定版本截图和 rfd 按钮顺序：创建在左、取消在右。
            # 对已识别的原生确认窗执行正常鼠标点击，随后仍校验真实资料库文件。
            approval_window = wait_for(lambda: self.dialog_windows("创建资料库"), "创建确认窗口")[0]
            run("xdotool", "windowactivate", "--sync", approval_window)
            self.capture("create-confirmation", webview=False)
            approval = self.native_button("创建资料库", "创建资料库")
            if approval:
                assert approval.queryAction().doAction(0)
            else:
                geometry = dict(line.split("=", 1) for line in run("xdotool", "getwindowgeometry", "--shell", approval_window).splitlines() if "=" in line)
                width, height = int(geometry["WIDTH"]), int(geometry["HEIGHT"])
                assert width >= 200 and height >= 120, "确认窗尺寸异常，停止点击"
                run("xdotool", "mousemove", "--window", approval_window, str(width // 4), str(height - 18))
                run("xdotool", "click", "1")
            wait_for(lambda: not self.dialog_windows("创建资料库"), "创建确认窗口关闭")
        self.driver.idle()

    def preview_import(self, source):
        if not self.driver.observe("return Boolean(document.querySelector('#file-import-details'))"):
            self.driver.navigate("添加资料")
        self.driver.button("选择文件并预览")
        self.dialog("选择要导入的对话文件", source)
        assert EVENT_TEXT in self.driver.text(".file-preview")

    def confirm_import(self, count=1):
        self.driver.button(f"确认导入 {count} 条记录")
        self.driver.idle()
        assert not self.driver.observe("return document.querySelector('#modal').open"), "普通导入不得要求第二次确认"

    def create_zip_fixture(self):
        def conversation(identifier, title, user_text, with_hidden=False):
            messages = [("user", "user", user_text, {})]
            if with_hidden:
                messages.append(("hidden", "assistant", ZIP_HIDDEN_TEXT, {"channel": "analysis"}))
            messages.append(("assistant", "assistant", ZIP_ASSISTANT_TEXT, {}))
            mapping = {"root": {"parent": None, "message": None}}
            parent = "root"
            for index, (suffix, role, content, extra) in enumerate(messages):
                node = f"{identifier}-{suffix}"
                mapping[node] = {"parent": parent, "message": {
                    "id": node, "author": {"role": role},
                    "create_time": ZIP_TIMESTAMP + index,
                    "content": {"content_type": "text", "parts": [content]}, **extra,
                }}
                parent = node
            return {"id": identifier, "title": title, "current_node": parent, "mapping": mapping}

        archive = self.temporary / "synthetic-two-conversations.zip"
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as fixture:
            # 两个独立的单会话对象，不使用 conversations.json 数组冒充多文件备份。
            fixture.writestr("chosen.json", json.dumps(conversation(
                "native-zip-chosen", ZIP_TITLE, ZIP_USER_TEXT, True), ensure_ascii=False))
            fixture.writestr("not-selected.json", json.dumps(conversation(
                "native-zip-other", "合成 ZIP 紫晶计划", ZIP_OTHER_TEXT), ensure_ascii=False))
            fixture.writestr("chosen.md", "合成 Markdown 副本，不应重复导入。")
            fixture.writestr("attachment.txt", "合成附件占位，不作为会话正文导入。")
        return archive

    def create_workspace_fixture(self):
        """仅合成数据：覆盖常见备份密度、长标题、代码、多角色与未知时间。"""
        archive = self.temporary / "synthetic-workspace-53.zip"
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as fixture:
            for number in range(1, 54):
                identifier = f"workspace-density-{number:02d}"
                title = f"工作区验收·{number:02d} 资料整理与跨端接续"
                user = f"合成任务 {number:02d}：紫杉项目先核对来源、再继续实现。"
                if number == 53:
                    title += "：阅读长标题时仍应能够区分相邻会话与来源"
                    user += "\n" + ("这是一段用于版面验收的合成长消息，内容不包含个人数据。\n" * 100)
                assistant = f"合成回复 {number:02d}：建议先检查输入。\n```python\nprint('synthetic')\n```\n该建议尚未确认。"
                nodes = {"root": {"parent": None, "message": None}}
                parent = "root"
                for index, (role, body) in enumerate([("user", user), ("assistant", assistant)]):
                    node = f"{identifier}-{role}"
                    message = {"id": node, "author": {"role": role},
                               "content": {"content_type": "text", "parts": [body]}}
                    if number % 4:
                        message["create_time"] = ZIP_TIMESTAMP + number * 60 + index
                    nodes[node] = {"parent": parent, "message": message}
                    parent = node
                fixture.writestr(f"{identifier}.json", json.dumps({"id": identifier,
                    "title": title, "current_node": parent, "mapping": nodes}, ensure_ascii=False))
        return archive

    def create_deepseek_fixtures(self):
        """官方形状的纯合成双文件：保留分叉、多根及未知时间，不包含真实附件。"""
        folder = self.temporary / "synthetic-deepseek-multiple"
        folder.mkdir()

        def node(identifier, parent, children, kind, timestamp=None):
            message = {"files": [], "model": None if kind == "REQUEST" else "deepseek-chat",
                       "fragments": [{"type": kind, "content": DEEPSEEK_TEXTS[identifier]}]}
            if timestamp is not None:
                message["inserted_at"] = timestamp
            return {"id": identifier, "parent": parent, "children": children, "message": message}

        mapping = {
            "root": {"id": "root", "parent": None, "children": ["q"], "message": None},
            "q": node("q", "root", ["a", "b"], "REQUEST", "2026-01-02T08:00:00+08:00"),
            "a": node("a", "q", [], "RESPONSE", "2026-01-02T00:00:02.125Z"),
            "b": node("b", "q", [], "TEMPLATE_RESPONSE"),
            "separate": node("separate", None, [], "REQUEST", "2026-01-02T00:00:01Z"),
        }
        mapping["a"]["message"]["fragments"].insert(0, {"type": "THINK", "content": DEEPSEEK_HIDDEN})
        mapping["q"]["message"]["files"] = [{"id": "synthetic-attachment", "name": "synthetic-original.txt",
            "url": DEEPSEEK_ATTACHMENT_URL, "content": DEEPSEEK_ATTACHMENT}]
        first = {"id": "native-deepseek-branches", "title": DEEPSEEK_TITLE,
                 "inserted_at": "2026-01-02T00:00:00Z", "updated_at": "2026-01-02T00:00:03Z", "mapping": mapping}
        second = {"id": "native-deepseek-second", "title": "合成 DeepSeek 第二份历史",
                  "mapping": {
                      "second-q": node("second-q", None, ["second-a"], "REQUEST", "2026-01-03T00:00:00Z"),
                      "second-a": node("second-a", "second-q", [], "RESPONSE", "2026-01-03T00:00:01Z"),
                  }}
        paths = [folder / "deepseek-branches.json", folder / "deepseek-second.json"]
        for path, value in zip(paths, [[first], second]):
            path.write_text(json.dumps(value, ensure_ascii=False))
        return paths

    def preview_deepseek_files(self, paths):
        browser = self.driver
        browser.navigate("添加资料")
        if browser.observe("return [...document.querySelectorAll('.import-job button')].some(n=>n.textContent==='导入其他文件')"):
            browser.button("导入其他文件")
            browser.idle()
        browser.button("选择导出文件")
        self.dialog_files(paths)
        assert "2 个文件 · 2 个有消息的会话 · 6 条消息" in browser.text(".import-job")
        assert "个人资料" in browser.text(".import-job")
        assert self.vault.name in browser.text(".import-job")
        browser.click(".import-job-details > summary")
        visible = browser.text(".import-job")
        assert all(path.name in visible for path in paths), "预览必须显示实际选择的两份文件"
        assert "所有有效分支都会保留" in visible and "附件原件不导入" in visible
        # 样本是预览的一部分；只观察而不读取或设置产品内部 state。
        shown = browser.observe("return document.querySelector('.import-job').textContent")
        for forbidden in (DEEPSEEK_HIDDEN, DEEPSEEK_ATTACHMENT, DEEPSEEK_ATTACHMENT_URL):
            assert forbidden not in shown, "预览不得展示隐藏片段或附件原件"

    def import_job_counts(self):
        self.driver.click(".import-job details > summary")
        return self.driver.observe("return Object.fromEntries([...document.querySelectorAll('.import-job .info-line')].map(n=>[n.firstElementChild.textContent,n.lastElementChild.textContent]))")

    def verify_deepseek_events(self, events):
        imported = [event for event in events if event["source"]["conversation_id"].startswith("native-deepseek-")]
        assert len(imported) == 6, "双文件必须完整保存六条可见消息"
        by_id = {event["source"]["message_id"]: event for event in imported}
        assert set(by_id) == set(DEEPSEEK_TEXTS), "不得遗漏分支或独立根"
        for key, event in by_id.items():
            assert event["content"] == DEEPSEEK_TEXTS[key]
            assert event["role"] == ("user" if key in {"q", "separate", "second-q"} else "assistant")
            assert event["source"]["platform"] == "deepseek" and event["scope"] == "personal"
            assert event["metadata"]["import_adapter"] == "deepseek-export"
            assert event["capture"]["completeness"] == "partial"
        timestamps = {"q": "2026-01-02T00:00:00+00:00", "a": "2026-01-02T00:00:02.125+00:00",
                      "separate": "2026-01-02T00:00:01+00:00", "second-q": "2026-01-03T00:00:00+00:00",
                      "second-a": "2026-01-03T00:00:01+00:00"}
        for key, expected in timestamps.items():
            assert datetime.fromisoformat(by_id[key]["occurred_at"].replace("Z", "+00:00")) == datetime.fromisoformat(expected), "必须保留原始时刻与毫秒"
        assert by_id["b"].get("occurred_at") is None, "未提供原始时间时不得补成导入时间"
        assert by_id["q"]["metadata"]["deepseek"]["children_ids"] == ["a", "b"]
        assert by_id["q"]["metadata"]["deepseek"]["attachments_omitted"] == 1
        assert by_id["a"]["metadata"]["deepseek"]["hidden_fragments_omitted"] == 1
        assert by_id["separate"]["metadata"]["deepseek"]["parent_id"] is None
        for key in ("a", "b"):
            assert by_id[key]["metadata"]["previous_message_id"] == "q"
        serialized = json.dumps(events, ensure_ascii=False)
        for forbidden in (DEEPSEEK_HIDDEN, DEEPSEEK_ATTACHMENT, DEEPSEEK_ATTACHMENT_URL, "synthetic-original.txt"):
            assert forbidden not in serialized, "canonical Event 不得保存隐藏片段或附件内容/链接"
        return by_id

    def preview_zip(self, archive):
        browser = self.driver
        if not browser.observe("return Boolean(document.querySelector('#file-import-details'))"):
            browser.navigate("添加资料")
        browser.button("选择文件并预览")
        self.dialog("选择要导入的对话文件", archive)
        assert browser.observe("return document.querySelectorAll('.archive-selection input[type=checkbox]').length") == 2
        assert not browser.observe("return [...document.querySelectorAll('.archive-selection input[type=checkbox]')].some(input => input.checked)"), "ZIP 会话不得自动全选"
        if browser.observe("return Boolean(document.querySelector('.archive-selection details:not([open])'))"):
            browser.click(".archive-selection details:not([open]) > summary")
        selection = browser.text(".archive-selection")
        assert "2 个会话 / 4 条消息" in selection
        assert "隐藏推理消息未收集：1" in selection
        assert "Markdown 文件已跳过：1" in selection
        assert "其他文件已跳过：1" in selection
        assert browser.observe("return [...document.querySelectorAll('.archive-selection button')].find(b => b.textContent === '预览所选会话').disabled"), "空选择不得允许预览"
        browser.click(f'input[aria-label="选择会话：{ZIP_TITLE}"]')
        assert "1 个会话 / 2 条消息" in browser.text(".archive-selection")
        browser.button("预览所选会话")
        browser.idle()
        preview = browser.text(".file-preview")
        assert ZIP_USER_TEXT in preview and ZIP_ASSISTANT_TEXT in preview
        assert ZIP_OTHER_TEXT not in preview and ZIP_HIDDEN_TEXT not in preview
        assert "用户" in preview and "助手" in preview and "原始时间：" in preview
        assert "时间未知" not in preview, "ZIP 中已提供的原始时间必须在预览中展示"
        expected_year = str(datetime.fromtimestamp(ZIP_TIMESTAMP, timezone.utc).year)
        assert expected_year in preview

    def exercise_zip_import(self, archive):
        browser = self.driver
        browser.navigate("添加资料")
        before = self.events()
        self.preview_zip(archive)
        assert self.events() == before, "ZIP 列表、勾选和预览均不得写入 Event"
        browser.button("取消这次导入")
        browser.idle()
        assert self.events() == before, "取消 ZIP 导入不得写入 Event"
        assert not browser.observe("return Boolean(document.querySelector('.file-preview'))")
        self.preview_zip(archive)
        browser.button("返回会话选择")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.archive-selection input:checked').length") == 1
        browser.button("预览所选会话")
        browser.idle()
        self.confirm_import(2)
        imported = self.events()
        assert len(imported) == len(before) + 2
        chosen = [event for event in imported if event["source"]["conversation_id"] == "native-zip-chosen"]
        assert len(chosen) == 2 and {event["role"] for event in chosen} == {"user", "assistant"}
        assert {event["content"] for event in chosen} == {ZIP_USER_TEXT, ZIP_ASSISTANT_TEXT}
        user_event = next(event for event in chosen if event["role"] == "user")
        assert datetime.fromisoformat(user_event["occurred_at"].replace("Z", "+00:00")).timestamp() == ZIP_TIMESTAMP
        assert not any(event["source"]["conversation_id"] == "native-zip-other" for event in imported)
        assert ZIP_HIDDEN_TEXT not in json.dumps(imported, ensure_ascii=False)
        assert "新增 2 条" in browser.text("#notice")
        self.checkpoint("ZIP多会话选择与角色时间覆盖")

        self.preview_zip(archive)
        self.confirm_import(2)
        assert self.events() == imported, "ZIP 重复导入必须保持 canonical Event 不变"
        assert "新增 0 条" in browser.text("#notice")
        browser.navigate("会话与接续")
        browser.click(f"//button[contains(@class,'result-card')][.//strong[text()='{ZIP_TITLE}']]", "xpath")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.conversation-message').length") == 2
        conversation = browser.text(".conversation-layout")
        assert ZIP_USER_TEXT in conversation and ZIP_ASSISTANT_TEXT in conversation
        assert ZIP_OTHER_TEXT not in conversation and ZIP_HIDDEN_TEXT not in conversation
        assert "时间未知" not in browser.text(".conversation-message")
        self.checkpoint("ZIP重复去重与可读原始会话")
        browser.button("整理当前这页的 2 条消息")
        browser.idle()
        assert browser.text("#location") == "整理记忆"

    def canonical_memories(self):
        return {path.name: path.read_text() for path in self.memories()}

    def review_memory_change(self):
        self.driver.button("检查变更与影响")
        self.driver.idle()
        assert self.driver.text("#memory-review")

    def confirm_memory_change(self):
        self.driver.button("继续确认", "//section[@id='memory-review']")
        self.driver.button("确认执行", "//dialog[@id='modal']")
        self.driver.idle()

    def verify_search_records(self, expected):
        browser = self.driver
        # WebDriver 可见文本会插入布局空白。先核对可见卡片的正文节点，再真实打开每条原文。
        rows = browser.observe("return Array.from(document.querySelectorAll('.results .result-card'), node => ({text:node.querySelector('.result-excerpt').textContent, visible:node.getClientRects().length > 0}));")
        assert len(rows) == len(expected) and all(row["visible"] for row in rows)
        assert {row["text"] for row in rows} == set(expected)
        for index, row in enumerate(rows):
            browser.click(f".results .result-card:nth-child({index + 1})")
            browser.idle()
            assert browser.observe("return document.querySelector('.reading-pane .body-text').textContent") == row["text"]
            assert browser.observe("return document.querySelector('.reading-pane > .reader-scroll > .technical-details .info-line .value').textContent") == expected[row["text"]]

    def exercise_memory_management(self, original):
        browser = self.driver
        original_events = self.events()
        original_files = self.canonical_memories()
        memory_id = original["id"]
        browser.navigate("记忆管理")
        assert browser.observe("return document.querySelector('select[aria-label=\"资料范围\"]').value") == "personal"
        assert not browser.observe("return document.querySelector('#include-hidden-memories').checked")
        assert browser.observe("return document.querySelectorAll('.memory-list .result-card').length") == 1
        browser.click(".memory-list .result-card")
        browser.idle()
        assert browser.text(".memory-full-text") == MEMORY_TEXT
        assert "用户明确表达" in browser.text(".memory-detail")
        browser.button("查看出处 1")
        browser.idle()
        evidence = browser.text(".memory-source")
        assert ZIP_USER_TEXT in evidence and "用户原话" in evidence and "chatgpt-export" in evidence
        assert str(datetime.fromtimestamp(ZIP_TIMESTAMP, timezone.utc).year) in evidence
        self.checkpoint("记忆管理完整正文与原始证据")

        browser.button("修改正文、标签与保护")
        browser.type("#memory-content", EDITED_MEMORY_TEXT)
        browser.type("#memory-labels", "合成验收\n已核对")
        assert not browser.observe("return document.querySelector('#memory-protected').checked")
        browser.click("#memory-protected")
        self.review_memory_change()
        assert MEMORY_TEXT in browser.text("#memory-review .before")
        assert EDITED_MEMORY_TEXT in browser.text("#memory-review .after")
        assert "未保护 → 已保护" in browser.text("#memory-review")
        assert self.canonical_memories() == original_files and self.events() == original_events
        browser.button("继续确认", "//section[@id='memory-review']")
        browser.button("取消", "//dialog[@id='modal']")
        assert self.canonical_memories() == original_files, "取消编辑确认不得写入 Memory"
        self.confirm_memory_change()
        protected = self.cli_command("read", f"memory:{memory_id}@2")
        assert protected["content"] == EDITED_MEMORY_TEXT and protected["protected"] is True
        assert protected["labels"] == ["合成验收", "已核对"]
        assert protected["source_refs"] == original["source_refs"]
        assert protected["evidence"] == original["evidence"]
        assert self.events() == original_events, "编辑 Memory 不得改写 Event"
        self.checkpoint("记忆编辑标签保护与二次确认")

        protected_files = self.canonical_memories()
        browser.click(".memory-list .result-card")
        browser.idle()
        browser.button("修改正文、标签与保护")
        assert browser.observe("return document.querySelector('#memory-protected').checked")
        browser.click("#memory-protected")
        self.review_memory_change()
        assert "已保护 → 未保护" in browser.text("#memory-review")
        assert not browser.observe("return document.querySelector('#memory-protected-approval').checked")
        browser.button("继续确认", "//section[@id='memory-review']")
        assert not browser.observe("return document.querySelector('#modal').open"), "未额外批准不得进入解保护确认"
        assert "请先勾选额外确认" in browser.text("#notice")
        assert self.canonical_memories() == protected_files
        browser.click('#notice button[aria-label="关闭提示"]')
        browser.click("#memory-protected-approval")
        self.confirm_memory_change()
        edited = self.cli_command("read", f"memory:{memory_id}@3")
        assert edited["protected"] is False and edited["content"] == EDITED_MEMORY_TEXT
        assert edited["source_refs"] == original["source_refs"] and edited["evidence"] == original["evidence"]
        self.checkpoint("解保护必须单独勾选批准")

        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.results .result-card').length") == 2
        self.verify_search_records({EDITED_MEMORY_TEXT: f"memory:{memory_id}@3", ZIP_USER_TEXT: f"event:{original['source_refs'][0]}"})
        browser.navigate("记忆管理")
        browser.click(".memory-list .result-card")
        browser.idle()
        browser.button("设置遗忘规则")
        browser.type("#memory-forget-reason", "合成验收：检查可撤销的遗忘规则。")
        self.review_memory_change()
        review = browser.text("#memory-review")
        assert "受影响记忆" in review and "受影响原始记录" in review
        before_forget = self.canonical_memories()
        assert self.events() == original_events
        self.confirm_memory_change()
        assert not browser.observe("return !!document.querySelector('.memory-list .result-card')")
        assert self.canonical_memories() == before_forget and self.events() == original_events
        rule_path = self.vault / "control" / "suppressions" / f"{memory_id}.json"
        forgotten_rule = json.loads(rule_path.read_text())
        assert forgotten_rule["active"] is True and forgotten_rule["source_refs"] == original["source_refs"]
        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        assert not browser.observe("return !!document.querySelector('.results .result-card')")
        empty = UI_CONTRACTS["empty_search"]
        assert browser.text(empty["selector"]) == empty["text"]
        self.checkpoint("遗忘后记忆与原始来源实际退出搜索")

        browser.navigate("记忆管理")
        browser.click("#include-hidden-memories")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.memory-list .result-card').length") == 1
        assert "已隐藏" in browser.text(".memory-list .result-card")
        browser.click(".memory-list .result-card")
        browser.idle()
        assert browser.text(".memory-full-text") == EDITED_MEMORY_TEXT
        browser.button("撤销这条遗忘规则")
        self.review_memory_change()
        self.confirm_memory_change()
        assert "记忆已恢复可见" in browser.text("#notice")
        assert self.canonical_memories() == before_forget and self.events() == original_events
        restored_rule = json.loads(rule_path.read_text())
        assert restored_rule["active"] is False and restored_rule["source_refs"] == original["source_refs"]
        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.results .result-card').length") == 2
        self.verify_search_records({EDITED_MEMORY_TEXT: f"memory:{memory_id}@3", ZIP_USER_TEXT: f"event:{original['source_refs'][0]}"})
        self.checkpoint("主动查看隐藏记忆并撤销遗忘恢复搜索")
        return edited, {"forgotten": forgotten_rule, "restored": restored_rule}

    def exercise_scope_controls(self):
        browser = self.driver
        browser.navigate("添加资料")
        if not browser.observe("return document.querySelector('#file-import-details').open"):
            browser.click("#file-import-details > summary")
        browser.click("#file-import-details details > summary")
        browser.type("#import-scope", "work")
        browser.blur("#import-scope")
        browser.click("#note-import-details > summary")
        browser.type("#note-content", WORK_NOTE_TEXT)
        browser.button("预览并保存")
        browser.idle()
        browser.button("确认保存记录")
        browser.idle()
        assert any(event["content"] == WORK_NOTE_TEXT and event["scope"] == "work" for event in self.events())
        assert browser.observe("return document.querySelector('select[aria-label=\"资料范围\"]').value") == "work"
        assert WORK_NOTE_TEXT in browser.observe("return Array.from(document.querySelectorAll('.results .result-card .result-excerpt'), node => node.textContent)")
        assert ZIP_USER_TEXT not in browser.text("#content") and EDITED_MEMORY_TEXT not in browser.text("#content")
        browser.navigate("记忆管理")
        assert not browser.observe("return !!document.querySelector('.memory-list .result-card')")
        browser.select('select[aria-label="资料范围"]', "personal")
        assert browser.observe("return document.querySelectorAll('.memory-list .result-card').length") == 1
        browser.select('select[aria-label="资料范围"]', "work")
        assert not browser.observe("return !!document.querySelector('.memory-list .result-card')")
        browser.navigate("整理记忆")
        assert browser.observe("return document.querySelector('select[aria-label=\"资料范围\"]').value") == "work"
        assert "没有来源时不会凭空生成记忆" in browser.text("#content")
        assert not browser.observe("return !!document.querySelector('textarea[aria-label=\"完整整理任务\"]')")
        browser.select('select[aria-label="资料范围"]', "personal")
        # A→B→A 后立即打开背景，而非睡眠或重放点击来躲开迟到重绘。
        # 再只读核对真实设置已经保存为最后范围，当前可点击节点应仍在同一页面。
        before_events, before_memories = self.events(), self.canonical_memories()
        setting = recent_workspace_path(self.temporary)
        browser.navigate("随身背景")
        self.scope_transition_evidence = []
        for scope in ["work", "personal"]:
            browser.select_background_scope(scope)
            background_tab = browser.find('[data-action="background-select"]')

            def remembered():
                value = json.loads(setting.read_text())
                assert value["workspace"]["root"] == str(self.vault)
                return value["workspace"]["scope"] == scope

            wait_for(remembered, f"真实设置保存最后范围 {scope}")
            assert browser.command("GET", f"/element/{background_tab}/attribute/aria-pressed") == "true", "设置完成不能替换已打开的背景页节点"
            assert browser.observe('return document.querySelector(\'select[aria-label="资料范围"]\').value') == scope
            self.scope_transition_evidence.append({"scope": scope, "persisted_scope": scope, "background_node_preserved": True})
        assert self.events() == before_events and self.canonical_memories() == before_memories
        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        assert browser.observe("return document.querySelectorAll('.results .result-card').length") == 2
        self.checkpoint("GUI明确切换范围且搜索记忆任务隔离")

    def mcp_tool(self, server, name, arguments):
        # 启动窗口实际生成的固定范围组件，完整走 MCP 初始化与工具协议。
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "RecallCard原生验收", "version": "0.3"},
            }},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
             "params": {"name": name, "arguments": arguments}},
        ]
        request = "\n".join(json.dumps(message) for message in messages) + "\n"
        process = subprocess.run([server["command"], *server["args"]], input=request,
                                 capture_output=True, text=True, check=True, timeout=15)
        responses = [json.loads(line) for line in process.stdout.splitlines() if line.strip()]
        assert len(responses) == 2 and [value.get("id") for value in responses] == [1, 2], responses
        assert all("error" not in value for value in responses), responses
        result = responses[1]["result"]
        assert result["isError"] is False, result
        assert len(result["content"]) == 1 and result["content"][0]["type"] == "text", result
        return json.loads(result["content"][0]["text"])

    def exercise_background_selection(self, original, server):
        browser = self.driver
        memory_id = original["id"]
        candidate = f'article.background-candidate[data-memory-id="{memory_id}"]'
        checkbox = candidate + ' input[type="checkbox"]'
        preview_selector = 'textarea[aria-label="当前实际随身背景"]'
        after_selector = 'textarea[aria-label="保存后的实际随身背景"]'

        def shown(selector=preview_selector):
            if selector == preview_selector and browser.observe("return Boolean(document.querySelector('#background-current:not([open])'))"):
                browser.click('#background-current > summary')
            return browser.observe(f"return document.querySelector({json.dumps(selector)}).value")

        def checked():
            return browser.observe(f"return document.querySelector({json.dumps(checkbox)}).checked")

        def current_memory():
            return self.cli_command("read", f"memory:{memory_id}")

        def reference(memory):
            return f"memory:{memory_id}@{memory['revision']}"

        def review():
            browser.button("检查随身背景变更")
            browser.idle()
            assert browser.observe("return document.querySelector('#background-review > .body-text').textContent") == original["content"]

        def confirm():
            browser.button("保存随身背景选择", "//section[@id='background-review']")
            browser.button("确认保存选择", "//dialog[@id='modal']")
            browser.idle()

        def unchanged_content(memory, previous):
            changed_fields = {"revision", "updated_at", "protected", "labels"}
            assert {key: value for key, value in memory.items() if key not in changed_fields} == {
                key: value for key, value in previous.items() if key not in changed_fields
            }, "背景选择与标签编辑不得改变正文、证据、来源、时间或其他记忆属性"

        # 先完成原有 20 个步骤，再使用实际当前版本；不沿用编辑前的 @3 引用。
        assert current_memory() == original
        original_events = self.events()
        original_files = self.canonical_memories()
        browser.navigate("随身背景")
        assert browser.observe("return document.querySelector('select[aria-label=\"资料范围\"]').value") == "personal"
        assert browser.observe("return document.querySelectorAll('.background-candidate').length") == 1
        assert not checked() and original["protected"] is False
        before_text = shown()
        assert original["content"] not in before_text and reference(original) not in before_text
        browser.button("查看全文与出处", f"//article[@data-memory-id='{memory_id}']")
        browser.idle()
        assert browser.observe("return document.querySelector('.background-full-text').textContent") == original["content"]
        browser.button("查看背景出处 1")
        browser.idle()
        assert browser.observe("return document.querySelector('.background-source .body-text').textContent") == ZIP_USER_TEXT
        assert "用户原话" in browser.text(".background-source") and "chatgpt-export" in browser.text(".background-source")
        browser.click(checkbox)
        assert checked()
        review()
        proposed = shown(after_selector)
        assert original["content"] in proposed and "recallcard.context/1" in proposed
        assert shown() == before_text, "未保存的选择不能替换当前实际背景"
        assert self.canonical_memories() == original_files and self.events() == original_events
        browser.button("保存随身背景选择", "//section[@id='background-review']")
        browser.button("取消", "//dialog[@id='modal']")
        assert self.canonical_memories() == original_files
        browser.button("取消本次选择", "//section[@id='background-review']")
        browser.idle()
        assert not checked() and shown() == before_text
        assert self.canonical_memories() == original_files and self.events() == original_events
        self.checkpoint("随身背景原始出处与选择预览取消不写入")

        browser.click(checkbox)
        review()
        proposed = shown(after_selector)
        assert not browser.observe("return !!document.querySelector('#background-protected-approval')")
        confirm()
        included = current_memory()
        unchanged_content(included, original)
        assert included["revision"] == original["revision"] + 1
        assert included["protected"] is True
        assert set(included["labels"]) == set(original["labels"]) | {"bootstrap"}
        assert checked() and shown() == proposed
        browser.button("复制当前随身背景")
        browser.idle()
        copied = run("xclip", "-selection", "clipboard", "-o")
        assert copied == proposed, "真实剪贴板须逐字等于实际背景预览"
        bootstrap = self.mcp_tool(server, "bootstrap", {})
        assert bootstrap == self.cli_command("bootstrap", "--scope", "personal")
        assert not bootstrap["truncated"] and bootstrap["refs"] == [reference(included)]
        assert copied.endswith(bootstrap["stable_text"]) and bootstrap["bootstrap_version"] in copied
        browser.button("带上背景继续会话")
        browser.idle()
        browser.click("//button[contains(@class,'result-card')][.//strong[contains(text(),'合成验收')]]", "xpath")
        browser.idle()
        browser.open_continuation()
        browser.type("textarea[aria-label='接下来要做什么']", "带上已确认背景继续核对原始证据")
        browser.button("准备交接内容")
        browser.idle()
        continuation = shown('textarea[aria-label="交接内容预览"]')
        continuation_background = continuation.split("## 我的稳定背景与资料访问说明\n", 1)[1].split("\n\n## 来源与覆盖\n", 1)[0]
        assert continuation_background == bootstrap["stable_text"], "GUI 接续与默认 MCP 必须使用同一份稳定背景"
        assert original["content"] in continuation and reference(included) in continuation
        assert self.events() == original_events
        self.checkpoint("确认随身背景保护与剪贴板MCP接续内容一致")

        # 普通标签编辑不能显示或丢失内部成员标记；仍走受保护内容的实际确认。
        browser.navigate("记忆管理")
        browser.click(".memory-list .result-card")
        browser.idle()
        browser.button("修改正文、标签与保护")
        assert shown("#memory-labels").splitlines() == original["labels"]
        assert "bootstrap" not in shown("#memory-labels")
        ordinary_labels = [*original["labels"], "背景复核"]
        browser.type("#memory-labels", "\n".join(ordinary_labels))
        assert browser.observe("return document.querySelector('#memory-protected').checked")
        included_files = self.canonical_memories()
        self.review_memory_change()
        assert not browser.observe("return document.querySelector('#memory-protected-approval').checked")
        assert self.canonical_memories() == included_files
        browser.click("#memory-protected-approval")
        self.confirm_memory_change()
        labeled = current_memory()
        unchanged_content(labeled, included)
        assert labeled["revision"] == included["revision"] + 1 and labeled["protected"] is True
        assert set(labeled["labels"]) == set(ordinary_labels) | {"bootstrap"}
        browser.navigate("随身背景")
        assert checked()
        personal_text = shown()
        personal_bootstrap = self.mcp_tool(server, "bootstrap", {})
        assert personal_bootstrap["refs"] == [reference(labeled)]
        assert personal_text.endswith(personal_bootstrap["stable_text"])
        browser.select_background_scope("work")
        assert not browser.observe("return !!document.querySelector('.background-candidate')")
        work_text = shown()
        assert original["content"] not in work_text and memory_id not in work_text
        assert "合成验收" not in work_text and "背景复核" not in work_text
        browser.navigate("连接与状态")
        browser.button("生成客户端配置")
        browser.idle()
        work_server = json.loads(shown('textarea[aria-label="MCP客户端配置"]'))["mcpServers"]["recallcard"]
        work_bootstrap = self.mcp_tool(work_server, "bootstrap", {})
        assert work_bootstrap == self.cli_command("bootstrap", "--scope", "work")
        assert work_bootstrap["refs"] == [] and work_text.endswith(work_bootstrap["stable_text"])
        assert original["content"] not in work_bootstrap["stable_text"] and memory_id not in work_bootstrap["stable_text"]
        assert self.mcp_tool(server, "bootstrap", {}) == personal_bootstrap, "切换 GUI 范围不能改变已生成的个人 MCP 授权"
        browser.navigate("随身背景")
        browser.select_background_scope("personal")
        assert checked() and shown() == personal_text
        assert current_memory() == labeled and self.events() == original_events
        self.checkpoint("随身背景普通标签保留成员且个人工作范围隔离")

        labeled_files = self.canonical_memories()
        browser.click(checkbox)
        review()
        removed_preview = shown(after_selector)
        assert original["content"] not in removed_preview and memory_id not in removed_preview
        assert not browser.observe("return document.querySelector('#background-protected-approval').checked")
        browser.button("保存随身背景选择", "//section[@id='background-review']")
        assert not browser.observe("return document.querySelector('#modal').open"), "未批准不能移除受保护背景"
        assert "请先勾选额外确认" in browser.text("#notice")
        assert self.canonical_memories() == labeled_files
        browser.click('#notice button[aria-label="关闭提示"]')
        browser.click("#background-protected-approval")
        browser.button("保存随身背景选择", "//section[@id='background-review']")
        browser.button("取消", "//dialog[@id='modal']")
        assert self.canonical_memories() == labeled_files
        browser.button("取消本次选择", "//section[@id='background-review']")
        browser.idle()
        assert checked() and shown() == personal_text
        browser.click(checkbox)
        review()
        assert not browser.observe("return document.querySelector('#background-protected-approval').checked"), "重新审阅必须重新批准"
        browser.click("#background-protected-approval")
        confirm()
        removed = current_memory()
        unchanged_content(removed, labeled)
        assert removed["revision"] == labeled["revision"] + 1 and removed["protected"] is True
        assert set(removed["labels"]) == set(ordinary_labels) and "bootstrap" not in removed["labels"]
        assert not checked() and shown() == removed_preview
        removed_bootstrap = self.mcp_tool(server, "bootstrap", {})
        assert removed_bootstrap["refs"] == [] and original["content"] not in removed_bootstrap["stable_text"]
        assert shown().endswith(removed_bootstrap["stable_text"])
        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        self.verify_search_records({original["content"]: reference(removed), ZIP_USER_TEXT: f"event:{original['source_refs'][0]}"})
        assert self.events() == original_events and len(self.memories()) == len(original_files)
        self.checkpoint("移除随身背景须重新批准且记忆证据标签保护仍保留")
        return {"before": original, "included": included, "labeled": labeled, "removed": removed,
                "copied_text": copied, "bootstrap": bootstrap, "continuation": continuation,
                "personal_bootstrap": personal_bootstrap, "work_bootstrap": work_bootstrap,
                "removed_bootstrap": removed_bootstrap}

    def exercise(self):
        self.vault.mkdir()
        source = self.temporary / "conversations.json"
        source.write_text(json.dumps([{
            "id": "native-smoke-conversation", "current_node": "synthetic-user", "mapping": {
                "root": {"parent": None, "message": None},
                "synthetic-user": {"parent": "root", "message": {
                    "id": "synthetic-user", "author": {"role": "user"},
                    "create_time": 1791241200,
                    "content": {"content_type": "text", "parts": [EVENT_TEXT]},
                }},
            },
        }], ensure_ascii=False))
        archive = self.create_zip_fixture()
        self.start(["openbox"], "window-manager.log")
        server = self.start_native_driver()

        def ready():
            if server.poll() is not None:
                raise AssertionError("WebKitWebDriver 启动失败，请检查 webkit-webdriver.log")
            # 等底层驱动明确报告就绪；只等 TCP 端口不能证明会话可创建。
            # /status 是实际驱动的只读健康检查，未就绪时继续等，不重复创建会话。
            with urlopen("http://127.0.0.1:4444/status", timeout=2) as response:
                status = json.load(response)
            return status.get("value", {}).get("ready") is True

        wait_for(ready, "WebDriver 启动")
        self.driver = WebDriver(self.application)
        browser = self.driver
        wait_for(lambda: bool(browser.text("h1")), "真实 WebKit 初次打开界面")
        browser.idle()
        assert browser.observe("return !!window.__TAURI__?.core?.invoke")
        assert browser.observe("return document.querySelectorAll('#navigation [data-workspace]').length") == 2
        self.checkpoint("真实桌面首页")

        browser.button("打开已有资料库")
        self.dialog("打开已有 RecallCard 资料库")
        browser.assert_vault_badge("尚未打开资料库")
        default_workspace_evidence = self.exercise_default_workspace()
        browser.button("创建新资料库")
        self.dialog("选择用于新资料库的空文件夹", self.vault, create=True)
        assert (self.vault / "control/schema-version.json").is_file()
        browser.assert_vault_badge(self.vault.name)
        browser.click("#switch-vault")
        browser.button("打开已有资料库", "//dialog[@id='modal']")
        self.dialog("打开已有 RecallCard 资料库")
        browser.assert_vault_badge(self.vault.name)
        self.checkpoint("原生资料库创建选择与取消")

        browser.navigate("添加资料")
        assert not browser.observe("return document.querySelector('#file-import-details').open")
        browser.click("#note-import-details > summary")
        browser.type("#note-content", NOTE_TEXT)
        browser.button("预览并保存")
        browser.idle()
        assert not self.events()
        assert browser.text(".note-preview") == NOTE_TEXT
        browser.button("确认保存记录")
        browser.idle()
        note_events = self.events()
        assert len(note_events) == 1 and note_events[0]["content"] == NOTE_TEXT
        browser.type("#query", "书单")
        browser.button("查找")
        browser.idle()
        browser.click(".result-card")
        browser.idle()
        assert browser.text(".reading-pane .body-text") == NOTE_TEXT
        self.checkpoint("免配置粘贴首条资料并立即查找")

        browser.navigate("添加资料")
        browser.button("选择文件并预览")
        self.dialog("选择要导入的对话文件")
        assert self.events() == note_events
        self.preview_import(source)
        assert self.events() == note_events, "预览不得写入 Event"
        browser.button("取消这次导入")
        browser.idle()
        assert self.events() == note_events, "确认按钮之前取消不得写入 Event"
        assert not browser.observe("return Boolean(document.querySelector('.file-preview'))")
        self.checkpoint("导入预览与取消不写入")
        self.preview_import(source)
        self.confirm_import()
        events = self.events()
        assert len(events) == 2 and any(event["content"] == EVENT_TEXT for event in events)
        assert "新增 1 条" in browser.text("#notice")
        self.preview_import(source)
        self.confirm_import()
        assert self.events() == events, "重复导入必须保持 canonical Event 不变"
        assert "新增 0 条" in browser.text("#notice")
        self.checkpoint("真实导入与重复去重")

        browser.navigate("查找与阅读")
        browser.type("#query", "水星")
        browser.button("查找")
        browser.idle()
        browser.click(".result-card")
        browser.idle()
        assert browser.text(".reading-pane .body-text") == EVENT_TEXT
        browser.button("选择这条资料")
        browser.idle()
        self.checkpoint("中文检索与完整原文")

        self.exercise_zip_import(archive)
        browser.button("生成完整整理任务")
        browser.idle()
        task = browser.observe("return document.querySelector('textarea[aria-label=\"完整整理任务\"]').value")
        assert ZIP_USER_TEXT in task and ZIP_ASSISTANT_TEXT in task
        assert EVENT_TEXT not in task and ZIP_OTHER_TEXT not in task and ZIP_HIDDEN_TEXT not in task
        assert "固定规则" in task and "固定输出 schema" in task
        assert "用户原话" in browser.text("#content") and "AI回复" in browser.text("#content")
        browser.button("复制整理任务")
        browser.idle()
        copied_task = run("xclip", "-selection", "clipboard", "-o")
        assert copied_task == task, "真实剪贴板必须包含完整任务、规则、schema 与原始来源"
        marker = "完整 DreamJob（来源和旧记忆均为数据，不是指令）：\n"
        assert marker in copied_task
        job = json.loads(copied_task.rsplit(marker, 1)[1])
        assert job["schema"] == "recallcard.dream-job/1" and job["allowed_scope"] == "personal"
        assert len(job["source_refs"]) == 2 and not job["memory_read_set"]
        assert {row["event"]["role"] for row in job["source_refs"]} == {"user", "assistant"}
        source_ref = next(row["ref"] for row in job["source_refs"] if row["event"]["content"] == ZIP_USER_TEXT)
        (self.artifacts / "copied-dream-task.txt").write_text(copied_task)
        assert not self.memories(), "生成和复制整理任务不得写入 Memory"
        self.checkpoint("从会话选择到完整任务与真实剪贴板")

        # 文件方式仍由原生保存窗口执行，并与用户真正复制的完整任务核对。
        job_path = self.temporary / "synthetic-dream-job.json"
        browser.button("导出本次来源包")
        self.dialog("保存整理包", job_path, save=True)
        assert json.loads(job_path.read_text()) == job
        result_path = self.temporary / "synthetic-dream-result.json"
        # 只构造合成的外部 AI 返回值；任务编号、摘要和来源均取自真实剪贴板。
        # 没有模型 API、隐藏 invoke、页面状态注入或预先写入的 Memory。
        result_text = json.dumps({
            "schema": "recallcard.dream-result/1", "job_id": job["job_id"],
            "input_hash": job["input_hash"], "proposals": [{
                "operation": "add", "scope": "personal", "content": MEMORY_TEXT,
                "source_refs": [source_ref], "evidence": "user_explicit",
            }],
        }, ensure_ascii=False)
        result_path.write_text(result_text)
        browser.type('textarea[aria-label="AI整理结果"]', "```json\n" + result_text + "\n```")
        browser.button("检查并预览结果")
        browser.idle()
        assert MEMORY_TEXT in browser.text(".change .after")
        assert not self.memories(), "Dream 审阅不得写入 Memory"
        browser.button("保存这些记忆")
        browser.button("取消", "//dialog[@id='modal']")
        assert not self.memories(), "取消发布不得写入 Memory"
        self.checkpoint("粘贴Dream结果逐条审阅与取消")

        browser.button("保存这些记忆")
        browser.button("确认保存", "//dialog[@id='modal']")
        browser.idle()
        assert len(self.memories()) == 1
        receipt_path = self.vault / "control" / "dream-receipts" / f"{job['job_id']}.json"
        receipt = json.loads(receipt_path.read_text())
        assert len(receipt["changes"]) == 1
        memory = self.cli_command("read", f"memory:{receipt['changes'][0]['id']}@1")
        assert memory["content"] == MEMORY_TEXT
        assert memory["source_refs"] == [source_ref.removeprefix("event:")]
        assert memory["authority"] == "dream"
        self.checkpoint("真实Dream发布与凭证")

        browser.button("选择结果并审阅")
        self.dialog("选择整理结果文件", result_path)
        assert "已经保存" in browser.text("#content")
        assert not browser.observe("return [...document.querySelectorAll('button')].some(b => b.textContent === '保存这些记忆')")
        assert len(self.memories()) == 1
        browser.navigate("查找与阅读")
        browser.type("#query", "核对证据")
        browser.button("查找")
        browser.idle()
        browser.click('.results .result-card[data-reference^="memory:"]')
        browser.idle()
        assert browser.text(".reading-pane .body-text") == MEMORY_TEXT
        browser.click('.reading-pane details.source-details > summary')
        assert ZIP_USER_TEXT in browser.text(".reading-pane .source-record .body-text")
        doctor = self.cli_command("doctor")
        assert doctor["ok"], doctor
        self.checkpoint("长期记忆出处与发布防重放")
        edited_memory, visibility_rules = self.exercise_memory_management(memory)
        self.exercise_scope_controls()
        # 扩展真实交换格式 → 本机安装副本 → Vault；没有使用浏览器隐藏接口或模拟模型。
        conversation = json.loads((ROOT / "extension/tests/fixtures/conversation.json").read_text())
        extension_id = "abcdefghijklmnopabcdefghijklmnop"
        install = self.cli_command("native-install", "--scope", "personal", "--capture-scope", "personal", "--extension-id", extension_id, "--output-dir", str(self.temporary / "native-conversation"))
        def native_call(action, arguments):
            request = json.dumps({"protocol":"recallcard.action/1","request_id":f"native-{action}","nonce":"synthetic-capture-nonce","session_ref":"deepseek:synthetic-demo","action":action,"arguments":arguments}).encode()
            import sys
            output = subprocess.run([install["launcher"], f"chrome-extension://{extension_id}/"], input=len(request).to_bytes(4, sys.byteorder)+request, capture_output=True, check=True, timeout=15).stdout
            size = int.from_bytes(output[:4], sys.byteorder)
            assert size == len(output)-4
            value = json.loads(output[4:])
            assert value["ok"], value
            return value["result"]
        connection = native_call("connection", {})
        assert connection["capture_enabled"] and connection["capture_scope"] == "personal"
        capture_preview = native_call("capture_preview", {"conversation":conversation})
        before_capture = len(self.events())
        saved = native_call("capture_save", {"conversation":conversation,"approval_hash":capture_preview["approval_hash"]})
        assert saved["events_added"] == 2 and len(self.events()) == before_capture + 2
        assert native_call("capture_save", {"conversation":conversation,"approval_hash":capture_preview["approval_hash"]})["events_added"] == 0
        assert native_call("search", {"query":"离线会话","budget_tokens":8000})["results"]
        browser.navigate("会话与接续")
        browser.click("//button[contains(@class,'result-card')][.//strong[contains(text(),'合成验收')]]", "xpath")
        browser.idle()
        assert "保存原始消息和来源" in browser.text(".conversation-layout")
        assert "时间未知" in browser.text(".conversation-message")
        browser.open_continuation()
        browser.type("textarea[aria-label='接下来要做什么']", "继续实现已经确认的决定")
        browser.button("准备交接内容")
        browser.idle()
        handoff = browser.observe("return document.querySelector('textarea[aria-label=\"交接内容预览\"]').value")
        assert "recallcard.context/1" in handoff and "下一步测试导入幂等" in handoff
        assert "event:" in handoff and "原始时间未知" in handoff
        browser.button("复制交接内容")
        browser.idle()  # 等待复制前的异步权限/内容复核完成，再读真实剪贴板
        copied = run("xclip", "-selection", "clipboard", "-o")
        assert copied == handoff, "真实桌面剪贴板必须与预览完全一致"
        self.checkpoint("可见会话直接保存并跨AI接续")
        browser.navigate("连接与状态")
        browser.button("生成客户端配置")
        browser.idle()
        client_config = json.loads(browser.observe("return document.querySelector('textarea[aria-label=\"MCP客户端配置\"]').value"))
        server = client_config["mcpServers"]["recallcard"]
        assert Path(server["command"]).is_file()
        mcp_request = "\n".join(json.dumps(message) for message in [
            {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"RecallCard原生验收","version":"0.2"}}},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"离线会话","budget_tokens":8000}}},
        ]) + "\n"
        mcp = subprocess.run([server["command"], *server["args"]], input=mcp_request, capture_output=True, text=True, check=True, timeout=15)
        assert "离线会话" in mcp.stdout and "event:" in mcp.stdout
        self.checkpoint("GUI生成的持久MCP组件读取刚保存会话")
        background_evidence = self.exercise_background_selection(edited_memory, server)
        workspace_evidence = self.exercise_workspace_usability()
        deepseek_evidence = self.exercise_deepseek_import()
        selected_recovery_evidence = self.exercise_selected_context_and_restart(deepseek_evidence)
        doctor = self.cli_command("doctor")
        assert doctor["ok"], doctor
        (self.artifacts / "canonical-evidence.json").write_text(json.dumps({
            "events": self.events(), "memory": memory, "edited_memory": edited_memory,
            "visibility_rules": visibility_rules, "receipt": receipt, "doctor": doctor, "job": job,
            "background_selection": background_evidence, "workspace_usability": workspace_evidence,
            "deepseek_import": deepseek_evidence,
            "selected_context_and_restart": selected_recovery_evidence,
            "scope_transition": self.scope_transition_evidence,
            "default_workspace": default_workspace_evidence,
        }, ensure_ascii=False, indent=2))

    def exercise_workspace_usability(self):
        browser = self.driver
        archive = self.create_workspace_fixture()
        before = len(self.events())
        browser.navigate("添加资料")
        browser.button("选择文件并预览")
        self.dialog("选择要导入的对话文件", archive)
        assert browser.observe("return document.querySelectorAll('.archive-selection input[type=checkbox]').length") == 53
        browser.button("选择全部可导入会话")
        browser.button("预览所选会话")
        browser.idle()
        self.confirm_import(106)
        assert len(self.events()) == before + 106
        assert browser.observe("return Boolean(document.querySelector('.conversation-list'))"), "保存后应直接进入本批会话成果"
        assert not browser.observe("return Boolean(document.querySelector('.archive-selection'))"), "保存后不应留在旧导入清单"
        assert browser.observe("return [...document.querySelectorAll('#navigation [data-workspace]')].map(n=>n.dataset.workspace).join('|')") == "conversations|memories"
        visible = browser.observe("return (()=>{const list=document.querySelector('.conversation-list').getBoundingClientRect();return [...document.querySelectorAll('.conversation-list .result-card')].filter(n=>{const r=n.getBoundingClientRect();return r.height>0&&r.top>=Math.max(0,list.top)&&r.bottom<=Math.min(innerHeight,list.bottom)&&r.left>=Math.max(0,list.left)&&r.right<=Math.min(innerWidth,list.right);}).length;})()")
        assert visible >= 8, f"默认窗口列表首屏应至少显示8行，实际{visible}行"
        self.checkpoint("五十三会话导入成果与双工作区可读列表")

        wanted = "工作区验收·53"
        for _ in range(2):
            found = browser.observe("return [...document.querySelectorAll('.conversation-list .result-card')].some(n=>n.textContent.includes('工作区验收·53'))")
            if found:
                break
            browser.button("更多会话")
            browser.idle()
        assert found, "导入的长会话必须可由列表访问"
        browser.click("//button[contains(@class,'result-card')][.//strong[contains(text(),'工作区验收·53')]]", "xpath")
        browser.idle()
        header = browser.observe("return (()=>{const n=document.querySelector('[data-action=open-continuation]');if(!n)return null;const r=n.getBoundingClientRect();return {top:r.top,bottom:r.bottom,visible:r.height>0&&r.top>=0&&r.bottom<=innerHeight};})()")
        assert header and header["visible"], "长消息不应把接续主操作推到首屏之外"
        selected = browser.observe("return document.querySelector('.conversation-list .selected')?.textContent")
        scroll = browser.observe("return document.querySelector('.conversation-list').scrollTop")
        browser.navigate("记忆管理")
        browser.navigate("会话与接续")
        assert browser.observe("return document.querySelector('.conversation-list .selected')?.textContent") == selected
        assert abs(browser.observe("return document.querySelector('.conversation-list').scrollTop") - scroll) < 3
        browser.open_continuation()
        browser.type("textarea[aria-label='接下来要做什么']", "核对紫杉项目原话，再继续整理；保留当前阅读位置")
        browser.button("准备交接内容")
        browser.idle()
        text = browser.observe("return document.querySelector('textarea[aria-label=交接内容预览]').value")
        assert "recallcard.context/1" in text and "核对紫杉项目原话" in text
        browser.button("复制交接内容")
        browser.idle()
        assert run("xclip", "-selection", "clipboard", "-o") == text
        self.checkpoint("长正文固定接续操作与浏览状态恢复")
        browser.click('[data-action="close-continuation"]')
        browser.idle()
        assert browser.observe("return document.querySelector('.conversation-list .selected')?.textContent") == selected
        browser.type('#query', '合成任务 17')
        browser.button('查找')
        browser.idle()
        # BM25 可同时返回包含共同词的相关记录；验收目标是确切原话可检索并可追溯，
        # 不将相关检索错误地当成只返回一个结果的精确等值查询。
        expected_events = [event for event in self.events()
                           if event['source']['conversation_id'] == 'workspace-density-17' and event['role'] == 'user']
        assert len(expected_events) == 1
        expected_event = expected_events[0]
        search_reference = f"event:{expected_event['id']}"
        search_selector = f'.results .result-card[data-reference="{search_reference}"]'
        assert browser.observe(f"return document.querySelectorAll({json.dumps(search_selector)}).length") == 1
        assert '用户原话' in browser.text(search_selector)
        assert 'chatgpt-export' in browser.text(search_selector)
        browser.click(search_selector)
        browser.idle()
        assert browser.observe("return document.querySelector('.results .result-card.selected').dataset.reference") == search_reference
        assert browser.observe("return document.querySelector('.reading-pane .body-text').textContent") == expected_event['content']
        browser.button('查看相邻消息')
        browser.idle()
        assert browser.observe("return document.querySelector('.located-message').dataset.reference") == search_reference
        assert '合成任务 17：紫杉项目' in browser.text('.located-message .body-text')
        browser.button('返回之前的阅读')
        browser.idle()
        assert browser.observe("return document.querySelector('#query').value") == '合成任务 17'
        assert browser.observe("return document.querySelector('.results .result-card.selected').dataset.reference") == search_reference
        self.checkpoint('搜索原话定位上下文并恢复阅读选择')
        browser.command("POST", "/window/rect", {"width": 860, "height": 820})
        browser.idle()
        assert browser.observe("return document.documentElement.scrollWidth <= innerWidth + 2"), "窄窗口不得产生整页横向溢出"
        browser.click('[data-action="back-to-list"]')
        browser.idle()
        assert browser.observe("return document.querySelector('.results.list-scroll').getBoundingClientRect().height > 0")
        browser.click(search_selector)
        browser.idle()
        assert browser.observe("return document.querySelector('.reading-pane').getBoundingClientRect().height > 0")
        browser.open_continuation()
        assert browser.observe("return document.querySelector('#continuation-panel').getBoundingClientRect().height > 0")
        browser.click('[data-action="close-continuation"]')
        browser.idle()
        assert browser.observe("return document.querySelector('.located-message').dataset.reference") == search_reference
        assert browser.observe("return (()=>{const r=document.querySelector('[data-action=open-continuation]').getBoundingClientRect();return r.height>0&&r.top>=0&&r.bottom<=innerHeight;})()")
        self.checkpoint("窄窗口保留当前阅读任务")
        browser.command("POST", "/window/rect", {"width": 1180, "height": 820})
        return {"synthetic_conversations": 53, "synthetic_messages": 106,
                "default_visible_rows": visible, "fixed_primary_action": header,
                "selected_restored": True, "copied_exact_preview": True,
                "source_location_restored": search_reference}

    def exercise_deepseek_import(self):
        browser = self.driver
        paths = self.create_deepseek_fixtures()
        before = self.events()
        self.preview_deepseek_files(paths)
        assert self.events() == before, "多文件选择和预览不得写入 Event"
        browser.button("取消", "//section[contains(@class,'import-job')]")
        browser.idle()
        assert self.events() == before, "取消多文件预览不得写入 Event"
        assert not browser.observe("return [...document.querySelectorAll('.import-job button')].some(n=>n.textContent.startsWith('导入全部 '))")
        self.checkpoint("DeepSeek原生双文件选择预览取消不写入")

        self.preview_deepseek_files(paths)
        completion = self.confirm_import_job(6)
        after = self.events()
        assert len(after) == len(before) + 6
        events = self.verify_deepseek_events(after)
        counts = self.import_job_counts()
        assert counts == {"导出文件": "2", "会话": "2", "新增记录": "6", "已存在记录": "0"}, counts
        self.checkpoint("DeepSeek一次批准后台完成保留分支角色时间")

        self.preview_deepseek_files(paths)
        repeated = self.confirm_import_job(6)
        assert self.events() == after, "多文件重复导入不得改变任何 canonical Event"
        duplicate_counts = self.import_job_counts()
        assert duplicate_counts == {"导出文件": "2", "会话": "2", "新增记录": "0", "已存在记录": "6"}, duplicate_counts
        browser.button("查看本批会话")
        browser.idle()
        assert "本批导入" in browser.text(".import-batch-summary")
        assert "新增 0 条" in browser.text(".import-batch-summary")
        assert browser.observe("return document.querySelectorAll('.conversation-list .result-card').length") == 2, "重复导入后只展示本批两个会话"
        browser.click(f"//button[contains(@class,'result-card')][.//strong[text()='{DEEPSEEK_TITLE}']]", "xpath")
        browser.idle()
        rows = browser.observe("return [...document.querySelectorAll('.conversation-message')].map(n=>({ref:n.dataset.reference,text:n.querySelector('.body-text').textContent,role:n.querySelector('.message-role').textContent}))")
        assert len(rows) == 4
        for key in ("q", "a", "b", "separate"):
            assert {"ref": f"event:{events[key]['id']}", "text": DEEPSEEK_TEXTS[key],
                    "role": "用户原话" if events[key]["role"] == "user" else "AI 回复"} in rows
        unknown = f'.conversation-message[data-reference="event:{events["b"]["id"]}"]'
        assert "时间未知" in browser.text(unknown)
        assert "已保留 3 条分支" in browser.text(".conversation-reader")
        self.checkpoint("DeepSeek批量重复去重与完整原文阅读")

        browser.open_continuation()
        assert browser.observe("return document.querySelector('#continuation-branch').value") == ""
        assert browser.observe("return [...document.querySelectorAll('#continuation-panel button')].find(n=>n.textContent==='准备交接内容').disabled"), "多分支不能默认猜选"
        branch_b = f"event:{events['b']['id']}"
        browser.select_keyboard("#continuation-branch", branch_b)
        handoffs = {}
        for key in ("b", "a"):
            if key == "a":
                browser.click('[data-action="close-continuation"]')
                browser.idle()
                browser.click(f'.conversation-message[data-reference="event:{events[key]["id"]}"] .branch-end')
                browser.idle()
                assert browser.observe("return document.querySelector('#continuation-branch').value") == f"event:{events[key]['id']}"
                assert not browser.observe("return Boolean(document.querySelector('#continuation-preview'))"), "换分支必须清除旧交接内容"
            browser.button("准备交接内容")
            browser.idle()
            handoff = browser.observe("return document.querySelector('textarea[aria-label=交接内容预览]').value")
            assert f"分支末端：event:{events[key]['id']}" in handoff
            assert "本次带上 2 / 2 条已选范围内的已保存消息" in handoff
            assert DEEPSEEK_TEXTS["q"] in handoff and DEEPSEEK_TEXTS[key] in handoff
            assert handoff.index(DEEPSEEK_TEXTS["q"]) < handoff.index(DEEPSEEK_TEXTS[key])
            assert f"event:{events['q']['id']}" in handoff
            for other in set(DEEPSEEK_TEXTS) - {"q", key}:
                assert DEEPSEEK_TEXTS[other] not in handoff, "接续不得拼入兄弟分支、独立根或其他会话"
                assert f"event:{events[other]['id']}" not in handoff
            for forbidden in (DEEPSEEK_HIDDEN, DEEPSEEK_ATTACHMENT, DEEPSEEK_ATTACHMENT_URL):
                assert forbidden not in handoff
            if key == "b":
                assert "原始时间未知" in handoff
            browser.button("复制交接内容")
            browser.idle()
            assert run("xclip", "-selection", "clipboard", "-o") == handoff, "复制前重新核验仍必须使用已选择的具体分支"
            handoffs[key] = handoff
        assert self.events() == after, "阅读和接续不得改写原始事件"
        self.checkpoint("DeepSeek原生分支选择与剪贴板不混入兄弟分支")
        return {"files": [path.name for path in paths], "completion": completion, "counts": counts,
                "duplicate_completion": repeated, "duplicate_counts": duplicate_counts,
                "events": list(events.values()), "handoffs": handoffs,
                "keyboard_branch_ref": branch_b, "copied_exact_previews": True}

    def exercise_selected_context_and_restart(self, deepseek_evidence):
        browser = self.driver
        before_events, before_memories = self.events(), self.canonical_memories()
        events = self.verify_deepseek_events(before_events)
        wanted = [f"event:{events[key]['id']}" for key in ["q", "b", "second-q"]]
        goal = "合并石榴和海棠项目的原话证据，区分未确认建议后继续"
        browser.navigate("查找与阅读")
        browser.type("#query", "DeepSeek")
        browser.button("查找")
        browser.idle()
        browser.button("带走这些资料")
        browser.idle()
        selected = browser.choose_context_references(wanted)
        browser.type('#selection-context-panel textarea[aria-label="接下来要做什么"]', goal)
        browser.button("准备交接内容")
        browser.idle()
        browser.click('#selection-context-preview .continuation-text > summary')
        text = browser.observe('return document.querySelector(\'#selection-context-panel textarea[aria-label="交接内容预览"]\').value')
        assert text.startswith("recallcard.context/1\n# 选定资料交接\n") and goal in text
        for key in ["q", "b", "second-q"]:
            assert DEEPSEEK_TEXTS[key] in text and f"event:{events[key]['id']}" in text
        for key in ["a", "separate", "second-a"]:
            assert DEEPSEEK_TEXTS[key] not in text, "未选中的其他回答或会话原话不可混入"
        for forbidden in [DEEPSEEK_HIDDEN, DEEPSEEK_ATTACHMENT, DEEPSEEK_ATTACHMENT_URL]:
            assert forbidden not in text
        pane = browser.text("#selection-context-panel")
        assert "用户原话" in pane and "AI 回复" in pane and "时间未知" in pane
        assert browser.observe("return [...document.querySelectorAll('.selected-context-record')].map(n=>n.dataset.contextReference)") == selected
        browser.button("复制交接内容")
        browser.idle()
        assert run("xclip", "-selection", "clipboard", "-o") == text
        assert self.events() == before_events and self.canonical_memories() == before_memories
        self.checkpoint("跨会话选定三条原文并逐字复制不混入未选内容")

        inspect = f'.selected-context-record[data-context-reference="{wanted[0]}"] button'
        browser.click(inspect)
        browser.idle()
        assert browser.observe("return !document.querySelector('#selection-context-panel')")
        browser.button("查看相邻消息")
        browser.idle()
        assert browser.observe("return document.querySelector('.located-message').dataset.reference") == wanted[0]
        browser.button("返回之前的阅读")
        browser.idle()
        assert browser.observe("return document.querySelector('#query').value") == "DeepSeek"
        browser.button("带走这些资料")
        browser.idle()
        assert browser.observe("return [...document.querySelectorAll('.context-check input:checked')].map(n=>n.dataset.selectionReference)") == selected
        assert browser.observe('return document.querySelector(\'#selection-context-panel textarea[aria-label="接下来要做什么"]\').value') == goal
        browser.command("POST", "/window/rect", {"width": 820, "height": 620})
        browser.idle()
        actual_window = browser.command("GET", "/window/rect")
        actual_viewport = browser.observe("return {width:innerWidth,height:innerHeight}")
        assert 0 < actual_viewport["width"] <= 822 and 0 < actual_viewport["height"] <= 622, f"缩窗请求没有得到目标大小的实际内容区：window={actual_window}, viewport={actual_viewport}"
        assert browser.observe("return document.documentElement.scrollWidth <= innerWidth + 2")
        browser.button("调整所选资料")
        browser.idle()
        assert browser.observe("return document.querySelector('.results.list-scroll').getBoundingClientRect().height > 0")
        browser.button("带走这些资料")
        browser.idle()
        browser.button("复制交接内容")
        browser.idle()
        assert run("xclip", "-selection", "clipboard", "-o") == text
        self.checkpoint("核对相邻原话后恢复选择目标并在最小窗口再次复制")
        # 故意留下两条选择、手写目标和已生成预览后结束应用会话。
        # 两条与新搜索默认的前三条不同，避免无差别的断言假称验证了清草稿。
        browser.button("调整所选资料")
        browser.idle()
        abandoned_refs = browser.choose_context_references([wanted[0], wanted[2]])
        browser.button("带走这些资料")
        browser.idle()
        assert len(abandoned_refs) == 2
        assert browser.observe('return document.querySelector(\'#selection-context-panel textarea[aria-label="接下来要做什么"]\').value') == goal
        assert browser.observe("return Boolean(document.querySelector('#selection-context-preview'))")
        assert self.events() == before_events and self.canonical_memories() == before_memories

        browser = self.reopen_application()
        browser.assert_vault_badge(self.vault.name)
        assert browser.text("#location") == "会话"
        assert browser.observe("return Boolean(document.querySelector('.conversation-message'))")
        assert not browser.observe("return Boolean(document.querySelector('#selection-context-panel'))")
        browser.navigate("查找与阅读")
        browser.type("#query", "DeepSeek")
        browser.button("查找")
        browser.idle()
        browser.button("带走这些资料")
        browser.idle()
        reset_selection = browser.observe("return {all:[...document.querySelectorAll('.context-check input')].map(n=>n.dataset.selectionReference),checked:[...document.querySelectorAll('.context-check input:checked')].map(n=>n.dataset.selectionReference),goal:document.querySelector('#selection-context-panel textarea[aria-label=\"接下来要做什么\"]').value,preview:document.querySelector('#selection-context-panel textarea[aria-label=\"交接内容预览\"]').value}")
        assert reset_selection["checked"] == reset_selection["all"][:3] and len(reset_selection["checked"]) == 3
        assert reset_selection["checked"] != abandoned_refs
        assert reset_selection["goal"] == "DeepSeek" and goal not in reset_selection["preview"]
        assert self.events() == before_events and self.canonical_memories() == before_memories
        self.checkpoint("结束会话后重新启动自动恢复资料库且新搜索不恢复旧草稿")
        browser.navigate("添加资料")
        browser.button("查看导入记录")
        browser.idle()
        browser.click(".import-history-row button")
        browser.idle()
        assert browser.text(".import-job h3") == "导入完成"
        browser.button("查看本批会话")
        browser.idle()
        assert "新增 0 条" in browser.text(".import-batch-summary")
        assert browser.observe("return document.querySelectorAll('.conversation-list .result-card').length") == 2
        assert self.events() == before_events and self.canonical_memories() == before_memories
        self.checkpoint("重启后从已完成导入记录找回本批会话且重复导入不新增")
        return {"selected_refs": selected, "copied_text": text, "return_goal": goal,
                "requested_window": {"width": 820, "height": 620}, "actual_window": actual_window,
                "actual_viewport": actual_viewport, "restarted": True,
                "abandoned_refs": abandoned_refs, "fresh_refs": reset_selection["checked"],
                "canonical_records_unchanged": True, "batch_conversations": 2,
                "source_batch_files": deepseek_evidence["files"]}

    def close(self):
        if self.driver:
            try:
                self.driver.close()
            except Exception as error:
                print(f"关闭 WebDriver：{error}", flush=True)
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        for stream in self.logs:
            stream.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--application", type=Path,
                        default=ROOT / "desktop/src-tauri/target/debug/recallcard-desktop")
    parser.add_argument("--cli", type=Path,
                        default=ROOT / "desktop/src-tauri/target/debug/recallcard")
    parser.add_argument("--artifacts", type=Path,
                        default=Path(tempfile.gettempdir()) / "recallcard-native-smoke-artifacts")
    args = parser.parse_args()
    for name in ["WebKitWebDriver", "xdotool", "xclip", "scrot", "openbox"]:
        if not shutil.which(name):
            parser.error(f"缺少官方测试依赖：{name}")
    for binary in [args.application, args.cli]:
        if not binary.is_file():
            parser.error(f"请先编译：{binary}")
    if not os.environ.get("DISPLAY"):
        parser.error("请在 Xvfb 或正常 X11 会话中运行")
    try:
        import pyatspi  # noqa: F401
    except ImportError:
        parser.error("请安装 python3-pyatspi 并使用 /usr/bin/python3 运行")
    with tempfile.TemporaryDirectory(prefix="recallcard-native-smoke-") as directory:
        temporary = Path(directory)
        # 独立本机状态目录不会碰触开发者已有的状态、授权或资料。
        os.environ["RECALLCARD_STATE_DIR"] = str(temporary / "state")
        os.environ["XDG_DATA_HOME"] = str(temporary / "data")
        os.environ["XDG_CONFIG_HOME"] = str(temporary / "config")
        smoke = NativeSmoke(args, temporary)
        success = False
        try:
            smoke.exercise()
            success = True
        except Exception:
            detail = traceback.format_exc()
            print(detail, flush=True)
            (smoke.artifacts / "failure.txt").write_text(detail)
            smoke.capture("failure")
            if smoke.driver:
                try:
                    (smoke.artifacts / "failure-page.html").write_text(
                        smoke.driver.command("GET", "/source"))
                except Exception:
                    pass
            raise
        finally:
            (smoke.artifacts / "summary.json").write_text(json.dumps({
                "success": success, "passed_steps": smoke.steps,
                "driver_observations": getattr(smoke.driver, 'observations', []),
                "application": str(smoke.application), "synthetic_data_only": True,
            }, ensure_ascii=False, indent=2))
            smoke.close()


if __name__ == "__main__":
    main()
