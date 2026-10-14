"""云端宿主协议只使用内存中的合成向量与假transport，不连接真实提供器。"""
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_worker import event_lexical as lexical
from recallcard_worker import host_encoding as host
from recallcard_worker import source_cloud_once as once
from recallcard_worker.embedding import TransportResponse, canonical
from recallcard_worker.event_index import CONFIG_SCHEMA, SCHEMA, EventIndex, EventIndexError, digest, serve, decode_blob


def settings(path):
    return {"schema": CONFIG_SCHEMA, "database": str(path), "scope": "synthetic",
            "events_approved": True, "attachments_approved": True, "query_approved": True,
            "host_approval_identity": "a" * 64,
            "space": {"provider": host.CLOUD_PROVIDER, "endpoint": "https://synthetic.invalid/v1/embeddings",
            "model": "synthetic-model", "revision": "fixture-r1", "dimensions": 4,
            "tokenizer": "fixture-t1", "preprocessing_version": "utf8-v1", "chunking_version": "fixture-c1",
            "document_instruction": " doc prefix ", "query_instruction": " query prefix ", "normalization": "l2"}}


def request(ident, op, **values):
    return {"schema": SCHEMA, "id": ident, "op": op, **values}


def response(ident=2, vectors=None, **changes):
    return {"schema": host.SCHEMA, "id": ident, "ok": True,
            "vectors": vectors if vectors is not None else [[1, 2, 3, 4]], **changes}


def source_request(config, ident=3, kind="event", inputs=None):
    return {"schema": once.SCHEMA, "id": ident, "scope": config["scope"], "kind": kind,
            "space": config["space"], "space_signature": digest(canonical(config["space"])),
            "inputs": inputs if inputs is not None else [" doc prefix \n synthetic text "]}


class HostEncoderTests(unittest.TestCase):
    def make(self, reply):
        incoming = io.BytesIO(reply if isinstance(reply, bytes) else canonical(reply) + b"\n")
        outgoing = io.BytesIO()
        encoder = host.HostEncoder(incoming, outgoing, dimensions=4, space_signature="a" * 64)
        encoder.begin_job(2)
        return encoder, outgoing

    def test_one_callback_and_no_preprocessing(self):
        encoder, outgoing = self.make(response())
        self.assertEqual(encoder.encode("attachment", [" prefix \n original "]), [[1, 2, 3, 4]])
        self.assertEqual(json.loads(outgoing.getvalue()), {
            "schema": host.SCHEMA, "id": 2, "kind": "attachment",
            "space_signature": "a" * 64, "inputs": [" prefix \n original "]})
        with self.assertRaisesRegex(host.HostEncodingError, "host_encoding_budget"):
            encoder.encode("query", ["second"])
        self.assertEqual(len(outgoing.getvalue().splitlines()), 1)

    def test_bad_responses_poison_channel_without_echo(self):
        replies = [
            response(ident=3), response(ident=True), response(schema="unknown"),
            response(ok=1), response(vectors=[]), response(vectors=[[1, 2]]),
            response(vectors=[[True, 2, 3, 4]]), response(vectors=[[0, 0, 0, 0]]),
            response(vectors=[[10**400, 2, 3, 4]]), response(extra="synthetic-private"),
            b'{"schema":"recallcard.host-encode/1","id":2,"id":2,"ok":true,"vectors":[[1,2,3,4]]}\n',
            b'{"schema":"recallcard.host-encode/1","id":2,"ok":true,"vectors":[[NaN,2,3,4]]}\n',
            b"x" * (host.MAX_FRAME + 1), b"", canonical(response()),
            {"schema": host.SCHEMA, "id": 2, "ok": False, "error": {"code": "synthetic-private"}},
        ]
        for reply in replies:
            with self.subTest(reply_type=type(reply).__name__):
                encoder, _ = self.make(reply)
                with self.assertRaises(host.HostEncodingError) as caught:
                    encoder.encode("event", ["synthetic"])
                self.assertIn(caught.exception.code, ("host_protocol_error", "invalid_vector"))
                self.assertTrue(encoder.failed)
                self.assertNotIn("synthetic-private", str(caught.exception))

    def test_large_valid_vector_response_has_independent_256k_limit(self):
        values = [0.12345678901234567] * 8192
        reply = canonical(response(vectors=[values])) + b"\n"
        self.assertGreater(len(reply), host.MAX_FRAME)
        self.assertLess(len(reply), host.MAX_RESPONSE_FRAME)
        encoder = host.HostEncoder(io.BytesIO(reply), io.BytesIO(), dimensions=8192, space_signature="a" * 64)
        encoder.begin_job(2)
        self.assertEqual(encoder.encode("query", ["synthetic"]), [values])
        encoder = host.HostEncoder(io.BytesIO(b" " * host.MAX_RESPONSE_FRAME + b"\n"),
                                   io.BytesIO(), dimensions=4, space_signature="a" * 64)
        encoder.begin_job(2)
        with self.assertRaisesRegex(host.HostEncodingError, "host_protocol_error"):
            encoder.encode("query", ["synthetic"])

    def test_fixed_host_failure_is_not_provider_text(self):
        encoder, _ = self.make({"schema": host.SCHEMA, "id": 2, "ok": False,
                                "error": {"code": "configuration_changed"}})
        with self.assertRaisesRegex(host.HostEncodingError, "configuration_changed"):
            encoder.encode("query", ["synthetic"])
        self.assertFalse(encoder.failed)

    def test_limits_before_callback(self):
        for inputs, kind in [(["x"] * 5, "event"), (["x"] * 2, "query"),
                             (["x" * 8193], "event"), ([""], "event"), (["x"], "unknown")]:
            encoder, outgoing = self.make(response())
            with self.assertRaises(host.HostEncodingError):
                encoder.encode(kind, inputs)
            self.assertEqual(outgoing.getvalue(), b"")
        encoder = host.HostEncoder(io.BytesIO(), io.BytesIO(), dimensions=8192, space_signature="a" * 64)
        encoder.begin_job(2)
        with self.assertRaisesRegex(host.HostEncodingError, "input_limit"):
            encoder.encode("event", ["x", "y"])


class CloudEventIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = settings(self.root / "index.sqlite")
        self.worker = EventIndex(self.config, encoder=lambda kind, texts: [[1, 2, 3, 4] for _ in texts])

    def tearDown(self):
        self.temp.cleanup()

    def put(self, worker=None, kind="event", with_lexical=False):
        worker = worker or self.worker
        begin = worker.execute(request(1, "begin", source_epoch=digest(b"synthetic snapshot"),
                                       **({"lexical_version": lexical.VERSION} if with_lexical else {})))
        self.assertTrue(begin["ok"], begin)
        text = "synthetic source"
        ref = "event:synthetic" if kind == "event" else "attachment:evt_" + "a" * 64 + "/" + digest(text.encode())
        put = request(2, "upsert", build_id=begin["result"]["build_id"], batch_id="synthetic-batch",
            source={"ref": ref, "content_hash": digest(text.encode()), "scope": "synthetic",
                    "role": "user" if kind == "event" else None, "session_ref": None,
                    "occurred_at": None, "text_bytes": len(text)}, source_chunk_count=1,
            chunks=[{"ordinal": 0, "start_byte": 0, "end_byte": len(text), "text_sha256": digest(text.encode()), "text": text}])
        if with_lexical:
            put["chunks"][0].update(lexical_before="", lexical_after="")
        return put

    def publish(self, worker=None, with_lexical=False):
        worker = worker or self.worker
        result = worker.execute(self.put(worker, with_lexical=with_lexical))
        self.assertTrue(result["ok"], result)
        finish = worker.execute(request(3, "finish", **{key: result["result"][key] for key in
            ("build_id", "source_epoch", "source_count", "chunk_count", "receipt")}))
        self.assertTrue(finish["ok"], finish)

    def test_only_explicit_cloud_provider_and_bound_approval(self):
        for identity in (None, "x", 12, "G" * 64):
            config = {**self.config, "host_approval_identity": identity}
            with self.assertRaisesRegex(EventIndexError, "invalid_config"):
                EventIndex(config)
        no_identity = dict(self.config)
        del no_identity["host_approval_identity"]
        with self.assertRaisesRegex(EventIndexError, "invalid_config"):
            EventIndex(no_identity)
        local = {**self.config, "space": {**self.config["space"], "provider": "local-openai-compatible"}}
        with self.assertRaisesRegex(EventIndexError, "invalid_local_config"):
            EventIndex(local)
        for endpoint in ("http://127.0.0.1:1234/v1/embeddings", "https://u:p@synthetic.invalid/e", "https://synthetic.invalid/e?secret=x"):
            with self.assertRaises(EventIndexError):
                EventIndex({**self.config, "space": {**self.config["space"], "endpoint": endpoint}})

    def test_no_host_encoder_cannot_fall_back_to_network(self):
        self.worker.encoder = None
        with patch("recallcard_worker.event_index.LocalQueryClient", side_effect=AssertionError), \
             patch("recallcard_worker.event_index.LocalIndexClient", side_effect=AssertionError), \
             patch("recallcard_worker.embedding.os.environ", {}):
            result = self.worker.execute(self.put())
        self.assertEqual(result["error"]["code"], "host_encoder_required")

    def test_serve_query_callback_cache_and_truthful_coverage(self):
        self.publish()
        self.worker.encoder = None
        incoming = io.BytesIO(b"".join(canonical(item) + b"\n" for item in [
            request(2, "query", query=" original "), response(), request(3, "query", query=" original ")]))
        outgoing = io.BytesIO()
        with patch("recallcard_worker.embedding.os.environ", {}), \
             patch("recallcard_worker.event_index.LocalQueryClient", side_effect=AssertionError):
            serve(self.worker, incoming, outgoing)
        callback, first, second = map(json.loads, outgoing.getvalue().splitlines())
        self.assertEqual(callback["inputs"], [" query prefix \n original "])
        self.assertEqual(callback["kind"], "query")
        self.assertEqual(callback["space_signature"], self.worker.signature)
        self.assertTrue(first["ok"], first)
        self.assertTrue(second["ok"], second)
        self.assertEqual(first["result"]["coverage"]["query_encoding"], "cloud")
        self.assertEqual(second["result"]["coverage"]["query_encoding"], "cache")
        self.assertEqual(first["result"]["coverage"]["external_calls"], 1)
        self.assertEqual(second["result"]["coverage"]["external_calls"], 0)
        self.assertTrue(first["result"]["coverage"]["host_managed"])
        self.assertFalse(first["result"]["coverage"]["local_only"])

    def test_serve_event_and_attachment_kind_and_preparation(self):
        for kind in ("event", "attachment"):
            with self.subTest(kind=kind):
                config = {**self.config, "database": str(self.root / f"{kind}.sqlite")}
                worker = EventIndex(config)
                put = self.put(worker, kind)
                incoming = io.BytesIO(canonical(put) + b"\n" + canonical(response()) + b"\n")
                outgoing = io.BytesIO()
                serve(worker, incoming, outgoing)
                callback, result = map(json.loads, outgoing.getvalue().splitlines())
                self.assertEqual(callback["kind"], kind)
                self.assertEqual(callback["inputs"], [" doc prefix \nsynthetic source"])
                self.assertTrue(result["ok"], result)

    def test_malformed_callback_stops_stream_before_next_job(self):
        self.publish()
        self.worker.encoder = None
        incoming = io.BytesIO(b"".join(canonical(item) + b"\n" for item in [
            request(2, "query", query="synthetic"), response(ident=3), request(9, "status")]))
        outgoing = io.BytesIO()
        serve(self.worker, incoming, outgoing)
        lines = list(map(json.loads, outgoing.getvalue().splitlines()))
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[1]["error"]["code"], "host_protocol_error")
        self.assertEqual(json.loads(incoming.readline())["id"], 9)

    def test_duplicate_callback_response_is_rejected_and_stream_stops(self):
        self.publish()
        self.worker.encoder = None
        incoming = io.BytesIO(b"".join(canonical(item) + b"\n" for item in [
            request(2, "query", query="synthetic"), response(), response(), request(9, "status")]))
        outgoing = io.BytesIO()
        serve(self.worker, incoming, outgoing)
        lines = list(map(json.loads, outgoing.getvalue().splitlines()))
        self.assertEqual(len(lines), 3)
        self.assertTrue(lines[1]["ok"])
        self.assertEqual(lines[2]["error"]["code"], "host_protocol_error")
        self.assertEqual(json.loads(incoming.readline())["id"], 9)

    def test_request_cannot_change_provider_or_endpoint(self):
        for key in ("endpoint", "provider", "space", "host_approval_identity"):
            result = self.worker.execute(request(3, "status", **{key: "synthetic"}))
            self.assertEqual(result["error"]["code"], "invalid_request")

    def test_revocation_during_encoding_does_not_write_query_cache(self):
        path = self.root / "config.json"
        path.write_bytes(canonical(self.config)); path.chmod(0o600)
        worker = EventIndex(self.config, encoder=self.worker.encoder, config_path=path)
        self.publish(worker)
        def revoke(kind, texts):
            path.write_bytes(canonical({**self.config, "query_approved": False}))
            return [[1, 2, 3, 4]]
        worker.encoder = revoke
        result = worker.execute(request(4, "query", query="synthetic private question"))
        self.assertEqual(result["error"]["code"], "configuration_changed")
        with sqlite3.connect(worker.path) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM queries").fetchone()[0], 0)

    def test_revocation_after_cache_read_cannot_touch_cache_clock(self):
        path = self.root / "config.json"
        path.write_bytes(canonical(self.config)); path.chmod(0o600)
        worker = EventIndex(self.config, encoder=self.worker.encoder, config_path=path)
        self.publish(worker)
        first = worker.execute(request(4, "query", query="synthetic cached question"))
        self.assertTrue(first["ok"], first)
        with sqlite3.connect(worker.path) as db:
            old_clock = db.execute("SELECT value FROM metadata WHERE key='query_clock'").fetchone()[0]
            old_cache = db.execute("SELECT input_hash,vector,used FROM queries").fetchall()
        def revoke(blob, dimensions):
            path.write_bytes(canonical({**self.config, "host_approval_identity": "c" * 64}))
            return decode_blob(blob, dimensions)
        with patch("recallcard_worker.event_index.decode_blob", side_effect=revoke):
            result = worker.execute(request(5, "query", query="synthetic cached question"))
        self.assertEqual(result["error"]["code"], "configuration_changed")
        with sqlite3.connect(worker.path) as db:
            self.assertEqual(db.execute("SELECT value FROM metadata WHERE key='query_clock'").fetchone()[0], old_clock)
            self.assertEqual(db.execute("SELECT input_hash,vector,used FROM queries").fetchall(), old_cache)

    def test_all_cloud_failures_remain_fail_closed(self):
        self.publish(with_lexical=True)
        for error_code in ("network_error", "provider_error", "rate_limited", "provider_auth", "missing_key",
                           "network_budget_exhausted", "redirect_blocked", "model_mismatch", "invalid_vector"):
            def fail(kind, texts):
                raise host.HostEncodingError(error_code)
            self.worker.encoder = fail
            result = self.worker.execute(request(4, "query", query="synthetic failed question"))
            self.assertFalse(result["ok"], result)
            self.assertEqual(result["error"]["code"], error_code)
        with sqlite3.connect(self.worker.path) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM queries").fetchone()[0], 0)

    def test_write_transaction_rechecks_config_before_commit(self):
        path = self.root / "config.json"
        path.write_bytes(canonical(self.config)); path.chmod(0o600)
        worker = EventIndex(self.config, encoder=self.worker.encoder, config_path=path)
        with self.assertRaisesRegex(EventIndexError, "configuration_changed"):
            with worker.connection(write=True) as db:
                db.execute("UPDATE metadata SET value='42' WHERE key='query_clock'")
                path.write_bytes(canonical({**self.config, "host_approval_identity": "b" * 64}))
        with sqlite3.connect(worker.path) as db:
            self.assertEqual(db.execute("SELECT value FROM metadata WHERE key='query_clock'").fetchone()[0], "0")


class SourceCloudOnceTests(unittest.TestCase):
    def setUp(self):
        self.config = settings(Path("/unused/synthetic.sqlite"))
        self.calls = []
        self.environ = {once.KEY_ENV: "synthetic-key-memory-only"}

    def transport(self, endpoint, payload, key, timeout, response_limit):
        self.calls.append((endpoint, payload, key, timeout, response_limit))
        content = json.loads(payload)
        return TransportResponse(200, canonical({"model": content["model"], "data": [
            {"index": i, "embedding": [1, 2, 3, 4]} for i, _ in enumerate(content["input"])]}), {})

    def run_request(self, payload, transport=None, **changes):
        incoming = io.BytesIO(payload if isinstance(payload, bytes) else canonical(payload) + b"\n")
        outgoing = io.BytesIO()
        args = {"endpoint": self.config["space"]["endpoint"], "scope": "synthetic", "kind": "event",
                "key_env": once.KEY_ENV, "transport": transport or self.transport, **changes}
        with patch("recallcard_worker.embedding.os.environ", self.environ):
            code = once.run_once(incoming, outgoing, **args)
        return code, json.loads(outgoing.getvalue()), outgoing.getvalue()

    def test_single_call_exact_http_body_without_second_prefix(self):
        inputs = [' doc prefix \n 原文 " \\ \t \b \f \r \n / \u2028\u2029 ']
        payload = source_request(self.config, inputs=inputs)
        code, result, _ = self.run_request(payload)
        self.assertEqual(code, 0)
        self.assertTrue(result["ok"])
        self.assertEqual(len(self.calls), 1)
        endpoint, raw, key, timeout, response_limit = self.calls[0]
        self.assertEqual(raw, canonical({"dimensions": 4, "encoding_format": "float", "input": inputs, "model": "synthetic-model"}))
        self.assertEqual(json.loads(raw)["input"], inputs)
        self.assertEqual(endpoint, self.config["space"]["endpoint"])
        self.assertEqual(key, "synthetic-key-memory-only")
        self.assertEqual(timeout, 5.0)
        self.assertEqual(response_limit, 65536 + 4 * 40)

    def test_query_event_attachment_use_distinct_approvals(self):
        for kind in ("query", "event", "attachment"):
            code, result, _ = self.run_request(source_request(self.config, kind=kind), kind=kind)
            self.assertEqual(code, 0, result)
        self.assertEqual(len(self.calls), 3)

    def test_extra_requests_or_trailing_data_rejected_before_network(self):
        valid = canonical(source_request(self.config)) + b"\n"
        for raw in (valid + valid, valid + b" ", valid + b"\n", valid[:-1]):
            code, result, _ = self.run_request(raw)
            self.assertEqual(code, 2)
            self.assertFalse(result["ok"])
        self.assertEqual(self.calls, [])

    def test_unknown_duplicate_nonfinite_and_boundaries_fail_closed(self):
        original = source_request(self.config)
        cases = [dict(original, schema="unknown"), dict(original, id=True), dict(original, scope="other"),
                 dict(original, kind="attachment"), dict(original, space_signature="b" * 64),
                 dict(original, inputs=["x"] * 5), dict(original, inputs=["x" * 8193]),
                 dict(original, unknown="synthetic-private"),
                 b'{"schema":"recallcard.source-cloud-once/1","id":1,"id":2}\n',
                 b'{"secret":NaN}\n', b"x" * (host.MAX_FRAME + 1)]
        for item in cases:
            code, result, raw = self.run_request(item)
            self.assertEqual(code, 2)
            self.assertFalse(result["ok"])
            self.assertNotIn(b"synthetic-private", raw)
            self.assertNotIn(b"synthetic-key-memory-only", raw)
        self.assertEqual(self.calls, [])

    def test_scope_endpoint_model_and_key_env_are_bound(self):
        original = source_request(self.config)
        for kwargs in ({"scope": "other"}, {"endpoint": "https://other.invalid/e"},
                       {"endpoint": "http://127.0.0.1:1234/v1/embeddings"}, {"key_env": "HOME"}):
            code, _, _ = self.run_request(original, **kwargs)
            self.assertEqual(code, 2)
        self.assertEqual(self.calls, [])
        def wrong_model(*args):
            self.calls.append(args)
            return TransportResponse(200, canonical({"model": "wrong-model", "data": [{"index": 0, "embedding": [1, 2, 3, 4]}]}), {})
        code, result, _ = self.run_request(original, transport=wrong_model)
        self.assertEqual(code, 2)
        self.assertEqual(result["error"]["code"], "model_mismatch")
        self.assertEqual(len(self.calls), 1)

    def test_no_retry_redirect_or_provider_error_body_echo(self):
        for status in (302, 401, 429, 500):
            calls = []
            def failing(*args):
                calls.append(args)
                return TransportResponse(status, b"synthetic-private-provider-body", {"Location": "https://other.invalid/e"})
            code, result, raw = self.run_request(source_request(self.config), transport=failing)
            self.assertEqual(code, 2)
            self.assertEqual(len(calls), 1)
            self.assertNotIn(b"synthetic-private", raw)
            self.assertIn(result["error"]["code"], ("redirect_blocked", "provider_auth", "rate_limited", "provider_error"))

    def test_large_provider_response_and_serialized_body_are_bounded(self):
        def large(*args):
            return TransportResponse(200, b"x" * (host.MAX_RESPONSE_FRAME + 1), {})
        code, result, _ = self.run_request(source_request(self.config), transport=large)
        self.assertEqual(code, 2)
        self.assertEqual(result["error"]["code"], "response_limit")
        # 控制字符JSON转义后体积膨胀，不能只检查输入UTF8字节数。
        code, result, _ = self.run_request(source_request(self.config, inputs=["x" + "\x01" * 8000] * 4))
        self.assertEqual(code, 2)
        self.assertEqual(self.calls, [])

    def test_large_valid_once_response_is_bounded_independently(self):
        self.config["space"]["dimensions"] = 8192
        calls = []
        def large_vector(endpoint, payload, key, timeout, limit):
            calls.append(limit)
            raw = canonical({"model": "synthetic-model", "data": [
                {"index": 0, "embedding": [0.12345678901234567] * 8192}]})
            self.assertGreater(len(raw), host.MAX_FRAME)
            self.assertLess(len(raw), host.MAX_RESPONSE_FRAME)
            return TransportResponse(200, raw, {})
        code, result, raw = self.run_request(source_request(self.config), transport=large_vector)
        self.assertEqual(code, 0, result)
        self.assertEqual(len(result["vectors"][0]), 8192)
        self.assertGreater(len(raw), host.MAX_FRAME)
        self.assertLessEqual(len(raw), host.MAX_RESPONSE_FRAME)
        self.assertEqual(calls, [host.MAX_RESPONSE_FRAME])

    def test_missing_key_uses_only_injected_environment(self):
        self.environ.clear()
        code, result, _ = self.run_request(source_request(self.config))
        self.assertEqual(code, 2)
        self.assertEqual(result["error"]["code"], "missing_key")
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
