"""生产排名核回放的原创小合同；不读取样本集，不运行 Cargo 或下载依赖。

编译合同在 setUpClass 中仅构建一次，产物自动清理且始终位于 /tmp。
没有可用 rustc 时跳过编译合同；独立词法提取合同仍然运行。
"""
import hashlib
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path
import unittest

if __package__:
    from .production import ProductionRank, _extract_functions, _frame, _MAGIC
else:
    from production import ProductionRank, _extract_functions, _frame, _MAGIC


REPO = Path(__file__).resolve().parents[2]
_SYNTHETIC_FUNCTION = (
    'pub fn tokenize(text: &str) -> Vec<String> { '
    'let raw = r###"} fn rank() {"###; '
    "let c = '{'; /* { /* } */ } */ vec![text.to_owned()] }"
)


class ExtractionContracts(unittest.TestCase):
    """无需编译器：只验证语法边界与 fail-closed 合同。"""

    def test_raw_strings_chars_and_nested_comments_preserve_body(self):
        self.assertEqual(
            _extract_functions(_SYNTHETIC_FUNCTION, ["tokenize"]),
            {"tokenize": _SYNTHETIC_FUNCTION},
        )

    def test_duplicate_top_level_function_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "重复"):
            _extract_functions(_SYNTHETIC_FUNCTION + "\n" + _SYNTHETIC_FUNCTION, ["tokenize"])

    def test_nested_only_function_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "未找到唯一顶层函数"):
            _extract_functions("mod inner {\n" + _SYNTHETIC_FUNCTION + "\n}", ["tokenize"])

    def test_attribute_drift_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "签名或属性"):
            _extract_functions("#[cfg(any())]\n" + _SYNTHETIC_FUNCTION, ["tokenize"])

    def test_signature_drift_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "签名或属性"):
            _extract_functions(_SYNTHETIC_FUNCTION.replace("text: &str", "text: String"), ["tokenize"])

    def test_unterminated_function_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "语法边界不完整"):
            _extract_functions(_SYNTHETIC_FUNCTION[:-1], ["tokenize"])

    def test_unterminated_raw_literal_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "未闭合的原始字符串"):
            _extract_functions(_SYNTHETIC_FUNCTION.replace('"###;', '"##;'), ["tokenize"])

    def test_unterminated_comment_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "未闭合的块注释"):
            _extract_functions(_SYNTHETIC_FUNCTION + "\n/*", ["tokenize"])


class ProductionRankContracts(unittest.TestCase):
    """仅用临时原创字符串验证真实 Rust 核与协议，不覆盖完整 Vault 链路。"""

    @classmethod
    def setUpClass(cls):
        compiler = shutil.which("rustc")
        if compiler is None:
            raise unittest.SkipTest("没有 rustc，跳过编译合同；词法合同仍独立运行")
        version = subprocess.run([compiler, "--version"], capture_output=True, timeout=30)
        if version.returncode:
            raise unittest.SkipTest("rustc 工具链不可用，跳过编译合同；词法合同仍独立运行")
        cls.temporary = tempfile.TemporaryDirectory(prefix="recall-rank-unittest-", dir="/tmp")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.rank = ProductionRank(REPO, cls.temporary.name, rustc=compiler)
        cls.documents = [
            {"source_id": "z", "text": "Cache"},
            {"source_id": "a", "text": "Cache"},
            {"source_id": "near", "text": "Cached"},
        ]
        cls.valid_empty_request = _MAGIC + _frame("Cache") + _frame("exact") + struct.pack("<I", 0)

    def assert_empty_query(self, query):
        self.assertEqual(self.rank.terms(query, "exact"), [])
        self.assertEqual(self.rank.search(query, "exact", self.documents), [])

    def assert_protocol_rejects(self, request):
        result = subprocess.run([str(self.rank.binary)], input=request, capture_output=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertTrue(result.stderr)

    def test_exact_identifier_boundary_and_stable_source_tie(self):
        rows = self.rank.search("Cache", "exact", self.documents)
        self.assertEqual([row["id"] for row in rows], ["a", "z"])
        self.assertEqual(rows[0]["score"], rows[1]["score"])

    def test_identifier_prefix_includes_longer_identifier(self):
        rows = self.rank.search("Cache", "identifier_prefix", self.documents)
        self.assertEqual({row["id"] for row in rows}, {"a", "z", "near"})

    def test_natural_question_keeps_ascii_identifier_exact(self):
        rows = self.rank.search("Cache", "natural_question", self.documents)
        self.assertEqual([row["id"] for row in rows], ["a", "z"])

    def test_empty_string_returns_no_terms_or_results(self):
        self.assert_empty_query("")

    def test_spaces_return_no_terms_or_results(self):
        self.assert_empty_query("   ")

    def test_control_whitespace_returns_no_terms_or_results(self):
        self.assert_empty_query("\n\t")

    def test_punctuation_returns_no_terms_or_results(self):
        self.assert_empty_query("？？？")

    def test_nul_returns_no_terms_or_results(self):
        self.assert_empty_query("\x00")

    def test_unrelated_query_returns_no_results(self):
        self.assertEqual(self.rank.search("absent", "exact", self.documents), [])

    def test_fields_evidence_and_neutral_defaults(self):
        rows = self.rank.search("Cache", "exact", [
            {"source_id": "neutral", "text": "Cache"},
            {"source_id": "user", "text": "Cache", "evidence": "UserExplicit"},
            {"source_id": "assistant", "text": "Cache", "evidence": "AssistantSuggestion"},
            {"source_id": "entity", "text": "none", "entities": ["Cache"], "evidence": "UserExplicit"},
            {"source_id": "tentative", "text": "Cache", "state": "tentative"},
        ])
        self.assertEqual([row["id"] for row in rows], ["user", "entity", "neutral", "tentative", "assistant"])

    def test_nul_newline_and_unicode_framing(self):
        text = '\x00first\nsecond\t"} Cache 中🙂'
        rows = self.rank.search("Cache", "exact", [{"source_id": text, "text": text}])
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], text)

    def test_quoted_long_han_stays_exact(self):
        self.assertEqual(
            self.rank.terms('"这是哪个项目的缓存设置"', "natural_question"),
            ["这是哪个项目的缓存设置"],
        )

    def test_real_ranking_terms_returns_sorted_han_bigrams(self):
        self.assertEqual(
            self.rank.terms("这是哪个项目的缓存设置", "natural_question"),
            ["个项", "哪个", "存设", "是哪", "的缓", "目的", "缓存", "设置", "这是", "项目"],
        )

    def test_independent_processes_return_identical_scores(self):
        # search 每次启动新的独立 Rust 进程，共用 setUpClass 已编译的唯一二进制。
        self.assertEqual(
            self.rank.search("Cache", "exact", self.documents),
            self.rank.search("Cache", "exact", list(reversed(self.documents))),
        )

    def test_duplicate_source_id_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "必须唯一"):
            self.rank.search("x", "exact", [{"source_id": "a", "text": "x"}] * 2)

    def test_invalid_mode_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "不支持的 lexical_match"):
            self.rank.terms("x", "unknown")

    def test_string_instead_of_vector_is_rejected(self):
        with self.assertRaisesRegex(TypeError, "必须是字符串列表"):
            self.rank.search("x", "exact", [{"source_id": "a", "text": "x", "entities": "x"}])

    def test_output_inside_repository_is_rejected_before_write(self):
        output = REPO / "rank-test-must-not-be-created"
        self.assertFalse(output.exists())
        with self.assertRaisesRegex(ValueError, "编译目录必须位于 /tmp|编译产物不能写入项目仓库"):
            ProductionRank(REPO, output)
        self.assertFalse(output.exists())

    def test_protocol_rejects_bad_magic(self):
        self.assert_protocol_rejects(b"NOTRANK!" + self.valid_empty_request[8:])

    def test_protocol_rejects_truncation(self):
        self.assert_protocol_rejects(self.valid_empty_request[:-1])

    def test_protocol_rejects_trailing_bytes(self):
        self.assert_protocol_rejects(self.valid_empty_request + b"\0")

    def test_protocol_rejects_invalid_utf8(self):
        self.assert_protocol_rejects(_MAGIC + struct.pack("<I", 1) + b"\xff" + self.valid_empty_request[8:])

    def test_protocol_rejects_oversized_frame(self):
        self.assert_protocol_rejects(_MAGIC + struct.pack("<I", 2**32 - 1))

    def test_identity_hashes_match_sources_and_compiled_artifacts(self):
        identity = self.rank.identity
        for filename, expected in identity["source_sha256"].items():
            with self.subTest(source=filename):
                self.assertEqual(hashlib.sha256((REPO / filename).read_bytes()).hexdigest(), expected)
        generated = Path(identity["generated_rust"])
        self.assertEqual(hashlib.sha256(generated.read_bytes()).hexdigest(), identity["generated_rust_sha256"])
        self.assertEqual(hashlib.sha256(self.rank.binary.read_bytes()).hexdigest(), identity["binary_sha256"])
        self.assertFalse(identity["full_vault_e2e"])
        self.assertIn("source_id", identity["reference_mapping"])
        self.assertTrue(generated.is_relative_to(Path("/tmp").resolve()))
        self.assertTrue(self.rank.binary.is_relative_to(Path("/tmp").resolve()))


if __name__ == "__main__":
    unittest.main()
