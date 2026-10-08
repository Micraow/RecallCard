#!/usr/bin/env python3
"""0.7 原生项目连接闭环：只操作临时合成项目，不执行 Codex/Claude。"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import traceback
from urllib.request import urlopen

from first_use_native import FirstUseJourney, JourneyDriver, DIALOG, ORIGINAL, UPDATE
from native_smoke import ROOT, wait_for, run


class ConnectedJourney(FirstUseJourney):
    def cli_result(self, *args):
        return json.loads(run(str(self.cli), "--vault", str(self.vault), "--json", *args))["result"]

    def project_dialog(self, project):
        title = "选择此 Agent 工作的项目目录"
        self.dialog_count += 1
        window = wait_for(lambda: self.dialog_windows(title), "真实项目目录选择器")[0]
        run("xdotool", "windowactivate", "--sync", window)
        wait_for(lambda: self.accessible_dialog(title), "项目目录辅助功能树可读")
        try:
            # Recent 是虚拟结果列表，正确粘贴路径也不代表已进入文件系统目录。
            # 与已验证的原生文件选择流程相同，先正常导航到 Home。
            run("xdotool", "key", "--clearmodifiers", "alt+Home")
            wait_for(lambda: self.native_button(title, "Open") if self.native_window_active(title, window) else None,
                     "退出Recent后真实目录的Open按钮可用", stable_reads=2)
            self.capture(f"dialog-{self.dialog_count:02d}-home-folder", webview=False)
            self.navigate_file_folder(title, window, project)
            def selection_ready():
                if self.dialog_completed(title):
                    return True
                return self.native_button(title, "Select") or self.native_button(title, "Open")
            button = wait_for(selection_ready, "目录已选中或真实确认按钮可用")
            if button is not True:
                assert button.queryAction().doAction(0), "目录确认未被接受"
            wait_for(lambda: self.dialog_completed(title), "项目选择器关闭")
        finally:
            if self.dialog_windows(title):
                self.describe_dialog(title, "after-navigation")

    def exercise(self):
        archive = self.create_archive()
        project = self.temporary / "synthetic-agent-project"
        project.mkdir()
        original_instruction = "# 合成项目约定\n请保留这份用户维护的规则。\n"
        (project / "AGENTS.md").write_text(original_instruction)
        self.start(["openbox"], "window-manager.log")
        server = self.start_native_driver()
        def ready():
            assert server.poll() is None, "WebKitWebDriver 已退出"
            with urlopen("http://127.0.0.1:4444/status", timeout=2) as response:
                return json.load(response).get("value", {}).get("ready") is True
        wait_for(ready, "原生驱动就绪")
        self.driver = JourneyDriver(self.application)
        browser = self.driver
        wait_for(lambda: self.content("导入聊天记录"), "全新首次入口")
        browser.button("导入聊天记录")
        self.dialog(DIALOG, archive)
        wait_for(lambda: self.content("让下一次对话接得上"), "导入后真实连接首页", timeout=60)
        identifier = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
        self.vault = self.temporary / "data" / identifier / "vault"
        wait_for(lambda: len(self.events()) == 3, "真实官方形状归档落库")
        assert not self.memories(), "未启用模型不能伪造整理结果"
        assert self.cli_result("connections")["entries"] == []
        self.assert_clean()
        self.checkpoint("全新导入后显示真实连接状态且未自动授予权限")

        wait_for(lambda: browser.observe("return Boolean(document.querySelector('.app-shell')?.dataset.buildVersion && document.querySelector('.app-shell')?.dataset.buildCommit)"), "GUI构建身份就绪")
        gui = browser.observe("return (()=>{const d=document.querySelector('.app-shell').dataset;return {version:d.buildVersion,commit:d.buildCommit,dirty:d.buildDirty==='true'}})()")
        cli = self.cli_result("status")["build"]
        assert gui == {key: cli[key] for key in ["version", "commit", "dirty"]}
        assert gui["version"].startswith("0.7.") and gui["commit"] not in {"", "unknown"}
        assert not gui["dirty"], "验收必须使用冻结的干净构建"
        if self.expected_build_commit:
            assert gui["commit"] == self.expected_build_commit, "实际程序不属于预期的编译提交"
        self.build_identity = {"gui": gui, "cli": cli}
        (self.artifacts / "build-identity.json").write_text(json.dumps(self.build_identity, ensure_ascii=False, indent=2))

        browser.button("原始资料")
        wait_for(lambda: browser.observe("return document.querySelectorAll('.source-row').length") == 1, "来源已加载")
        browser.click(".source-row")
        wait_for(lambda: ORIGINAL in browser.text(".conversation-article"), "实际原话详情")
        self.checkpoint("导入的原始来源可读可追溯")
        browser.button("连接")
        wait_for(lambda: self.content("连接你常用的 AI"), "连接中心")
        browser.button("连接 Codex")
        browser.button("选择项目文件夹", "//dialog")
        self.project_dialog(project)
        wait_for(lambda: self.content("将合并这些文件"), "真实配置预览")
        assert str(project) in browser.text("dialog")
        assert browser.observe("return [...document.querySelectorAll('dialog button')].find(b=>b.textContent.trim()==='授权并安装连接')?.disabled") is True
        assert self.cli_result("connections")["entries"] == []
        assert (project / "AGENTS.md").read_text() == original_instruction
        assert not (project / ".codex/config.toml").exists()
        self.checkpoint("原生选择项目后只预览不写文件也不提前授权")

        browser.click(".setup-consent input")
        browser.button("授权并安装连接", "//dialog")
        wait_for(lambda: self.content("项目接入配置已写入") and self.content("本机试读通过"), "一次批准后配置及真实CLI试读完成", timeout=60)
        assert "收到客户端读取" not in browser.text("dialog")
        self.assert_clean()
        entries = self.cli_result("connections")["entries"]
        assert len(entries) == 1 and entries[0]["grant"]["client_kind"] == "codex"
        entry = entries[0]
        connection = entry["id"]
        assert entry["grant"]["recall_scopes"] == ["personal"] and not entry["grant"]["capture_scopes"]
        assert entry["last_read_at"] is None and entry["last_bootstrap_at"] is None
        assert (project / "AGENTS.md").read_text().startswith(original_instruction)
        assert "connection-context" in (project / "AGENTS.md").read_text()
        assert connection in (project / ".codex/config.toml").read_text()
        snapshot = self.vault / "generated/bootstrap" / connection / "context.json"
        first = json.loads(snapshot.read_text())
        assert first["scopes"] == ["personal"] and first["directory"]
        assert not first["profile"], "未整理的来源只展示目录"
        self.checkpoint("一次批准写入真实项目配置并本机试读不伪造宿主回执")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")

        browser.button("首页")
        browser.button("补充最新情况")
        browser.type("#latest-update", UPDATE)
        browser.button("保存近况", "//dialog")
        wait_for(lambda: len(self.events()) == 4 and self.content("已保存你的补充"), "近况实际追加")
        assert not snapshot.exists(), "正本变化立即使旧文件快照失效"
        refreshed = self.cli_result("connection-context", connection, "--scope", "personal", "--budget-bytes", "4096")
        assert refreshed["snapshot"] != first["snapshot"] and snapshot.exists()
        config = {"command": str(self.cli), "args": ["--vault", str(self.vault), "mcp", "--scope", "personal", "--connection-id", connection]}
        found = self.mcp_tool(config, "search", {"query": "周四回访", "budget_bytes": 3000})
        assert UPDATE in json.dumps(found, ensure_ascii=False)
        (self.artifacts / "same-vault-managed-mcp.json").write_text(json.dumps(found, ensure_ascii=False, indent=2))
        self.checkpoint("补充后入口自动刷新且受管理MCP从同一正本读到更新")

        browser.close()
        self.driver = None
        wait_for(ready, "窗口关闭后可重新启动")
        self.driver = JourneyDriver(self.application)
        browser = self.driver
        wait_for(lambda: self.content("让下一次对话接得上"), "重启恢复首页")
        browser.button("连接")
        wait_for(lambda: self.content("连接授权与实际状态") and self.content("Codex"), "重启后连接状态")
        before = (project / ".codex/config.toml").read_bytes()
        health = self.cli_result("connection-check", connection, "--scope", "personal")
        assert health["installation"] == "configured" and health["verification_scope"] == "client_request"
        assert (project / ".codex/config.toml").read_bytes() == before
        self.checkpoint("重启保留配置并区分本机试读和实际客户端请求")
        browser.resize(820, 760)
        wait_for(lambda: self.content("连接你常用的 AI"), "窄窗连接中心加载")
        assert browser.observe("return document.documentElement.scrollWidth <= innerWidth")
        self.checkpoint("820窗口连接状态与操作完整可读")
        browser.button("撤销访问")
        browser.button("确认撤销访问", "//dialog")
        wait_for(lambda: self.cli_result("connections")["entries"][0]["revoked"], "真实撤权已持久化")
        assert not snapshot.exists()
        denied = subprocess.run([str(self.cli), "--vault", str(self.vault), "--json", "connection-context", connection, "--scope", "personal"], capture_output=True, text=True)
        assert denied.returncode != 0 and UPDATE not in denied.stdout
        assert len(self.events()) == 4
        self.checkpoint("撤权清除文件入口并拒绝旧连接继续读取且正本保留")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--application", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--expected-build-commit", default=os.environ.get("GITHUB_SHA"))
    args = parser.parse_args()
    for name in ["WebKitWebDriver", "xdotool", "xclip", "scrot", "openbox"]:
        if not shutil.which(name): parser.error(f"缺少官方验收依赖：{name}")
    if not os.environ.get("DISPLAY"): parser.error("请在独立 Xvfb 会话内运行")
    import pyatspi  # noqa: F401 - ensure accessible native file chooser support
    for binary in [args.application, args.cli]:
        if not binary.is_file(): parser.error(f"请先构建同快照程序：{binary}")
    with tempfile.TemporaryDirectory(prefix="recallcard-connected-") as directory:
        temporary = Path(directory)
        synthetic_home = temporary / "home"
        synthetic_home.mkdir()
        # 仅本验收进程和它启动的临时应用使用合成HOME，不读写runner已有宿主配置。
        os.environ["HOME"] = str(synthetic_home)
        for name, suffix in [("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("RECALLCARD_STATE_DIR", "state")]:
            os.environ[name] = str(temporary / suffix)
        smoke = ConnectedJourney(args, temporary)
        smoke.expected_build_commit = args.expected_build_commit
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
            (smoke.artifacts / "summary.json").write_text(json.dumps({"success": success, "suite": "v0.7-connected-context-native", "passed_steps": smoke.steps, "application": str(smoke.application), "cli": str(smoke.cli), "build": getattr(smoke, "build_identity", None), "synthetic_data_only": True, "invoke_mocked": False, "external_chatgpt_verified": False, "external_agents_executed": False}, ensure_ascii=False, indent=2))
            try:
                if smoke.vault.exists():
                    subprocess.run([str(smoke.cli), "--vault", str(smoke.vault), "--json", "service", "stop"], timeout=15, capture_output=True)
            finally:
                smoke.close()


if __name__ == "__main__":
    main()
