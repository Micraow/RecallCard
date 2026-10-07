"""不启动浏览器：核对真实原生验收驱动的动作顺序和不确定写入处理。"""
import importlib.util
import json
from http.client import RemoteDisconnected
from pathlib import Path
from types import SimpleNamespace
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("native_controls", Path(__file__).with_name("native_smoke.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class NativeControlsTest(unittest.TestCase):
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
        driver = self.driver(); driver.observe = Mock(return_value=True); driver.click = Mock(side_effect=RemoteDisconnected("uncertain"))
        with patch.object(module.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout="123\n")) as probe:
            driver.button("选择文件并预览")
            self.assertEqual(driver.click.call_count, 1)
            self.assertEqual(probe.call_args.args[0], ["xdotool", "search", "--onlyvisible", "--name", "^选择要导入的对话文件$"])

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


if __name__ == "__main__":
    unittest.main()
