#!/usr/bin/env python3
"""v0.6 首次体验的真实 Tauri/WebKitGTK 验收。

只使用临时合成 ChatGPT 官方形状 ZIP，经真实原生文件选择器导入。
不替换 invoke，不预灌资料，不创建外部账号/隧道/模型凭据。
旧 native_smoke 的工具用于真实 WebDriver 和文件窗口；不执行其旧 UI 场景。
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import traceback
from urllib.request import urlopen
import zipfile

from native_smoke import ROOT, NativeSmoke, WebDriver, DriverError, wait_for, run

TITLE = "合成验收：琥珀报告的交付"
ORIGINAL = "合成原话：琥珀报告第三版已在周二交付。现在只需要修正图四的单位，不再撰写正文。"
UPDATE = "合成最新情况：琥珀报告图四已勘误，改为周四回访。"
DIALOG = "添加来源：选择官方导出文件"


class JourneyDriver(WebDriver):
    def idle(self):
        # 新 GUI 没有旧 CLI 风格的 operation 状态节点。
        wait_for(lambda: self.observe("return Boolean(document.querySelector('.app-shell')) && ![...document.querySelectorAll('.topbar button')].some(n=>n.disabled)"), "新 GUI 可交互")

    def button(self, label, container=""):
        self.click(f"{container}//button[normalize-space(.)='{label}']", "xpath")


class FirstUseJourney(NativeSmoke):
    def dialog_completed(self, title, next_title=None):
        if self.dialog_windows(title):
            return False
        return self.driver.observe("return Boolean(document.querySelector('.app-shell')) && ![...document.querySelectorAll('.topbar button')].some(n=>n.disabled)") is True

    def content(self, text):
        return text in self.driver.text("body")

    def assert_clean(self):
        error = self.driver.observe("return [...document.querySelectorAll('[role=alert]')].map(n=>n.textContent).join('\\n')")
        assert not error, f"界面显示错误：{error}"
        assert not self.driver.observe("return Boolean(document.querySelector('.demo-banner'))"), "原生不能加载合成mock入口"

    def create_archive(self):
        def message(mid, role, text, timestamp):
            return {"id": mid, "author": {"role": role}, "create_time": timestamp,
                    "content": {"content_type": "text", "parts": [text]}, "metadata": {}}
        record = {"id": "first-use-synthetic", "title": TITLE, "create_time": 1791241200,
                  "current_node": "u2", "mapping": {
            "root": {"id": "root", "parent": None, "children": ["u1"], "message": None},
            "u1": {"id": "u1", "parent": "root", "children": ["a1"], "message": message("u1", "user", "合成旧计划：琥珀报告仍在撰写，预计周二提交。", 1791241200)},
            "a1": {"id": "a1", "parent": "u1", "children": ["u2"], "message": message("a1", "assistant", "可以先核对图表单位。此句为 AI 建议。", 1791241260)},
            "u2": {"id": "u2", "parent": "a1", "children": [], "message": message("u2", "user", ORIGINAL, 1791327600)},
        }}
        path = self.temporary / "official-shaped-synthetic.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("conversations.json", json.dumps([record], ensure_ascii=False))
            archive.writestr("chat.html", "<html>合成导出视图，不应重复导入</html>")
        return path

    def loaded_home(self):
        return self.driver.observe("return document.querySelector('.context-excerpt')?.textContent || ''") and self.content(ORIGINAL)

    def exercise(self):
        archive = self.create_archive()
        self.start(["openbox"], "window-manager.log")
        server = self.start_native_driver()
        def ready():
            assert server.poll() is None, "WebKitWebDriver 已退出"
            with urlopen("http://127.0.0.1:4444/status", timeout=2) as response:
                return json.load(response).get("value", {}).get("ready") is True
        wait_for(ready, "原生驱动可用")
        self.driver = JourneyDriver(self.application)
        browser = self.driver
        wait_for(lambda: self.content("导入聊天记录"), "首次导入入口")
        assert not browser.observe("return Boolean(document.querySelector('.app-shell'))"), "必须从全新首次使用开始"
        self.checkpoint("首次只需导入不要求格式范围或模型")
        # Primary click opens default personal workspace and native chooser in one flow.
        browser.button("导入聊天记录")
        self.dialog(DIALOG, archive)
        wait_for(self.loaded_home, "真实导入后原话自动出现在首页", timeout=60)
        self.assert_clean()
        assert self.content("从这些原话继续") and not browser.observe("return Boolean(document.querySelector('.background-fact'))")
        assert not browser.observe("return Boolean(document.querySelector('dialog[open]'))"), "导入不应要求第二次批准"
        identifier = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
        self.vault = self.temporary / "data" / identifier / "vault"
        assert self.vault.resolve().is_relative_to(self.temporary.resolve())
        wait_for(lambda: len(self.events()) == 3, "同一本地正本实际保存3条消息")
        assert not self.memories(), "无模型导入不能伪造提炼记忆"
        assert all(event["scope"] == "personal" for event in self.events())
        original_events = self.events()
        self.checkpoint("官方形状ZIP经真实选择器导入后即刻阅读原话")

        wait_for(lambda: browser.observe("return Boolean(document.querySelector('.app-shell')?.dataset.buildVersion && document.querySelector('.app-shell')?.dataset.buildCommit)"), "GUI 实际构建身份就绪")
        gui_build = browser.observe("return (()=>{const d=document.querySelector('.app-shell').dataset;return {version:d.buildVersion,commit:d.buildCommit,dirty:d.buildDirty==='true'}})()")
        cli_status = json.loads(run(str(self.cli), "--vault", str(self.vault), "--json", "status"))
        cli_build = cli_status["result"]["build"]
        assert gui_build == {key: cli_build[key] for key in ["version", "commit", "dirty"]}, (gui_build, cli_build)
        assert gui_build["commit"] not in {"unknown", ""}
        (self.artifacts / "build-identity.json").write_text(json.dumps({"gui": gui_build, "cli": cli_build}, ensure_ascii=False, indent=2))

        browser.button("回到出处")
        wait_for(lambda: self.content("已定位到这条原话"), "精确定位原话")
        assert ORIGINAL in browser.text(".focused-message")
        assert "AI 助手" not in browser.text(".focused-message"), "原话不可冒用AI建议"
        self.checkpoint("从背景一键回到确切原话及原始时间")
        browser.button("背景")
        wait_for(self.loaded_home, "返回背景")
        browser.button("补充最新情况")
        browser.type("#latest-update", "这句取消，不应该保存。")
        browser.button("取消", "//dialog")
        assert self.events() == original_events
        browser.button("补充最新情况")
        browser.type("#latest-update", UPDATE)
        browser.button("保存近况", "//dialog")
        wait_for(lambda: self.content("已保存你的补充"), "一次保存新近况")
        wait_for(lambda: len(self.events()) == 4, "补充只追加一条原话")
        assert not self.memories(), "补充原话不伪称已提炼Memory"
        original_by_id = {event["id"]: event for event in original_events}
        saved_by_id = {event["id"]: event for event in self.events()}
        assert all(saved_by_id.get(key) == event for key, event in original_by_id.items())
        additions = [event for key, event in saved_by_id.items() if key not in original_by_id]
        assert len(additions) == 1 and additions[0]["content"] == UPDATE
        self.assert_clean()
        self.checkpoint("取消不写入且近况一次保存原话保持不变")
        browser.type("#global-search", "周四回访")
        search = browser.find("#global-search")
        browser.command("POST", f"/element/{search}/value", {"text": "\ue007"})
        wait_for(lambda: UPDATE in browser.text(".search-result"), "补充立即可被真实搜索找到")
        self.assert_clean()
        self.checkpoint("补充近况立即可搜索")
        result = self.mcp_tool({"command": str(self.cli), "args": ["--vault", str(self.vault), "mcp", "--scope", "personal"]}, "search", {"query": "周四回访", "budget_tokens": 2000})
        assert UPDATE in json.dumps(result, ensure_ascii=False)
        (self.artifacts / "same-vault-mcp.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
        self.checkpoint("真实stdioMCP从同一正本读到刚保存近况")

        browser.button("背景")
        wait_for(lambda: self.content("连接 ChatGPT"), "返回背景入口")
        browser.button("连接 ChatGPT")
        wait_for(lambda: self.content("本机读取待授权"), "本机只读配置检查完成")
        assert self.content("实际检索到的背景、原文和出处会提供给 OpenAI")
        assert self.content("ChatGPT 官方连接与真实读取尚未验证")
        self.assert_clean()
        self.checkpoint("ChatGPT真实宿主未授权保持待连接")
        browser.button("继续查看本机背景", "//dialog")

        browser.resize(820, 760)
        wait_for(lambda: browser.observe("return document.querySelectorAll('.context-conversation').length") >= 1, "820背景已加载")
        self.checkpoint("820窗口加载后的背景")
        browser.button("来源")
        wait_for(lambda: browser.observe("return document.querySelectorAll('.source-row').length") >= 1, "820来源列表已加载")
        browser.click(".source-row")
        wait_for(lambda: browser.observe("return Boolean(document.querySelector('.conversation-article h1'))"), "820来源详情已加载")
        self.checkpoint("820窗口已加载来源详情")
        browser.button("来源列表")
        wait_for(lambda: browser.observe("return document.querySelector('.collection-panel').getBoundingClientRect().width > 0"), "返回来源列表")
        assert browser.observe("return document.documentElement.scrollWidth <= innerWidth"), "不应横向溢出"
        self.checkpoint("820详情返回来源列表")
        before_restart = self.events()
        browser.close()
        self.driver = None
        wait_for(ready, "窗口关闭后驱动可重新启动")
        self.driver = JourneyDriver(self.application)
        browser = self.driver
        wait_for(lambda: browser.observe("return Boolean(document.querySelector('.app-shell'))"), "恢复已有本地资料")
        wait_for(lambda: browser.observe("return document.querySelectorAll('.context-conversation').length") >= 1, "恢复后背景已加载")
        assert self.events() == before_restart
        self.assert_clean()
        self.checkpoint("重启恢复已有背景不要求重新导入")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--application", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    for name in ["WebKitWebDriver", "xdotool", "xclip", "scrot", "openbox"]:
        if not shutil.which(name): parser.error(f"缺少官方验收依赖：{name}")
    if not os.environ.get("DISPLAY"): parser.error("请在独立 Xvfb 会话内运行")
    import pyatspi  # noqa: F401 - ensure accessible native file chooser support
    for binary in [args.application, args.cli]:
        if not binary.is_file(): parser.error(f"请先构建同快照程序：{binary}")
    with tempfile.TemporaryDirectory(prefix="recallcard-first-use-") as directory:
        temporary = Path(directory)
        for name, suffix in [("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("RECALLCARD_STATE_DIR", "state")]:
            os.environ[name] = str(temporary / suffix)
        smoke = FirstUseJourney(args, temporary)
        success = False
        try:
            smoke.exercise()
            success = True
        except Exception:
            detail = traceback.format_exc()
            (smoke.artifacts / "failure.txt").write_text(detail)
            smoke.capture("failure")
            if smoke.driver:
                try: (smoke.artifacts / "failure-page.html").write_text(smoke.driver.command("GET", "/source"))
                except Exception: pass
            raise
        finally:
            (smoke.artifacts / "summary.json").write_text(json.dumps({"success": success, "suite": "v0.6-first-use-native", "passed_steps": smoke.steps, "synthetic_data_only": True, "invoke_mocked": False, "external_chatgpt_verified": False}, ensure_ascii=False, indent=2))
            try:
                if smoke.vault.exists():
                    subprocess.run([str(smoke.cli), "--vault", str(smoke.vault), "--json", "service", "stop"], timeout=15, capture_output=True)
            finally:
                smoke.close()


if __name__ == "__main__":
    main()
