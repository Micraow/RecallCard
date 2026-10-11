"""导航协议的纯合成、无网络回归；不证明模型能生成正确分类。"""
import unittest
from copy import deepcopy
from test_dream_api import fixture, result_for
from recallcard_dream.client import validate_job, validate_result, DreamError, OUTPUT_SCHEMA, SYSTEM_INSTRUCTIONS

class NavigationTests(unittest.TestCase):
    def test_old_job_and_result_are_unchanged(self):
        job=fixture(); self.assertEqual(validate_job(job),job)
        result=result_for(job); self.assertEqual(validate_result(result,job),result)

    def test_nav_memory_snapshot_and_result_are_accepted_without_mutation(self):
        job=fixture(); hints=[{"path":"topics/network","description":"合成简介","aliases":["合成别名"]}]
        job["memory_read_set"][0]["memory"]["navigation"]=deepcopy(hints)
        self.assertEqual(validate_job(job),job)
        result=result_for(job,"update"); result["proposals"][0]["navigation"]=hints
        original=deepcopy(result); self.assertEqual(validate_result(result,job),original)
        self.assertIn("navigation",OUTPUT_SCHEMA["properties"]["proposals"]["items"]["properties"])
        self.assertIn("navigation",SYSTEM_INSTRUCTIONS)

    def test_update_cannot_silently_keep_or_drop_navigation(self):
        job=fixture(); job["memory_read_set"][0]["memory"]["navigation"]=[{"path":"old"}]
        for nav in ("absent",None):
            result=result_for(job,"update")
            if nav is None: result["proposals"][0]["navigation"]=None
            with self.assertRaises(DreamError):validate_result(result,job)
        result["proposals"][0]["navigation"]=[];self.assertEqual(validate_result(result,job),result)

    def test_invalid_paths_bounds_and_noop_modification_fail(self):
        for hints in ([{"path":p}] for p in ["", "A", "../secret", "a//b", "_root", "labels/abc", "a/b/c/d/e/f/g", "中文", "a%2fb"]):
            result=result_for();result["proposals"][0]["navigation"]=hints
            with self.subTest(hints=hints),self.assertRaises(DreamError):validate_result(result,fixture())
        invalid=[[{"path":"a"},{"path":"a"}], [{"path":"a","description":"字"*171}], [{"path":"a","aliases":["a"]*9}], [{"path":"a","title":"bad\nname"}], [{"path":"a","keywords":["\u0080"]}], [{"path":"a","extra":"x"}]]
        for hints in invalid:
            result=result_for(); result["proposals"][0]["navigation"]=hints
            with self.subTest(hints=hints), self.assertRaises(DreamError): validate_result(result,fixture())
        result=result_for(operation="noop");result["proposals"][0]["navigation"]=[{"path":"a"}]
        with self.assertRaises(DreamError):validate_result(result,fixture())

    def test_python_checks_default_expanded_byte_size(self):
        result=result_for();result["proposals"][0]["navigation"]=[{"path":f"p{i}","description":"d"*512,"keywords":[f"{j:096}" for j in range(16)]} for i in range(8)]
        with self.assertRaises(DreamError):validate_result(result,fixture())
