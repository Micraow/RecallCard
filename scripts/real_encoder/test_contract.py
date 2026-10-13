"""纯 stdlib 单测；不下载模型、不加载 FastEmbed、不创建监听端口。"""
import hashlib
import math
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from adapter import Encoder, check_lengths, check_vectors, deny_external_network
from common import HERE, datasets, read_json, require_ci, summarize, verify_file


class FakeTokenizer:
    def encode(self, text, *, add_special_tokens):
        assert add_special_tokens
        return type("Encoding", (), {"ids": [0] * (len(text) + 2)})()


class Contracts(unittest.TestCase):
    def test_original_fixture_is_unchanged_12_sources_52_queries_nine_synonyms(self):
        primary, extra = datasets()
        self.assertEqual((len(primary["corpus"]), len(primary["queries"])), (12, 52))
        self.assertEqual(sum(q["category"] == "synonym_question" for q in primary["queries"]), 9)
        self.assertNotEqual(primary["name"], extra["name"])
        self.assertEqual(len(extra["queries"]), 3)

    def test_git_blob_digest_hashes_header_and_original_bytes(self):
        data = b"synthetic\n"
        expected = {"bytes": len(data), "git_blob_sha1": hashlib.sha1(b"blob 10\0" + data).hexdigest()}
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "config.json"
            path.write_bytes(data)
            self.assertEqual(verify_file(path, expected)["sha256"], hashlib.sha256(data).hexdigest())
            path.write_bytes(b"synthetic!")
            with self.assertRaises(ValueError):
                verify_file(path, expected)

    def test_missing_hash_length_symlink_and_wrong_sha256_fail_closed(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "model"
            path.write_bytes(b"x")
            for expected in ({"bytes": 1}, {"bytes": 2, "sha256": "0" * 64}, {"bytes": 1, "sha256": "0" * 64}):
                with self.assertRaises(ValueError):
                    verify_file(path, expected)
            link = Path(folder) / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                verify_file(link, {"bytes": 1, "sha256": hashlib.sha256(b"x").hexdigest()})

    def test_pinned_manifest_is_under_download_cap(self):
        lock = read_json(HERE / "model-lock.json")
        self.assertEqual(lock["dimensions"], 512)
        self.assertEqual(lock["max_tokens"], 512)
        self.assertLess(sum(v["bytes"] for v in lock["files"].values()), 100 * 1024**2)
        self.assertEqual(lock["files"]["model_optimized.onnx"]["sha256"], "1294ea4b6331115a353d81f96b85e8c8d7fdcc284453d5b2fab5b016230aad38")
        for name, entry in lock["files"].items():
            self.assertEqual(Path(name).name, name)
            self.assertTrue("sha256" in entry or "git_blob_sha1" in entry)

    def test_token_limit_counts_special_tokens_without_truncation(self):
        tokenizer = FakeTokenizer()
        self.assertEqual(check_lengths(tokenizer, ["x" * 510]), [512])
        with self.assertRaises(ValueError):
            check_lengths(tokenizer, ["x" * 511])

    def test_vector_dimension_count_nan_and_normalization(self):
        self.assertEqual(check_vectors([[1, 0]], 1, 2), [[1.0, 0.0]])
        for vectors, count in (([[1]], 1), ([[math.nan, 0]], 1), ([[0, 0]], 1), ([[2, 0]], 1), ([[1, 0]], 2)):
            with self.assertRaises(ValueError):
                check_vectors(vectors, count, 2)

    def test_network_guard_blocks_non_loopback_and_dns(self):
        deny_external_network("socket.connect", (None, ("127.0.0.1", 1)))
        deny_external_network("socket.connect", (None, ("::1", 1, 0, 0)))
        for event, args in (("socket.connect", (None, ("192.0.2.1", 443))),
                            ("socket.connect", (None, ("huggingface.co", 443))),
                            ("socket.getaddrinfo", ("huggingface.co", 443, 0, 0, 0))):
            with self.assertRaises(RuntimeError):
                deny_external_network(event, args)

    def test_runner_gate_rejects_local_and_self_hosted(self):
        for env in ({}, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"}):
            with patch.dict(os.environ, env, clear=True), self.assertRaises(RuntimeError):
                require_ci()

    def test_allowlist_rejects_unknown_input_and_double_prefix_before_model_use(self):
        encoder = Encoder.__new__(Encoder)
        encoder.lock = read_json(HERE / "model-lock.json")
        prefix = encoder.lock["query_instruction"]
        encoder.allowed_inputs = frozenset([prefix + "\n合成问题"])
        for text in ("陌生正文", prefix + "\n" + prefix + "\n合成问题"):
            with self.assertRaisesRegex(ValueError, "synthetic_allowlist_required"):
                encoder.encode({"model": encoder.lock["model"], "input": [text], "dimensions": 512, "encoding_format": "float"})

    def test_successful_adapter_preserves_order_and_does_not_reprefix(self):
        encoder = Encoder.__new__(Encoder)
        encoder.lock = read_json(HERE / "model-lock.json")
        supplied = ["合成文档", encoder.lock["query_instruction"] + "\n合成问题"]
        encoder.allowed_inputs = frozenset(supplied)
        encoder.tokenizer = FakeTokenizer()
        encoder.requests = encoder.inputs = encoder.input_bytes = encoder.max_tokens_seen = 0
        encoder.encoding_seconds = []
        encoder.first_embedding_seconds = None
        seen = []

        class FakeModel:
            def embed(self, texts, *, batch_size, parallel):
                seen.extend(texts)
                self.asserted = (batch_size, parallel)
                return [[1.0] + [0.0] * 511, [0.0, 1.0] + [0.0] * 510]

        encoder.model = FakeModel()
        payload = {"model": encoder.lock["model"], "input": supplied, "dimensions": 512, "encoding_format": "float"}
        result = encoder.encode(payload)
        self.assertEqual(seen, supplied)
        self.assertEqual([row["index"] for row in result["data"]], [0, 1])
        self.assertEqual((encoder.requests, encoder.inputs), (1, 2))
        self.assertEqual(encoder.model.asserted, (8, None))
        encoder.requests = 128
        with self.assertRaisesRegex(ValueError, "request_budget_exceeded"):
            encoder.encode(payload)

    def test_source_hit_and_false_candidates_are_separate(self):
        rows = [{"category": "synonym", "expected_sources": ["right"], "returned_sources": ["wrong", "right"]},
                {"category": "unrelated", "expected_sources": [], "returned_sources": ["wrong"]},
                {"category": "correction", "expected_sources": ["new"], "returned_sources": ["old", "new"],
                 "obsolete_or_conflicting_sources": ["old"]}]
        result = summarize(rows)["all"]
        self.assertEqual(result["positive_source_hit_at_1"], 0)
        self.assertEqual(result["positive_source_hit_at_3"], 2)
        self.assertEqual(result["negative_queries_with_any_candidate"], 1)
        self.assertEqual(result["non_expected_candidates_at_3"], 3)
        self.assertEqual(result["obsolete_or_conflicting_at_1"], 1)

    def test_existing_prefix_contract_includes_one_newline(self):
        # 读实际生产纯函数，不启动 worker 或本地 HTTP。
        import sys
        sys.path.insert(0, str(HERE.parents[1] / "python"))
        from recallcard_worker.embedding import EmbeddingSpace
        lock = read_json(HERE / "model-lock.json")
        space = EmbeddingSpace(provider="local-openai-compatible", endpoint="http://127.0.0.1:1/v1/embeddings",
            model=lock["model"], revision="synthetic", dimensions=512,
            document_instruction="", query_instruction=lock["query_instruction"], normalization="l2",
            preprocessing_version="utf8-v1")
        self.assertEqual(space.prepare("合成问题", "query"), lock["query_instruction"] + "\n合成问题")
        self.assertEqual(space.prepare("合成文档", "corpus"), "合成文档")


if __name__ == "__main__":
    unittest.main()
