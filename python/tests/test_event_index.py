"""独立 Event 索引只用合成文字/数值；这些测试不证明模型语义质量。"""
import io
import json
import os
from pathlib import Path
import random
import sqlite3
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import tracemalloc
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from unittest.mock import patch
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_worker.event_index import (
    CONFIG_SCHEMA, SCHEMA, MAX_LINE, EventIndex, EventIndexError,
    canonical, digest, read_config, serve, vector_blob, decode_blob,
    main,
)


def settings(path):
    return {"schema": CONFIG_SCHEMA, "database": str(path), "scope": "synthetic",
            "events_approved": True, "query_approved": True, "disk_cap_bytes": 16 * 1024**2,
            "backend": "python", "space": {"provider": "local-openai-compatible",
            "endpoint": "http://127.0.0.1:46709/v1/embeddings", "model": "synthetic-v1",
            "revision": "fixture-r1", "dimensions": 4, "tokenizer": "fixture-t1",
            "preprocessing_version": "utf8-v1", "chunking_version": "fixture-c1",
            "document_instruction": "", "query_instruction": "", "normalization": "l2"}}


class Encoder:
    def __init__(self):
        self.calls = []
        self.vectors = {}
    def __call__(self, kind, texts):
        self.calls.append((kind, list(texts)))
        return [self.vectors.get(text, [1., float(sum(text.encode()) % 17 + 1), 2., 3.]) for text in texts]


class EventIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = settings(self.root / "index.sqlite")
        self.encoder = Encoder()
        self.worker = EventIndex(self.config, encoder=self.encoder)
        self.ident = 0
    def tearDown(self):
        self.temp.cleanup()
    def request(self, op, **values):
        self.ident += 1
        return {"schema": SCHEMA, "id": self.ident, "op": op, **values}
    def ok(self, op, worker=None, **values):
        response = (worker or self.worker).execute(self.request(op, **values))
        self.assertTrue(response["ok"], response)
        return response["result"]
    def error(self, code, op, worker=None, **values):
        response = (worker or self.worker).execute(self.request(op, **values))
        self.assertFalse(response["ok"], response)
        self.assertEqual(response["error"]["code"], code, response)
        return response
    def begin(self, name="snapshot-1", worker=None):
        return self.ok("begin", worker=worker, source_epoch=digest(name.encode()))
    def source(self, i=0, content="synthetic text", **changes):
        raw = content.encode()
        return {"ref": f"event:synthetic-{i}", "content_hash": digest(raw), "scope": "synthetic",
                "role": "user", "session_ref": "synthetic-session:one", "occurred_at": "2026-01-01T00:00:00Z",
                "text_bytes": len(raw), **changes}
    def chunk(self, content="synthetic text", ordinal=0, start=0):
        return {"ordinal": ordinal, "start_byte": start, "end_byte": start+len(content.encode()),
                "text_sha256": digest(content.encode()), "text": content}
    def put(self, build, i=0, content="synthetic text", worker=None):
        return self.ok("upsert", worker=worker, build_id=build["build_id"], batch_id=f"batch-{i}",
                       source=self.source(i, content), source_chunk_count=1, chunks=[self.chunk(content)])
    def finish(self, result, worker=None):
        return self.ok("finish", worker=worker, **{k: result[k] for k in ("build_id", "source_epoch", "source_count", "chunk_count", "receipt")})
    def published(self):
        return self.finish(self.put(self.begin()))

    def test_build_query_has_no_text_or_vector_and_preserves_metadata(self):
        finish = self.published()
        result = self.ok("query", query="a new question", limit=10, expected_generation=finish["generation"])
        self.assertEqual(result["coverage"]["query_encoding"], "local")
        self.assertEqual(result["coverage"]["scanned_chunks"], 1)
        self.assertEqual(result["source_epoch"], finish["source_epoch"])
        row = result["results"][0]
        self.assertEqual(row["ref"], "event:synthetic-0")
        self.assertEqual(row["role"], "user")
        self.assertEqual(row["content_hash"], digest(b"synthetic text"))
        self.assertNotIn("text", row)
        self.assertNotIn("vector", row)
        self.assertEqual(self.ok("query", query="a new question")["coverage"]["query_encoding"], "cache")
        with sqlite3.connect(self.config["database"]) as db:
            columns = [row[1] for table in ("sources", "chunks", "vectors", "queries", "batches") for row in db.execute(f"PRAGMA table_info({table})")]
        self.assertNotIn("text", columns)
        self.assertNotIn(b"synthetic text", Path(self.config["database"]).read_bytes())
        self.assertNotIn(b"a new question", Path(self.config["database"]).read_bytes())

    def test_deterministic_resume_and_batch_idempotence(self):
        build = self.begin()
        first = self.put(build)
        restarted = EventIndex(self.config, encoder=self.encoder)
        resumed = self.begin(worker=restarted)
        self.assertEqual(resumed["build_id"], build["build_id"])
        self.assertEqual(resumed["receipt"], first["receipt"])
        self.assertEqual(resumed["accepted_batches"], 1)
        replay = self.put(build, worker=restarted)
        self.assertTrue(replay["already_applied"])
        self.assertEqual(len(self.encoder.calls), 1)
        self.finish(replay, worker=restarted)
        replay = self.put(build, worker=restarted)
        self.assertTrue(replay["already_applied"])
        self.error("batch_conflict", "upsert", build_id=build["build_id"], batch_id="batch-0",
                   source=self.source(content="different"), source_chunk_count=1, chunks=[self.chunk("different")])

    def test_long_source_chunk_count_ordinal_ranges_and_finish(self):
        build = self.begin()
        source = self.source(content="first second")
        result = self.ok("upsert", build_id=build["build_id"], batch_id="part-0", source=source,
                         source_chunk_count=2, chunks=[self.chunk("first", 0, 0)])
        self.assertEqual(result["pending_sources"], 1)
        self.error("incomplete_sources", "finish", **{k: result[k] for k in ("build_id", "source_epoch", "source_count", "chunk_count", "receipt")})
        result = self.ok("upsert", build_id=build["build_id"], batch_id="part-1", source=source,
                         source_chunk_count=2, chunks=[self.chunk("second", 1, 6)])
        self.assertEqual(result["pending_sources"], 0)
        self.finish(result)
        self.assertEqual(len(self.ok("query", query="question")["results"]), 1)

    def test_finish_receipt_epoch_and_counts_are_required(self):
        result = self.put(self.begin())
        correct = {k: result[k] for k in ("build_id", "source_epoch", "source_count", "chunk_count", "receipt")}
        for key in ("receipt", "source_epoch"):
            self.error("snapshot_mismatch", "finish", **{**correct, key: digest(b"wrong")})
        self.error("snapshot_count_mismatch", "finish", **{**correct, "chunk_count": 2})
        self.finish(result)

    def test_cross_generation_reuse_and_removed_source_gc(self):
        self.published()
        second = self.begin("snapshot-2")
        first = self.put(second, 7)
        self.assertEqual(first["encoded_chunks"], 0)
        self.assertEqual(first["reused_chunks"], 1)
        second_row = self.put(second, 8, "new synthetic document")
        self.finish(second_row)
        self.assertEqual({r["ref"] for r in self.ok("query", query="question")["results"]}, {"event:synthetic-7", "event:synthetic-8"})
        empty = self.begin("snapshot-3")
        self.finish(empty)
        status = self.ok("status")
        self.assertEqual(status["source_count"], 0)
        self.assertEqual(status["vector_count"], 0)
        self.assertEqual(self.ok("query", query="question")["results"], [])
        with sqlite3.connect(self.config["database"]) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM generations").fetchone()[0], 1)
            self.assertEqual(db.execute("SELECT COUNT(*) FROM chunks").fetchone()[0], 0)

    def test_partial_build_preserves_old_published_generation(self):
        old = self.published()
        newer = self.begin("snapshot-2")
        self.put(newer, 1, "new input")
        result = self.ok("query", query="question")
        self.assertEqual(result["generation"], old["generation"])
        self.assertEqual(result["results"][0]["ref"], "event:synthetic-0")
        self.error("generation_mismatch", "query", query="question", expected_generation=newer["build_id"])

    def test_permissions_are_independent_even_for_cached_queries(self):
        self.published()
        self.ok("query", query="cached")
        no_query = EventIndex({**self.config, "query_approved": False}, encoder=self.encoder)
        self.error("query_not_approved", "query", worker=no_query, query="cached")
        no_events = EventIndex({**self.config, "events_approved": False}, encoder=self.encoder)
        self.error("events_not_approved", "begin", worker=no_events, source_epoch=digest(b"new"))
        self.ok("query", worker=no_events, query="cached")
        self.assertEqual(len(self.encoder.calls), 2)

    def test_requests_cannot_override_path_space_scope_or_approval(self):
        for op, extra in (("status", {"database": "/tmp/other"}), ("begin", {"source_epoch": digest(b"a"), "events_approved": True}),
                          ("query", {"query": "q", "endpoint": "https://outside.invalid/v1/embeddings"}), ("query", {"query": "q", "scope": "foreign"})):
            self.error("invalid_request", op, **extra)
        self.assertEqual(self.encoder.calls, [])

    def test_scope_hash_text_length_and_range_rejected_before_encoding(self):
        build = self.begin()
        good_source, good_chunk = self.source(), self.chunk()
        for source, chunk in [({**good_source, "scope": "foreign"}, good_chunk),
                              (good_source, {**good_chunk, "text_sha256": digest(b"wrong")}),
                              (good_source, {**good_chunk, "end_byte": good_chunk["end_byte"]+1}),
                              (good_source, {**good_chunk, "ordinal": 1})]:
            response = self.worker.execute(self.request("upsert", build_id=build["build_id"], batch_id="bad", source=source, source_chunk_count=1, chunks=[chunk]))
            self.assertFalse(response["ok"])
        self.assertEqual(self.encoder.calls, [])
        result = self.put(build, content="合成的中文原文")
        self.finish(result)
        self.assertEqual(self.ok("query", query="换种说法")["results"][0]["end_byte"], len("合成的中文原文".encode()))

    def test_failed_encoding_does_not_publish_or_store_partial_vectors(self):
        old = self.published()
        new = self.begin("new")
        def failing(kind, texts):
            raise RuntimeError("synthetic_private_marker")
        worker = EventIndex(self.config, encoder=failing)
        response = self.error("worker_error", "upsert", worker=worker, build_id=new["build_id"], batch_id="failing",
                              source=self.source(8, "unknown fixture"), source_chunk_count=1, chunks=[self.chunk("unknown fixture")])
        self.assertNotIn("synthetic_private_marker", json.dumps(response))
        self.assertEqual(self.ok("query", query="q")["generation"], old["generation"])
        self.assertEqual(self.ok("status")["chunk_count"], 0)

    def test_model_wait_is_outside_sqlite_transaction(self):
        build = self.begin()
        def encode(kind, texts):
            with sqlite3.connect(self.config["database"], timeout=0) as db:
                db.execute("BEGIN IMMEDIATE")
                db.rollback()
            return [[1, 0, 0, 0] for _ in texts]
        worker = EventIndex(self.config, encoder=encode)
        result = self.put(build, worker=worker)
        self.finish(result, worker=worker)
        self.ok("query", worker=worker, query="question")

    def test_query_cache_retains_only_32_and_does_not_store_queries(self):
        self.published()
        for i in range(35):
            self.ok("query", query=f"synthetic-query-{i}")
        with sqlite3.connect(self.config["database"]) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM queries").fetchone()[0], 32)
        self.assertEqual(self.ok("query", query="synthetic-query-34")["coverage"]["query_encoding"], "cache")
        self.assertEqual(self.ok("query", query="synthetic-query-0")["coverage"]["query_encoding"], "local")

    def test_paged_ranking_matches_exact_float32_reference(self):
        rng = random.Random(711)
        query = [rng.uniform(-1,1) for _ in range(4)]
        self.encoder.vectors["query"] = query
        build = self.begin()
        expected = []
        q = decode_blob(vector_blob(query, 4), 4)
        for i in range(300):
            label = f"numeric-fixture-{i}"
            values = [rng.uniform(-1,1) for _ in range(4)]
            self.encoder.vectors[label] = values
            row = self.put(build, i, label)
            score = sum(a*b for a,b in zip(q, decode_blob(vector_blob(values,4),4)))
            expected.append((score, f"event:synthetic-{i}"))
        self.finish(row)
        result = self.ok("query", query="query", limit=7)
        self.assertEqual([r["ref"] for r in result["results"]], [r[1] for r in sorted(expected, reverse=True)[:7]])
        self.assertEqual(result["coverage"]["scanned_chunks"], 300)
        self.assertEqual(result["coverage"]["page_rows"], 256)
        self.assertTrue(result["coverage"]["candidates_truncated"])

    def test_corrupt_vector_fails_closed(self):
        self.published()
        with sqlite3.connect(self.config["database"]) as db:
            db.execute("UPDATE vectors SET vector=?", (struct.pack('<ffff', float('nan'), 0, 0, 0),))
        self.error("corrupt_vector", "query", query="question")

    def test_shared_oversize_blob_is_rejected_before_page_allocation_or_encoding(self):
        build = self.begin()
        for i in range(256):
            result = self.put(build, i, "same synthetic input")
        self.finish(result)
        with sqlite3.connect(self.config["database"]) as db:
            db.execute("UPDATE vectors SET vector=zeroblob(262144)")
        before = len(self.encoder.calls)
        tracemalloc.start()
        try:
            result = self.worker.execute(self.request("query", query="synthetic question"))
            _, peak = tracemalloc.get_traced_memory()
        finally:
            tracemalloc.stop()
        self.assertFalse(result["ok"], result)
        self.assertIn(result["error"]["code"], ("corrupt_snapshot", "index_storage_error"))
        self.assertEqual(len(self.encoder.calls), before)
        self.assertLess(peak, 4 * 1024 * 1024)

    def test_published_rows_fields_and_relationships_fail_closed(self):
        self.published()
        path = Path(self.config["database"])
        pristine = path.read_bytes()
        changes = (
            "DELETE FROM sources", "DELETE FROM chunks", "DELETE FROM vectors",
            "UPDATE sources SET ref='event:orphan'",
            "UPDATE sources SET content_hash='invalid'",
            "UPDATE sources SET scope='outside'",
            "UPDATE sources SET role='invalid'",
            "UPDATE sources SET session_ref=printf('%01024d',1)",
            "UPDATE sources SET occurred_at=printf('%0128d',1)",
            "UPDATE sources SET received_chunks=0",
            "UPDATE sources SET expected_chunks=2,received_chunks=2",
            "UPDATE sources SET text_bytes=1",
            "UPDATE chunks SET ordinal=4",
            "UPDATE chunks SET end_byte=end_byte+1",
            "UPDATE chunks SET text_sha256='invalid'",
            "UPDATE chunks SET input_hash=printf('%064d',0)",
            "UPDATE vectors SET vector=zeroblob(4096)",
            "UPDATE generations SET state='building'",
            "UPDATE generations SET source_count=4",
            "UPDATE generations SET chunk_count=4",
        )
        try:
            for mutation in changes:
                with self.subTest(mutation=mutation):
                    path.write_bytes(pristine)
                    with sqlite3.connect(path) as db:
                        db.execute(mutation)
                    before = len(self.encoder.calls)
                    self.error("corrupt_snapshot", "query", query="synthetic question")
                    self.assertEqual(len(self.encoder.calls), before)
        finally:
            path.write_bytes(pristine)

    def test_event_best_chunk_precedes_top_k_and_source_set_hash(self):
        self.encoder.vectors.update({"A": [1, 0, 0, 0], "B": [0.9, 0.1, 0, 0], "Q": [1, 0, 0, 0]})
        build = self.begin()
        source = self.source(0, content="A" * 101)
        for i in range(101):
            self.ok("upsert", build_id=build["build_id"], batch_id=f"long-{i}", source=source,
                    source_chunk_count=101, chunks=[self.chunk("A", i, i)])
        self.finish(self.put(build, 1, "B"))
        result = self.ok("query", query="Q", limit=100)
        self.assertEqual([r["ref"] for r in result["results"]], ["event:synthetic-0", "event:synthetic-1"])
        self.assertEqual(result["results"][0]["start_byte"], 0)
        self.assertEqual(result["coverage"]["scanned_chunks"], 102)
        self.assertEqual(result["coverage"]["chunk_selection"], "per_event_best_chunk")
        self.assertFalse(result["coverage"]["candidates_truncated"])
        self.assertEqual(result["source_set_hash"], digest(canonical(["event:synthetic-0", "event:synthetic-1"])))

    def test_finish_failure_rolls_back_old_published_and_gc(self):
        old = self.published()
        new = self.put(self.begin("new"), 1, "new fixture")
        with sqlite3.connect(self.config["database"]) as db:
            db.execute("CREATE TRIGGER block_publish BEFORE UPDATE ON metadata WHEN NEW.key='published' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END")
        self.error("index_storage_error", "finish", **{k:new[k] for k in ("build_id", "source_epoch", "source_count", "chunk_count", "receipt")})
        self.assertEqual(self.ok("query", query="query")["generation"], old["generation"])
        with sqlite3.connect(self.config["database"]) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM vectors").fetchone()[0], 2)
            db.execute("DROP TRIGGER block_publish")
        self.finish(new)

    def test_disk_cap_failure_preserves_published_generation(self):
        old = self.published()
        new = self.begin("new")
        measured = self.worker._disk_bytes
        calls = 0
        def disk_size():
            nonlocal calls
            calls += 1
            # 读取缓存事务两次检查，写事务开始一次；在写事务提交前模拟日志加总超限。
            return self.worker.disk_cap + 1 if calls == 4 else measured()
        with patch.object(self.worker, '_disk_bytes', side_effect=disk_size):
            result = self.worker.execute(self.request("upsert", build_id=new["build_id"], batch_id="new",
                    source=self.source(2, "changed synthetic input"), source_chunk_count=1, chunks=[self.chunk("changed synthetic input")]))
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["error"]["code"], "disk_limit")
        self.assertEqual(self.ok("query", query="q")["generation"], old["generation"])
        self.assertEqual(self.ok("status")["chunk_count"], 0)

    def test_real_sqlite_page_cap(self):
        config = {**self.config, "database": str(self.root/'small.sqlite'), "disk_cap_bytes": 1024**2,
                  "space": {**self.config['space'], 'dimensions': 384}}
        worker = EventIndex(config, encoder=lambda kind, texts:[[1.]*384 for _ in texts])
        old = self.finish(self.put(self.begin(worker=worker), worker=worker), worker=worker)
        new = self.begin('large-new', worker=worker)
        failure = None
        for i in range(500):
            label=f"different-numeric-{i}"
            result=worker.execute(self.request('upsert',build_id=new['build_id'],batch_id=str(i),source=self.source(i,label),source_chunk_count=1,chunks=[self.chunk(label)]))
            if not result['ok']:
                failure=result
                break
        self.assertIsNotNone(failure)
        self.assertEqual(failure['error']['code'], 'disk_limit')
        self.assertEqual(self.ok('query',worker=worker,query='question')['generation'],old['generation'])
        self.assertLessEqual(worker._disk_bytes(),worker.disk_cap)

    def test_full_database_retains_room_for_empty_generation_gc(self):
        config = {**self.config, "database": str(self.root / "gc-cap.sqlite"), "disk_cap_bytes": 1024**2,
                  "space": {**self.config["space"], "dimensions": 384}}
        worker = EventIndex(config, encoder=lambda kind, texts: [[1.] * 384 for _ in texts])
        build = self.begin(worker=worker)
        last = None
        for i in range(500):
            label = f"unique synthetic {i}"
            result = worker.execute(self.request("upsert", build_id=build["build_id"], batch_id=str(i),
                                    source=self.source(i, label), source_chunk_count=1, chunks=[self.chunk(label)]))
            if not result["ok"]:
                self.assertEqual(result["error"]["code"], "disk_limit")
                break
            last = result["result"]
        else:
            self.fail("bounded fixture must fill the SQLite page budget")
        self.assertIsNotNone(last)
        self.finish(last, worker=worker)
        modes = set()
        for i in range(35):
            result = self.ok("query", worker=worker, query=f"different question {i}")
            modes.add(result["coverage"]["query_encoding"])
            self.assertTrue(result["results"])
        self.assertIn("local_uncached", modes)
        with sqlite3.connect(config["database"]) as db:
            self.assertLessEqual(db.execute("SELECT COUNT(*) FROM queries").fetchone()[0], 32)
        measured = worker._disk_bytes
        sizes = []
        def capture():
            size = measured()
            sizes.append(size)
            return size
        with patch.object(worker, "_disk_bytes", side_effect=capture):
            cleared = self.finish(self.begin("empty-after-clear", worker=worker), worker=worker)
        self.assertEqual(cleared["source_count"], 0)
        self.assertEqual(cleared["vector_count"], 0)
        self.assertLessEqual(max(sizes), worker.disk_cap)
        self.assertEqual(self.ok("query", worker=worker, query="question")["results"], [])

    @unittest.skipUnless(os.name == 'posix', 'POSIX权限位测试')
    def test_config_database_and_sidecar_permissions_and_symlinks(self):
        self.assertEqual(stat.S_IMODE(Path(self.config['database']).stat().st_mode), 0o600)
        config_path=self.root/'config.json'
        config_path.write_bytes(canonical(self.config));config_path.chmod(0o600)
        self.assertEqual(read_config(config_path),self.config)
        config_path.chmod(0o644)
        with self.assertRaisesRegex(EventIndexError,'unsafe_permissions'):read_config(config_path)
        config_path.chmod(0o600)
        config_link=self.root/'config-link';config_link.symlink_to(config_path)
        with self.assertRaisesRegex(EventIndexError,'unsafe_path'):read_config(config_link)
        for suffix in ('-journal','-wal','-shm'):
            link=Path(str(self.worker.path)+suffix);link.symlink_to(config_path)
            self.error('unsafe_path','status')
            link.unlink()
        hard=self.root/'database-hardlink';os.link(self.worker.path,hard)
        self.error('unsafe_path','status');hard.unlink()
        self.worker.path.chmod(0o644)
        self.error('unsafe_permissions','status')

    def test_config_size_ancestor_symlink_and_model_binding(self):
        cfg=self.root/'oversize.json';cfg.write_bytes(b' '*65537);cfg.chmod(0o600)
        with self.assertRaisesRegex(EventIndexError,'config_limit'):read_config(cfg)
        actual=self.root/'actual';actual.mkdir()
        link=self.root/'parent-link';link.symlink_to(actual,target_is_directory=True)
        with self.assertRaisesRegex(EventIndexError,'unsafe_path'):
            EventIndex({**self.config,'database':str(link/'index.sqlite')},encoder=self.encoder)
        for field in ('revision','tokenizer','chunking_version','query_instruction','document_instruction'):
            changed={**self.config,'space':{**self.config['space'],field:'different'}}
            with self.assertRaisesRegex(EventIndexError,'space_mismatch'):EventIndex(changed,encoder=self.encoder)
        for endpoint in ('https://external.invalid/v1/embeddings','http://localhost:123/v1/embeddings','http://127.0.0.1:0/v1/embeddings','http://user:pass@127.0.0.1:123/v1/embeddings'):
            with self.assertRaises(EventIndexError):EventIndex({**self.config,'space':{**self.config['space'],'endpoint':endpoint}},encoder=self.encoder)

    def test_foreign_database_is_not_modified_or_garbage_collected(self):
        foreign=self.root/'foreign.sqlite'
        with sqlite3.connect(foreign) as db:
            db.execute('CREATE TABLE precious(text TEXT)')
            db.execute("INSERT INTO precious VALUES ('synthetic keep')")
        foreign.chmod(0o600)
        with self.assertRaisesRegex(EventIndexError,'foreign_database'):
            EventIndex({**self.config,'database':str(foreign)},encoder=self.encoder)
        with sqlite3.connect(foreign) as db:
            self.assertEqual(db.execute('SELECT text FROM precious').fetchall(),[('synthetic keep',)])
            self.assertEqual(db.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall(),[('precious',)])

    def test_conflicting_source_is_rejected_before_encoding(self):
        build=self.begin()
        self.put(build)
        before=len(self.encoder.calls)
        self.error('source_conflict','upsert',build_id=build['build_id'],batch_id='conflict',source=self.source(content='changed'),source_chunk_count=1,chunks=[self.chunk('changed')])
        self.assertEqual(len(self.encoder.calls),before)

    def test_protocol_line_limits_duplicates_and_recovery(self):
        source=io.BytesIO(b'x'*(MAX_LINE+50)+b'\n'+b'{"schema":"x","schema":"y"}\n'+canonical(self.request('status'))+b'\n')
        output=io.BytesIO();serve(self.worker,source,output)
        responses=[json.loads(x) for x in output.getvalue().splitlines()]
        self.assertEqual(responses[0]['error']['code'],'input_limit')
        self.assertEqual(responses[1]['error']['code'],'invalid_request')
        self.assertTrue(responses[2]['ok'])

    def test_production_loopback_clients_and_no_auth_headers(self):
        received=[]
        class Handler(BaseHTTPRequestHandler):
            def do_POST(inner):
                raw=inner.rfile.read(int(inner.headers['Content-Length']))
                request=json.loads(raw);received.append((inner.path,dict(inner.headers),request))
                payload=canonical({'model':request['model'],'data':[{'index':i,'embedding':[1,2,3,4]} for i,_ in enumerate(request['input'])],'usage':{'prompt_tokens':1,'total_tokens':1}})
                inner.send_response(200);inner.send_header('Content-Type','application/json');inner.send_header('Content-Length',str(len(payload)));inner.end_headers();inner.wfile.write(payload)
            def log_message(*_):pass
        server=HTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            config={**self.config,'database':str(self.root/'real-local.sqlite'),'space':{**self.config['space'],'endpoint':f'http://127.0.0.1:{server.server_port}/v1/embeddings','query_instruction':'query prefix','document_instruction':'doc prefix'}}
            worker=EventIndex(config)
            self.finish(self.put(self.begin(worker=worker),worker=worker),worker=worker)
            self.ok('query',worker=worker,query='test question')
            self.assertEqual(len(received),2)
            self.assertEqual(received[0][2]['input'],['doc prefix\nsynthetic text'])
            self.assertEqual(received[1][2]['input'],['query prefix\ntest question'])
            for path,headers,request in received:
                self.assertEqual(path,'/v1/embeddings')
                self.assertNotIn('Authorization',headers)
        finally:
            server.shutdown();server.server_close();thread.join()

    def test_redirect_is_not_followed(self):
        received=[]
        class Handler(BaseHTTPRequestHandler):
            def do_POST(inner):
                received.append(inner.path);inner.rfile.read(int(inner.headers['Content-Length']))
                inner.send_response(302);inner.send_header('Location','https://external.invalid/v1/embeddings');inner.end_headers()
            def log_message(*_):pass
        server=HTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            cfg={**self.config,'database':str(self.root/'redirect.sqlite'),'space':{**self.config['space'],'endpoint':f'http://127.0.0.1:{server.server_port}/v1/embeddings'}}
            worker=EventIndex(cfg);build=self.begin(worker=worker)
            self.error('redirect_blocked','upsert',worker=worker,build_id=build['build_id'],batch_id='redirect',source=self.source(),source_chunk_count=1,chunks=[self.chunk()])
            self.assertEqual(received,['/v1/embeddings'])
        finally:
            server.shutdown();server.server_close();thread.join()

    def test_cli_status_only_and_no_fake_option(self):
        cfg=self.root/'config.json';cfg.write_bytes(canonical(self.config));cfg.chmod(0o600)
        command=[sys.executable,'-m','recallcard_worker.event_index','--config',str(cfg)]
        env={**os.environ,'PYTHONPATH':str(Path(__file__).resolve().parents[1]),'PYTHONDONTWRITEBYTECODE':'1'}
        result=subprocess.run(command,input=canonical(self.request('status'))+b'\n',capture_output=True,env=env,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(json.loads(result.stdout)['ok'])
        rejected=subprocess.run(command+['--fake'],capture_output=True,env=env,timeout=10)
        self.assertNotEqual(rejected.returncode,0)

    def test_unsupported_python_returns_protocol_error_without_reading_config(self):
        incoming = io.BytesIO(canonical(self.request("status")) + b"\n")
        outgoing = io.BytesIO()
        forbidden = str(self.root / "synthetic-do-not-read-config")
        with patch.object(sys, "version_info", (3, 10, 99)), \
             patch.object(sys, "stdin", SimpleNamespace(buffer=incoming)), \
             patch.object(sys, "stdout", SimpleNamespace(buffer=outgoing)), \
             patch("recallcard_worker.event_index.read_config", side_effect=AssertionError("must not read config")):
            self.assertEqual(main(["--config", forbidden]), 0)
            with self.assertRaisesRegex(EventIndexError, "python_version_unsupported"):
                EventIndex({**self.config, "database": str(self.root / "unsupported.sqlite")})
        result = json.loads(outgoing.getvalue())
        self.assertEqual(result, {"schema": SCHEMA, "id": self.ident, "ok": False,
                                  "error": {"code": "python_version_unsupported"}})
        self.assertNotIn(forbidden.encode(), outgoing.getvalue())
        self.assertFalse((self.root / "unsupported.sqlite").exists())


if __name__ == '__main__':
    unittest.main()
