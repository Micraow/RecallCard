"""生产 EventIndex + 小型真实 SQLite；只使用合成文本和数值encoder。

不监听HTTP，不下载模型；rank101用数值控制，仅证明独立候选路线，
不证明真实模型命中率、答案正确性或个人语料质量。
"""
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import tracemalloc
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_worker.event_index import EventIndex, SCHEMA, canonical, digest
from recallcard_worker import event_lexical as lexical
from recallcard_worker.embedding import EmbeddingError
from test_event_index import settings, Encoder


class EventLexicalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = settings(self.root / "index.sqlite")
        self.encoder = Encoder()
        self.worker = EventIndex(self.config, encoder=self.encoder)
        self.ident = 0

    def tearDown(self):
        self.temp.cleanup()

    def call(self, op, **values):
        self.ident += 1
        return self.worker.execute({"schema": SCHEMA, "id": self.ident, "op": op, **values})

    def ok(self, op, **values):
        result = self.call(op, **values)
        self.assertTrue(result["ok"], result)
        return result["result"]

    def error(self, code, op, **values):
        result = self.call(op, **values)
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["error"]["code"], code, result)
        return result

    def begin(self, name="first", version=lexical.VERSION):
        kwargs = {"lexical_version": version} if version else {}
        return self.ok("begin", source_epoch=digest(name.encode()), **kwargs)

    def chunks(self, body, cuts=None, *, contexts=True):
        raw = body.encode()
        cuts = [0, len(raw)] if cuts is None else cuts
        output = []
        for i, (start, end) in enumerate(zip(cuts, cuts[1:])):
            item = {"ordinal": i, "start_byte": start, "end_byte": end,
                    "text_sha256": digest(raw[start:end]), "text": raw[start:end].decode()}
            if contexts:
                item.update(lexical_before=raw[max(0, start - 256):start].decode(errors="ignore"),
                            lexical_after=raw[end:end + 256].decode(errors="ignore"))
            output.append(item)
        return output

    def source(self, name, body):
        return {"ref": "event:" + str(name), "scope": "synthetic", "content_hash": digest(body.encode()),
                "role": "user", "session_ref": None, "occurred_at": None, "text_bytes": len(body.encode())}

    def put(self, build, name, body, cuts=None, *, contexts=True, batch=None):
        chunks = self.chunks(body, cuts, contexts=contexts)
        return self.ok("upsert", build_id=build["build_id"], batch_id=batch or str(name),
                       source=self.source(name, body), source_chunk_count=len(chunks), chunks=chunks)

    def finish(self, result):
        return self.ok("finish", **{key: result[key] for key in ("build_id", "source_epoch", "receipt", "source_count", "chunk_count")})

    def test_unavailable_encoder_keeps_independent_lexical_without_fake_cosine(self):
        body = "prefix ZXQ-714 suffix"
        self.finish(self.put(self.begin(), "target", body, [0, 11, len(body)]))
        for code in ("local_provider_unavailable", "local_provider_error"):
            with self.subTest(code=code), patch.object(self.worker, "_encode", side_effect=EmbeddingError(code, "private-provider-detail")) as encode:
                page = self.ok("query", query="ZXQ-714")
                self.assertEqual(encode.call_count, 1)
                self.assertEqual(page["coverage"]["semantic"], {"status": "unavailable", "reason": code})
                self.assertEqual(page["coverage"]["query_encoding"], "unavailable")
                self.assertEqual(page["coverage"]["fusion"], "lexical_only")
                self.assertTrue(page["coverage"]["partial"])
                self.assertEqual(page["coverage"]["semantic_candidates"], 0)
                self.assertEqual(page["coverage"]["union_candidates"], 1)
                row = page["results"][0]
                self.assertEqual(row["ref"], "event:target")
                self.assertIsNone(row["score"])
                self.assertIsNone(row["retrieval"]["semantic_rank"])
                self.assertIsNone(row["retrieval"]["semantic_evidence"])
                self.assertEqual(row["retrieval"]["lexical_rank"], 1)
                evidence = row["retrieval"]["lexical_evidence"]
                self.assertEqual(body[evidence["start_byte"]:evidence["end_byte"]], "ZXQ-714")
                self.assertNotIn("private-provider-detail", json.dumps(page))
                with sqlite3.connect(self.config["database"]) as db:
                    self.assertEqual(db.execute("SELECT COUNT(*) FROM queries").fetchone()[0], 0)
        # 服务恢复时自动恢复两路；没有写入伪查询向量或粘滞退化态。
        healthy = self.ok("query", query="ZXQ-714")
        self.assertEqual(healthy["coverage"]["semantic"]["status"], "available")
        self.assertEqual(healthy["coverage"]["fusion"], lexical.FUSION)
        self.assertIsInstance(healthy["results"][0]["score"], float)
        with patch.object(self.worker, "_encode", side_effect=AssertionError("must use cache")):
            self.assertEqual(self.ok("query", query="ZXQ-714")["coverage"]["query_encoding"], "cache")

    def test_encoding_integrity_and_authorization_errors_never_become_lexical_success(self):
        self.finish(self.put(self.begin(), "target", "ZXQ-714"))
        for code in ("redirect_blocked", "response_limit", "local_budget_exhausted", "query_not_approved", "invalid_response", "invalid_vector"):
            with self.subTest(code=code), patch.object(self.worker, "_encode", side_effect=EmbeddingError(code, "synthetic")):
                expected = code if code in ("redirect_blocked", "response_limit", "local_budget_exhausted") else "local_encoding_failed"
                self.error(expected, "query", query="ZXQ-714")
        self.worker.query_approved = False
        with patch.object(self.worker, "_encode", side_effect=AssertionError("no encoding after revocation")):
            self.error("query_not_approved", "query", query="ZXQ-714")

    def test_legacy_and_semantic_only_queries_do_not_silently_change_mode(self):
        self.finish(self.put(self.begin(), "target", "ZXQ-714"))
        with patch.object(self.worker, "_encode", side_effect=EmbeddingError("local_provider_unavailable", "synthetic")):
            self.error("local_provider_unavailable", "query", query="ZXQ-714", retrieval_mode="semantic_only")
        self.finish(self.put(self.begin("legacy", version=None), "target", "ZXQ-714", contexts=False))
        with patch.object(self.worker, "_encode", side_effect=EmbeddingError("local_provider_unavailable", "synthetic")):
            self.error("local_provider_unavailable", "query", query="ZXQ-714")

    def test_fallback_rechecks_configuration_and_generation_after_provider_failure(self):
        path = self.root / "worker.json"
        path.write_bytes(canonical(self.config)); path.chmod(0o600)
        self.worker = EventIndex(self.config, encoder=self.encoder, config_path=path)
        self.finish(self.put(self.begin(), "target", "ZXQ-714"))
        def revoked(*_):
            path.write_bytes(canonical({**self.config, "query_approved": False}))
            raise EmbeddingError("local_provider_unavailable", "synthetic")
        with patch.object(self.worker, "_encode", side_effect=revoked):
            self.error("configuration_changed", "query", query="ZXQ-714")
        path.write_bytes(canonical(self.config))
        def changed(*_):
            with sqlite3.connect(self.config["database"]) as db:
                db.execute("DELETE FROM metadata WHERE key='published'")
            raise EmbeddingError("local_provider_unavailable", "synthetic")
        with patch.object(self.worker, "_encode", side_effect=changed):
            self.error("generation_changed", "query", query="ZXQ-714")

    def test_fallback_empty_and_limited_queries_report_missing_channel(self):
        self.finish(self.put(self.begin(), "target", "ZXQ-714"))
        with patch.object(self.worker, "_encode", side_effect=EmbeddingError("local_provider_unavailable", "synthetic")):
            for query, status in (("missing-code", "available"), ("😀", "unsupported_query"), ("a" * 129, "limited_query")):
                with self.subTest(query=query):
                    page = self.ok("query", query=query)
                    self.assertEqual(page["results"], [])
                    self.assertEqual(page["coverage"]["lexical"]["status"], status)
                    self.assertTrue(page["coverage"]["partial"])
                    self.assertEqual(page["coverage"]["fusion"], "lexical_only")

    def test_fallback_does_not_bypass_corrupt_vectors_or_source_scope(self):
        self.finish(self.put(self.begin(), "target", "ZXQ-714"))
        pristine = Path(self.config["database"]).read_bytes()
        for mutation, code in (("UPDATE sources SET scope='forbidden'", "corrupt_snapshot"),
                               ("UPDATE vectors SET vector=zeroblob(16)", "corrupt_vector"),
                               ("DELETE FROM lexical_terms", "corrupt_lexical_index")):
            with self.subTest(mutation=mutation):
                Path(self.config["database"]).write_bytes(pristine)
                with sqlite3.connect(self.config["database"]) as db:
                    db.execute(mutation)
                with patch.object(self.worker, "_encode", side_effect=EmbeddingError("local_provider_unavailable", "synthetic")):
                    self.error(code, "query", query="ZXQ-714")

    def test_independent_identifier_recalls_semantic_rank101_and_preserves_cosine(self):
        self.encoder.vectors["ZXQ-714"] = [1, 0, 0, 0]
        build = self.begin()
        for i in range(100):
            body = f"ordinary distractor {i}"
            self.encoder.vectors[body] = [1, .001 * (i + 1), 0, 0]
            self.put(build, f"distractor-{i:03}", body)
        body = "reference ZXQ-714 exact"
        self.encoder.vectors[body] = [-1, 0, 0, 0]
        self.finish(self.put(build, "target", body))
        semantic = self.ok("query", query="ZXQ-714", limit=100, retrieval_mode="semantic_only")
        self.assertNotIn("event:target", [row["ref"] for row in semantic["results"]])
        hybrid = self.ok("query", query="ZXQ-714", limit=10)
        target = hybrid["results"][0]
        self.assertEqual(target["ref"], "event:target")
        self.assertEqual(target["score"], -1)
        self.assertIsNone(target["retrieval"]["semantic_rank"])
        self.assertEqual(target["retrieval"]["lexical_rank"], 1)
        self.assertEqual(target["retrieval"]["lexical_evidence"],
                         {"start_byte": 10, "end_byte": 17, "text_sha256": digest(b"ZXQ-714")})
        self.assertEqual(hybrid["coverage"]["union_candidates"], 101)
        self.assertEqual(hybrid["coverage"]["semantic_candidates"], 100)
        self.assertEqual(hybrid["coverage"]["lexical_candidates"], 1)
        self.assertEqual(hybrid["coverage"]["fusion"], lexical.FUSION)
        self.assertEqual(hybrid, self.ok("query", query="ZXQ-714", limit=10) | {"coverage": hybrid["coverage"]})

    def test_whole_case_sensitive_identifier_does_not_match_neighbors(self):
        build = self.begin()
        bodies = ["ZXQ-714", "ZXQ-7140", "ZXQ-714.1", "preZXQ-714", "zxq-714", "C++", "C", "/foo/bar", "foo/bar"]
        for i, body in enumerate(bodies):
            result = self.put(build, i, body)
        self.finish(result)
        for expected in ("ZXQ-714", "C++", "C", "/foo/bar", "foo/bar"):
            with self.subTest(query=expected):
                page = self.ok("query", query=expected, limit=100)
                matched = [r["ref"] for r in page["results"] if r["retrieval"]["lexical_rank"]]
                self.assertEqual(matched, [f"event:{bodies.index(expected)}"])

    def test_cross_chunk_ascii_and_han_have_complete_original_span(self):
        for name, body, cut, query in (
            ("ascii", "prefix ZXQ-714 suffix", 11, "ZXQ-714"),
            ("han", "文字编号正确", len("文字编".encode()), "编号"),
        ):
            build = self.begin(name)
            self.finish(self.put(build, name, body, [0, cut, len(body.encode())]))
            row = self.ok("query", query=query)["results"][0]
            evidence = row["retrieval"]["lexical_evidence"]
            span = body.encode()[evidence["start_byte"]:evidence["end_byte"]]
            self.assertEqual(span.decode(), query)
            self.assertEqual(digest(span), evidence["text_sha256"])
            self.assertLess(evidence["start_byte"], cut)
            self.assertGreater(evidence["end_byte"], cut)
            fragment = body.encode()[:cut].decode().split()[-1] if name == "ascii" else "编"
            self.assertEqual(self.ok("query", query=fragment)["coverage"]["lexical_candidates"], 0)

    def test_semantic_and_lexical_evidence_keep_different_chunk_ranges(self):
        body = "strong semantic chunk " + "ZXQ-714 here"
        cut = len("strong semantic chunk ")
        self.encoder.vectors.update({"strong semantic chunk ": [1,0,0,0], "ZXQ-714 here": [-1,0,0,0], "ZXQ-714": [1,0,0,0]})
        self.finish(self.put(self.begin(), "target", body, [0, cut, len(body)]))
        row = self.ok("query", query="ZXQ-714")["results"][0]
        self.assertEqual(row["start_byte"], cut)
        self.assertEqual(row["score"], -1)
        self.assertEqual(row["retrieval"]["semantic_evidence"]["start_byte"], 0)
        self.assertEqual(row["retrieval"]["semantic_evidence"]["score"], 1)
        self.assertEqual(row["retrieval"]["semantic_rank"], 1)
        self.assertEqual(self.ok("query", query="ZXQ-714")["coverage"]["chunk_selection"], "lexical_primary_else_semantic_best")

    def test_legacy_read_and_explicit_refresh_reuse_old_vectors_without_implicit_migration(self):
        legacy = self.finish(self.put(self.begin(version=None), "old", "ZXQ-714", contexts=False))
        before = len(self.encoder.calls)
        self.worker = EventIndex(self.config, encoder=self.encoder)
        status = self.ok("status")
        self.assertEqual(status["lexical_status"], "legacy_unavailable")
        self.assertEqual(len(self.encoder.calls), before)
        with sqlite3.connect(self.config["database"]) as db:
            self.assertFalse(db.execute("SELECT 1 FROM sqlite_master WHERE name='lexical_terms'").fetchone())
        page = self.ok("query", query="ZXQ-714")
        self.assertEqual(page["coverage"]["lexical"]["status"], "legacy_unavailable")
        refresh = self.begin()
        self.assertNotEqual(refresh["build_id"], legacy["build_id"])
        self.assertEqual(refresh["accepted_batches"], 0)
        self.assertEqual(self.ok("query", query="ZXQ-714")["generation"], legacy["generation"])
        result = self.put(refresh, "old", "ZXQ-714")
        self.assertEqual(result["encoded_chunks"], 0)
        self.assertEqual(result["reused_chunks"], 1)
        self.finish(result)
        self.assertEqual(self.ok("query", query="ZXQ-714")["coverage"]["lexical"]["status"], "available")
        self.assertEqual([kind for kind, _ in self.encoder.calls], ["corpus", "query"])

    def test_correction_revocation_deletion_and_scope_are_synchronized(self):
        self.finish(self.put(self.begin(), "same", "old-code"))
        changed = self.begin("corrected")
        self.finish(self.put(changed, "same", "new-code"))
        self.assertEqual(self.ok("query", query="old-code")["coverage"]["lexical_candidates"], 0)
        self.assertEqual(self.ok("query", query="new-code")["coverage"]["lexical_candidates"], 1)
        with sqlite3.connect(self.config["database"]) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM lexical_generations").fetchone()[0], 1)
            self.assertEqual(db.execute("SELECT COUNT(*) FROM lexical_terms").fetchone()[0], 1)
            db.execute("UPDATE sources SET scope='forbidden'")
        before = len(self.encoder.calls)
        self.error("corrupt_snapshot", "query", query="private-request")
        self.assertEqual(len(self.encoder.calls), before)
        with sqlite3.connect(self.config["database"]) as db:
            db.execute("UPDATE sources SET scope='synthetic'")
        self.finish(self.begin("revoked-empty"))
        self.assertEqual(self.ok("query", query="new-code")["results"], [])
        with sqlite3.connect(self.config["database"]) as db:
            for table in ("lexical_chunks", "lexical_terms", "vectors"):
                self.assertEqual(db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0], 0)

    def test_finish_requires_lexical_coverage_and_manifest_integrity(self):
        result = self.put(self.begin(), "test", "ZXQ-714")
        with sqlite3.connect(self.config["database"]) as db:
            db.execute("DELETE FROM lexical_terms")
        self.error("corrupt_lexical_index", "finish", **{key: result[key] for key in ("build_id", "source_epoch", "receipt", "source_count", "chunk_count")})
        self.error("index_unavailable", "query", query="ZXQ-714")

    def test_corrupt_terms_fail_closed_before_encoding_and_bounded_allocation(self):
        self.finish(self.put(self.begin(), "test", "ZXQ-714"))
        path = Path(self.config["database"])
        pristine = path.read_bytes()
        mutations = (
            "DELETE FROM lexical_chunks", "DELETE FROM lexical_terms", "DELETE FROM lexical_generations",
            "UPDATE lexical_generations SET version='unknown'", "UPDATE lexical_generations SET term_count=2000001",
            "UPDATE lexical_chunks SET term_count=4097", "UPDATE lexical_chunks SET terms_hash=printf('%064d',0)",
            "UPDATE lexical_terms SET term_hash=printf('%064d',0)",
            "UPDATE lexical_terms SET source_ref='event:other'", "UPDATE lexical_terms SET ordinal=4",
            "UPDATE lexical_terms SET end_byte=end_byte+1", "UPDATE lexical_terms SET text_sha256=printf('%064d',0)",
            "UPDATE lexical_terms SET term_hash=zeroblob(262144)",
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                path.write_bytes(pristine)
                with sqlite3.connect(path) as db:
                    db.execute(mutation)
                before = len(self.encoder.calls)
                tracemalloc.start()
                try:
                    result = self.call("query", query="never encoded")
                    _, peak = tracemalloc.get_traced_memory()
                finally:
                    tracemalloc.stop()
                self.assertFalse(result["ok"], result)
                self.assertIn(result["error"]["code"], ("corrupt_lexical_index", "corrupt_snapshot", "lexical_version_unsupported", "index_storage_error"))
                self.assertEqual(before, len(self.encoder.calls))
                self.assertLess(peak, 4 * 1024**2)
        path.write_bytes(pristine)

    def test_budgets_context_and_unsupported_queries_are_honest(self):
        build = self.begin()
        body = "prefix ZXQ-714 suffix"
        chunks = self.chunks(body, [0, 11, len(body)])
        chunks[0]["lexical_after"] = "7"
        self.error("lexical_context_required", "upsert", build_id=build["build_id"], batch_id="bad", source=self.source("bad", body), source_chunk_count=2, chunks=chunks)
        self.assertEqual(self.encoder.calls, [])
        with patch.object(lexical, "MAX_CHUNK_TERMS", 1):
            self.error("lexical_chunk_term_limit", "upsert", build_id=build["build_id"], batch_id="too-many", source=self.source("bad", "one two"), source_chunk_count=1, chunks=self.chunks("one two"))
        with patch.object(lexical, "MAX_TOTAL_TERMS", 0):
            self.error("lexical_index_term_limit", "upsert", build_id=build["build_id"], batch_id="total", source=self.source("bad", "one"), source_chunk_count=1, chunks=self.chunks("one"))
        self.finish(self.put(build, "good", "ZXQ-714"))
        for query, reason in (
            (" ".join(f"word{i}" for i in range(33)), "lexical_query_term_limit"),
            ("a" * 129, "lexical_query_term_length_limit"),
            ("请帮我找到此前讨论过的方案选择原因以及后来修改决定时留下的具体证据和相关记录", "lexical_query_term_limit"),
        ):
            with self.subTest(query=query):
                page = self.ok("query", query=query)
                self.assertTrue(page["results"])
                self.assertEqual(page["coverage"]["lexical"]["status"], "limited_query")
                self.assertEqual(page["coverage"]["lexical"]["reason"], reason)
                self.assertEqual(page["coverage"]["lexical_candidates"], 0)
                self.assertEqual(page["coverage"]["lexical"]["postings_scanned"], 0)
                self.assertEqual(self.encoder.calls[-1], ("query", [query]))
                semantic = self.ok("query", query=query, retrieval_mode="semantic_only")
                self.assertEqual([r["ref"] for r in page["results"]], [r["ref"] for r in semantic["results"]])
                self.assertEqual(semantic["coverage"]["chunk_selection"], "per_event_best_chunk")
        result = self.ok("query", query="🙂")
        self.assertEqual(result["coverage"]["lexical"]["status"], "unsupported_query")
        self.assertTrue(result["results"])

    def test_posting_and_candidate_budgets_are_explicit_and_stable(self):
        build = self.begin()
        for i in range(12):
            result = self.put(build, str(i), "common phrase")
        self.finish(result)
        with patch.object(lexical, "MAX_POSTINGS", 6), patch.object(lexical, "MAX_CANDIDATE_CHUNKS", 3):
            first = self.ok("query", query="common", limit=100)
            second = self.ok("query", query="common", limit=100)
        self.assertTrue(first["coverage"]["lexical"]["candidates_truncated"])
        self.assertEqual(first["coverage"]["lexical"]["postings_scanned"], 6)
        self.assertEqual(first["coverage"]["lexical"]["candidate_chunks"], 3)
        self.assertEqual(first["results"], second["results"])
        self.assertLessEqual(first["coverage"]["union_candidates"], 200)

    def test_round_robin_does_not_starve_later_query_terms_at_global_budget(self):
        # 强制高频词的hash排在罕见编号之前，不能靠词hash的偶然顺序通过。
        pair = sorted((lexical.terms(term, query=True)[0][0], term) for term in ("common", "ZXQ-714"))
        common, rare = pair[0][1], pair[1][1]
        build = self.begin()
        for i in range(12):
            self.put(build, f"common-{i}", common)
        self.finish(self.put(build, "rare", rare))
        with patch.object(lexical, "MAX_POSTINGS", 4), patch.object(lexical, "MAX_CANDIDATE_CHUNKS", 2):
            result = self.ok("query", query=common + " " + rare, limit=100)
        coverage = result["coverage"]["lexical"]
        self.assertEqual(coverage["postings_scanned"], 4)
        self.assertEqual(coverage["candidate_chunks"], 2)
        self.assertEqual(coverage["query_terms_visited"], 2)
        self.assertEqual(coverage["posting_order"], "round_robin_terms")
        self.assertTrue(coverage["candidates_truncated"])
        rare_row = next(r for r in result["results"] if r["ref"] == "event:rare")
        self.assertIsNotNone(rare_row["retrieval"]["lexical_rank"])

    def test_real_sqlite_disk_failure_preserves_published_and_empty_gc(self):
        self.config = {**self.config, "database": str(self.root / "small.sqlite"), "disk_cap_bytes": 1024**2}
        self.worker = EventIndex(self.config, encoder=self.encoder)
        old = self.finish(self.put(self.begin("old"), "old", "oldkey"))
        build = self.begin("large")
        failure = None
        for i in range(100):
            body = " ".join(f"term-{i}-{n}" for n in range(100))
            response = self.call("upsert", build_id=build["build_id"], batch_id=str(i),
                source=self.source(i, body), source_chunk_count=1, chunks=self.chunks(body))
            if not response["ok"]:
                failure = response
                break
        self.assertIsNotNone(failure)
        self.assertEqual(failure["error"]["code"], "disk_limit")
        self.assertEqual(self.ok("query", query="oldkey")["generation"], old["generation"])
        self.assertLessEqual(self.worker._disk_bytes(), self.worker.disk_cap)
        self.finish(self.begin("empty"))
        self.assertEqual(self.ok("query", query="oldkey")["results"], [])
        with sqlite3.connect(self.config["database"]) as db:
            for table in ("lexical_chunks", "lexical_terms", "vectors"):
                self.assertEqual(db.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0], 0)

    def test_resume_idempotence_and_context_conflicts_are_bound_to_receipt(self):
        build = self.begin()
        body = "ABCDE"
        row = self.put(build, "one", body, [0, 3, 5])
        self.worker = EventIndex(self.config, encoder=self.encoder)
        resumed = self.begin()
        self.assertEqual(resumed["receipt"], row["receipt"])
        before = len(self.encoder.calls)
        replay = self.put(build, "one", body, [0, 3, 5])
        self.assertTrue(replay["already_applied"])
        self.assertEqual(len(self.encoder.calls), before)
        chunks = self.chunks(body, [0, 3, 5])
        chunks[0]["lexical_after"] = "XY"
        common = dict(build_id=build["build_id"], source=self.source("one", body), source_chunk_count=2, chunks=chunks)
        self.error("batch_conflict", "upsert", batch_id="one", **common)
        self.error("lexical_chunk_conflict", "upsert", batch_id="changed-context", **common)
        self.assertEqual(len(self.encoder.calls), before)
        self.finish(replay)

    def test_complete_han_boundary_emoji_context_and_long_ascii_fragments(self):
        body = "🙂" * 100 + " X" + "Y" * 160 + "Z 汉字序列 " + "🙂" * 100
        raw = body.encode()
        cut = len(("🙂" * 100 + " X" + "Y" * 100).encode())
        build = self.begin()
        self.finish(self.put(build, "boundary", body, [0, cut, len(raw)]))
        self.assertEqual(self.ok("query", query="Y" * 60 + "Z")["coverage"]["lexical_candidates"], 0)
        self.assertEqual(self.ok("query", query="汉字")["coverage"]["lexical_candidates"], 1)
        self.error("lexical_version_unsupported", "begin", source_epoch=digest(b"new"), lexical_version="future")

    def test_terms_and_context_do_not_persist_plaintext_or_change_embedding_inputs(self):
        body = "private-synthetic-marker-ZXQ714"
        build = self.begin()
        self.finish(self.put(build, "test", body, [0, 14, len(body)]))
        calls = [text for kind, texts in self.encoder.calls if kind == "corpus" for text in texts]
        self.assertEqual(calls, [body[:14], body[14:]])
        self.assertNotIn(body.encode(), Path(self.config["database"]).read_bytes())
        self.assertNotIn(b"private-synthetic", Path(self.config["database"]).read_bytes())
        with sqlite3.connect(self.config["database"]) as db:
            self.assertEqual(db.execute("SELECT version FROM lexical_generations").fetchone()[0], lexical.VERSION)
        self.assertNotIn(body, json.dumps(self.ok("query", query=body)))


if __name__ == "__main__":
    unittest.main()
