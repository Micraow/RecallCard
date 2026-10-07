#!/usr/bin/env python3
"""真实 Tauri/WebKitGTK 冒烟回归：仅使用临时合成数据，不替换 invoke。

在已安装官方 tauri-driver、WebKitWebDriver、xdotool、scrot、openbox 的
Linux 上运行：dbus-run-session -- xvfb-run -a python3 desktop/tests/native_smoke.py
应用按钮使用 W3C WebDriver；系统文件选择窗口使用正常 X11 键盘操作。
"""

import argparse
import base64
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time
import traceback
from urllib.error import HTTPError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[2]
ELEMENT = "element-6066-11e4-a52e-4f735466cecf"
EVENT_TEXT = "合成资料：native-smoke 水星项目的说明使用简洁中文。"
MEMORY_TEXT = "合成记忆：native-smoke 水星项目偏好简洁中文说明。"


class DriverError(RuntimeError):
    pass


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
                "tauri:options": {"application": str(application)},
            }},
        }, timeout=60)
        self.session = result["sessionId"]

    def request(self, method, path, data=None, timeout=15):
        body = None if data is None else json.dumps(data).encode()
        request = Request(self.address + path, data=body, method=method,
                          headers={"Content-Type": "application/json"})
        try:
            with urlopen(request, timeout=timeout) as response:
                result = json.load(response)
        except HTTPError as error:
            raise DriverError(error.read().decode()) from error
        value = result.get("value")
        if isinstance(value, dict) and "error" in value:
            raise DriverError(json.dumps(value, ensure_ascii=False))
        return value

    def command(self, method, path, data=None):
        return self.request(method, f"/session/{self.session}{path}", data)

    def find(self, selector, using="css selector"):
        return self.command("POST", "/element", {"using": using, "value": selector})[ELEMENT]

    def text(self, selector):
        return self.command("GET", f"/element/{self.find(selector)}/text")

    def click(self, selector, using="css selector"):
        element = wait_for(lambda: self.find(selector, using), f"找到按钮 {selector}")
        self.command("POST", f"/element/{element}/click", {})

    def button(self, label, container=""):
        # 测试中的中文标签没有引号；由浏览器正常派发点击，不调用业务方法。
        self.click(f"{container}//button[normalize-space(.)='{label}']", "xpath")

    def type(self, selector, value):
        element = self.find(selector)
        self.command("POST", f"/element/{element}/clear", {})
        self.command("POST", f"/element/{element}/value", {"text": value})

    def observe(self, script):
        # 仅观察 DOM 状态和错误；不注入后端、设置会话或绕过文件选择窗口。
        return self.command("POST", "/execute/sync", {"script": script, "args": []})

    def idle(self):
        wait_for(lambda: self.text("#operation") == "准备就绪", "本机操作结束")
        error = self.observe("return document.querySelector('#notice.error:not([hidden])')?.textContent")
        if error:
            raise AssertionError(f"应用显示错误：{error}")

    def navigate(self, label):
        # 导航按钮包含装饰图标，因此匹配最后一个文本节点。
        self.click(f"//nav[@id='navigation']/button[text()='{label}']", "xpath")
        self.idle()
        assert self.text("#location") == label

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
        self.application = args.application.resolve()

    def checkpoint(self, name):
        print(f"通过：{name}", flush=True)
        self.steps.append(name)
        self.capture(f"{len(self.steps):02d}-{name}")

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

    def start(self, executable, logfile):
        stream = (self.artifacts / logfile).open("w")
        self.logs.append(stream)
        process = subprocess.Popen(executable, stdout=stream, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        self.processes.append(process)
        return process

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

    def dialog(self, title, path=None, save=False):
        window = wait_for(lambda: self.dialog_windows(title), f"原生窗口：{title}")[0]
        run("xdotool", "windowactivate", "--sync", window)
        self.capture(f"dialog-{len(self.steps):02d}-{title}", webview=False)
        if path is None:
            run("xdotool", "key", "--clearmodifiers", "Escape")
        else:
            # 只通过文件选择窗口向应用授予本次合成路径访问权。
            run("xdotool", "key", "--clearmodifiers", "ctrl+l")
            run("xdotool", "type", "--clearmodifiers", "--delay", "1", str(path))
            run("xdotool", "key", "--clearmodifiers", "Return")
            time.sleep(0.7)
            if self.dialog_windows(title):
                # GTK 在目录导航后可能仍等待确认，保存窗口使用其正常保存快捷键。
                run("xdotool", "key", "--clearmodifiers", "alt+s" if save else "Return")
        wait_for(lambda: not self.dialog_windows(title), f"关闭原生窗口：{title}")
        self.driver.idle()

    def preview_import(self, source):
        self.driver.button("选择文件并预览")
        self.dialog("选择要导入的对话文件", source)
        assert EVENT_TEXT in self.driver.text(".file-preview")

    def confirm_import(self):
        self.driver.button("确认导入 1 条记录")
        self.driver.button("确认导入", "//dialog[@id='modal']")
        self.driver.idle()

    def exercise(self):
        self.cli_command("init")
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
        self.start(["openbox"], "window-manager.log")
        server = self.start(["tauri-driver"], "tauri-driver.log")

        def ready():
            if server.poll() is not None:
                raise AssertionError("tauri-driver 启动失败，请检查 tauri-driver.log")
            with socket.create_connection(("127.0.0.1", 4444), timeout=1):
                return True

        wait_for(ready, "WebDriver 启动")
        self.driver = WebDriver(self.application)
        browser = self.driver
        wait_for(lambda: browser.text("h1") == "把散落的想法，找回来。", "真实 WebKit 首页")
        browser.idle()
        assert browser.observe("return !!window.__TAURI__?.core?.invoke")
        self.checkpoint("真实桌面首页")

        browser.button("打开已有资料库")
        self.dialog("打开已有 RecallCard 资料库")
        assert browser.text("#vault-badge") == "尚未打开资料库"
        browser.button("打开已有资料库")
        self.dialog("打开已有 RecallCard 资料库", self.vault)
        assert browser.text("#vault-badge") == self.vault.name
        browser.click("#switch-vault")
        browser.button("打开已有资料库", "//dialog[@id='modal']")
        self.dialog("打开已有 RecallCard 资料库")
        assert browser.text("#vault-badge") == self.vault.name
        self.checkpoint("原生资料库选择与取消")

        browser.navigate("导入资料")
        browser.button("选择文件并预览")
        self.dialog("选择要导入的对话文件")
        assert not self.events()
        self.preview_import(source)
        assert not self.events(), "预览不得写入 Event"
        browser.button("确认导入 1 条记录")
        browser.button("取消", "//dialog[@id='modal']")
        assert not self.events(), "取消确认不得写入 Event"
        self.checkpoint("导入预览与取消不写入")
        self.confirm_import()
        events = self.events()
        assert len(events) == 1 and events[0]["content"] == EVENT_TEXT
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
        self.checkpoint("中文检索与完整原文")

        browser.navigate("整理记忆")
        job_path = self.temporary / "synthetic-dream-job.json"
        browser.button("导出本次来源包")
        self.dialog("保存本次整理的来源包", job_path, save=True)
        job = json.loads(job_path.read_text())
        assert job["source_refs"][0]["event"]["content"] == EVENT_TEXT
        source_ref = job["source_refs"][0]["ref"]
        result_path = self.temporary / "synthetic-dream-result.json"
        result_path.write_text(json.dumps({
            "schema": "recallcard.dream-result/1", "job_id": job["job_id"],
            "input_hash": job["input_hash"], "proposals": [{
                "operation": "add", "scope": "personal", "content": MEMORY_TEXT,
                "source_refs": [source_ref], "evidence": "user_explicit",
            }],
        }, ensure_ascii=False))
        browser.button("选择结果并审阅")
        self.dialog("选择 Dream 整理结果", result_path)
        assert MEMORY_TEXT in browser.text(".change .after")
        assert not self.memories(), "Dream 审阅不得写入 Memory"
        browser.button("确认发布本次变更")
        browser.button("取消", "//dialog[@id='modal']")
        assert not self.memories(), "取消发布不得写入 Memory"
        self.checkpoint("来源导出与Dream审阅取消")

        browser.button("确认发布本次变更")
        browser.button("确认发布", "//dialog[@id='modal']")
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
        self.dialog("选择 Dream 整理结果", result_path)
        assert "已经发布" in browser.text("#content")
        assert not browser.observe("return [...document.querySelectorAll('button')].some(b => b.textContent === '确认发布本次变更')")
        assert len(self.memories()) == 1
        browser.navigate("查找与阅读")
        browser.click("//button[contains(@class,'result-card')][.//span[text()='长期记忆']]", "xpath")
        browser.idle()
        assert browser.text(".reading-pane .body-text") == MEMORY_TEXT
        assert EVENT_TEXT in browser.text(".reading-pane .sample")
        doctor = self.cli_command("doctor")
        assert doctor["ok"], doctor
        self.checkpoint("长期记忆出处与发布防重放")
        (self.artifacts / "canonical-evidence.json").write_text(json.dumps({
            "events": events, "memory": memory, "receipt": receipt, "doctor": doctor,
            "job": job,
        }, ensure_ascii=False, indent=2))

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
    for name in ["tauri-driver", "WebKitWebDriver", "xdotool", "scrot", "openbox"]:
        if not shutil.which(name):
            parser.error(f"缺少官方测试依赖：{name}")
    for binary in [args.application, args.cli]:
        if not binary.is_file():
            parser.error(f"请先编译：{binary}")
    if not os.environ.get("DISPLAY"):
        parser.error("请在 Xvfb 或正常 X11 会话中运行")
    with tempfile.TemporaryDirectory(prefix="recallcard-native-smoke-") as directory:
        temporary = Path(directory)
        # 独立本机状态目录不会碰触开发者已有的状态、授权或资料。
        os.environ["RECALLCARD_STATE_DIR"] = str(temporary / "state")
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
                "application": str(smoke.application), "synthetic_data_only": True,
            }, ensure_ascii=False, indent=2))
            smoke.close()


if __name__ == "__main__":
    main()
