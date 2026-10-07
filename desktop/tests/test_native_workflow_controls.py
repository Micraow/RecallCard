"""不启动浏览器：核对真实原生验收驱动的动作顺序和不确定写入处理。"""
import importlib.util
from http.client import RemoteDisconnected
from pathlib import Path
from types import SimpleNamespace
import unittest
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
            self.assertEqual(driver.command.call_args_list[0].args, ("POST", "/element/input/clear", {}))
            self.assertEqual(driver.command.call_count, 1, "不再使用会丢失换行的 Send Keys")
            self.assertEqual(process.call_args_list[0].kwargs["input"], value)
            self.assertEqual(process.call_args_list[1].args[0], ["xdotool", "key", "--clearmodifiers", "ctrl+v"])
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
