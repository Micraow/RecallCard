"""只读传输可以恢复；不重复有副作用的 WebDriver 操作。"""
import importlib.util
import io
import json
from urllib.error import HTTPError
import pathlib
import unittest
from unittest.mock import patch
from http.client import RemoteDisconnected
spec=importlib.util.spec_from_file_location("native_smoke",pathlib.Path(__file__).with_name("native_smoke.py"))
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
class DriverTransportTest(unittest.TestCase):
    def driver(self):
        driver=object.__new__(module.WebDriver);driver.address="http://127.0.0.1:4444";return driver
    def test_successful_script_objects_can_include_application_error_fields(self):
        for value in [{"ready": True, "page": "导入会话", "error": None},
                      {"error": "合成页面自己的诊断信息", "message": "这不是协议响应"}]:
            with self.subTest(value=value), patch.object(module, "urlopen", return_value=io.BytesIO(json.dumps({"value": value}).encode())):
                self.assertEqual(self.driver().request("POST", "/session/demo/execute/sync", {"script": "return document.title", "args": []}), value)
    def test_http_protocol_errors_still_fail_without_retry(self):
        error = HTTPError("http://127.0.0.1:4444/session/demo/element", 404, "Not Found", {}, io.BytesIO(b'{"value":{"error":"no such element","message":"synthetic missing"}}'))
        with patch.object(module, "urlopen", side_effect=error) as request:
            with self.assertRaisesRegex(module.DriverError, "no such element"):
                self.driver().request("POST", "/session/demo/element", {"using": "css selector", "value": "#missing"})
            self.assertEqual(request.call_count, 1)
    def test_read_only_probe_recovers_closed_connection(self):
        with patch.object(module,"urlopen",side_effect=[RemoteDisconnected("closed"),io.BytesIO(b'{"value":true}')]) as request:
            self.assertTrue(self.driver().request("POST","/session/demo/execute/sync",{"script":"return document.readyState", "args":[]}))
            self.assertEqual(request.call_count,2)
    def test_click_and_session_creation_are_not_replayed(self):
        for path in ["/session/demo/element/1/click","/session"]:
            with patch.object(module,"urlopen",side_effect=RemoteDisconnected("closed")) as request:
                with self.assertRaises(RemoteDisconnected):self.driver().request("POST",path,{})
                self.assertEqual(request.call_count,1)
    def test_recovery_has_a_bound(self):
        with patch.object(module,"urlopen",side_effect=RemoteDisconnected("closed")) as request:
            with self.assertRaises(RemoteDisconnected):self.driver().request("GET","/session/demo/screenshot")
            self.assertEqual(request.call_count,3)
    def test_stale_text_relocates_same_selector_and_keeps_rendered_text(self):
        driver=self.driver()
        stale=module.DriverError(json.dumps({"value":{"error":"stale element reference"}}))
        with patch.object(driver,"find",side_effect=["old-node","new-node"]) as find, patch.object(driver,"command",side_effect=[stale,"完整合成原文末尾"]) as command, patch.object(module.time,"sleep"):
            self.assertEqual(driver.text(".message[data-reference='event:synthetic']"),"完整合成原文末尾")
            self.assertEqual([c.args for c in find.call_args_list],[(".message[data-reference='event:synthetic']",)]*2)
            self.assertEqual([c.args for c in command.call_args_list],[("GET","/element/old-node/text"),("GET","/element/new-node/text")])
    def test_persistently_stale_text_fails_after_three_fresh_observations(self):
        driver=self.driver();stale=module.DriverError(json.dumps({"value":{"error":"stale element reference"}}))
        with patch.object(driver,"find",return_value="node") as find, patch.object(driver,"command",side_effect=stale) as command, patch.object(module.time,"sleep"):
            with self.assertRaisesRegex(module.DriverError,"stale element reference"):driver.text(".synthetic")
            self.assertEqual(find.call_count,3);self.assertEqual(command.call_count,3)
    def test_missing_or_invalid_read_errors_are_not_hidden_by_stale_recovery(self):
        for body in [{"value":{"error":"no such element"}}, {"value":{"error":"invalid session id"}}, {"value":"bad"}, ["bad"], "not json"]:
            driver=self.driver();error=module.DriverError(body if isinstance(body,str) else json.dumps(body))
            with self.subTest(body=body), patch.object(driver,"find",return_value="node"), patch.object(driver,"command",side_effect=error) as command, patch.object(module.time,"sleep"):
                with self.assertRaises(module.DriverError):driver.text(".synthetic")
                self.assertEqual(command.call_count,1)
    def test_stale_scroll_preparation_relocates_before_one_click(self):
        driver = self.driver()
        stale = module.DriverError(json.dumps({"value": {"error": "stale element reference"}}))
        with patch.object(driver, "find", side_effect=["old", "current"]) as find, patch.object(driver, "command", side_effect=[stale, None, None]) as command, patch.object(module.time, "sleep"):
            driver.click(".source-row")
        self.assertEqual([call.args for call in find.call_args_list], [(".source-row", "css selector")] * 2)
        self.assertEqual([call.args[1] for call in command.call_args_list], ["/execute/sync", "/execute/sync", "/element/current/click"])
        self.assertEqual([call.args[2]["args"][0][module.ELEMENT] for call in command.call_args_list[:2]], ["old", "current"])

    def test_persistent_stale_scroll_never_dispatches_a_click(self):
        driver = self.driver()
        stale = module.DriverError(json.dumps({"value": {"error": "stale element reference"}}))
        with patch.object(driver, "find", return_value="replaced") as find, patch.object(driver, "command", side_effect=stale) as command, patch.object(module.time, "sleep"):
            with self.assertRaisesRegex(module.DriverError, "stale element reference"):
                driver.click(".source-row")
        self.assertEqual(find.call_count, 3)
        self.assertEqual([call.args[1] for call in command.call_args_list], ["/execute/sync"] * 3)

    def test_non_stale_scroll_errors_are_not_retried_or_clicked(self):
        for error in [module.DriverError('{"value":{"error":"no such element"}}'),
                      module.DriverError('{"value":{"error":"invalid session id"}}'),
                      module.DriverError('bad response'), ConnectionResetError('unknown scroll outcome')]:
            driver = self.driver()
            with self.subTest(error=error), patch.object(driver, "find", return_value="node") as find, patch.object(driver, "command", side_effect=error) as command:
                with self.assertRaises(type(error)):
                    driver.click(".source-row")
                self.assertEqual(find.call_count, 1)
                self.assertEqual(command.call_count, 1)
                self.assertEqual(command.call_args.args[1], "/execute/sync")

    def test_xpath_target_is_preserved_when_preparation_relocates(self):
        driver = self.driver()
        stale = module.DriverError('{"value":{"error":"stale element reference"}}')
        with patch.object(driver, "find", side_effect=["old", "current"]) as find, patch.object(driver, "command", side_effect=[stale, None, None]), patch.object(module.time, "sleep"):
            driver.click("//button[.='确认']", "xpath")
        self.assertEqual([call.args for call in find.call_args_list], [("//button[.='确认']", "xpath")] * 2)

    def test_click_stale_failure_does_not_replay_the_action(self):
        driver=self.driver();stale=module.DriverError(json.dumps({"value":{"error":"stale element reference"}}))
        with patch.object(driver,"find",return_value="node"), patch.object(driver,"command",side_effect=[None,stale]) as command:
            with self.assertRaisesRegex(module.DriverError,"stale element reference"):driver.click(".synthetic")
            self.assertEqual(command.call_count,2)
            self.assertEqual(command.call_args.args,("POST","/element/node/click",{}))

if __name__=="__main__":unittest.main()
