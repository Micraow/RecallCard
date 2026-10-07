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
if __name__=="__main__":unittest.main()
