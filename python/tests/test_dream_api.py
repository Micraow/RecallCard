"""全部来源与密钥均为合成 fixture；除注入的 fake transport 外禁止网络。"""
from copy import deepcopy
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import traceback
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_dream import (
    DreamClient, DreamError, NetworkApproval, TransportResponse,
    PROMPT_TEMPLATE_HASH, STABLE_PREFIX_BYTES, load_json, validate_job, validate_result,
)
from recallcard_dream.client import canonical, https_transport, OUTPUT_SCHEMA, MAX_JOB_BYTES
from recallcard_dream.__main__ import atomic_result_file, main

# 由 Rust CLI 用纯合成 Event/Memory 导出；摘要由 Rust 产生，不是 Python 猜测的序列化结果。
RUST_JOB = json.loads(r"""{
  "allowed_scope": "project:contract",
  "input_hash": "e1d58d93787aa045802bcc11949cb7f2f200f5c2332dddd32ce270c80d0ad19e",
  "job_id": "dream_e1d58d93787aa045802bcc11949cb7f2f200f5c2332dddd32ce270c80d0ad19e",
  "memory_read_set": [
    {
      "content_hash": "9ecec724784cd01d79eb1a80c3942de865078fdb1fb3816e228216a4e88fc181",
      "memory": {
        "authority": "user",
        "content": "合成接口测试：文档采用中文。",
        "entities": [],
        "evidence": "user_explicit",
        "id": "mem_39f10bb202544a259cf51a9c8156c6e3",
        "labels": [],
        "model_score": 0.0,
        "observed_at": null,
        "protected": true,
        "recorded_at": "2026-10-06T16:14:54.535724354Z",
        "revision": 1,
        "schema_version": 1,
        "scope": "project:contract",
        "source_refs": [
          "evt_fbd98ee1906e6c6a7bdee225550223f570dea447fd5731283afc32b3eb6ab066"
        ],
        "status": "active",
        "supersedes": [],
        "time_note": "",
        "updated_at": "2026-10-06T16:14:54.535724354Z",
        "valid_from": null,
        "valid_to": null
      },
      "ref": "memory:mem_39f10bb202544a259cf51a9c8156c6e3@1"
    }
  ],
  "operation": "extract",
  "output_schema": "recallcard.dream-result/1",
  "projection_version": "bounded-full-v1",
  "prompt_version": "manual-extract-v1",
  "schema": "recallcard.dream-job/1",
  "source_refs": [
    {
      "content_hash": "215b8c41b5e168f779a9061112aad954f3b9b57b1dd62aa85a936e6ecd1d9af4",
      "event": {
        "capture": {
          "completeness": null,
          "reason": null,
          "redacted": false,
          "redaction_count": 0
        },
        "captured_at": "2026-10-06T16:14:54.531934716Z",
        "caused_by": null,
        "content": "合成接口测试：文档采用中文，核心采用 Rust，可选 worker 采用 Python。",
        "id": "evt_fbd98ee1906e6c6a7bdee225550223f570dea447fd5731283afc32b3eb6ab066",
        "kind": "message",
        "metadata": null,
        "occurred_at": "2026-10-06T10:00:00Z",
        "origin": "user_input",
        "parts": [],
        "reply_to": null,
        "revision_of": null,
        "role": "user",
        "run_id": null,
        "schema_version": 1,
        "scope": "project:contract",
        "session_id": null,
        "source": {
          "account_namespace": "",
          "conversation_id": "synthetic-session",
          "message_id": "synthetic-message",
          "platform": "manual-test"
        },
        "step_id": null,
        "turn_id": null
      },
      "ref": "event:evt_fbd98ee1906e6c6a7bdee225550223f570dea447fd5731283afc32b3eb6ab066"
    }
  ]
}
""")
ENDPOINT = "https://dream.invalid/v1/chat/completions"
MODEL = "synthetic-model"
KEY = "synthetic-in-memory-key"


def fixture():
    return deepcopy(RUST_JOB)


def result_for(job=None, operation="add"):
    job = job or fixture()
    proposal = {"operation": operation, "scope": job["allowed_scope"]}
    if operation in {"add", "update", "supersede"}:
        proposal.update(content="合成结论：项目使用 Rust 与可选 Python。",
                        source_refs=[job["source_refs"][0]["ref"]], evidence="user_explicit")
    if operation in {"update", "supersede"}:
        proposal.update(target_ref=job["memory_read_set"][0]["ref"],
                        expected_revision=job["memory_read_set"][0]["memory"]["revision"])
    return {"schema": "recallcard.dream-result/1", "job_id": job["job_id"], "input_hash": job["input_hash"],
            "proposals": [proposal]}


def envelope(result=None, usage=None):
    return {"choices": [{"index": 0, "finish_reason": "stop", "message": {
        "role": "assistant", "content": canonical(result or result_for()).decode("utf-8")}}], "usage": usage}


class FakeTransport:
    def __init__(self, body=None, status=200, error=None):
        self.body = canonical(body if body is not None else envelope())
        self.status = status
        self.error = error
        self.calls = []

    def __call__(self, endpoint, payload, key, timeout, limit):
        self.calls.append((endpoint, payload, key, timeout, limit))
        if self.error:
            raise self.error
        return TransportResponse(self.status, self.body)


def approved():
    return NetworkApproval(True, ENDPOINT, ("project:contract",), True)


class DreamTests(unittest.TestCase):
    def setUp(self):
        network = patch.object(socket, "create_connection", side_effect=AssertionError("测试不得联网"))
        network.start()
        key = patch.dict(os.environ, {"RECALLCARD_DREAM_API_KEY": KEY})
        key.start()
        self.addCleanup(network.stop)
        self.addCleanup(key.stop)

    def client(self, transport=None, **kwargs):
        return DreamClient(ENDPOINT, MODEL, approved(), transport=transport or FakeTransport(), **kwargs)

    def rejects(self, fn, code=None):
        with self.assertRaises(DreamError) as caught:
            fn()
        if code:
            self.assertEqual(caught.exception.code, code)
        self.assertNotIn(KEY, str(caught.exception))
        return caught.exception

    def test_real_rust_export_is_accepted_without_mutating_input(self):
        job = fixture()
        self.assertEqual(validate_job(job), job)
        self.assertIsNot(validate_job(job), job)
        self.assertEqual(job["job_id"], "dream_" + job["input_hash"])

    def test_result_add_and_all_operations(self):
        for op in ("add", "update", "supersede", "noop", "conflict"):
            with self.subTest(operation=op):
                self.assertEqual(validate_result(result_for(operation=op), fixture())["proposals"][0]["operation"], op)

    def test_success_sends_one_bounded_request(self):
        fake = FakeTransport()
        answer = self.client(fake, max_output_tokens=321).execute(fixture())
        self.assertEqual(answer["result"], result_for())
        self.assertEqual(len(fake.calls), 1)
        endpoint, payload, key, timeout, limit = fake.calls[0]
        self.assertEqual((endpoint, key, timeout), (ENDPOINT, KEY, 30))
        request = json.loads(payload)
        self.assertEqual(request["max_completion_tokens"], 321)
        self.assertFalse(request["stream"])
        self.assertEqual(request["n"], 1)
        self.assertEqual(json.loads(request["messages"][-1]["content"]), fixture())
        self.assertEqual(answer["diagnostics"]["network_calls"], 1)
        self.assertEqual(answer["diagnostics"]["retries"], 0)

    def test_two_jobs_have_byte_identical_fixed_prefix(self):
        a, b = fixture(), fixture()
        b["input_hash"] = "a" * 64
        b["job_id"] = "dream_" + b["input_hash"]
        b["source_refs"][0]["event"]["content"] = "另一份合成动态材料"
        client = self.client()
        _, one = client.build_request(a)
        _, two = client.build_request(b)
        for payload in (one, two):
            self.assertEqual(canonical(json.loads(payload)["messages"][:-1]), STABLE_PREFIX_BYTES)
        marker = b',{"content":"{'
        self.assertIn(marker, one)
        self.assertEqual(one.split(marker, 1)[0], two.split(marker, 1)[0])
        self.assertNotEqual(one, two)
        self.assertNotIn(a["job_id"].encode(), STABLE_PREFIX_BYTES)
        self.assertNotIn(a["source_refs"][0]["event"]["content"].encode(), STABLE_PREFIX_BYTES)
        self.assertEqual(len(PROMPT_TEMPLATE_HASH), 64)

    def test_default_network_disabled_even_with_key(self):
        fake = FakeTransport()
        client = DreamClient(ENDPOINT, MODEL, transport=fake)
        self.rejects(lambda: client.execute(fixture()), "approval_required")
        self.assertEqual(fake.calls, [])

    def test_each_approval_is_required(self):
        cases = [NetworkApproval(False, ENDPOINT, ("project:contract",), True),
                 NetworkApproval(True, None, ("project:contract",), True),
                 NetworkApproval(True, ENDPOINT + "?other", ("project:contract",), True),
                 NetworkApproval(True, ENDPOINT, (), True),
                 NetworkApproval(True, ENDPOINT, ("personal",), True),
                 NetworkApproval(True, ENDPOINT, ("project:contract",), False)]
        for approval in cases:
            fake = FakeTransport()
            self.rejects(lambda: DreamClient(ENDPOINT, MODEL, approval, transport=fake).execute(fixture()))
            self.assertEqual(fake.calls, [])

    def test_scope_validation_covers_last_source_before_any_network(self):
        job = fixture()
        bad = deepcopy(job["source_refs"][0])
        bad["event"]["id"] = "evt_" + "b" * 64
        bad["ref"] = "event:" + bad["event"]["id"]
        bad["event"]["scope"] = "private:outside"
        job["source_refs"].append(bad)
        fake = FakeTransport()
        self.rejects(lambda: self.client(fake).execute(job), "scope_denied")
        self.assertEqual(fake.calls, [])

    def test_scope_validation_covers_old_memory_before_any_network(self):
        job = fixture()
        job["memory_read_set"][0]["memory"]["scope"] = "private:outside"
        fake = FakeTransport()
        self.rejects(lambda: self.client(fake).execute(job), "scope_denied")
        self.assertEqual(fake.calls, [])

    def test_unknown_nested_fields_cannot_carry_approval(self):
        for path in ((), ("source_refs", 0), ("source_refs", 0, "event"), ("memory_read_set", 0, "memory")):
            job = fixture()
            value = job
            for key in path:
                value = value[key]
            value["allow_network"] = True
            self.rejects(lambda: self.client().execute(job), "invalid_fields")

    def test_missing_snapshot_scope_is_not_inferred(self):
        for field, record in (("source_refs", "event"), ("memory_read_set", "memory")):
            job = fixture()
            del job[field][0][record]["scope"]
            self.rejects(lambda: self.client().execute(job), "invalid_fields")

    def test_bad_job_schema_binding_or_revision(self):
        for field, value in (("schema", "wrong"), ("operation", "consolidate"), ("prompt_version", "other"),
                             ("projection_version", "other"), ("output_schema", "other"),
                             ("input_hash", "not-a-hash"), ("job_id", "dream_" + "0" * 64)):
            job = fixture()
            job[field] = value
            self.rejects(lambda: validate_job(job))
        job = fixture()
        job["memory_read_set"][0]["memory"]["revision"] += 1
        self.rejects(lambda: validate_job(job), "invalid_reference")

    def test_duplicate_sources_and_memories(self):
        for key in ("source_refs", "memory_read_set"):
            job = fixture()
            job[key].append(deepcopy(job[key][0]))
            self.rejects(lambda: validate_job(job), "invalid_reference")

    def test_job_source_count_and_input_budget(self):
        for count in (0, 65):
            job = fixture()
            job["source_refs"] = job["source_refs"] * count
            self.rejects(lambda: validate_job(job), "input_limit")
        job = fixture()
        job["memory_read_set"] *= 33
        self.rejects(lambda: validate_job(job), "input_limit")
        self.rejects(lambda: self.client(max_job_bytes=10).execute(fixture()), "input_limit")
        self.rejects(lambda: self.client(max_request_bytes=10).execute(fixture()), "request_limit")

    def test_missing_key_never_calls_transport(self):
        fake = FakeTransport()
        with patch.dict(os.environ, {}, clear=True):
            self.rejects(lambda: self.client(fake).execute(fixture()), "missing_key")
        self.assertEqual(fake.calls, [])

    def test_bad_key_is_not_included_in_error(self):
        fake = FakeTransport()
        with patch.dict(os.environ, {"RECALLCARD_DREAM_API_KEY": "synthetic\r\nsecret"}):
            self.rejects(lambda: self.client(fake).execute(fixture()), "missing_key")
        self.assertEqual(fake.calls, [])

    def test_endpoint_restrictions(self):
        for endpoint in ("http://dream.invalid/v1/chat", "https://u:p@dream.invalid/v1/chat", "https://dream.invalid",
                         ENDPOINT + "?key=synthetic", ENDPOINT + "#fragment", ENDPOINT + "/", "https://dream.invalid:0/v1/chat",
                         "https://dream.invalid\\other/v1/chat", "https://dream.invalid/line\n"):
            self.rejects(lambda: DreamClient(endpoint, MODEL))

    def test_invalid_budgets_and_timeout(self):
        for kwargs in ({"max_requests": 0}, {"max_requests": 101}, {"max_request_bytes": 0}, {"max_job_bytes": MAX_JOB_BYTES + 1},
                       {"max_output_tokens": True}, {"max_output_tokens": 0}, {"max_output_tokens": 131073},
                       {"timeout": 0}, {"timeout": 121}, {"timeout": float("nan")}, {"timeout": True}):
            self.rejects(lambda: self.client(**kwargs))

    def test_request_count_budget_survives_failures(self):
        fake = FakeTransport(status=503)
        client = self.client(fake)
        self.rejects(lambda: client.execute(fixture()), "provider_error")
        self.rejects(lambda: client.execute(fixture()), "network_budget_exhausted")
        self.assertEqual(len(fake.calls), 1)

    def test_total_request_budget(self):
        _, request = self.client().build_request(fixture())
        fake = FakeTransport()
        client = self.client(fake, max_requests=2, max_total_request_bytes=len(request))
        client.execute(fixture())
        self.rejects(lambda: client.execute(fixture()), "network_budget_exhausted")
        self.assertEqual(len(fake.calls), 1)

    def test_http_errors_and_redirects_never_retry_or_expose_body(self):
        for status in (301, 302, 307, 308, 400, 401, 429, 500, 503):
            fake = FakeTransport({"echo": KEY + " synthetic-private-prompt"}, status=status)
            error = self.rejects(lambda: self.client(fake).execute(fixture()))
            self.assertNotIn("synthetic-private", str(error))
            self.assertEqual(len(fake.calls), 1)

    def test_transport_exceptions_are_sanitized(self):
        for error in (ValueError(KEY), RuntimeError("synthetic-private"), DreamError("secret", KEY), TimeoutError(KEY)):
            fake = FakeTransport(error=error)
            caught = self.rejects(lambda: self.client(fake).execute(fixture()))
            self.assertNotIn("synthetic-private", str(caught))
            self.assertNotIn(KEY, "".join(traceback.format_exception(caught)))
            self.assertNotIn("synthetic-private", "".join(traceback.format_exception(caught)))
            self.assertEqual(len(fake.calls), 1)

    def test_wrong_job_hash_or_schema_rejected_without_repair(self):
        for field, value in (("schema", "wrong"), ("job_id", "dream_" + "b" * 64), ("input_hash", "a" * 64)):
            result = result_for()
            result[field] = value
            fake = FakeTransport(envelope(result))
            self.rejects(lambda: self.client(fake).execute(fixture()))
            self.assertEqual(len(fake.calls), 1)

    def test_truncated_and_multiple_choices_rejected(self):
        for reason in ("length", "content_filter", "tool_calls", None):
            body = envelope()
            body["choices"][0]["finish_reason"] = reason
            self.rejects(lambda: self.client(FakeTransport(body)).execute(fixture()), "incomplete_response")
        for count in (0, 2):
            body = envelope()
            body["choices"] *= count
            self.rejects(lambda: self.client(FakeTransport(body)).execute(fixture()), "invalid_response")

    def test_refusal_tools_and_nontext_response_rejected(self):
        for key, value in (("refusal", "无法处理"), ("tool_calls", [{"name": "write"}]), ("function_call", {"name": "write"}),
                           ("role", "tool"), ("content", [])):
            body = envelope()
            body["choices"][0]["message"][key] = value
            self.rejects(lambda: self.client(FakeTransport(body)).execute(fixture()))

    def test_strict_json_no_fences_trailing_values_duplicates_or_nan(self):
        text = canonical(result_for()).decode()
        for content in ("```json\n" + text + "\n```", text + text, text[:-1],
                        text.replace('"schema":', '"schema":"duplicate","schema":'),
                        text.replace('"proposals":', '"score":NaN,"proposals":')):
            body = envelope()
            body["choices"][0]["message"]["content"] = content
            self.rejects(lambda: self.client(FakeTransport(body)).execute(fixture()))

    def test_response_and_result_size_bounds(self):
        self.rejects(lambda: self.client(max_response_bytes=16).execute(fixture()), "response_limit")
        self.rejects(lambda: self.client(max_result_bytes=16).execute(fixture()))
        fake = FakeTransport()
        fake.body = b"{" * (2 * 1024 * 1024 + 1)
        self.rejects(lambda: self.client(fake).execute(fixture()), "response_limit")

    def test_result_unknown_fields_and_empty_proposals(self):
        for key in ("authority", "protected", "approve", "commands"):
            result = result_for()
            result["proposals"][0][key] = True
            self.rejects(lambda: validate_result(result, fixture()), "invalid_fields")
        for count in (0, 33):
            result = result_for()
            result["proposals"] *= count
            self.rejects(lambda: validate_result(result, fixture()), "input_limit")

    def test_proposal_cannot_expand_scope_source_or_memory(self):
        cases = [("scope", "personal", "add"), ("source_refs", ["event:evt_" + "0" * 64], "add"),
                 ("target_ref", "memory:mem_" + "0" * 32 + "@1", "update"), ("expected_revision", 2, "update")]
        for key, value, operation in cases:
            result = result_for(operation=operation)
            result["proposals"][0][key] = value
            self.rejects(lambda: validate_result(result, fixture()))

    def test_duplicate_target_and_duplicate_aliased_source_rejected(self):
        result = result_for(operation="update")
        result["proposals"] *= 2
        self.rejects(lambda: validate_result(result, fixture()), "invalid_reference")
        result = result_for()
        result["proposals"][0]["source_refs"].append(fixture()["source_refs"][0]["event"]["id"])
        self.rejects(lambda: validate_result(result, fixture()), "invalid_reference")

    def test_noop_cannot_modify_and_add_requires_sources(self):
        result = result_for(operation="noop")
        result["proposals"][0]["target_ref"] = fixture()["memory_read_set"][0]["ref"]
        self.rejects(lambda: validate_result(result, fixture()))
        result = result_for()
        del result["proposals"][0]["source_refs"]
        self.rejects(lambda: validate_result(result, fixture()))

    def test_invalid_time_and_score(self):
        for key, value in (("valid_from", "上个月"), ("valid_to", "2026-13-01T00:00:00Z"),
                           ("observed_at", "2026-01-01"), ("model_score", float("nan")), ("model_score", True)):
            result = result_for()
            result["proposals"][0][key] = value
            self.rejects(lambda: validate_result(result, fixture()))
        result = result_for()
        result["proposals"][0].update(valid_from="2026-01-02T00:00:00Z", valid_to="2026-01-01T00:00:00Z")
        self.rejects(lambda: validate_result(result, fixture()), "invalid_time")

    def test_usage_and_cache_are_preserved_only_when_supplied(self):
        usage = {"prompt_tokens": 123, "completion_tokens": 45, "total_tokens": 168,
                 "prompt_tokens_details": {"cached_tokens": 100, "audio_tokens": 0},
                 "completion_tokens_details": {"reasoning_tokens": 5}, "cache_creation_input_tokens": 20}
        diagnostics = self.client(FakeTransport(envelope(usage=usage))).execute(fixture())["diagnostics"]
        self.assertEqual(diagnostics["usage"], usage)
        self.assertEqual(diagnostics["input_tokens"], 123)
        self.assertEqual(diagnostics["output_tokens"], 45)
        self.assertEqual(diagnostics["cache_read_tokens"], 100)
        self.assertEqual(diagnostics["cache_write_tokens"], 20)
        self.assertIsNone(diagnostics["estimated_cost"])
        self.assertEqual(diagnostics["cache_control"], "unknown")

    def test_missing_usage_and_missing_cache_are_null_not_zero(self):
        for usage in (None, {}, {"prompt_tokens": 12, "completion_tokens": 3}):
            diagnostics = self.client(FakeTransport(envelope(usage=usage))).execute(fixture())["diagnostics"]
            self.assertEqual(diagnostics["usage"], usage)
            self.assertIsNone(diagnostics["cache_read_tokens"])
            self.assertIsNone(diagnostics["cache_write_tokens"])
            if not usage:
                self.assertIsNone(diagnostics["input_tokens"])
                self.assertIsNone(diagnostics["output_tokens"])

    def test_compatible_provider_cache_fields(self):
        usage = {"cache_read_input_tokens": 9, "cache_creation_input_tokens": 2}
        diagnostics = self.client(FakeTransport(envelope(usage=usage))).execute(fixture())["diagnostics"]
        self.assertEqual(diagnostics["cache_read_tokens"], 9)
        self.assertEqual(diagnostics["cache_write_tokens"], 2)

    def test_invalid_usage_does_not_echo_private_content(self):
        for usage in ({"prompt_tokens": -1}, {"echo": KEY}, {"prompt_tokens": True}, "synthetic-private"):
            self.rejects(lambda: self.client(FakeTransport(envelope(usage=usage))).execute(fixture()), "invalid_response")

    def test_strict_loader_rejects_invalid_utf8_and_depth(self):
        for data in (b"\xff", b'{"a":1,"a":2}', b"NaN", b"1e999", b"[] []", b"[" * 40 + b"0" + b"]" * 40,
                     b'"\\ud800"'):
            self.rejects(lambda: load_json(data))

    def test_nanosecond_time_interval_is_not_truncated(self):
        result = result_for()
        result["proposals"][0].update(valid_from="2026-01-01T00:00:00.123456001Z",
                                     valid_to="2026-01-01T00:00:00.123456002Z")
        self.assertEqual(validate_result(result, fixture()), result)

    def test_boolean_choice_index_is_not_an_integer_index(self):
        body = envelope()
        body["choices"][0]["index"] = False
        self.rejects(lambda: self.client(FakeTransport(body)).execute(fixture()), "incomplete_response")

    def test_result_budget_includes_file_newline(self):
        result = result_for()
        size = len(canonical(result))
        self.rejects(lambda: validate_result(result, fixture(), max_result_bytes=size), "output_limit")
        self.assertEqual(validate_result(result, fixture(), max_result_bytes=size + 1), result)

    def test_output_schema_matches_stable_prompt(self):
        self.assertIn("DreamResult 固定输出 schema".encode(), STABLE_PREFIX_BYTES)
        self.assertEqual(OUTPUT_SCHEMA["properties"]["proposals"]["minItems"], 1)


class FileAndCliTests(unittest.TestCase):
    def test_atomic_output_no_overwrite_private_mode_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            atomic_result_file(path, result_for())
            before = path.read_bytes()
            self.assertEqual(json.loads(before), result_for())
            if os.name == "posix":
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(DreamError):
                atomic_result_file(path, result_for(operation="noop"))
            self.assertEqual(path.read_bytes(), before)
            atomic_result_file(path, result_for(operation="noop"), overwrite=True)
            self.assertEqual(json.loads(path.read_bytes())["proposals"][0]["operation"], "noop")
            self.assertEqual(sorted(p.name for p in Path(directory).iterdir()), ["result.json"])

    def test_exclusive_commit_handles_a_concurrent_output(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "result.json"
            original_link = os.link

            def race(source, destination):
                Path(destination).write_text("concurrent-result")
                return original_link(source, destination)

            with patch("recallcard_dream.__main__.os.link", side_effect=race):
                with self.assertRaises(DreamError) as error:
                    atomic_result_file(target, result_for())
            self.assertEqual(error.exception.code, "output_exists")
            self.assertEqual(target.read_text(), "concurrent-result")
            self.assertEqual(len(list(Path(directory).iterdir())), 1)

    def test_output_size_error_does_not_create_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            with self.assertRaises(DreamError):
                atomic_result_file(path, result_for(), max_bytes=1)
            self.assertFalse(path.exists())
            self.assertEqual(list(Path(directory).iterdir()), [])

    @unittest.skipUnless(hasattr(os, "symlink"), "平台不支持符号链接")
    def test_symlink_target_and_parent_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "real.json"
            target.write_text("original")
            link = root / "link.json"
            link.symlink_to(target)
            with self.assertRaises(DreamError):
                atomic_result_file(link, result_for(), overwrite=True)
            folder = root / "linked"
            folder.symlink_to(root, target_is_directory=True)
            with self.assertRaises(DreamError):
                atomic_result_file(folder / "out.json", result_for())
            self.assertEqual(target.read_text(), "original")

    def run_cli(self, extra, job=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "job.json").write_bytes(canonical(job if job is not None else fixture()))
            (root / "sitecustomize.py").write_text(
                "import socket\ndef deny(*a, **k): raise AssertionError('测试不得联网')\n"
                "socket.create_connection=deny\nsocket.getaddrinfo=deny\n")
            env = {**os.environ, "PYTHONPATH": directory + os.pathsep + str(Path(__file__).resolve().parents[1]),
                   "RECALLCARD_DREAM_API_KEY": KEY}
            result = subprocess.run([sys.executable, "-m", "recallcard_dream", "--job", str(root / "job.json"),
                                     *[str(root / "out.json") if arg == "OUTPUT" else arg for arg in extra]],
                                    capture_output=True, env=env, timeout=10)
            return result, (root / "out.json").exists()

    def test_subprocess_validate_only_offline(self):
        process, exists = self.run_cli(["--validate-only"])
        self.assertEqual(process.returncode, 0, process.stderr)
        response = json.loads(process.stdout)
        self.assertTrue(response["validated"])
        self.assertEqual(response["network_calls"], 0)
        self.assertFalse(process.stderr)
        self.assertFalse(exists)

    def test_subprocess_network_off_by_default(self):
        process, exists = self.run_cli(["--endpoint", ENDPOINT, "--model", MODEL, "--output", "OUTPUT"])
        self.assertEqual(process.returncode, 2)
        self.assertEqual(json.loads(process.stderr)["error"]["code"], "approval_required")
        self.assertFalse(exists)
        self.assertNotIn(KEY.encode(), process.stderr)

    def test_subprocess_each_incomplete_approval_stays_offline(self):
        base = ["--endpoint", ENDPOINT, "--model", MODEL, "--output", "OUTPUT", "--allow-network"]
        for extra in (["--approve-dream-data", "--approve-scope", "project:contract"],
                      ["--approve-endpoint", ENDPOINT, "--approve-scope", "project:contract"],
                      ["--approve-endpoint", ENDPOINT, "--approve-dream-data"]):
            process, exists = self.run_cli(base + extra)
            self.assertEqual(process.returncode, 2)
            self.assertIn(json.loads(process.stderr)["error"]["code"], {"approval_required", "scope_denied"})
            self.assertFalse(exists)

    def test_subprocess_mixed_scope_rejected_before_network(self):
        job = fixture()
        job["memory_read_set"][0]["memory"]["scope"] = "private:outside"
        process, exists = self.run_cli(["--validate-only"], job)
        self.assertEqual(json.loads(process.stderr)["error"]["code"], "scope_denied")
        self.assertFalse(exists)

    def test_subprocess_no_secret_argument(self):
        process, exists = self.run_cli(["--validate-only", "--api-key", KEY])
        self.assertEqual(process.returncode, 2)
        self.assertNotIn(KEY.encode(), process.stderr)
        self.assertFalse(exists)

    def test_cli_fake_success_writes_only_result_and_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "job.json", root / "result.json"
            source.write_bytes(canonical(fixture()))
            output, error = io.TextIOWrapper(io.BytesIO(), encoding="utf-8"), io.StringIO()
            fake = FakeTransport()
            with patch.dict(os.environ, {"RECALLCARD_DREAM_API_KEY": KEY}), patch("sys.stdout", output), patch("sys.stderr", error), \
                 patch("recallcard_dream.__main__.DreamClient", side_effect=lambda *a, **k: DreamClient(*a, transport=fake, **k)):
                code = main(["--job", str(source), "--output", str(target), "--endpoint", ENDPOINT, "--model", MODEL,
                             "--allow-network", "--approve-endpoint", ENDPOINT, "--approve-scope", "project:contract",
                             "--approve-dream-data"])
            self.assertEqual(code, 0, error.getvalue())
            self.assertEqual(json.loads(target.read_bytes()), result_for())
            response = json.loads(output.buffer.getvalue())
            self.assertTrue(response["requires_review"])
            self.assertNotIn(KEY.encode(), output.buffer.getvalue())
            self.assertNotIn(fixture()["source_refs"][0]["event"]["content"].encode(), output.buffer.getvalue())
            self.assertEqual(len(fake.calls), 1)

    def test_cli_existing_output_and_input_alias_fail_before_transport(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = Path(directory) / "job.json", Path(directory) / "result.json"
            source.write_bytes(canonical(fixture()))
            target.write_text("existing")
            for out, flag in ((target, []), (source, ["--overwrite"])):
                with patch("sys.stderr", io.StringIO()), patch("recallcard_dream.__main__.DreamClient") as client:
                    self.assertEqual(main(["--job", str(source), "--output", str(out), "--endpoint", ENDPOINT,
                                           "--model", MODEL, *flag]), 2)
                    client.assert_not_called()
            self.assertEqual(target.read_text(), "existing")


class TransportTests(unittest.TestCase):
    def test_https_does_not_follow_redirect_or_read_error_body(self):
        response = Mock(status=307)
        connection = Mock()
        connection.getresponse.return_value = response
        with patch("recallcard_dream.client.http.client.HTTPSConnection", return_value=connection) as constructor:
            result = https_transport(ENDPOINT, b"synthetic", KEY, 30, 1024)
        self.assertEqual(result.status, 307)
        response.read1.assert_not_called()
        constructor.assert_called_once()
        connection.request.assert_called_once()
        response.close.assert_called_once()
        connection.close.assert_called_once()

    def test_https_read_limit_and_tls_context(self):
        response = Mock(status=200)
        response.getheader.return_value = None
        response.read1.side_effect = [b"12345"]
        connection = Mock()
        connection.getresponse.return_value = response
        with patch("recallcard_dream.client.http.client.HTTPSConnection", return_value=connection) as constructor:
            with self.assertRaises(DreamError) as error:
                https_transport(ENDPOINT, b"synthetic", KEY, 30, 4)
        self.assertEqual(error.exception.code, "response_limit")
        context = constructor.call_args.kwargs["context"]
        self.assertTrue(context.check_hostname)
        self.assertEqual(connection.request.call_args.args[0], "POST")
        connection.close.assert_called_once()

    def test_https_rejects_partial_declared_response(self):
        response = Mock(status=200)
        response.getheader.return_value = "9"
        response.read1.side_effect = [b"{}", b""]
        connection = Mock()
        connection.getresponse.return_value = response
        with patch("recallcard_dream.client.http.client.HTTPSConnection", return_value=connection):
            with self.assertRaises(DreamError) as error:
                https_transport(ENDPOINT, b"synthetic", KEY, 30, 20)
        self.assertEqual(error.exception.code, "incomplete_response")

    def test_https_oversized_declared_response_is_not_read(self):
        response = Mock(status=200)
        response.getheader.return_value = "999"
        connection = Mock()
        connection.getresponse.return_value = response
        with patch("recallcard_dream.client.http.client.HTTPSConnection", return_value=connection):
            with self.assertRaises(DreamError) as error:
                https_transport(ENDPOINT, b"synthetic", KEY, 30, 20)
        self.assertEqual(error.exception.code, "response_limit")
        response.read1.assert_not_called()

    def test_https_success_reads_multiple_bounded_chunks(self):
        response = Mock(status=200)
        response.getheader.return_value = None
        response.read1.side_effect = [b"abc", b"def", b""]
        connection = Mock()
        connection.getresponse.return_value = response
        with patch("recallcard_dream.client.http.client.HTTPSConnection", return_value=connection):
            result = https_transport(ENDPOINT, b"synthetic", KEY, 30, 20)
        self.assertEqual(result.body, b"abcdef")
        self.assertGreaterEqual(connection.sock.settimeout.call_count, 4)


@unittest.skipUnless(os.environ.get("RECALLCARD_TEST_BINARY"), "设置 RECALLCARD_TEST_BINARY 可运行 Rust 跨语言合同")
class RustContractTests(unittest.TestCase):
    def test_rust_export_fake_api_then_rust_review_without_publish(self):
        binary = str(Path(os.environ["RECALLCARD_TEST_BINARY"]).resolve(strict=True))
        with tempfile.TemporaryDirectory() as directory:
            env = {**os.environ, "RECALLCARD_STATE_DIR": directory + "/state"}

            def rust(*args, value=None):
                process = subprocess.run([binary, "--vault", directory + "/vault", *args],
                                         input=canonical(value) if value is not None else None,
                                         capture_output=True, env=env, timeout=10)
                self.assertEqual(process.returncode, 0, process.stderr.decode(errors="replace"))
                return json.loads(process.stdout)

            rust("init")
            event = rust("capture", "--file", "-", value={
                "scope": "project:contract", "role": "user", "origin": "user_input", "content": "合成接口测试采用中文。",
                "source": {"platform": "synthetic", "conversation_id": "synthetic", "message_id": "synthetic"}})
            memory = rust("memory", "add", "--file", "-", value={
                "scope": "project:contract", "content": "合成接口测试采用中文。", "source_refs": [event["id"]], "evidence": "user_explicit"})
            job = rust("dream", "export", "--source", event["id"], "--memory", memory["id"], "--scope", "project:contract")
            fake = FakeTransport(envelope(result_for(job)))
            with patch.dict(os.environ, {"RECALLCARD_DREAM_API_KEY": KEY}), \
                 patch.object(socket, "create_connection", side_effect=AssertionError("测试不得联网")):
                answer = DreamClient(ENDPOINT, MODEL, approved(), transport=fake).execute(job)
            review = rust("dream", "review", "--file", "-", value=answer["result"])
            self.assertTrue(review["can_apply"])
            self.assertFalse(review["already_applied"])
            self.assertEqual(len(fake.calls), 1)
            self.assertEqual(len(list((Path(directory) / "vault" / "memories").glob("*.md"))), 1)
            self.assertFalse(list((Path(directory) / "vault" / "control" / "dream-receipts").glob("*.json")))


if __name__ == "__main__":
    unittest.main()
