"""不启动浏览器：核对真实原生验收驱动的动作顺序和不确定写入处理。"""
import importlib.util
import json
from http.client import RemoteDisconnected
from pathlib import Path
from types import SimpleNamespace
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("native_controls", Path(__file__).with_name("native_smoke.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class NativeControlsTest(unittest.TestCase):
    def test_scope_persistence_probe_matches_actual_tauri_data_directory(self):
        source = (module.ROOT / "desktop/src-tauri/src/main.rs").read_text()
        helper = source.split("fn recent_workspace_file(", 1)[1].split("#[tauri::command]", 1)[0]
        self.assertIn(".app_data_dir()", helper)
        self.assertNotIn(".app_config_dir()", helper)
        self.assertIn('directory.join("recent-workspace.json")', helper)
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            identifier = json.loads((module.ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
            path = module.recent_workspace_path(temporary)
            self.assertEqual(path, temporary / "data" / identifier / "recent-workspace.json")
            self.assertTrue(path.resolve().is_relative_to(temporary.resolve()))
            self.assertFalse(path.exists(), "验收路径检查不得写入设置")

    def test_context_selection_uses_current_visible_refs_and_normal_clicks(self):
        driver = self.driver()
        driver.click = Mock(); driver.idle = Mock()
        rows = [{"ref": "event:keep", "checked": True, "disabled": False},
                {"ref": "event:remove", "checked": True, "disabled": False},
                {"ref": "event:add", "checked": False, "disabled": False}]
        driver.observe = Mock(side_effect=[rows, ["event:keep", "event:add"]])
        self.assertEqual(driver.choose_context_references(["event:add", "event:keep"]), ["event:keep", "event:add"])
        self.assertEqual([call.args[0] for call in driver.click.call_args_list], [
            '.context-check input[data-selection-reference="event:remove"]',
            '.context-check input[data-selection-reference="event:add"]'])
        self.assertEqual(driver.idle.call_count, 2)
        for call in driver.observe.call_args_list:
            self.assertTrue(call.args[0].startswith("return "))
            self.assertNotIn("__TAURI__", call.args[0])

    def test_context_selection_refuses_missing_disabled_or_ambiguous_choices(self):
        for references, rows in [([], []), (["event:a", "event:a"], []),
                (["event:missing"], [{"ref": "event:a", "checked": False, "disabled": False}]),
                (["event:a"], [{"ref": "event:a", "checked": False, "disabled": True}]),
                (["event:a"], [{"ref": "event:a", "checked": False, "disabled": False}] * 2)]:
            driver = self.driver(); driver.click = Mock(); driver.observe = Mock(return_value=rows)
            with self.subTest(references=references, rows=rows), self.assertRaises(AssertionError):
                driver.choose_context_references(references)
            driver.click.assert_not_called()

    def test_explicit_application_restart_waits_for_driver_and_creates_one_session(self):
        smoke = object.__new__(module.NativeSmoke); smoke.application = Path("/synthetic/recallcard-desktop")
        previous = Mock(); previous.observations = [{"synthetic": "old"}]; smoke.driver = previous
        fresh = Mock(); fresh.text.return_value = "会话"
        response = Mock(); response.__enter__ = Mock(return_value=response); response.__exit__ = Mock(return_value=False)
        response.read.return_value = b'{"value":{"ready":true}}'
        with patch.object(module, "urlopen", return_value=response), patch.object(module, "wait_for", side_effect=self.immediate), patch.object(module, "WebDriver", return_value=fresh) as create:
            self.assertIs(smoke.reopen_application(), fresh)
        previous.close.assert_called_once_with(); create.assert_called_once_with(smoke.application)
        fresh.idle.assert_called_once_with(); self.assertEqual(fresh.observations, previous.observations)

    def test_uncertain_restart_session_creation_is_not_replayed(self):
        smoke = object.__new__(module.NativeSmoke); smoke.application = Path("/synthetic/recallcard-desktop")
        previous = Mock(); previous.observations = []; smoke.driver = previous
        response = Mock(); response.__enter__ = Mock(return_value=response); response.__exit__ = Mock(return_value=False)
        response.read.return_value = b'{"value":{"ready":true}}'
        with patch.object(module, "urlopen", return_value=response), patch.object(module, "wait_for", side_effect=self.immediate), patch.object(module, "WebDriver", side_effect=ConnectionResetError) as create:
            with self.assertRaises(ConnectionResetError): smoke.reopen_application()
        previous.close.assert_called_once_with(); create.assert_called_once_with(smoke.application)
        self.assertIsNone(smoke.driver)

    @staticmethod
    def immediate(check, description, **kwargs):
        value = check()
        if not value:
            raise AssertionError(description)
        return value

    def driver(self):
        driver = object.__new__(module.WebDriver)
        driver.address = "http://127.0.0.1:4444"
        driver.session = "synthetic"
        return driver

    def test_workspace_navigation_uses_visible_controls_and_no_product_injection(self):
        driver = self.driver(); driver.click = Mock(); driver.idle = Mock(); driver.button = Mock()
        driver.navigate("随身背景")
        self.assertEqual([call.args[0] for call in driver.click.call_args_list],
                         ['#navigation [data-workspace="memories"]', '[data-action="background-select"]'])
        self.assertEqual(driver.idle.call_count, 2)
        driver.click.reset_mock(); driver.navigate("添加资料")
        driver.click.assert_called_once_with('#import-button')
        driver.click.reset_mock(); driver.navigate("整理记忆")
        driver.click.assert_called_once_with('#navigation [data-workspace="memories"]')
        driver.button.assert_called_once_with("整理记忆")

    def default_workspace(self, directory, *, native_title=None, screen=None, create=True):
        smoke = object.__new__(module.NativeSmoke); smoke.temporary = Path(directory)
        smoke.driver = Mock(); smoke.checkpoint = Mock()
        smoke.driver.assert_vault_badge = Mock()
        smoke.dialog_windows = Mock(side_effect=lambda title: ["42"] if title == native_title else [])
        identifier = json.loads((module.ROOT / "desktop/src-tauri/tauri.conf.json").read_text())["identifier"]
        vault = Path(directory) / "data" / identifier / "vault"
        environment = {name: str(Path(directory) / suffix) for name, suffix in [
            ("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("RECALLCARD_STATE_DIR", "state")
        ]}
        def start(label):
            self.assertEqual(label, "开始使用")
            self.assertFalse(vault.exists(), "只有真实开始动作才可创建测试默认目录")
            if create:
                for path in [vault / "control", vault / "events", vault / "memories"]:
                    path.mkdir(parents=True, exist_ok=True)
                (vault / "control/schema-version.json").write_text(json.dumps({"schema_version": 1, "application": "RecallCard"}))
        smoke.driver.button.side_effect = start
        smoke.driver.observe.side_effect = [screen or {"ready": True, "page": "导入会话", "importVisible": True, "modal": False}, True]
        smoke.driver.text.return_value = "切换资料库"
        return smoke, environment, vault

    def test_default_workspace_starts_once_in_isolated_data_then_opens_visible_switcher(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, environment, vault = self.default_workspace(directory)
            with patch.dict(module.os.environ, environment):
                evidence = smoke.exercise_default_workspace()
            self.assertTrue((vault / "control/schema-version.json").is_file())
            self.assertEqual(evidence["event_count"], 0)
            self.assertEqual(evidence["relative_path"], str(vault.relative_to(Path(directory))))
            smoke.driver.button.assert_called_once_with("开始使用")
            smoke.driver.click.assert_called_once_with("#switch-vault")
            smoke.driver.assert_vault_badge.assert_called_once_with("vault")
            smoke.checkpoint.assert_called_once_with("开始使用直接初始化临时默认资料库并进入导入")

    def test_default_workspace_refuses_unisolated_environment_before_any_action(self):
        for variable in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "RECALLCARD_STATE_DIR"]:
            with self.subTest(variable=variable), tempfile.TemporaryDirectory() as directory:
                smoke, environment, vault = self.default_workspace(directory)
                environment[variable] = "/unapproved-user-directory"
                with patch.dict(module.os.environ, environment), self.assertRaisesRegex(AssertionError, "未隔离"):
                    smoke.exercise_default_workspace()
                smoke.driver.button.assert_not_called(); smoke.driver.click.assert_not_called()
                self.assertFalse(vault.exists())

    def test_default_workspace_refuses_existing_vault_instead_of_reusing_it(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, environment, vault = self.default_workspace(directory)
            vault.mkdir(parents=True)
            with patch.dict(module.os.environ, environment), self.assertRaisesRegex(AssertionError, "首次起步"):
                smoke.exercise_default_workspace()
            smoke.driver.button.assert_not_called(); smoke.driver.click.assert_not_called()

    def test_default_workspace_rejects_native_picker_extra_confirmation_and_hidden_import_page(self):
        cases = [
            {"native_title": "选择用于新资料库的空文件夹"}, {"native_title": "创建资料库"},
            {"native_title": module.DEEPSEEK_DIALOG},
            {"screen": {"ready": True, "page": "导入会话", "importVisible": True, "modal": True}},
            {"screen": {"ready": True, "page": "导入会话", "importVisible": False, "modal": False}},
            {"screen": {"ready": True, "page": "会话", "importVisible": True, "modal": False}},
            {"screen": {"ready": True, "page": "导入会话", "importVisible": True, "error": "无法创建资料库"}},
        ]
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                smoke, environment, _ = self.default_workspace(directory, **case)
                with patch.dict(module.os.environ, environment), patch.object(module, "wait_for", side_effect=self.immediate), self.assertRaises(AssertionError):
                    smoke.exercise_default_workspace()
                smoke.driver.button.assert_called_once_with("开始使用")
                smoke.driver.click.assert_not_called(); smoke.checkpoint.assert_not_called()

    def test_default_workspace_requires_actual_schema_and_empty_event_memory_directories(self):
        for failure in ["missing", "schema", "events", "memories"]:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                smoke, environment, vault = self.default_workspace(directory, create=failure != "missing")
                start = smoke.driver.button.side_effect
                def invalid_start(label):
                    start(label)
                    if failure == "schema":
                        (vault / "control/schema-version.json").write_text('{"schema_version": 99}')
                    elif failure == "events":
                        (vault / "events/unexpected.jsonl").write_text('{}\n')
                    elif failure == "memories":
                        (vault / "memories/unexpected.md").write_text('合成非空记忆')
                smoke.driver.button.side_effect = invalid_start
                with patch.dict(module.os.environ, environment), self.assertRaises(FileNotFoundError if failure == "missing" else AssertionError):
                    smoke.exercise_default_workspace()
                smoke.driver.click.assert_not_called(); smoke.checkpoint.assert_not_called()

    def test_background_scope_change_requires_reset_tab_then_one_visible_reopen(self):
        driver = self.driver(); driver.select = Mock(); driver.observe = Mock(return_value="true")
        driver.click = Mock(); driver.idle = Mock()
        driver.select_background_scope("work")
        contract = module.UI_CONTRACTS["scope_background"]
        driver.select.assert_called_once_with(contract["scope_selector"], "work")
        driver.click.assert_called_once_with(contract["background_tab"])
        driver.idle.assert_called_once()
        driver.observe.return_value = "false"; driver.click.reset_mock()
        with self.assertRaises(AssertionError):
            driver.select_background_scope("personal")
        driver.click.assert_not_called()

    def test_vault_badge_checks_visible_dom_and_records_rendered_text_difference(self):
        driver = self.driver(); driver.observe = Mock(return_value={'text': '合成资料库', 'visible': True})
        driver.text = Mock(return_value='')
        driver.assert_vault_badge('合成资料库')
        self.assertEqual(driver.observations[0]['webdriver_rendered_text'], '')
        self.assertEqual(driver.observations[0]['dom'], {'text': '合成资料库', 'visible': True})
        self.assertTrue(driver.observe.call_args.args[0].startswith('return '))

    def test_vault_badge_does_not_accept_invisible_or_wrong_content(self):
        def immediate(check, description, **kwargs):
            if not check():
                raise AssertionError(description)
        for sample in [{'text': '错误资料库', 'visible': True}, {'text': '合成资料库', 'visible': False}]:
            driver = self.driver(); driver.observe = Mock(return_value=sample); driver.text = Mock()
            with patch.object(module, 'wait_for', side_effect=immediate):
                with self.assertRaisesRegex(AssertionError, '资料库标识不符'):
                    driver.assert_vault_badge('合成资料库')
            driver.text.assert_not_called()

    def test_workspace_density_fixture_is_synthetic_and_preserves_roles_and_unknown_times(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke); smoke.temporary = Path(directory)
            with zipfile.ZipFile(smoke.create_workspace_fixture()) as archive:
                self.assertEqual(len(archive.namelist()), 53)
                messages = []
                for name in archive.namelist():
                    conversation = json.loads(archive.read(name))
                    self.assertTrue(conversation['id'].startswith('workspace-density-'))
                    self.assertIn('工作区验收', conversation['title'])
                    messages.extend(node['message'] for node in conversation['mapping'].values() if node.get('message'))
                self.assertEqual(len(messages), 106)
                self.assertEqual({message['author']['role'] for message in messages}, {'user', 'assistant'})
                self.assertTrue(any('create_time' not in message for message in messages))
                self.assertTrue(any(len(message['content']['parts'][0]) > 2500 for message in messages))

    def test_direct_webkit_capabilities_match_official_tauri_linux_mapping(self):
        with patch.object(module.WebDriver, "request", return_value={"sessionId": "synthetic"}) as request:
            driver = module.WebDriver(Path("/synthetic/recallcard-desktop"))
            self.assertEqual(driver.session, "synthetic")
            self.assertEqual(request.call_args.args, ("POST", "/session", {"capabilities": {"alwaysMatch": {
                "browserName": "wry", "webkitgtk:browserOptions": {"binary": "/synthetic/recallcard-desktop", "args": []},
            }}}))
            self.assertEqual(request.call_count, 1)

    def test_native_driver_start_environment_and_failure_log_are_local(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke)
            smoke.artifacts = Path(directory); smoke.logs = []; smoke.processes = []; smoke.driver = None
            child = Mock(); child.poll.return_value = 0
            with patch.object(module.subprocess, "Popen", return_value=child) as start:
                self.assertIs(smoke.start_native_driver(), child)
                self.assertEqual(start.call_args.args[0], ["WebKitWebDriver", "--host=127.0.0.1", "--port=4444"])
                environment = start.call_args.kwargs["env"]
                self.assertEqual(environment["TAURI_AUTOMATION"], "true")
                self.assertEqual(environment["TAURI_WEBVIEW_AUTOMATION"], "true")
                self.assertNotIn("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", environment)
                self.assertEqual(start.call_args.kwargs["stderr"], subprocess.STDOUT)
                start.call_args.kwargs["stdout"].write("合成驱动错误记录\n")
            smoke.close()
            self.assertEqual((Path(directory) / "webkit-webdriver.log").read_text(), "合成驱动错误记录\n")

    def test_native_driver_cleanup_closes_session_and_reaps_process(self):
        smoke = object.__new__(module.NativeSmoke); smoke.driver = Mock(); smoke.logs = []
        child = Mock(); child.poll.return_value = None
        child.wait.side_effect = [subprocess.TimeoutExpired("synthetic", 5), 0]
        smoke.processes = [child]
        smoke.close()
        smoke.driver.close.assert_called_once()
        child.terminate.assert_called_once(); child.kill.assert_called_once()
        self.assertEqual(child.wait.call_count, 2)

    def test_native_driver_start_failure_preserves_log_and_does_not_create_session(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke)
            smoke.artifacts = Path(directory); smoke.logs = []; smoke.processes = []; smoke.driver = None
            with patch.object(module.subprocess, "Popen", side_effect=FileNotFoundError("合成缺失驱动")), patch.object(module, "WebDriver") as session:
                with self.assertRaises(FileNotFoundError):
                    smoke.start_native_driver()
                session.assert_not_called()
            self.assertEqual(smoke.processes, [])
            smoke.close()
            self.assertTrue((Path(directory) / "webkit-webdriver.log").exists())
            self.assertTrue(all(log.closed for log in smoke.logs))

    def test_target_is_located_and_scrolled_before_one_real_click(self):
        driver = self.driver()
        driver.command = Mock(side_effect=[{module.ELEMENT: "target"}, None, None])
        driver.click("#synthetic-save")
        calls = driver.command.call_args_list
        self.assertEqual(calls[0].args, ("POST", "/element", {"using": "css selector", "value": "#synthetic-save"}))
        self.assertEqual(calls[1].args[1], "/execute/sync")
        self.assertIn("scrollIntoView", calls[1].args[2]["script"])
        self.assertNotIn("click", calls[1].args[2]["script"])
        self.assertEqual(calls[2].args, ("POST", "/element/target/click", {}))

    def test_file_and_dream_controls_open_their_own_collapsed_section(self):
        for label, section in [("选择文件并预览", "file-import-details"), ("选择结果并审阅", "dream-file-options"), ("导出本次来源包", "dream-file-options")]:
            with self.subTest(label=label):
                driver = self.driver(); driver.observe = Mock(return_value=False); driver.click = Mock()
                driver.button(label)
                self.assertEqual(driver.click.call_args_list[0].args, (f"#{section} > summary",))
                self.assertEqual(driver.click.call_args_list[1].args, (f"//button[normalize-space(.)='{label}']", "xpath"))
                self.assertEqual(driver.click.call_count, 2)
                driver.observe.return_value = True; driver.click.reset_mock(); driver.button(label)
                self.assertEqual(driver.click.call_count, 1)

    def test_uncertain_picker_click_checks_visible_native_window_without_replaying(self):
        for label, title in [("选择文件并预览", "选择要导入的对话文件"), ("选择导出文件", module.DEEPSEEK_DIALOG)]:
            with self.subTest(label=label):
                driver = self.driver(); driver.observe = Mock(return_value=True); driver.click = Mock(side_effect=RemoteDisconnected("uncertain"))
                with patch.object(module.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout="123\n")) as probe:
                    driver.button(label)
                    self.assertEqual(driver.click.call_count, 1)
                    self.assertEqual(probe.call_args.args[0], ["xdotool", "search", "--onlyvisible", "--name", f"^{module.re.escape(title)}$"])

    def test_uncertain_multiple_picker_without_visible_window_stops_without_replaying(self):
        driver = self.driver(); driver.click = Mock(side_effect=RemoteDisconnected("uncertain"))
        with patch.object(module.subprocess, "run", return_value=SimpleNamespace(returncode=1, stdout="")), patch.object(module, "wait_for", side_effect=self.immediate):
            with self.assertRaises(AssertionError):
                driver.button("选择导出文件")
        driver.click.assert_called_once()

    def test_uncertain_data_write_is_not_treated_as_a_picker_or_replayed(self):
        driver = self.driver(); driver.click = Mock(side_effect=RemoteDisconnected("uncertain"))
        with patch.object(module.subprocess, "run") as probe:
            with self.assertRaises(RemoteDisconnected):
                driver.button("确认执行", "//dialog[@id='modal']")
            self.assertEqual(driver.click.call_count, 1)
            probe.assert_not_called()

    def test_clear_and_type_are_never_replayed_after_transport_failure(self):
        driver = self.driver()
        for endpoint in ["/session/synthetic/element/input/clear", "/session/synthetic/element/input/value"]:
            with self.subTest(endpoint=endpoint), patch.object(module, "urlopen", side_effect=RemoteDisconnected("uncertain")) as request:
                with self.assertRaises(RemoteDisconnected):
                    driver.request("POST", endpoint, {"text": "合成文字"})
                self.assertEqual(request.call_count, 1)

    def test_multiline_input_pastes_once_and_verifies_exact_newlines(self):
        driver = self.driver(); driver.find = Mock(return_value="input"); driver.command = Mock()
        driver.click = Mock(); value = '```json\n{"合成":true}\n```'; driver.observe = Mock(return_value=value)
        with patch.object(module.subprocess, "run") as process:
            driver.type("#result", value)
            driver.command.assert_not_called()
            driver.find.assert_not_called()  # 点击自行定位；不在粘贴前保存旧输入框引用
            self.assertEqual(process.call_args_list[0].kwargs["input"], value)
            self.assertEqual(process.call_args_list[1].args[0], ["xdotool", "key", "--clearmodifiers", "ctrl+a", "ctrl+v"])
            self.assertEqual(process.call_count, 2)
            driver.click.assert_called_once_with("#result")
            driver.observe.assert_called_once()

    def test_input_readback_mismatch_stops_before_product_confirmation(self):
        driver = self.driver(); driver.find = Mock(return_value="input"); driver.command = Mock()
        driver.click = Mock(); driver.observe = Mock(return_value='```json{} ```')
        def immediate(check, description, **kwargs):
            if not check():
                raise AssertionError(description)
        with patch.object(module.subprocess, "run") as process, patch.object(module, "wait_for", side_effect=immediate):
            with self.assertRaisesRegex(AssertionError, "完整保留"):
                driver.type("#result", '```json\n{}\n```')
            self.assertEqual(process.call_count, 2)

    def test_controlled_input_replacement_does_not_reuse_an_element_handle(self):
        driver = self.driver()
        driver.find = Mock(side_effect=AssertionError("输入方法不得保存元素引用"))
        driver.command = Mock(side_effect=AssertionError("不能清空后继续写过期元素"))
        driver.click = Mock()  # 真实 click 自行定位并聚焦一次
        driver.observe = Mock(side_effect=["旧节点的值", "work"])
        with patch.object(module.subprocess, "run") as process:
            driver.type("#import-scope", "work")
            driver.click.assert_called_once_with("#import-scope")
            self.assertEqual(driver.observe.call_count, 2)
            self.assertEqual(process.call_count, 2, "重读节点不能再次粘贴")

    def test_search_contract_opens_each_visible_record_and_checks_full_reference(self):
        smoke = object.__new__(module.NativeSmoke)
        smoke.driver = Mock()
        smoke.driver.observe.side_effect = [
            [{"text": "合成记忆原文", "visible": True}, {"text": "合成事件原文", "visible": True}],
            "合成记忆原文", "memory:mem_synthetic@3", "合成事件原文", "event:evt_synthetic",
        ]
        smoke.verify_search_records({"合成记忆原文": "memory:mem_synthetic@3", "合成事件原文": "event:evt_synthetic"})
        self.assertEqual(smoke.driver.click.call_count, 2)
        self.assertEqual(smoke.driver.idle.call_count, 2)

    def test_hidden_card_or_wrong_full_record_fails_search_contract(self):
        smoke = object.__new__(module.NativeSmoke); smoke.driver = Mock()
        smoke.driver.observe.return_value = [{"text": "合成原文", "visible": False}]
        with self.assertRaises(AssertionError):
            smoke.verify_search_records({"合成原文": "event:evt_synthetic"})
        smoke.driver.click.assert_not_called()
        smoke.driver.observe.side_effect = [[{"text": "合成原文", "visible": True}], "错误正文"]
        with self.assertRaises(AssertionError):
            smoke.verify_search_records({"合成原文": "event:evt_synthetic"})

    def test_deepseek_fixture_has_two_files_branches_roots_and_omitted_payloads(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke); smoke.temporary = Path(directory)
            paths = smoke.create_deepseek_fixtures()
            self.assertEqual(len(paths), 2)
            self.assertEqual(set(paths[0].parent.iterdir()), set(paths))
            first = json.loads(paths[0].read_text())[0]
            second = json.loads(paths[1].read_text())
            mapping = first["mapping"]
            self.assertNotIn("current_node", first)
            self.assertEqual([key for key, node in mapping.items() if node["parent"] is None], ["root", "separate"])
            self.assertEqual(mapping["q"]["children"], ["a", "b"])
            self.assertEqual(mapping["a"]["parent"], mapping["b"]["parent"])
            self.assertNotIn("inserted_at", mapping["b"]["message"])
            self.assertEqual(mapping["q"]["message"]["inserted_at"], "2026-01-02T08:00:00+08:00")
            self.assertEqual(mapping["a"]["message"]["inserted_at"], "2026-01-02T00:00:02.125Z")
            self.assertEqual(mapping["a"]["message"]["fragments"][0], {"type": "THINK", "content": module.DEEPSEEK_HIDDEN})
            self.assertEqual(mapping["q"]["message"]["files"][0]["content"], module.DEEPSEEK_ATTACHMENT)
            visible = [fragment["content"] for node in [*mapping.values(), *second["mapping"].values()]
                       if node["message"] for fragment in node["message"]["fragments"] if fragment["type"] != "THINK"]
            self.assertCountEqual(visible, module.DEEPSEEK_TEXTS.values())

    def test_canonical_deepseek_contract_rejects_missing_sibling_time_role_and_payload_leaks(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke); smoke.temporary = Path(directory)
            first, second = smoke.create_deepseek_fixtures()
            conversations = [json.loads(first.read_text())[0], json.loads(second.read_text())]
            events = []
            for conversation in conversations:
                for key, node in conversation["mapping"].items():
                    message = node["message"]
                    if message is None:
                        continue
                    visible = next(fragment for fragment in message["fragments"] if fragment["type"] != "THINK")
                    events.append({"id": f"evt_{key}", "content": visible["content"],
                        "role": "user" if visible["type"] == "REQUEST" else "assistant", "scope": "personal",
                        "occurred_at": message.get("inserted_at"),
                        "source": {"platform": "deepseek", "conversation_id": conversation["id"], "message_id": key},
                        "capture": {"completeness": "partial"},
                        "metadata": {"import_adapter": "deepseek-export", "previous_message_id": node["parent"],
                            "deepseek": {"parent_id": node["parent"], "children_ids": node["children"],
                                "attachments_omitted": len(message["files"]),
                                "hidden_fragments_omitted": sum(fragment["type"] == "THINK" for fragment in message["fragments"])}}})
            self.assertEqual(set(smoke.verify_deepseek_events(events)), set(module.DEEPSEEK_TEXTS))
            with self.assertRaisesRegex(AssertionError, "六条"):
                smoke.verify_deepseek_events([event for event in events if event["source"]["message_id"] != "a"])
            for key, field, value in [("b", "occurred_at", "2026-10-07T00:00:00Z"),
                                      ("a", "occurred_at", "2026-01-02T00:00:02Z"),
                                      ("second-a", "occurred_at", "2026-01-04T00:00:01Z"),
                                      ("b", "role", "user")]:
                with self.subTest(key=key, field=field):
                    altered = json.loads(json.dumps(events))
                    next(event for event in altered if event["source"]["message_id"] == key)[field] = value
                    with self.assertRaises(AssertionError):
                        smoke.verify_deepseek_events(altered)
            for hidden in (module.DEEPSEEK_HIDDEN, module.DEEPSEEK_ATTACHMENT, module.DEEPSEEK_ATTACHMENT_URL):
                with self.subTest(hidden=hidden):
                    altered = json.loads(json.dumps(events)); altered[0]["metadata"]["unexpected"] = hidden
                    with self.assertRaisesRegex(AssertionError, "不得保存"):
                        smoke.verify_deepseek_events(altered)

    def test_webdriver_option_branch_select_uses_visible_control_actual_option_order_once(self):
        driver = self.driver(); driver.click = Mock(); driver.idle = Mock()
        driver.observe = Mock(side_effect=[[
            {"value": "", "disabled": False}, {"value": "event:b", "disabled": False},
            {"value": "event:a", "disabled": False},
        ], "event:a"])
        with patch.object(module, "run") as action:
            driver.select_option("#continuation-branch", "event:a")
            driver.click.assert_called_once_with('#continuation-branch option[value="event:a"]')
            action.assert_not_called()
        self.assertTrue(all(call.args[0].startswith("return ") for call in driver.observe.call_args_list))
        driver.idle.assert_called_once()

    def test_webdriver_option_branch_select_refuses_missing_duplicate_or_disabled_options(self):
        for options in [[], [{"value": "event:a", "disabled": True}],
                        [{"value": "event:a", "disabled": False}] * 2]:
            with self.subTest(options=options):
                driver = self.driver(); driver.click = Mock(); driver.observe = Mock(return_value=options)
                with patch.object(module, "run") as action, self.assertRaises(AssertionError):
                    driver.select_option("#continuation-branch", "event:a")
                driver.click.assert_not_called(); action.assert_not_called()

    def test_webdriver_option_branch_select_mismatch_or_occluded_click_never_retries(self):
        driver = self.driver(); driver.click = Mock(); driver.idle = Mock()
        driver.observe = Mock(side_effect=[[{"value": "event:a", "disabled": False}], "event:b"])
        with patch.object(module, "run") as action, patch.object(module, "wait_for", side_effect=self.immediate):
            with self.assertRaisesRegex(AssertionError, "确切分支"):
                driver.select_option("#continuation-branch", "event:a")
            action.assert_not_called(); driver.click.assert_called_once(); driver.idle.assert_not_called()
        driver.click = Mock(side_effect=module.DriverError("element click intercepted"))
        driver.observe = Mock(return_value=[{"value": "event:a", "disabled": False}])
        with patch.object(module, "run") as action, self.assertRaises(module.DriverError):
            driver.select_option("#continuation-branch", "event:a")
        action.assert_not_called(); driver.click.assert_called_once()

    def test_background_import_waits_past_idle_until_actual_complete_dom(self):
        smoke = object.__new__(module.NativeSmoke); smoke.driver = Mock()
        running = {"heading": "正在导入", "processed": 2, "total": 6, "modal": False}
        complete = {"heading": "导入完成", "processed": 6, "total": 6, "modal": False}
        smoke.driver.observe.side_effect = [running, complete]
        with patch.object(module.time, "sleep"):
            self.assertEqual(smoke.confirm_import_job(6), complete)
        smoke.driver.button.assert_called_once_with("导入全部 6 条消息")
        smoke.driver.idle.assert_called_once()
        self.assertEqual(smoke.driver.observe.call_count, 2)
        self.assertTrue(all("#operation" not in call.args[0] for call in smoke.driver.observe.call_args_list))

    def test_background_import_rejects_failure_pause_progress_error_or_second_confirmation(self):
        for status in [
            {"heading": "已暂停"}, {"heading": "正在暂停"}, {"heading": "可以继续上次导入"},
            {"heading": "导入尚未完成"}, {"heading": "正在导入", "error": "暂时无法读取进度"},
            {"heading": "导入完成", "processed": 5, "total": 6},
            {"heading": "导入完成", "processed": 6, "total": 7},
            {"heading": "导入完成", "processed": 6, "total": 6, "modal": True},
        ]:
            with self.subTest(status=status):
                smoke = object.__new__(module.NativeSmoke); smoke.driver = Mock()
                smoke.driver.observe.return_value = status
                with self.assertRaises(AssertionError):
                    smoke.confirm_import_job(6)
                smoke.driver.button.assert_called_once_with("导入全部 6 条消息")
                smoke.driver.observe.assert_called_once()

    def test_background_import_uncertain_start_is_not_replayed_or_polled_as_success(self):
        smoke = object.__new__(module.NativeSmoke); smoke.driver = Mock()
        smoke.driver.button.side_effect = RemoteDisconnected("uncertain")
        with self.assertRaises(RemoteDisconnected):
            smoke.confirm_import_job(6)
        smoke.driver.button.assert_called_once(); smoke.driver.idle.assert_not_called(); smoke.driver.observe.assert_not_called()

    def test_native_file_selection_requires_visible_sensitive_selected_cells(self):
        api = SimpleNamespace(ROLE_TABLE_CELL=1, STATE_SHOWING=2, STATE_SENSITIVE=3, STATE_SELECTED=4)
        def cell(name, states, role=1):
            node = Mock(); node.name = name; node.getRole.return_value = role
            node.getState.return_value.contains.side_effect = lambda state: state in states
            return node
        shown = cell("one.json", {2, 3, 4})
        hidden = cell("two.json", {3, 4})
        unselected = cell("two.json", {2, 3})
        disabled = cell("two.json", {2, 4})
        other_role = cell("two.json", {2, 3, 4}, role=5)
        smoke = object.__new__(module.NativeSmoke); smoke.accessible_dialog = Mock()
        smoke.accessible_nodes = Mock(return_value=[(node, 0) for node in [shown, hidden, unselected, disabled, other_role]])
        with patch.dict(sys.modules, {"pyatspi": api}):
            self.assertEqual(smoke.native_file_cells(module.DEEPSEEK_DIALOG, {"one.json", "two.json"}, selected=True), {"one.json": shown})
            self.assertEqual(smoke.native_file_cells(module.DEEPSEEK_DIALOG, {"one.json", "two.json"}), {"one.json": shown, "two.json": unselected})
            smoke.accessible_nodes.return_value = [(shown, 0), (shown, 0)]
            with self.assertRaisesRegex(AssertionError, "同名"):
                smoke.native_file_cells(module.DEEPSEEK_DIALOG, {"one.json"})

    def multiple_picker(self, directory, selected=True, focused=True, accepted=True):
        smoke = object.__new__(module.NativeSmoke)
        smoke.temporary = Path(directory); paths = smoke.create_deepseek_fixtures()
        smoke.driver = Mock(); smoke.dialog_count = 0; smoke.capture = Mock(); smoke.describe_dialog = Mock()
        smoke.navigate_file_folder = Mock()
        smoke.native_file_list_focused = Mock(return_value=True)
        native = {"open": True}
        smoke.dialog_windows = Mock(side_effect=lambda title: ["42"] if native["open"] else [])
        smoke.accessible_dialog = Mock(return_value=object())
        cells = {}
        for path in paths:
            node = Mock()
            node.queryComponent.return_value.getExtents.return_value = SimpleNamespace(x=100, y=200, width=200, height=24)
            cells[path.name] = node
        smoke.native_file_cells = Mock(side_effect=lambda *_args, **kwargs: cells if not kwargs.get('selected') or selected else {paths[0].name: cells[paths[0].name]})
        approval = Mock()
        def approve(_):
            if accepted:
                native["open"] = False
            return accepted
        approval.queryAction.return_value.doAction.side_effect = approve
        smoke.native_button = Mock(return_value=approval)
        def execute(*args):
            if args[1] == "getwindowgeometry":
                return "X=0\nY=0\nWIDTH=900\nHEIGHT=700\n"
            if args[1] == "getactivewindow":
                return "42\n" if focused else "99\n"
            return ""
        return smoke, paths, approval, execute

    def test_multiple_picker_selects_visible_files_then_verifies_both_before_one_open(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, paths, approval, execute = self.multiple_picker(directory)
            order = []
            smoke.native_file_cells.side_effect = lambda *args, **kwargs: (order.append("selected" if kwargs else "visible") or {
                path.name: self.file_cell() for path in paths})
            original = approval.queryAction.return_value.doAction.side_effect
            approval.queryAction.return_value.doAction.side_effect = lambda value: (order.append("open") or original(value))
            with patch.dict(sys.modules, {"pyatspi": SimpleNamespace(DESKTOP_COORDS=0)}), patch.object(module, "run", side_effect=execute) as action, patch.object(module.time, "sleep"), patch.object(module, "wait_for", side_effect=self.immediate):
                smoke.dialog_files(paths)
            self.assertEqual(order, ["visible", "selected", "selected", "open"])
            smoke.navigate_file_folder.assert_called_once_with(module.DEEPSEEK_DIALOG, "42", paths[0].parent)
            self.assertFalse(any(call.args[1] == "type" for call in action.call_args_list))
            self.assertFalse(any(call.args[1] in ["click", "mousemove", "getwindowgeometry"] for call in action.call_args_list))
            self.assertEqual(sum(call.args == ("xdotool", "key", "--clearmodifiers", "ctrl+a") for call in action.call_args_list), 1)
            smoke.native_file_cells.assert_any_call(module.DEEPSEEK_DIALOG, {path.name for path in paths}, selected=True)
            approval.queryAction.return_value.doAction.assert_called_once_with(0)
            smoke.driver.idle.assert_called_once(); smoke.describe_dialog.assert_called_once_with(module.DEEPSEEK_DIALOG, "visible-files")

    @staticmethod
    def file_cell():
        cell = Mock()
        cell.queryComponent.return_value.getExtents.return_value = SimpleNamespace(x=100, y=200, width=200, height=24)
        return cell

    def test_multiple_picker_missing_selection_or_lost_focus_stops_before_open(self):
        for selected, focused in [(False, True), (True, False)]:
            with self.subTest(selected=selected, focused=focused), tempfile.TemporaryDirectory() as directory:
                smoke, paths, approval, execute = self.multiple_picker(directory, selected=selected, focused=focused)
                with patch.dict(sys.modules, {"pyatspi": SimpleNamespace(DESKTOP_COORDS=0)}), patch.object(module, "run", side_effect=execute) as action, patch.object(module.time, "sleep"), patch.object(module, "wait_for", side_effect=self.immediate):
                    with self.assertRaises(AssertionError):
                        smoke.dialog_files(paths)
                approval.queryAction.return_value.doAction.assert_not_called()
                smoke.native_button.assert_not_called(); smoke.driver.idle.assert_not_called()
                self.assertEqual(smoke.describe_dialog.call_args.args, (module.DEEPSEEK_DIALOG,))
                self.assertLessEqual(sum(call.args == ("xdotool", "click", "1") for call in action.call_args_list), 1)

    def test_multiple_picker_rejected_open_is_never_replayed(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, paths, approval, execute = self.multiple_picker(directory, accepted=False)
            with patch.dict(sys.modules, {"pyatspi": SimpleNamespace(DESKTOP_COORDS=0)}), patch.object(module, "run", side_effect=execute), patch.object(module.time, "sleep"), patch.object(module, "wait_for", side_effect=self.immediate):
                with self.assertRaisesRegex(AssertionError, "未接受点击"):
                    smoke.dialog_files(paths)
            approval.queryAction.return_value.doAction.assert_called_once_with(0)
            smoke.driver.idle.assert_not_called()

    def test_file_list_focus_requires_unique_visible_sensitive_files_table(self):
        api = SimpleNamespace(ROLE_TABLE=1, STATE_SHOWING=2, STATE_SENSITIVE=3, STATE_FOCUSED=4)
        def table(name="Files", states={2, 3, 4}):
            node = Mock(); node.getRole.return_value = api.ROLE_TABLE; node.name = name
            node.getState.return_value.contains.side_effect = lambda state: state in states
            return node
        smoke = object.__new__(module.NativeSmoke); smoke.accessible_dialog = Mock(); smoke.accessible_nodes = Mock()
        with patch.dict(sys.modules, {"pyatspi": api}):
            for node in [table("Recent"), table(states={2, 3}), table(states={3, 4}), table(states={2, 4})]:
                smoke.accessible_nodes.return_value = [(node, 0)]
                self.assertFalse(smoke.native_file_list_focused(module.DEEPSEEK_DIALOG))
            smoke.accessible_nodes.return_value = [(table(), 0)]
            self.assertTrue(smoke.native_file_list_focused(module.DEEPSEEK_DIALOG))
            focused = table(); actual = focused.queryTable.return_value
            smoke.accessible_nodes.return_value = [(focused, 0)]
            actual.nRows = 8; actual.getSelectedRows.return_value = [0, 1]
            self.assertFalse(smoke.native_file_list_focused(module.DEEPSEEK_DIALOG, expected_rows=2), "Recent额外行不能只看目标子集")
            actual.nRows = 2
            self.assertTrue(smoke.native_file_list_focused(module.DEEPSEEK_DIALOG, expected_rows=2, all_selected=True))
            for selected in [[], [0], [0, 1, 2], [0, 0]]:
                actual.getSelectedRows.return_value = selected
                self.assertFalse(smoke.native_file_list_focused(module.DEEPSEEK_DIALOG, expected_rows=2, all_selected=True))
            smoke.accessible_nodes.return_value = [(table(), 0), (table(), 0)]
            with self.assertRaisesRegex(AssertionError, "不唯一"):
                smoke.native_file_list_focused(module.DEEPSEEK_DIALOG)

    def test_multiple_picker_never_sends_select_all_without_file_list_focus(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, paths, approval, execute = self.multiple_picker(directory)
            smoke.native_file_list_focused.return_value = False
            with patch.object(module, "run", side_effect=execute) as action, patch.object(module, "wait_for", side_effect=self.immediate):
                with self.assertRaisesRegex(AssertionError, "原生列表"):
                    smoke.dialog_files(paths)
            self.assertFalse(any(call.args[-1] == "ctrl+a" for call in action.call_args_list))
            approval.queryAction.return_value.doAction.assert_not_called()
            smoke.driver.idle.assert_not_called()

    def test_multiple_picker_refuses_extra_files_mixed_folders_or_duplicate_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, paths, _, _ = self.multiple_picker(directory)
            extra = Path(directory) / "extra.json"; extra.write_text("{}")
            with patch.object(module, "run") as action:
                for inputs in [[paths[0], paths[0]], [paths[0], extra]]:
                    with self.subTest(inputs=inputs), self.assertRaises(AssertionError):
                        smoke.dialog_files(inputs)
                (paths[0].parent / "unapproved.json").write_text("{}")
                with self.assertRaisesRegex(AssertionError, "未批准"):
                    smoke.dialog_files(paths)
                action.assert_not_called(); smoke.dialog_windows.assert_not_called()

    def test_native_location_entry_requires_unique_visible_focused_editable_text(self):
        api = SimpleNamespace(ROLE_TEXT=1, STATE_SHOWING=2, STATE_SENSITIVE=3, STATE_FOCUSED=4, STATE_EDITABLE=5)
        def entry(states):
            node = Mock(); node.getRole.return_value = api.ROLE_TEXT
            node.getAttributes.return_value = ["placeholder-text:Location"]
            node.getState.return_value.contains.side_effect = lambda state: state in states
            return node
        correct = entry({2, 3, 4, 5})
        smoke = object.__new__(module.NativeSmoke); smoke.accessible_dialog = Mock(); smoke.accessible_nodes = Mock()
        with patch.dict(sys.modules, {"pyatspi": api}):
            for absent in [2, 3, 4, 5]:
                smoke.accessible_nodes.return_value = [(entry({2, 3, 4, 5} - {absent}), 0)]
                self.assertIsNone(smoke.native_location_entry(module.DEEPSEEK_DIALOG))
            smoke.accessible_nodes.return_value = [(correct, 0)]
            self.assertIs(smoke.native_location_entry(module.DEEPSEEK_DIALOG), correct)
            search = entry({2, 3, 4, 5}); search.getAttributes.return_value = ["placeholder-text:Search"]
            smoke.accessible_nodes.return_value = [(search, 0)]
            self.assertIsNone(smoke.native_location_entry(module.DEEPSEEK_DIALOG))
            smoke.accessible_nodes.return_value = [(correct, 0), (correct, 0)]
            with self.assertRaisesRegex(AssertionError, "不唯一"):
                smoke.native_location_entry(module.DEEPSEEK_DIALOG)

    def test_native_folder_navigation_pastes_and_reads_back_before_one_return(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke = object.__new__(module.NativeSmoke); smoke.artifacts = Path(directory); smoke.dialog_count = 1
            smoke.capture = Mock(); smoke.describe_dialog = Mock(); node = Mock()
            node.queryText.return_value.getText.return_value = directory + "/"
            smoke.native_location_entry = Mock(return_value=node)
            with patch.object(module, "run", return_value="42\n") as action, patch.object(module.subprocess, "run") as paste, patch.object(module, "wait_for", side_effect=self.immediate):
                smoke.navigate_file_folder(module.DEEPSEEK_DIALOG, "42", Path(directory))
            self.assertEqual(paste.call_count, 1); self.assertEqual(paste.call_args.kwargs['input'], directory + "/")
            self.assertEqual(action.call_args_list[-1].args, ("xdotool", "key", "--clearmodifiers", "Return"))
            self.assertEqual(sum(call.args[-1] == "Return" for call in action.call_args_list), 1)
            self.assertGreaterEqual(node.queryText.return_value.getText.call_count, 2)
            self.assertFalse(any(call.args[-1] == "ctrl+l" for call in action.call_args_list), "已聚焦位置框不能被切换隐藏")
            evidence = json.loads((smoke.artifacts / "dialog-01-location.json").read_text())
            self.assertEqual(evidence, {"expected": directory + "/", "observed": directory + "/"})

    def test_native_folder_mismatched_text_or_lost_focus_stops_without_return(self):
        for text_ok, focused in [(False, True), (True, False)]:
            with self.subTest(text_ok=text_ok, focused=focused), tempfile.TemporaryDirectory() as directory:
                smoke = object.__new__(module.NativeSmoke); smoke.artifacts = Path(directory); smoke.dialog_count = 1
                smoke.capture = Mock(); smoke.describe_dialog = Mock(); node = Mock()
                node.queryText.return_value.getText.return_value = directory + "/" if text_ok else "/synthetic/wrong/"
                smoke.native_location_entry = Mock(return_value=node)
                with patch.object(module, "run", return_value="42\n" if focused else "99\n") as action, patch.object(module.subprocess, "run"), patch.object(module, "wait_for", side_effect=self.immediate):
                    with self.assertRaises(AssertionError):
                        smoke.navigate_file_folder(module.DEEPSEEK_DIALOG, "42", Path(directory))
                self.assertFalse(any(call.args[-1] == "Return" for call in action.call_args_list))

    def test_native_folder_removes_only_verified_selected_completion_once(self):
        with tempfile.TemporaryDirectory() as directory:
            expected = directory + "/"; observed = {"text": expected + "deepseek-"}
            smoke = object.__new__(module.NativeSmoke); smoke.artifacts = Path(directory); smoke.dialog_count = 1
            smoke.capture = Mock(); smoke.describe_dialog = Mock(); node = Mock(); text = node.queryText.return_value
            smoke.native_location_entry = Mock(return_value=node)
            text.getText.side_effect = lambda *_: observed["text"]
            text.getNSelections.return_value = 1; text.getSelection.return_value = (len(expected), len(observed["text"]))
            def execute(*args):
                if args[-1] == "BackSpace": observed["text"] = expected
                return "42\n"
            with patch.object(module, "run", side_effect=execute) as action, patch.object(module.subprocess, "run"), patch.object(module, "wait_for", side_effect=self.immediate):
                smoke.navigate_file_folder(module.DEEPSEEK_DIALOG, "42", Path(directory))
            keys = [call.args[-1] for call in action.call_args_list]
            self.assertEqual(keys.count("BackSpace"), 1); self.assertEqual(keys.count("Return"), 1)
            self.assertLess(keys.index("BackSpace"), keys.index("Return"))
            self.assertEqual(json.loads((smoke.artifacts / 'dialog-01-location.json').read_text())["observed"], expected)
            self.assertEqual(json.loads((smoke.artifacts / 'dialog-01-completion.json').read_text())["observed"], expected + "deepseek-")

    def test_native_folder_never_deletes_unselected_partial_or_multiple_selections(self):
        for count, offset in [(0, 0), (2, 0), (1, -1), (1, 1)]:
            with self.subTest(count=count, offset=offset), tempfile.TemporaryDirectory() as directory:
                expected = directory + "/"; observed = expected + "deepseek-"
                smoke = object.__new__(module.NativeSmoke); smoke.artifacts = Path(directory); smoke.dialog_count = 1
                smoke.capture = Mock(); smoke.describe_dialog = Mock(); node = Mock(); text = node.queryText.return_value
                smoke.native_location_entry = Mock(return_value=node)
                text.getText.return_value = observed; text.getNSelections.return_value = count
                text.getSelection.return_value = (len(expected) + offset, len(observed))
                with patch.object(module, "run", return_value="42\n") as action, patch.object(module.subprocess, "run"), patch.object(module, "wait_for", side_effect=self.immediate):
                    with self.assertRaisesRegex(AssertionError, "补全尾部"):
                        smoke.navigate_file_folder(module.DEEPSEEK_DIALOG, "42", Path(directory))
                self.assertFalse(any(call.args[-1] in ["BackSpace", "Return"] for call in action.call_args_list))

    def test_multiple_picker_closed_during_navigation_is_not_reopened_or_confirmed(self):
        with tempfile.TemporaryDirectory() as directory:
            smoke, paths, approval, execute = self.multiple_picker(directory)
            smoke.navigate_file_folder.side_effect = lambda *_: setattr(smoke.dialog_windows, 'side_effect', lambda _: [])
            with patch.object(module, "run", side_effect=execute), patch.object(module, "wait_for", side_effect=self.immediate):
                with self.assertRaisesRegex(AssertionError, "已经关闭"):
                    smoke.dialog_files(paths)
            smoke.navigate_file_folder.assert_called_once()
            approval.queryAction.return_value.doAction.assert_not_called()
            smoke.native_file_cells.assert_not_called(); smoke.driver.idle.assert_not_called()


if __name__ == "__main__":
    unittest.main()
