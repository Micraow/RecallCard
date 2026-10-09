"""全部使用虚构文本与内存传输，不访问任何模型或外部网络。"""
import copy
from dataclasses import asdict, replace
import hashlib
import io
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_worker.embedding import (
    CORPUS_SCHEMA, MAX_BATCH_BYTES, MAX_TEXT_BYTES, EmbeddingClient, EmbeddingError,
    EmbeddingSpace, NetworkApproval, TransportResponse, canonical, index_corpus, load_json,
    reciprocal_rank_fusion, search_index, text_hash, validate_index, vector,
)
from recallcard_worker.__main__ import Worker, atomic_json_file, read_json_file, serve

SPACE = EmbeddingSpace(
    provider="fixture-provider", endpoint="https://embedding.invalid/v1/embeddings",
    model="fixture-model", dimensions=3, revision="fixture-revision-1",
    document_instruction="", query_instruction="", normalization="l2", preprocessing_version="utf8-v1",
)


def document(ref="memory:fixture_a@1", text="合成材料甲", scope="project:fixture"):
    return {"ref": ref, "text": text, "content_hash": text_hash(text), "scope": scope}


def corpus(*documents, generation="fixture-generation-1", allowed=None):
    return {"schema": CORPUS_SCHEMA, "generation": generation,
            "scope": allowed or sorted({d["scope"] for d in documents}), "documents": list(documents)}


class FakeTransport:
    def __init__(self, responses=None):
        self.calls = []
        self.responses = list(responses or [])

    def __call__(self, endpoint, body, key, timeout, response_limit):
        payload = json.loads(body)
        self.calls.append(payload)
        if self.responses:
            response = self.responses.pop(0)
            if isinstance(response, Exception):
                raise response
            if isinstance(response, TransportResponse):
                return response
            return TransportResponse(200, canonical(response), {})
        values = [{"index": i, "embedding": [1.0 + (hashlib.sha256(t.encode()).digest()[j] / 255.0)
                                                for j in range(payload["dimensions"])]}
                  for i, t in enumerate(payload["input"])]
        return TransportResponse(200, canonical({"model": payload["model"], "data": list(reversed(values)),
                                                "usage": {"prompt_tokens": 7 * len(values), "total_tokens": 7 * len(values)}}), {})


class EmbeddingTests(unittest.TestCase):
    def setUp(self):
        self.environment = patch.dict(os.environ, {"RECALLCARD_EMBEDDING_API_KEY": "synthetic-key-never-sent"})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def client(self, fake=None, **kwargs):
        return EmbeddingClient(NetworkApproval(True, SPACE.endpoint, ("project:fixture", "personal"), ("corpus", "query")),
                               transport=fake or FakeTransport(), sleep=lambda _: None, **kwargs)

    def build(self, source=None, **kwargs):
        return index_corpus(SPACE, source or corpus(document()), self.client(), **kwargs)["index"]

    def assertError(self, code, function, *args, **kwargs):
        with self.assertRaises(EmbeddingError) as caught:
            function(*args, **kwargs)
        self.assertEqual(code, caught.exception.code)
        return caught.exception

    def test_signature_covers_every_identity_field(self):
        changes = {"provider": "another", "endpoint": "https://another.invalid/v1/embeddings", "model": "another",
                   "dimensions": 2, "revision": "new", "document_instruction": "文档前缀", "query_instruction": "查询前缀",
                   "normalization": "none", "preprocessing_version": "future-version"}
        self.assertEqual(SPACE.signature, EmbeddingSpace.from_dict(dict(reversed(list(asdict(SPACE).items())))).signature)
        for name, value in changes.items():
            with self.subTest(field=name):
                self.assertNotEqual(SPACE.signature, replace(SPACE, **{name: value}).signature)

    def test_configuration_rejects_invalid_dimensions(self):
        for size in [0, -1, 8193, True, 1.5, "3", None]:
            with self.subTest(size=size):
                self.assertError("invalid_input", EmbeddingSpace.from_dict, {**asdict(SPACE), "dimensions": size})

    def test_configuration_rejects_unknown_fields_and_preprocessing(self):
        self.assertError("invalid_input", EmbeddingSpace.from_dict, {**asdict(SPACE), "api_key": "not-allowed"})
        self.assertError("invalid_space", EmbeddingSpace.from_dict, {**asdict(SPACE), "preprocessing_version": "v2"})
        self.assertError("invalid_space", EmbeddingSpace.from_dict, {**asdict(SPACE), "normalization": "magic"})

    def test_endpoint_requires_exact_https_without_credentials(self):
        for endpoint in ["http://embedding.invalid/embeddings", "https://key@embedding.invalid/v1/embeddings",
                         "https://embedding.invalid/v1/embeddings?secret=x", "https://embedding.invalid/v1/embeddings#x",
                         "https://embedding.invalid", "https://embedding.invalid:99999/x", "https://embedding.invalid/x\n"]:
            with self.subTest(endpoint=endpoint):
                self.assertError("invalid_endpoint", EmbeddingSpace.from_dict, {**asdict(SPACE), "endpoint": endpoint})

    def test_loopback_space_is_offline_only_and_cannot_send_credentials(self):
        for endpoint in ("http://127.0.0.1:46709/v1/embeddings", "http://[::1]:46709/v1/embeddings"):
            space = EmbeddingSpace.from_dict({**asdict(SPACE), "endpoint": endpoint})
            transport = FakeTransport()
            client = EmbeddingClient(approval=NetworkApproval(True, endpoint, ("project:fixture",), ("corpus",)), transport=transport)
            self.assertError("invalid_endpoint", client.embed, space, ["合成资料"], ["project:fixture"], "corpus")
            self.assertEqual(transport.calls, [])
        for endpoint in ("http://localhost:46709/v1/embeddings", "http://127.0.0.1.evil.invalid/x", "http://key@127.0.0.1/x", "http://127.0.0.1/x?q=x"):
            self.assertError("invalid_endpoint", EmbeddingSpace.from_dict, {**asdict(SPACE), "endpoint": endpoint})

    def test_no_implicit_preprocessing_or_empty_inputs(self):
        self.assertEqual("  材料\r\n", SPACE.prepare("  材料\r\n", "corpus"))
        self.assertEqual("文档\n内容", replace(SPACE, document_instruction="文档").prepare("内容", "corpus"))
        for text in ["", " \n", "x" * (MAX_TEXT_BYTES + 1), "\ud800", "x\x00"]:
            self.assertError("invalid_input", SPACE.prepare, text, "corpus")
        self.assertError("invalid_input", replace(SPACE, document_instruction="x").prepare, "x" * MAX_TEXT_BYTES, "corpus")

    def test_vectors_reject_dimension_type_nonfinite_and_zero(self):
        for values in [[], [1, 2], [0, 0, 0], [1, True, 2], [1, "2", 3], [1, None, 3],
                       [1, math.nan, 3], [1, math.inf, 3], [1, -math.inf, 3], [10**1000, 1, 1]]:
            with self.subTest(values=str(values)[:50]):
                self.assertError("invalid_vector", vector, values, 3)
        self.assertEqual([0.6, 0.8, 0.0], vector([3, 4, 0], 3, "l2"))
        self.assertEqual([3.0, 4.0, 0.0], vector([3, 4, 0], 3))

    def test_build_and_exact_reuse_are_offline(self):
        fake = FakeTransport()
        source = corpus(document())
        original = index_corpus(SPACE, source, self.client(fake))
        self.assertEqual(1, len(fake.calls))
        result = index_corpus(SPACE, source, EmbeddingClient(), original["index"])
        self.assertEqual(original["index"], result["index"])
        self.assertEqual(1, result["diagnostics"]["reused_documents"])
        self.assertEqual(0, result["diagnostics"]["network_calls"])

    def test_empty_corpus_needs_no_key_or_network(self):
        with patch.dict(os.environ, {}, clear=True):
            result = index_corpus(SPACE, corpus(), EmbeddingClient())
        self.assertEqual([], result["index"]["documents"])
        self.assertTrue(result["index"]["complete"])

    def test_duplicate_input_is_only_embedded_once(self):
        fake = FakeTransport()
        result = index_corpus(SPACE, corpus(document(), document("memory:fixture_b@1")), self.client(fake))
        self.assertEqual(1, len(fake.calls[0]["input"]))
        self.assertEqual(1, result["diagnostics"]["deduplicated_documents"])
        self.assertEqual(result["index"]["documents"][0]["vector"], result["index"]["documents"][1]["vector"])

    def test_renamed_ref_reuses_actual_input_not_display_metadata(self):
        previous = self.build()
        changed = corpus(document("memory:fixture_renamed@2"), generation="new")
        result = index_corpus(SPACE, changed, EmbeddingClient(), previous)
        self.assertEqual(1, result["diagnostics"]["reused_documents"])
        self.assertEqual("memory:fixture_renamed@2", result["index"]["documents"][0]["ref"])
        self.assertEqual("new", result["index"]["generation"])

    def test_changed_text_recomputes_and_deleted_ref_disappears(self):
        previous = self.build(corpus(document(), document("memory:fixture_b@1", "另外一段")))
        fake = FakeTransport()
        changed = corpus(document(text="修改后的合成材料"))
        result = index_corpus(SPACE, changed, self.client(fake), previous)
        self.assertEqual(1, len(result["index"]["documents"]))
        self.assertEqual(["修改后的合成材料"], fake.calls[0]["input"])
        self.assertEqual(0, result["diagnostics"]["reused_documents"])

    def test_signature_change_never_reuses_even_equal_dimension(self):
        previous = self.build()
        for space in [replace(SPACE, model="other-model"), replace(SPACE, query_instruction="查找相关材料")]:
            fake = FakeTransport()
            result = index_corpus(space, corpus(document()), self.client(fake), previous)
            self.assertEqual(0, result["diagnostics"]["reused_documents"])
            self.assertEqual(1, len(fake.calls))

    def test_hash_mismatch_rejected_before_any_transmission(self):
        fake = FakeTransport()
        doc = document()
        doc["content_hash"] = "0" * 64
        self.assertError("hash_mismatch", index_corpus, SPACE, corpus(doc), self.client(fake))
        self.assertEqual([], fake.calls)
        doc["content_hash"] = "not-a-hash"
        self.assertError("invalid_hash", index_corpus, SPACE, corpus(doc), self.client(fake))

    def test_duplicate_ref_and_invalid_ref_rejected(self):
        self.assertError("duplicate_ref", index_corpus, SPACE, corpus(document(), document()), self.client())
        self.assertError("invalid_ref", index_corpus, SPACE, corpus(document("memory:../../escape")), self.client())

    def test_mixed_scope_validated_before_any_transmission(self):
        fake = FakeTransport()
        source = corpus(document(), document("memory:private@1", "完全虚构的私密材料", "private"))
        self.assertError("scope_denied", index_corpus, SPACE, source, self.client(fake), batch_size=1)
        self.assertEqual([], fake.calls)
        source["scope"] = ["project:fixture"]
        self.assertError("scope_denied", index_corpus, SPACE, source, self.client(fake))

    def test_network_requires_destination_scope_and_data_approval(self):
        for approval in [NetworkApproval(), NetworkApproval(True, "https://wrong.invalid/v1/embeddings", ("project:fixture",), ("corpus",)),
                         NetworkApproval(True, SPACE.endpoint, ("project:fixture",), ("query",)),
                         NetworkApproval(True, SPACE.endpoint, (), ("corpus",))]:
            fake = FakeTransport()
            client = EmbeddingClient(approval, transport=fake)
            with self.assertRaises(EmbeddingError):
                index_corpus(SPACE, corpus(document()), client)
            self.assertEqual([], fake.calls)

    def test_api_key_only_read_from_environment(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertError("missing_key", index_corpus, SPACE, corpus(document()), self.client())
        with patch.dict(os.environ, {"RECALLCARD_EMBEDDING_API_KEY": "invalid\nkey"}):
            self.assertError("missing_key", index_corpus, SPACE, corpus(document()), self.client())

    def test_inputs_bound_by_batch_count_and_byte_size(self):
        fake = FakeTransport()
        docs = [document(f"memory:fixture_{i}@1", str(i) + "x" * 8000) for i in range(33)]
        result = index_corpus(SPACE, corpus(*docs), self.client(fake))
        self.assertEqual(33, len(result["index"]["documents"]))
        self.assertGreater(len(fake.calls), 1)
        self.assertTrue(all(sum(len(t.encode()) for t in call["input"]) <= MAX_BATCH_BYTES for call in fake.calls))
        self.assertError("input_limit", self.client().embed, SPACE, ["a"] * 65, ["project:fixture"], "corpus")

    def test_response_order_is_indexed_not_assumed(self):
        response = {"model": SPACE.model, "data": [{"index": 1, "embedding": [0, 1, 0]}, {"index": 0, "embedding": [1, 0, 0]}]}
        values = self.client(FakeTransport([response])).embed(SPACE, ["a", "b"], ["project:fixture"], "corpus")
        self.assertEqual([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], values)

    def test_bad_response_model_count_indices_and_vectors(self):
        cases = [("model_mismatch", {"model": "other", "data": []}),
                 ("invalid_response", {"model": SPACE.model, "data": []}),
                 ("invalid_response", {"model": SPACE.model, "data": [{"index": True, "embedding": [1, 0, 0]}]}),
                 ("invalid_response", {"model": SPACE.model, "data": [{"index": 1, "embedding": [1, 0, 0]}]}),
                 ("invalid_vector", {"model": SPACE.model, "data": [{"index": 0, "embedding": [1, 0]}]})]
        for code, response in cases:
            with self.subTest(code=code):
                self.assertError(code, self.client(FakeTransport([response])).embed, SPACE, ["a"], ["project:fixture"], "corpus")

    def test_duplicate_response_index_rejected(self):
        response = {"model": SPACE.model, "data": [{"index": 0, "embedding": [1, 0, 0]}] * 2}
        self.assertError("invalid_response", self.client(FakeTransport([response])).embed, SPACE, ["a", "b"], ["project:fixture"], "corpus")

    def test_rate_limits_have_bounded_retries_and_retry_after(self):
        fake = FakeTransport([TransportResponse(429, b"SECRET_RAW_INPUT", {"Retry-After": "100000"})] * 3)
        sleeps = []
        client = self.client(fake)
        client.sleep = sleeps.append
        error = self.assertError("rate_limited", client.embed, SPACE, ["a"], ["project:fixture"], "corpus")
        self.assertEqual(3, len(fake.calls))
        self.assertEqual([5.0, 5.0], sleeps)
        self.assertEqual(2, client.diagnostics()["retries"])
        self.assertNotIn("SECRET", str(error))

    def test_transient_and_connection_failures_retry(self):
        for error in [TransportResponse(503, b"", {}), URLError("PRIVATE_INPUT")]:
            fake = FakeTransport([error])
            client = self.client(fake)
            self.assertEqual(1, len(client.embed(SPACE, ["a"], ["project:fixture"], "corpus")))
            self.assertEqual(2, len(fake.calls))
        fake = FakeTransport([URLError("PRIVATE_INPUT")] * 3)
        error = self.assertError("network_error", self.client(fake).embed, SPACE, ["a"], ["project:fixture"], "corpus")
        self.assertNotIn("PRIVATE", str(error))

    def test_auth_and_redirect_and_bad_request_do_not_retry(self):
        for status, code in [(401, "provider_auth"), (403, "provider_auth"), (302, "redirect_blocked"), (400, "provider_error")]:
            fake = FakeTransport([TransportResponse(status, b"raw-key-secret", {})])
            self.assertError(code, self.client(fake).embed, SPACE, ["a"], ["project:fixture"], "corpus")
            self.assertEqual(1, len(fake.calls))

    def test_budget_applies_to_retries_and_worker_lifetime(self):
        fake = FakeTransport([TransportResponse(503, b"", {})])
        client = self.client(fake, max_network_calls=1)
        self.assertError("network_budget_exhausted", client.embed, SPACE, ["a"], ["project:fixture"], "corpus")
        self.assertEqual(1, len(fake.calls))
        fake = FakeTransport()
        client = self.client(fake, max_transmitted_bytes=5)
        client.embed(SPACE, ["abc"], ["project:fixture"], "corpus")
        self.assertError("network_budget_exhausted", client.embed, SPACE, ["abc"], ["project:fixture"], "corpus")
        self.assertEqual(1, len(fake.calls))

    def test_partial_checkpoint_reuses_successful_batches(self):
        fake = FakeTransport()
        original_call = fake.__call__
        count = 0
        def interrupted(*args):
            nonlocal count
            count += 1
            return original_call(*args) if count == 1 else TransportResponse(400, b"", {})
        checkpoints = []
        source = corpus(document(), document("memory:fixture_b@1", "材料乙"))
        error = self.assertError("provider_error", index_corpus, SPACE, source, self.client(interrupted), batch_size=1,
                                 checkpoint=checkpoints.append)
        self.assertEqual(1, len(checkpoints))
        self.assertFalse(error.partial_index["complete"])
        self.assertEqual(1, len(error.partial_index["documents"]))
        resumed = index_corpus(SPACE, source, self.client(), error.partial_index)
        self.assertEqual(1, resumed["diagnostics"]["reused_documents"])
        self.assertTrue(resumed["index"]["complete"])
        self.assertError("index_incomplete", search_index, error.partial_index, [1, 0, 0], space_signature=SPACE.signature,
                         allowed_scopes=["project:fixture"], allowed_refs=[document()["ref"]])

    def test_diagnostics_no_raw_text_key_or_unverified_cache_saving(self):
        result = index_corpus(SPACE, corpus(document()), self.client(price_per_million_tokens=0.2, pricing_date="2026-10-06", pricing_currency="USD"))
        rendered = json.dumps(result, ensure_ascii=False)
        self.assertNotIn(document()["text"], rendered)
        self.assertNotIn("synthetic-key", rendered)
        self.assertEqual(7, result["diagnostics"]["input_tokens"])
        self.assertAlmostEqual(0.0000014, result["diagnostics"]["estimated_cost"])
        self.assertIsNone(result["diagnostics"]["cache_read_tokens"])

    def test_missing_usage_is_unknown_not_zero_bill(self):
        response = {"model": SPACE.model, "data": [{"index": 0, "embedding": [1, 0, 0]}]}
        client = self.client(FakeTransport([response]), price_per_million_tokens=1, pricing_date="2026-10-06", pricing_currency="USD")
        client.embed(SPACE, ["a"], ["project:fixture"], "corpus")
        self.assertIsNone(client.diagnostics()["estimated_cost"])
        self.assertIsNone(client.diagnostics()["input_tokens"])

    def test_index_integrity_signature_and_normalization(self):
        index = self.build()
        broken = copy.deepcopy(index)
        broken["space"]["model"] = "another"
        self.assertError("signature_mismatch", validate_index, broken)
        broken = copy.deepcopy(index)
        broken["documents"][0]["vector"] = [10, 0, 0]
        self.assertError("invalid_vector", validate_index, broken)
        broken = copy.deepcopy(index)
        broken["documents"][0]["scope"] = "private"
        self.assertError("scope_denied", validate_index, broken)

    def test_conflicting_cache_vector_for_same_input_rejected(self):
        index = self.build(corpus(document(), document("memory:fixture_b@1")))
        index["documents"][1]["vector"] = [1, 0, 0]
        self.assertError("cache_conflict", validate_index, index)

    def test_cosine_has_no_scope_or_revoked_ref_leakage(self):
        index = self.build(corpus(document(), document("memory:personal@1", "另一条合成内容", "personal")))
        result = search_index(index, [1, 0, 0], space_signature=SPACE.signature,
                              allowed_scopes=["project:fixture"], allowed_refs=[d["ref"] for d in index["documents"]])
        self.assertEqual([document()["ref"]], [r["ref"] for r in result["results"]])
        empty = search_index(index, [1, 0, 0], space_signature=SPACE.signature,
                             allowed_scopes=["project:fixture", "personal"], allowed_refs=[])
        self.assertEqual([], empty["results"])

    def test_query_encoder_signature_mismatch(self):
        self.assertError("signature_mismatch", search_index, self.build(), [1, 0, 0], space_signature="0" * 64,
                         allowed_scopes=["project:fixture"], allowed_refs=[document()["ref"]])

    def test_rrf_filters_before_ranking_and_deduplicates(self):
        a, b, secret = "memory:a@1", "memory:b@1", "memory:secret@1"
        result = reciprocal_rank_fusion([[secret, a, a, b], [b]], {a, b})
        self.assertEqual(b, result[0]["ref"])
        self.assertAlmostEqual(1 / 61, result[1]["score"])
        self.assertAlmostEqual(1 / 62 + 1 / 61, result[0]["score"])
        self.assertNotIn(secret, str(result))

    def test_query_cache_reuses_encoding_but_rechecks_current_permissions(self):
        index = self.build(corpus(document(), document("memory:fixture_b@1", "材料乙")))
        fake = FakeTransport()
        worker = Worker(self.client(fake))
        request = {"id": "q1", "op": "query", "index": index, "query": "虚构查询",
                   "allowed_scopes": ["project:fixture"], "allowed_refs": [d["ref"] for d in index["documents"]]}
        self.assertTrue(worker.handle(request)["ok"])
        request["allowed_refs"] = [document()["ref"]]
        result = worker.handle(request)
        self.assertTrue(result["result"]["query_cache_hit"])
        self.assertEqual([document()["ref"]], [r["ref"] for r in result["result"]["results"]])
        self.assertEqual(1, len(fake.calls))

    def test_query_cache_is_bounded(self):
        client = self.client()
        for number in range(33):
            client.query_vector(SPACE, f"合成查询 {number}", ["project:fixture"])
        self.assertEqual(32, len(client.query_cache))
        self.assertNotIn((SPACE.signature, text_hash("合成查询 0")), client.query_cache)

    def test_bad_query_filters_and_stale_generation_never_transmit(self):
        index = self.build()
        fake = FakeTransport()
        worker = Worker(self.client(fake))
        request = {"id": 1, "op": "query", "index": index, "query": "材料",
                   "allowed_scopes": ["project:fixture"], "allowed_refs": [document()["ref"]]}
        for change in [{"limit": -1}, {"expected_generation": "new-generation"}, {"lexical_refs": ["../secret"]}]:
            result = worker.handle({**request, **change})
            self.assertFalse(result["ok"])
        self.assertEqual([], fake.calls)

    def test_query_vector_and_empty_access_need_no_network(self):
        index = self.build()
        worker = Worker()
        request = {"id": 1, "op": "query_vector", "index": index, "query_vector": [1, 0, 0],
                   "space_signature": SPACE.signature, "allowed_scopes": ["project:fixture"], "allowed_refs": [document()["ref"]]}
        self.assertTrue(worker.handle(request)["ok"])
        request.pop("query_vector")
        request.pop("space_signature")
        request.update(op="query", query="虚构查询", allowed_refs=[])
        self.assertEqual([], worker.handle(request)["result"]["results"])

    def test_untrusted_request_cannot_grant_network_or_select_files(self):
        worker = Worker()
        request = {"id": 1, "op": "index", "space": asdict(SPACE), "corpus": corpus(document())}
        for addition in [{"allow_network": True}, {"api_key": "fixture"}, {"corpus_path": "/private/path"}]:
            self.assertEqual("invalid_input", worker.handle({**request, **addition})["error"]["code"])
        self.assertEqual("approval_required", worker.handle(request)["error"]["code"])

    def test_strict_json_rejects_duplicates_nonfinite_invalid_utf8(self):
        for raw in [b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":Infinity}', b'\xff', b'{broken']:
            self.assertError("invalid_json", load_json, raw)

    def test_stdio_recovers_after_invalid_or_oversized_lines(self):
        valid = canonical({"id": 3, "op": "space_signature", "space": asdict(SPACE)}) + b"\n"
        source = io.BytesIO(b'{invalid}\n' + b'x' * 1025 + b'\n' + valid)
        destination = io.BytesIO()
        with patch("recallcard_worker.__main__.MAX_JSON_BYTES", 1024):
            serve(Worker(), source, destination)
        results = [json.loads(line) for line in destination.getvalue().splitlines()]
        self.assertEqual([False, False, True], [r["ok"] for r in results])
        self.assertEqual(3, results[-1]["id"])

    def test_unexpected_transport_error_is_sanitized(self):
        def broken(*args):
            raise RuntimeError("SECRET_KEY_AND_RAW_TEXT")
        worker = Worker(self.client(broken))
        result = worker.handle({"id": 1, "op": "index", "space": asdict(SPACE), "corpus": corpus(document())})
        self.assertEqual("worker_error", result["error"]["code"])
        self.assertNotIn("SECRET", str(result))

    def test_atomic_cache_write_is_private_and_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "cache.json"
            atomic_json_file(str(path), self.build())
            self.assertEqual(SPACE.signature, read_json_file(str(path))["space_signature"])
            if os.name == "posix":
                self.assertEqual(0o600, path.stat().st_mode & 0o777)
                link = Path(directory) / "link.json"
                link.symlink_to(path)
                self.assertError("unsafe_path", atomic_json_file, str(link), {})

    def test_subprocess_stdio_is_optional_offline_and_clean(self):
        environment = {**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1])}
        request = {"id": 1, "op": "index", "space": asdict(SPACE), "corpus": corpus(document())}
        process = subprocess.run([sys.executable, "-m", "recallcard_worker", "--stdio"], input=canonical(request) + b"\n",
                                 capture_output=True, env=environment, timeout=5)
        self.assertEqual(0, process.returncode)
        self.assertEqual(b"", process.stderr)
        response = json.loads(process.stdout)
        self.assertFalse(response["ok"])
        self.assertEqual("approval_required", response["error"]["code"])

    def test_file_cli_reuses_cache_without_network_or_key(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = corpus(document())
            (root / "space.json").write_bytes(canonical(asdict(SPACE)))
            (root / "corpus.json").write_bytes(canonical(source))
            atomic_json_file(str(root / "cache.json"), self.build(source))
            environment = {**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1])}
            environment.pop("RECALLCARD_EMBEDDING_API_KEY", None)
            command = [sys.executable, "-m", "recallcard_worker", "index", "--space", str(root / "space.json"),
                       "--corpus", str(root / "corpus.json"), "--cache", str(root / "cache.json")]
            process = subprocess.run(command, capture_output=True, env=environment, timeout=5)
            self.assertEqual(0, process.returncode, process.stderr)
            self.assertEqual(1, json.loads(process.stdout)["diagnostics"]["reused_documents"])
            self.assertFalse((root / "cache.json.pending").exists())
            before = (root / "cache.json").read_bytes()
            (root / "corpus.json").write_bytes(canonical(corpus(document(text="修改的合成材料"))))
            process = subprocess.run(command, capture_output=True, env=environment, timeout=5)
            self.assertEqual(2, process.returncode)
            self.assertEqual("approval_required", json.loads(process.stderr)["error"]["code"])
            self.assertEqual(before, (root / "cache.json").read_bytes())

    def test_stdio_index_failure_returns_resumable_checkpoint(self):
        result = Worker().handle({"id": 2, "op": "index", "space": asdict(SPACE), "corpus": corpus(document())})
        self.assertFalse(result["ok"])
        self.assertFalse(result["partial_index"]["complete"])
        self.assertEqual([], result["partial_index"]["documents"])
        self.assertNotIn(document()["text"], json.dumps(result, ensure_ascii=False))

    def test_https_transport_uses_tls_no_proxy_and_no_redirect(self):
        from recallcard_worker.embedding import https_transport, _NoRedirect
        from urllib.request import HTTPSHandler, ProxyHandler, Request
        from unittest.mock import MagicMock
        response = MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = b"{}"
        response.status = 200
        response.headers = {}
        opener = MagicMock()
        opener.open.return_value = response
        with patch("recallcard_worker.embedding.build_opener", return_value=opener) as builder:
            result = https_transport(SPACE.endpoint, b"{}", "synthetic-key", 2, 64)
        handlers = builder.call_args.args
        self.assertTrue(any(isinstance(h, HTTPSHandler) for h in handlers))
        self.assertEqual({}, next(h for h in handlers if isinstance(h, ProxyHandler)).proxies)
        redirect = next(h for h in handlers if isinstance(h, _NoRedirect))
        self.assertIsNone(redirect.redirect_request(Request(SPACE.endpoint), None, 302, "", {}, "https://other.invalid"))
        self.assertEqual(200, result.status)
        self.assertEqual(65, response.read.call_args.args[0])
        self.assertEqual(SPACE.endpoint, opener.open.call_args.args[0].full_url)

    def test_response_limits_and_nan_rejected(self):
        fake = FakeTransport([TransportResponse(200, b"x" * 70000, {})])
        self.assertError("response_limit", self.client(fake).embed, SPACE, ["a"], ["project:fixture"], "corpus")
        raw = b'{"model":"fixture-model","data":[{"index":0,"embedding":[NaN,1,2]}]}'
        fake = FakeTransport([TransportResponse(200, raw, {})])
        self.assertError("invalid_json", self.client(fake).embed, SPACE, ["a"], ["project:fixture"], "corpus")

    def test_configuration_limits(self):
        for arguments in [{"max_retries": 6}, {"max_network_calls": 0}, {"max_transmitted_bytes": 0},
                          {"timeout": math.nan}, {"timeout": 121}, {"price_per_million_tokens": -1},
                          {"price_per_million_tokens": 1}, {"key_env": "bad-name"}]:
            with self.subTest(arguments=arguments):
                with self.assertRaises(EmbeddingError):
                    EmbeddingClient(**arguments)


if __name__ == "__main__":
    unittest.main()
