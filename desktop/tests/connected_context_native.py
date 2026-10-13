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
import zipfile
from urllib.request import urlopen

from first_use_native import FirstUseJourney, JourneyDriver, DIALOG, ORIGINAL, UPDATE
from native_smoke import ROOT, wait_for, run


class ConnectedJourney(FirstUseJourney):
    def create_archive(self):
        """真实导入链路使用独立合成原件与缺失原件，绝不读取个人归档。"""
        path = super().create_archive()
        with zipfile.ZipFile(path) as archive:
            records = json.loads(archive.read("conversations.json"))
        record = records[0]
        payload = b"%PDF-1.4\nSYNTHETIC OPAQUE ATTACHMENT\n%%EOF\n"
        present = {"id": "file_SYNTHETIC_PRESENT", "name": "合成图表.pdf",
                   "mime_type": "application/pdf", "size": len(payload)}
        missing = {"id": "file_SYNTHETIC_MISSING", "name": "合成未附原件.pdf",
                   "mime_type": "application/pdf", "size": 99}
        for node in record["mapping"].values():
            node.pop("children", None)  # 官方样本可以只有 parent。
        upload = record["mapping"]["u1"]["message"]
        upload["content"] = {"content_type": "text", "parts": [""]}
        upload["metadata"]["attachments"] = [present]
        record["mapping"]["u2"]["message"]["metadata"]["attachments"] = [missing]
        library = [{"file_id": a["id"], "file_name": a["name"],
                    "mime_type": a["mime_type"], "file_size_bytes": a["size"], "state": "ready"}
                   for a in [present, missing]]
        manifest = {"version": 1, "manifest_file": "export_manifest.json",
                    "export_files": [{"path": a["id"] + ".dat", "size_bytes": a["size"]}
                                     for a in [present, missing]],
                    "logical_files": {a["id"] + ".dat": {"files": [a["id"] + ".dat"], "sharded": False}
                                      for a in [present, missing]}}
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, value in [("conversations.json", records), ("library_files.json", library),
                                ("export_manifest.json", manifest),
                                ("conversation_asset_file_names.json", {present["id"] + ".dat": "合成归档别名.pdf"})]:
                archive.writestr(name, json.dumps(value, ensure_ascii=False))
            archive.writestr(present["id"] + ".dat", payload)
        return path

    def capture(self, name, webview=True):
        if self.driver and webview:
            # 业务谓词成立早于WebKit绘制；只等绘制，不修改页面或代替用户动作。
            ready = self.driver.observe("return Promise.race([document.fonts.ready.then(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))),new Promise(resolve=>setTimeout(()=>resolve(false),3000))])")
            assert ready is True, "截图等待绘制超时"
            observed = self.driver.observe("return {text:document.body.innerText,viewport:{width:innerWidth,height:innerHeight},modal:[...document.querySelectorAll('dialog[open]')].map(n=>({text:n.innerText,width:n.getBoundingClientRect().width,height:n.getBoundingClientRect().height}))}")
            (self.artifacts / f"{name}-visible.json").write_text(json.dumps(observed, ensure_ascii=False, indent=2))
        super().capture(name, webview)

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

    def canonical_summary(self):
        """Bounded synthetic acceptance evidence, without message bodies or grants."""
        try:
            if not self.vault.exists():
                return {"vault_exists": False, "events": 0, "memories": 0,
                        "original_present": False, "update_present": False}
            events = self.events()
            return {"vault_exists": True, "events": len(events),
                    "memories": len(self.memories()),
                    "original_present": any(event.get("content") == ORIGINAL for event in events),
                    "update_present": any(event.get("content") == UPDATE for event in events)}
        except Exception as error:
            return {"readable": False, "error_type": type(error).__name__}

    def exercise(self):
        identifier = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
        self.vault = self.temporary / "data" / identifier / "vault"
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
        events = self.events()
        upload = next(event for event in events if event["source"]["message_id"] == "u1")
        latest = next(event for event in events if event["source"]["message_id"] == "u2")
        assert upload["kind"] == "file" and not upload["content"]
        assert upload["metadata"]["source_assets"]["files"][0]["payload_status"] == "retained"
        assert latest["metadata"]["source_assets"]["files"][0]["payload_status"] == "missing"
        present_row = ".source-message[data-reference='event:" + upload["id"] + "']"
        missing_row = ".source-message[data-reference='event:" + latest["id"] + "']"
        wait_for(lambda: "原件已保留" in browser.text(present_row), "附件-only 消息显示保留状态")
        assert "合成图表.pdf" in browser.text(present_row)
        assert "合成归档别名.pdf" in browser.text(present_row)
        assert "application/pdf" in browser.text(present_row)
        assert "合成未附原件.pdf" not in browser.text(present_row)
        assert browser.observe("return document.querySelectorAll('.message-assets a,.message-assets button,.message-assets iframe,.message-assets object,.message-assets embed').length") == 0
        browser.click(present_row)
        self.checkpoint("附件only原消息显示真实保留状态与独立别名")
        browser.click(missing_row)
        wait_for(lambda: "所提供归档中未找到原件" in browser.text(missing_row), "缺失附件仍绑定其原消息")
        assert "合成未附原件.pdf" in browser.text(missing_row)
        assert ORIGINAL in browser.text(missing_row)
        assert "原件已保留" not in browser.text(missing_row)
        self.checkpoint("缺失原件明确显示且没有串联到其他消息")
        browser.button("查看记录")
        wait_for(lambda: self.content("导入记录"), "实际导入记录页")
        browser.button("处理明细")
        wait_for(lambda: "2 条附件引用" in browser.text(".import-coverage"), "真实任务报告附件计数")
        assert "1 条附件引用已保留原始字节" in browser.text(".import-coverage")
        assert "1 条附件引用在所提供归档中未找到原件" in browser.text(".import-coverage")
        self.checkpoint("导入报告同时显示保留和缺失的附件引用数")
        self.attachment_acceptance = {"message_references": 2, "retained": 1, "missing": 1,
                                      "attachment_only": 1, "ui_report_verified": True,
                                      "original_message_association_verified": True}
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

        # 平台无关闭环：真实 React → Tauri → 正本 → 系统剪贴板；无外部网站或模型。
        browser.type("#global-search", "周四回访")
        run("xdotool", "key", "Return")
        wait_for(lambda: self.content("搜索「周四回访」") and self.content("带到任意 AI"), "按问题检索已存原话")
        browser.button("带到任意 AI")
        browser.button("预览将复制的内容", "//dialog")
        wait_for(lambda: self.content("完整复制内容") and self.content("用户原话"), "复制范围与来源预览")
        preview = browser.observe("return document.querySelector('#portable-preview').value")
        assert UPDATE in preview and "recallcard.context/1" in preview
        assert self.content("字节 UTF-8") and self.content("附件原件及附件正文")
        browser.button("核对并复制上下文", "//dialog")
        wait_for(lambda: self.content("已复制。请到任意 AI 粘贴"), "用户点击触发实际剪贴板写入")
        assert run("xclip", "-selection", "clipboard", "-o") == preview
        assert len(self.events()) == 4 and not self.memories(), "复制不应生成记忆或回流事件"
        self.checkpoint("任意AI交接显示来源长度和排除范围且系统剪贴板全文一致")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")
        assert not browser.observe("return Boolean(document.querySelector('dialog[open]'))")
        browser.button("首页")
        browser.button("粘贴 AI 回答")
        browser.type("#pasted-ai-reply", "合成取消的回答，不得保存。")
        browser.button("预览回答", "//dialog")
        wait_for(lambda: self.content("确认为 AI 回答保存"), "保存前显式预览")
        browser.button("取消", "//dialog//div[contains(@class,'modal-footer')]")
        assert len(self.events()) == 4
        browser.button("粘贴 AI 回答")
        reply = "合成AI回答：建议周四回访时核对图四单位。这是助手建议，用户尚未确认。"
        browser.type("#pasted-ai-reply", reply)
        browser.button("预览回答", "//dialog")
        wait_for(lambda: self.content("确认为 AI 回答保存"), "普通回答无协议标记也能预览")
        assert len(self.events()) == 4 and self.content("AI 回答，非用户事实")
        self.checkpoint("普通回答保存前显示助手来源并且取消不落库")
        browser.button("确认为 AI 回答保存", "//dialog")
        wait_for(lambda: len(self.events()) == 5 and self.content("已保存为 AI 回答"), "普通回答实际保存")
        saved = [event for event in self.events() if event.get("content") == reply]
        assert len(saved) == 1 and saved[0]["role"] == "assistant" and saved[0]["origin"] == "assistant_output"
        assert saved[0].get("occurred_at") is None and saved[0].get("reply_to") is None
        assert saved[0]["metadata"]["exclude_from_automatic_memory"] is True and not self.memories()
        browser.type("#global-search", "合成AI回答")
        run("xdotool", "key", "Return")
        wait_for(lambda: self.content(reply), "已保存回答可按原文找回")
        browser.click(".search-result")
        wait_for(lambda: self.content("手动保存的 AI 回答") and self.content(reply), "搜索结果打开真实助手来源")
        self.checkpoint("粘贴回答只追加一条助手来源且可搜索回到原文")
        self.portable_acceptance = {"clipboard_full_match":True,"copy_added_events":0,"cancel_added_events":0,
            "reply_events_added":1,"reply_role":"assistant","reply_original_time_known":False,
            "automatic_memory":False,"reply_searchable":True,"external_site_used":False}

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
        assert len(self.events()) == 5
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
            (smoke.artifacts / "summary.json").write_text(json.dumps({"success": success, "suite": "v0.7-connected-context-native", "passed_steps": smoke.steps, "application": str(smoke.application), "cli": str(smoke.cli), "build": getattr(smoke, "build_identity", None), "synthetic_data_only": True, "invoke_mocked": False, "external_chatgpt_verified": False, "external_agents_executed": False, "canonical": smoke.canonical_summary(), "attachment_acceptance": getattr(smoke, "attachment_acceptance", None), "portable_acceptance": getattr(smoke, "portable_acceptance", None)}, ensure_ascii=False, indent=2))
            try:
                if smoke.vault.exists():
                    subprocess.run([str(smoke.cli), "--vault", str(smoke.vault), "--json", "service", "stop"], timeout=15, capture_output=True)
            finally:
                smoke.close()


if __name__ == "__main__":
    main()
