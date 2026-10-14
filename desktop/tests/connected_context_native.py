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
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.request import urlopen

from first_use_native import FirstUseJourney, JourneyDriver, DIALOG, ORIGINAL, UPDATE
from native_smoke import ROOT, ELEMENT, wait_for, run


ATTACHMENT_QUERY = "COPY_BODY_NATIVE_823"
ATTACHMENT_TEXT = ATTACHMENT_QUERY + "\n合成独立附件正文🙂。内容只在文件里，不是上传者原话。\n"


class ConnectedJourney(FirstUseJourney):
    def wait_for_message_text(self, selector, required, forbidden, description):
        """同一精确消息的一次渲染快照；异步刷新不能拆散附件归属断言。"""
        def observed():
            text = self.driver.text(selector)
            if not isinstance(text, str):
                return None
            assert not any(value in text for value in forbidden), "消息混入了其他附件或错误保留状态"
            return text if all(value in text for value in required) else None
        # 只读等待沿用明确时限；永久缺失/不完整仍失败，不重放点击或输入。
        return wait_for(observed, description)

    def check_opening_background(self):
        """首页真实可达，选择已有合成Memory；不使用模型整理或伪造invoke。"""
        browser = self.driver
        browser.button("首页")
        assert browser.observe("return document.querySelector('#recall-question').value") == ""
        browser.button("开局背景")
        wait_for(lambda: self.content("当前没有可选记忆"), "没有Memory时不伪造常驻事实")
        assert self.content("不需要先输入问题")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")
        events = self.events()
        source = next(event for event in events if event.get("content") == UPDATE)
        text = "合成开局背景：琥珀报告已勘误，周四回访。"
        path = self.temporary / "opening-memory.synthetic.json"
        path.write_text(json.dumps({"content": text, "scope": "personal", "source_refs": [source["id"]], "evidence": "user_explicit", "labels": ["synthetic"]}, ensure_ascii=False))
        created = self.cli_command("--json", "memory", "add", "--file", str(path))
        read = lambda: self.cli_command("--json", "read", created["id"])
        before = read()
        assert before["protected"] is True, "真实创建默认保护记忆，加入背景也必须逐次批准"
        browser.button("开局背景")
        wait_for(lambda: self.content(text), "实际Memory候选可见")
        assert text not in browser.observe("return document.querySelector('#opening-preview').value")
        browser.button("加入开局背景", "//dialog")
        wait_for(lambda: self.content("确认加入背景"), "加入先出现实际背景预览")
        assert text in browser.observe("return document.querySelector('#opening-preview').value")
        assert read() == before and self.events() == events
        browser.button("取消本次选择", "//dialog")
        wait_for(lambda: self.content("从现有记忆选择"), "取消返回背景选择")
        assert read() == before and text not in browser.observe("return document.querySelector('#opening-preview').value")
        self.checkpoint("首页无需提问可打开开局背景且取消选择零写入")
        browser.button("加入开局背景", "//dialog")
        wait_for(lambda: self.content("确认加入背景"), "重新审阅受保护记忆的加入选择")
        assert browser.observe("return !document.querySelector('#opening-protected-approval').checked && [...document.querySelectorAll('dialog .modal-footer button')].find(n=>n.textContent==='确认加入背景').disabled")
        browser.button("确认加入背景", "//dialog")
        assert read() == before and self.events() == events, "未批准时禁用的确认不能修改记忆"
        browser.click("#opening-protected-approval")
        wait_for(lambda: browser.observe("return [...document.querySelectorAll('dialog .modal-footer button')].find(n=>n.textContent==='确认加入背景').disabled === false"), "明确批准后才允许确认加入")
        assert read() == before, "勾选批准本身不能提前写入"
        browser.button("确认加入背景", "//dialog")
        candidate_selector = '[data-background-id="' + created["id"] + '"]'
        wait_for(lambda: browser.observe("return document.querySelector(" + json.dumps(candidate_selector) + ")?.textContent.includes('本次带上') && document.querySelector('#opening-preview')?.value.includes(" + json.dumps(text) + ")"), "确认已落库后使用真实bootstrap投影")
        selected = read()
        assert selected["revision"] == before["revision"] + 1
        assert selected["protected"] is True and "bootstrap" in selected["labels"]
        assert selected["content"] == before["content"] and selected["source_refs"] == before["source_refs"]
        preview = browser.observe("return document.querySelector('#opening-preview').value")
        assert "recallcard.context/1" in preview and f'memory:{created["id"]}@{selected["revision"]}' in preview
        connection = self.cli_result("connections")["entries"][0]["id"]
        entry = self.cli_result("connection-context", connection, "--scope", "personal", "--budget-bytes", "4096")
        mcp_config = {"command": str(self.cli), "args": ["--vault", str(self.vault), "mcp", "--scope", "personal", "--connection-id", connection]}
        bootstrap = self.mcp_tool(mcp_config, "bootstrap", {})
        assert entry["schema"] == "recallcard.agent-entry/2"
        assert entry["bootstrap_version"] == bootstrap["bootstrap_version"] and entry["bootstrap_version"] in preview
        assert [item["ref"] for item in entry["profile"]] == bootstrap["refs"] == [f'memory:{created["id"]}@{selected["revision"]}']
        assert entry["profile"][0]["text"] == text and entry["profile"][0]["text_truncated"] is False
        browser.button("核对并复制开局背景", "//dialog")
        wait_for(lambda: self.content("开局背景已复制"), "开局背景实际写入系统剪贴板")
        assert run("xclip", "-selection", "clipboard", "-o") == preview
        self.checkpoint("开局背景确认后带真实记忆引用且剪贴板全文一致")
        browser.button("移出开局背景", "//dialog")
        wait_for(lambda: self.content("确认移出背景"), "移出受保护背景先预览")
        assert browser.observe("return [...document.querySelectorAll('dialog .modal-footer button')].find(n=>n.textContent==='确认移出背景').disabled")
        assert read() == selected
        browser.click("#opening-protected-approval")
        browser.button("确认移出背景", "//dialog")
        wait_for(lambda: self.content("加入开局背景") and browser.observe("return !document.querySelector('#opening-preview')?.value.includes(" + json.dumps(text) + ")"), "移出后真实投影不再带旧背景")
        removed = read()
        assert removed["revision"] == selected["revision"] + 1
        assert removed["protected"] is True and "bootstrap" not in removed["labels"]
        assert removed["content"] == before["content"] and removed["source_refs"] == before["source_refs"]
        assert len(self.memories()) == 1 and self.events() == events
        assert not Path(entry["file_path"]).exists(), "移出立即使受管理旧文件失效"
        refreshed = self.cli_result("connection-context", connection, "--scope", "personal", "--budget-bytes", "4096")
        bootstrap = self.mcp_tool(mcp_config, "bootstrap", {})
        assert refreshed["profile"] == bootstrap["refs"] == []
        assert refreshed["bootstrap_version"] == bootstrap["bootstrap_version"] != entry["bootstrap_version"]
        assert text not in json.dumps(refreshed, ensure_ascii=False) and text not in bootstrap["stable_text"]
        self.opening_background_acceptance = {"home_without_question": True, "empty_memory_honest": True, "cancel_zero_writes": True, "protected_inclusion_requires_approval": True, "same_bootstrap_projection": True, "managed_file_and_mcp_selection_match": True, "explicit_protected_removal": True, "clipboard_full_match": True, "memory_and_sources_preserved": True, "model_invoked": False, "external_site_used": False}
        self.checkpoint("明确确认后只移出常驻选择且记忆保护与原话都保留")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")

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
        text_file = {"id": "file_SYNTHETIC_TEXT", "name": "合成文本附件.txt",
                     "mime_type": "text/plain", "size": len(ATTACHMENT_TEXT.encode("utf-8"))}
        assets = [present, missing, text_file]
        for node in record["mapping"].values():
            node.pop("children", None)  # 官方样本可以只有 parent。
        upload = record["mapping"]["u1"]["message"]
        upload["content"] = {"content_type": "text", "parts": [""]}
        upload["metadata"]["attachments"] = [present]
        record["mapping"]["u2"]["message"]["metadata"]["attachments"] = [missing]
        record["mapping"]["a1"]["message"]["metadata"]["attachments"] = [text_file]
        library = [{"file_id": a["id"], "file_name": a["name"],
                    "mime_type": a["mime_type"], "file_size_bytes": a["size"], "state": "ready"}
                   for a in assets]
        manifest = {"version": 1, "manifest_file": "export_manifest.json",
                    "export_files": [{"path": a["id"] + ".dat", "size_bytes": a["size"]}
                                     for a in assets],
                    "logical_files": {a["id"] + ".dat": {"files": [a["id"] + ".dat"], "sharded": False}
                                      for a in assets}}
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, value in [("conversations.json", records), ("library_files.json", library),
                                ("export_manifest.json", manifest),
                                ("conversation_asset_file_names.json", {present["id"] + ".dat": "合成归档别名.pdf"})]:
                archive.writestr(name, json.dumps(value, ensure_ascii=False))
            archive.writestr(present["id"] + ".dat", payload)
            archive.writestr(text_file["id"] + ".dat", ATTACHMENT_TEXT.encode("utf-8"))
        return path

    def check_attachment_copy(self):
        """真实导入UTF-8原件→React/Tauri检索→手动复制；不请求外部模型或网站。"""
        browser = self.driver
        events = self.events()
        parent = next(event for event in events if event["source"]["message_id"] == "a1")
        asset = parent["metadata"]["source_assets"]["files"][0]
        reference = "attachment:" + parent["id"] + "/" + asset["object_sha256"]
        original = self.vault / "objects" / asset["object_sha256"]
        assert original.read_bytes() == ATTACHMENT_TEXT.encode("utf-8")
        assert ATTACHMENT_QUERY not in parent["content"] and ATTACHMENT_QUERY not in asset["name"]
        browser.button("首页")
        browser.type("#recall-question", ATTACHMENT_QUERY)
        browser.button("准备这次对话")
        wait_for(lambda: self.content("附件正文，非用户原话"), "附件正文独有关键词命中可选候选")
        assert browser.observe("return document.querySelectorAll('dialog fieldset input:checked').length") == 1
        assert self.content("不会附带原始文件")
        browser.button("预览将复制的内容", "//dialog")
        wait_for(lambda: bool(browser.observe("return document.querySelector('#portable-preview')?.value")), "附件复制前可审阅完整文字")
        preview = browser.observe("return document.querySelector('#portable-preview').value")
        assert ATTACHMENT_TEXT in preview and reference in preview
        assert "event:" + parent["id"] in preview and asset["object_sha256"] in preview
        assert "附件内容，非用户原话" in preview and "作者未核验" in preview
        assert parent["content"] not in preview, "没有选择父消息，不得夹带其正文"
        browser.button("核对并复制上下文", "//dialog")
        wait_for(lambda: self.content("已复制。请到任意 AI 粘贴"), "附件实际写入系统剪贴板")
        clipboard = lambda: subprocess.check_output(["xclip", "-selection", "clipboard", "-o"], timeout=5)
        assert clipboard() == preview.encode("utf-8"), "剪贴板字节必须与完整预览一致"
        self.checkpoint("附件独立证据进入复制包且系统剪贴板全文一致")
        # 放入不同的用户剪贴板哨兵，防止“错误地重写旧预览后报错”也被判为通过。
        sentinel = "合成剪贴板哨兵：拒绝后必须保持此内容🙂".encode("utf-8")
        subprocess.run(["xclip", "-selection", "clipboard", "-i"], input=sentinel,
                       check=True, timeout=5, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        assert clipboard() == sentinel
        try:
            original.write_bytes(b"x" * len(ATTACHMENT_TEXT.encode("utf-8")))
            browser.button("核对并复制上下文", "//dialog")
            wait_for(lambda: self.content("所选资料不可访问、已变化或已被遗忘"), "原件篡改使复制前复核拒绝")
            assert not browser.observe("return Boolean(document.querySelector('#portable-preview'))")
            assert clipboard() == sentinel, "拒绝后不得覆写系统剪贴板"
        finally:
            original.write_bytes(ATTACHMENT_TEXT.encode("utf-8"))
        assert self.events() == events and not self.memories()
        self.attachment_acceptance.update({"body_only_query_hit": True, "independent_evidence": True,
                                          "clipboard_utf8_exact": True, "tampered_original_blocks_copy": True,
                                          "model_invoked": False, "external_site_used": False})
        self.checkpoint("复制前原件核验拒绝篡改且未写剪贴板或正本")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")

    def check_local_event_index(self):
        """真实UI/回环传输合同；固定合成向量，不是实际模型语义质量。"""
        requests = []
        class Encoder(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                requests.extend(value["input"])
                vectors = [[1.0, 0.0] if text == "synthetic-vector-query" or text == UPDATE else [0.0, 1.0] for text in value["input"]]
                payload = json.dumps({"model": "native-synthetic-contract", "data": [{"index": i, "embedding": v} for i, v in enumerate(vectors)]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
        server = HTTPServer(("127.0.0.1", 0), Encoder)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            browser = self.driver
            before = self.events()
            browser.button("设置")
            browser.button("配置普通对话本机索引")
            assert not browser.observe("return document.querySelector('#event-corpus').checked || document.querySelector('#event-query').checked")
            for key, value in {"endpoint": f"http://127.0.0.1:{server.server_port}/v1/embeddings", "model": "native-synthetic-contract", "revision": "fake-vector-contract-v1", "tokenizer": "no-real-tokenizer-test-only", "dimensions": "2"}.items():
                browser.type("#event-" + key, value)
            browser.click("#event-corpus")
            browser.click("#event-query")
            browser.button("核对普通对话范围", "//dialog")
            assert not requests, "查看范围不能提前编码"
            browser.click("#event-confirm")
            browser.button("确认保存普通对话配置", "//dialog")
            wait_for(lambda: not browser.observe("return Boolean(document.querySelector('dialog[open]'))"), "本机配置保存")
            assert not requests, "保存配置不能暗中编码整个聊天库"
            browser.button("更新普通对话索引")
            wait_for(lambda: self.content("本轮索引完成"), "分批普通Event索引完成", timeout=90)
            assert not self.memories() and self.events() == before
            assert UPDATE in requests and all(isinstance(text, str) for text in requests)
            self.checkpoint("独立选择普通对话本机范围后分批索引且原话不变")
            browser.button("首页")
            browser.type("#recall-question", "synthetic-vector-query")
            browser.button("准备这次对话")
            wait_for(lambda: self.content("普通对话已直接参与本机语义检索"), "问题实际走本机Event链")
            assert "synthetic-vector-query" in requests
            assert browser.observe("return document.querySelector('dialog fieldset label')?.textContent.includes(" + json.dumps(UPDATE) + ")") is True
            assert self.content("此入口没有云回退")
            assert self.content("相似候选可能完全无关")
            lexical_note = "普通对话词法索引已独立寻找完整编号和汉字词组候选，并结合语义排序；字面匹配不代表事实正确。"
            lexical_position = browser.observe("return [...document.querySelectorAll('dialog aside[aria-label=\"本次检索覆盖\"] p')].findIndex(n=>n.textContent===" + json.dumps(lexical_note) + ")")
            assert lexical_position >= 0
            lexical_selector = f'dialog aside[aria-label="本次检索覆盖"] p:nth-child({lexical_position + 1})'
            lexical_element = browser.find(lexical_selector)
            browser.command("POST", "/execute/sync", {"script": "arguments[0].scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});", "args": [{ELEMENT: lexical_element}]})
            wait_for(lambda: self.exact_visible_text(lexical_selector, lexical_note), "独立词法通道说明全文真实可见")
            self.capture("13-independent-lexical-coverage-visible")
            support_selector = "dialog fieldset label small.context-note"
            support_element = wait_for(lambda: browser.find(support_selector), "候选核对说明真实存在")
            browser.command("POST", "/execute/sync", {"script": "arguments[0].scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});", "args": [{ELEMENT: support_element}]})
            support_text = "这段检索片段没有完整匹配精确锚点；只保留为语义候选，不表示整个事件不含该标识。"
            wait_for(lambda: self.exact_visible_text(support_selector, support_text), "候选不确定说明全文可见")
            self.capture("13-candidate-support-visible")
            browser.button("预览将复制的内容", "//dialog")
            wait_for(lambda: bool(browser.observe("return document.querySelector('#portable-preview')?.value")), "本机Event命中可进入原上下文预览")
            assert UPDATE in browser.observe("return document.querySelector('#portable-preview').value")
            connection = self.cli_result("connections")["entries"][0]["id"]
            mcp_config = {"command": str(self.cli), "args": ["--vault", str(self.vault), "mcp", "--scope", "personal", "--connection-id", connection]}
            recalled = self.mcp_tool(mcp_config, "recall_search", {"query": "synthetic-vector-query"})
            original = next(event for event in self.events() if event.get("content") == UPDATE)
            assert recalled["results"][0]["ref"] == "event:" + original["id"]
            assert recalled["coverage"]["cloud_fallback"] is False
            assert recalled["results"][0]["relevance"] == "unassessed"
            assert recalled["results"][0]["retrieval_support"]["protected"] is False
            resolved = self.mcp_tool(mcp_config, "recall_fetch", {"id": recalled["results"][0]["id"]})
            assert resolved["results"][0]["ref"] == recalled["results"][0]["ref"]
            assert resolved["results"][0]["snapshot"] == recalled["results"][0]["snapshot"]
            assert resolved["results"][0]["text"] == UPDATE
            lexical_read = self.mcp_tool(mcp_config, "recall_search", {"query": "周四"})
            lexical_source = next(row for row in lexical_read["results"] if row["ref"] == "event:" + original["id"])
            assert lexical_read["coverage"]["scopes"][0]["lexical"]["status"] == "available"
            assert lexical_source["retrieval"]["lexical_rank"] is not None
            assert lexical_source["retrieval"]["lexical_evidence_verification"] == "one_complete_query_term"
            assert "周四" in lexical_source["text"]
            lexical_full = self.mcp_tool(mcp_config, "recall_fetch", {"id": lexical_source["id"]})
            assert lexical_full["results"][0]["ref"] == lexical_source["ref"]
            assert lexical_full["results"][0]["text"] == UPDATE
            assert self.events() == before and not self.memories()
            self.local_event_acceptance = {"fake_encoder_contract_only": True, "actual_model_quality_verified": False, "actual_loopback_http": True, "raw_event_without_memory": True, "separate_explicit_approval": True, "no_calls_before_index_request": True, "canonical_unchanged": True, "new_query_local_encoded": True, "same_source_in_context_preview": True, "managed_mcp_same_event_and_snapshot": True, "candidate_support_visible": True, "candidate_not_answerable_claim": True, "independent_lexical_coverage_visible": True, "managed_mcp_lexical_term_source_verified": True, "cloud_fallback": False}
            self.checkpoint("本机新问题返回原Event并接入复制预览而不冒充真实模型质量")
            browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")
            browser.button("设置")
            browser.button("停用普通对话索引")
            wait_for(lambda: not self.content("停用普通对话索引"), "本机Event配置可单独停用")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def capture(self, name, webview=True):
        if self.driver and webview:
            # 业务谓词成立早于WebKit绘制；只等绘制，不修改页面或代替用户动作。
            ready = self.driver.observe("return Promise.race([document.fonts.ready.then(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))),new Promise(resolve=>setTimeout(()=>resolve(false),3000))])")
            assert ready is True, "截图等待绘制超时"
            observed = self.driver.observe("return {text:document.body.innerText,viewport:{width:innerWidth,height:innerHeight},modal:[...document.querySelectorAll('dialog[open]')].map(n=>({text:n.innerText,width:n.getBoundingClientRect().width,height:n.getBoundingClientRect().height}))}")
            (self.artifacts / f"{name}-visible.json").write_text(json.dumps(observed, ensure_ascii=False, indent=2))
        super().capture(name, webview)

    def exact_visible_text(self, selector, expected):
        # WebDriver rendered-text 对 -webkit-line-clamp 元素可能漏字；用完整 DOM
        # 文本、真实可见矩形与无内容裁切共同校验，不能只用关键词或前缀放宽验收。
        sample = self.driver.observe("return (()=>{const n=document.querySelector(" + json.dumps(selector) + ");if(!n)return null;const r=n.getBoundingClientRect(),s=getComputedStyle(n);return {text:n.textContent,visible:r.width>0&&r.height>0&&r.top>=0&&r.bottom<=innerHeight&&s.visibility==='visible'&&s.display!=='none'&&s.opacity!=='0',unclipped:n.scrollHeight<=n.clientHeight+1};})()")
        return bool(sample and sample["text"] == expected and sample["visible"] and sample["unclipped"])

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
        self.wait_for_message_text(
            present_row, ("原件已保留", "合成图表.pdf", "合成归档别名.pdf", "application/pdf"),
            ("合成未附原件.pdf",), "附件-only 消息显示完整保留状态、名称、别名和类型")
        wait_for(lambda: browser.observe("return Boolean(document.querySelector(" + json.dumps(present_row) + ")) && document.querySelectorAll('.message-assets a,.message-assets button,.message-assets iframe,.message-assets object,.message-assets embed').length === 0"),
                 "目标原消息存在且附件没有可交互或嵌入内容")
        browser.click(present_row)
        self.checkpoint("附件only原消息显示真实保留状态与独立别名")
        browser.click(missing_row)
        self.wait_for_message_text(
            missing_row, ("所提供归档中未找到原件", "合成未附原件.pdf", ORIGINAL),
            ("原件已保留",), "缺失附件仍绑定其完整原消息")
        self.checkpoint("缺失原件明确显示且没有串联到其他消息")
        browser.button("查看记录")
        wait_for(lambda: self.content("导入记录"), "实际导入记录页")
        browser.button("处理明细")
        wait_for(lambda: "3 条附件引用" in browser.text(".import-coverage"), "真实任务报告附件计数")
        assert "2 条附件引用已保留原始字节" in browser.text(".import-coverage")
        assert "1 条附件引用在所提供归档中未找到原件" in browser.text(".import-coverage")
        self.checkpoint("导入报告同时显示保留和缺失的附件引用数")
        self.attachment_acceptance = {"message_references": 3, "retained": 2, "missing": 1,
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
        browser.type("#recall-question", "周四回访何时安排")
        browser.button("准备这次对话")
        wait_for(lambda: self.content("用户原话")
                 and self.content("尚未配置普通对话本机索引，本次只在本地按关键词寻找原话和记忆。")
                 and self.content("此入口不会改用云编码；未命中或未覆盖不表示没有相关历史。"),
                 "首页问题实际检索并明确未配置索引、关键词路径与无云回退")
        browser.button("预览将复制的内容", "//dialog")
        wait_for(lambda: bool(browser.observe("return document.querySelector('#portable-preview')?.value")), "问题A已有完整预览")
        assert len(self.events()) == 4 and not self.memories()
        browser.type("#portable-goal", "MissingSyntheticQZX987")
        wait_for(lambda: self.content("问题已改变"), "新问题不能沿用旧召回")
        assert not browser.observe("return Boolean(document.querySelector('#portable-preview'))")
        assert browser.observe("return [...document.querySelectorAll('dialog .modal-footer button')].find(n=>n.textContent.includes('预览将复制')).disabled")
        browser.button("按这个问题找资料", "//dialog")
        wait_for(lambda: self.content("这次没有找到匹配来源"), "空召回不复活旧来源")
        assert browser.observe("return document.querySelectorAll('dialog fieldset input:checked').length") == 0
        assert not browser.observe("return Boolean(document.querySelector('#portable-preview'))")
        browser.type("#portable-goal", "周四回访何时安排")
        browser.button("按这个问题找资料", "//dialog")
        wait_for(lambda: self.content("用户原话") and not self.content("问题已改变"), "重新按当前问题找到来源")
        browser.button("预览将复制的内容", "//dialog")
        wait_for(lambda: self.content("完整复制内容") and self.content("用户原话"), "复制范围与来源预览")
        preview = browser.observe("return document.querySelector('#portable-preview').value")
        assert UPDATE in preview and "recallcard.context/1" in preview
        assert self.content("按问题寻找字面候选")
        assert self.content("字节 UTF-8") and self.content("未勾选的附件正文及附件原始文件")
        browser.button("核对并复制上下文", "//dialog")
        wait_for(lambda: self.content("已复制。请到任意 AI 粘贴"), "用户点击触发实际剪贴板写入")
        assert run("xclip", "-selection", "clipboard", "-o") == preview
        assert len(self.events()) == 4 and not self.memories(), "复制不应生成记忆或回流事件"
        self.checkpoint("任意AI交接显示来源长度和排除范围且系统剪贴板全文一致")
        browser.button("关闭", "//dialog//div[contains(@class,'modal-footer')]")
        assert not browser.observe("return Boolean(document.querySelector('dialog[open]'))")
        self.check_attachment_copy()
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
        search_selector = '.search-result[data-reference="event:' + saved[0]["id"] + '"]'
        wait_for(lambda: self.exact_visible_text(search_selector + " p", reply), "搜索结果绑定已保存Event且完整原文可见")
        rendered_exact = reply in browser.text(search_selector)
        browser.click(search_selector)
        source_selector = '.source-message[data-reference="event:' + saved[0]["id"] + '"]'
        wait_for(lambda: self.exact_visible_text(source_selector + " .prose p", reply), "返回同一助手Event且完整原文可见")
        assert browser.observe("return Boolean(document.querySelector(" + json.dumps(source_selector + " .role-avatar.assistant") + "))")

        self.checkpoint("粘贴回答只追加一条助手来源且可搜索回到原文")
        self.portable_acceptance = {"clipboard_full_match":True,"home_question_search":True,"unpunctuated_question_hit":True,"changed_question_researched":True,"no_match_drops_old_choices":True,"coverage_visible":True,"copy_added_events":0,"cancel_added_events":0,
            "reply_events_added":1,"reply_role":"assistant","reply_original_time_known":False,
            "automatic_memory":False,"reply_searchable":True,"external_site_used":False,
            "search_dom_exact_visible":True,"source_dom_exact_visible":True,"search_webdriver_rendered_exact":rendered_exact}

        self.check_local_event_index()
        self.check_opening_background()
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
            (smoke.artifacts / "summary.json").write_text(json.dumps({"success": success, "suite": "v0.7-connected-context-native", "passed_steps": smoke.steps, "application": str(smoke.application), "cli": str(smoke.cli), "build": getattr(smoke, "build_identity", None), "synthetic_data_only": True, "invoke_mocked": False, "external_chatgpt_verified": False, "external_agents_executed": False, "canonical": smoke.canonical_summary(), "attachment_acceptance": getattr(smoke, "attachment_acceptance", None), "portable_acceptance": getattr(smoke, "portable_acceptance", None), "local_event_acceptance": getattr(smoke, "local_event_acceptance", None), "opening_background_acceptance": getattr(smoke, "opening_background_acceptance", None)}, ensure_ascii=False, indent=2))
            try:
                if smoke.vault.exists():
                    subprocess.run([str(smoke.cli), "--vault", str(smoke.vault), "--json", "service", "stop"], timeout=15, capture_output=True)
            finally:
                smoke.close()


if __name__ == "__main__":
    main()
