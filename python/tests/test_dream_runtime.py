"""stdio 适配器的合成协议测试；不调用真实供应商，不读取个人归档。"""
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_dream.client import DreamClient, DreamError, MAX_REQUEST_BYTES, canonical
from recallcard_dream.runtime import (
    MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, REQUEST_SCHEMA, handle_request, main,
)
from test_dream_api import ENDPOINT, KEY, MODEL, FakeTransport, envelope, fixture, result_for


def request_fixture():
    return {
        "schema": REQUEST_SCHEMA,
        "config": {
            "provider": {"endpoint": ENDPOINT, "model": MODEL},
            "scope": "project:contract",
            "consent": {
                "endpoint": ENDPOINT, "model": MODEL, "scope": "project:contract",
                "send_source_snapshots": True, "send_memory_snapshots": True,
                "auto_apply": True, "accepted_at": "2026-10-08T01:00:00Z",
            },
            "budget": {"max_output_tokens_per_call": 512, "max_request_bytes_per_call": MAX_REQUEST_BYTES},
        },
        "job": fixture(),
    }


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        network = patch.object(socket, "create_connection", side_effect=AssertionError("测试不得联网"))
        network.start()
        key = patch.dict(os.environ, {"RECALLCARD_DREAM_API_KEY": KEY})
        key.start()
        self.addCleanup(network.stop)
        self.addCleanup(key.stop)

    def run_request(self, request=None, transport=None):
        transport = transport or FakeTransport(envelope(usage={"prompt_tokens": 123, "completion_tokens": 45}))
        factory = Mock(side_effect=lambda *args, **kwargs: DreamClient(*args, transport=transport, **kwargs))
        response = handle_request(canonical(request or request_fixture()), client_factory=factory)
        return response, transport, factory

    def assert_rejected_before_call(self, request, code=None):
        factory = Mock(side_effect=AssertionError("不得构造联网执行器"))
        data = request if isinstance(request, bytes) else canonical(request)
        response = handle_request(data, client_factory=factory)
        self.assertIs(response["ok"], False)
        self.assertEqual(set(response), {"ok", "error"})
        self.assertEqual(set(response["error"]), {"code", "message"})
        if code:
            self.assertEqual(response["error"]["code"], code)
        factory.assert_not_called()
        return response

    def test_success_uses_one_call_and_exact_approval(self):
        response, transport, factory = self.run_request()
        self.assertEqual(set(response), {"ok", "result", "usage", "verification"})
        self.assertIs(response["ok"], True)
        self.assertEqual(response["result"], result_for())
        self.assertEqual(response["verification"], "provider_reported")
        self.assertEqual(response["usage"], {
            "input_tokens": 123, "output_tokens": 45,
            "request_bytes": len(transport.calls[0][1]), "network_calls": 1,
        })
        factory.assert_called_once()
        args, kwargs = factory.call_args
        self.assertEqual(args[:2], (ENDPOINT, MODEL))
        approval = args[2]
        self.assertIs(approval.enabled, True)
        self.assertIs(approval.dream_data, True)
        self.assertEqual(approval.endpoint, ENDPOINT)
        self.assertEqual(approval.scopes, ("project:contract",))
        self.assertEqual(kwargs["max_requests"], 1)
        self.assertEqual(kwargs["max_output_tokens"], 512)
        self.assertEqual(kwargs["max_total_request_bytes"], MAX_REQUEST_BYTES)
        self.assertEqual(len(transport.calls), 1)
        self.assertEqual(transport.calls[0][2], KEY)
        sent = json.loads(transport.calls[0][1])
        self.assertEqual(sent["max_completion_tokens"], 512)
        self.assertEqual(json.loads(sent["messages"][-1]["content"]), fixture())
        self.assertNotIn(KEY, canonical(response).decode())

    def test_configured_wait_limits_reach_the_worker_without_changing_data_or_budget(self):
        request = request_fixture()
        request["config"]["timeouts"] = {"connect_seconds": 15, "read_seconds": 300, "operation_seconds": 600}
        response, transport, factory = self.run_request(request)
        self.assertIs(response["ok"], True)
        kwargs = factory.call_args.kwargs
        self.assertEqual((kwargs["connect_timeout"], kwargs["read_timeout"], kwargs["timeout"]), (15, 300, 600))
        self.assertEqual(transport.calls[0][3], 600)
        self.assertEqual(json.loads(transport.calls[0][1])["messages"][-1]["content"], canonical(fixture()).decode())
        for bad in ({"connect_seconds":121}, {"read_seconds":31}, {"operation_seconds":1801}, {"operation_seconds":True}, {"other":2}):
            request["config"]["timeouts"] = bad
            self.assert_rejected_before_call(request)

    def test_absent_or_nonobject_consent_never_calls(self):
        for consent in (None, False, True, [], "approved", 1):
            with self.subTest(consent=consent):
                request = request_fixture()
                request["config"]["consent"] = consent
                self.assert_rejected_before_call(request, "approval_required")
        request = request_fixture()
        del request["config"]["consent"]
        self.assert_rejected_before_call(request, "approval_required")

    def test_every_consent_field_required_and_exact(self):
        for field in request_fixture()["config"]["consent"]:
            with self.subTest(field=field):
                request = request_fixture()
                del request["config"]["consent"][field]
                self.assert_rejected_before_call(request, "approval_required")
        for field, value in (("endpoint", ENDPOINT + "/other"), ("model", "other-model"),
                             ("scope", "project:other"), ("send_source_snapshots", False),
                             ("send_memory_snapshots", False), ("auto_apply", False)):
            with self.subTest(field=field):
                request = request_fixture()
                request["config"]["consent"][field] = value
                self.assert_rejected_before_call(request, "approval_required")

    def test_truthy_values_do_not_grant_consent(self):
        for field in ("send_source_snapshots", "send_memory_snapshots", "auto_apply"):
            for value in (1, "true", [], {}, None):
                with self.subTest(field=field, value=value):
                    request = request_fixture()
                    request["config"]["consent"][field] = value
                    self.assert_rejected_before_call(request, "approval_required")

    def test_consent_timestamp_is_required_timezone_aware_rfc3339(self):
        for timestamp in (None, 0, "", "2026-10-08", "2026-10-08T01:00:00", "2026-02-30T00:00:00Z"):
            with self.subTest(timestamp=timestamp):
                request = request_fixture()
                request["config"]["consent"]["accepted_at"] = timestamp
                self.assert_rejected_before_call(request, "invalid_time")

    def test_extra_consent_fields_cannot_extend_authorization(self):
        request = request_fixture()
        request["config"]["consent"]["all_scopes"] = True
        self.assert_rejected_before_call(request, "approval_required")

    def test_job_scope_must_match_consent_and_config(self):
        request = request_fixture()
        request["config"]["scope"] = "project:other"
        request["config"]["consent"]["scope"] = "project:other"
        self.assert_rejected_before_call(request, "scope_denied")

    def test_scheduler_fields_and_optional_known_provider_kind_accepted(self):
        request = request_fixture()
        request["config"].update(enabled=True, interval_seconds=60, schema_version=1)
        request["config"]["budget"].update(max_calls_per_day=3, max_tokens_per_day=10000)
        request["config"]["provider"]["kind"] = "openai_compatible"
        response, transport, _ = self.run_request(request)
        self.assertIs(response["ok"], True)
        self.assertEqual(len(transport.calls), 1)
        self.assertNotIn(b"accepted_at", transport.calls[0][1])
        self.assertNotIn(b"max_calls_per_day", transport.calls[0][1])

    def test_config_shape_and_provider_keys_are_checked(self):
        for config in (None, False, [], "secret"):
            request = request_fixture()
            request["config"] = config
            self.assert_rejected_before_call(request, "invalid_config")
        for field in ("provider", "scope", "budget"):
            request = request_fixture()
            del request["config"][field]
            self.assert_rejected_before_call(request, "invalid_config")
        request = request_fixture()
        request["config"]["provider"]["api_key"] = "must-not-be-used"
        response = self.assert_rejected_before_call(request, "invalid_fields")
        self.assertNotIn("must-not-be-used", canonical(response).decode())
        request["config"]["provider"] = {"endpoint": ENDPOINT, "model": MODEL, "kind": "shell"}
        self.assert_rejected_before_call(request, "invalid_config")

    def test_endpoint_and_model_validation_before_call(self):
        for endpoint in ("http://example.invalid/v1/chat/completions", "https://key@dream.invalid/v1", ENDPOINT + "?key=x"):
            request = request_fixture()
            request["config"]["provider"]["endpoint"] = endpoint
            request["config"]["consent"]["endpoint"] = endpoint
            self.assert_rejected_before_call(request, "invalid_endpoint")
        request = request_fixture()
        request["config"]["provider"]["model"] = ""
        request["config"]["consent"]["model"] = ""
        self.assert_rejected_before_call(request, "invalid_field")

    def test_budget_limits_and_types_before_call(self):
        for field, bad_values in (
                ("max_output_tokens_per_call", (0, 131073, True, 1.5, "512")),
                ("max_request_bytes_per_call", (0, MAX_REQUEST_BYTES + 1, True, None))):
            for value in bad_values:
                with self.subTest(field=field, value=value):
                    request = request_fixture()
                    request["config"]["budget"][field] = value
                    self.assert_rejected_before_call(request, "invalid_field")
        for budget in (None, [], {}, {"max_output_tokens_per_call": 1}):
            request = request_fixture()
            request["config"]["budget"] = budget
            self.assert_rejected_before_call(request, "invalid_config")

    def test_full_request_limit_includes_fixed_prompt(self):
        request = request_fixture()
        request["config"]["budget"]["max_request_bytes_per_call"] = len(canonical(request["job"]))
        response, transport, _ = self.run_request(request)
        self.assertEqual(response["error"]["code"], "request_limit")
        self.assertEqual(transport.calls, [])

    def test_missing_environment_key_does_not_call(self):
        with patch.dict(os.environ, {}, clear=True):
            response, transport, _ = self.run_request()
        self.assertEqual(response["error"]["code"], "missing_key")
        self.assertEqual(transport.calls, [])

    def test_provider_reported_usage_may_be_unknown_not_zero(self):
        for usage in (None, {}, {"prompt_tokens": 123}):
            response, _, _ = self.run_request(transport=FakeTransport(envelope(usage=usage)))
            self.assertIs(response["ok"], True)
            self.assertIsNone(response["usage"]["output_tokens"])
            if not usage:
                self.assertIsNone(response["usage"]["input_tokens"])

    def test_malformed_provider_usage_fails_without_retry(self):
        for usage in ("secret-body", {"prompt_tokens": True}, {"completion_tokens": -1},
                      {"prompt_tokens": 2**63}, {"secret/key": 1}):
            response, transport, _ = self.run_request(transport=FakeTransport(envelope(usage=usage)))
            self.assertEqual(response["error"]["code"], "invalid_response")
            self.assertEqual(len(transport.calls), 1)
            self.assertNotIn("secret", canonical(response).decode())

    def test_result_revalidated_for_injected_client(self):
        completed = {"result": result_for(), "diagnostics": {
            "input_tokens": None, "output_tokens": None, "request_bytes": 20, "network_calls": 1,
        }}
        completed["result"]["job_id"] = "dream_" + "a" * 64
        client = Mock()
        client.execute.return_value = completed
        response = handle_request(canonical(request_fixture()), client_factory=Mock(return_value=client))
        self.assertEqual(response["error"]["code"], "job_mismatch")

    def test_injected_diagnostics_cannot_escape_safe_usage_contract(self):
        base = {"input_tokens": 1, "output_tokens": None, "request_bytes": 20, "network_calls": 1}
        for key, value in (("input_tokens", True), ("output_tokens", -1), ("request_bytes", 0),
                           ("request_bytes", MAX_REQUEST_BYTES + 1), ("network_calls", 2),
                           ("network_calls", True)):
            with self.subTest(key=key, value=value):
                diagnostics = dict(base, **{key: value})
                client = Mock()
                client.execute.return_value = {"result": result_for(), "diagnostics": diagnostics}
                response = handle_request(canonical(request_fixture()), client_factory=Mock(return_value=client))
                self.assertEqual(response["error"]["code"], "invalid_response")
        client = Mock()
        client.execute.return_value = {"result": result_for(), "diagnostics": dict(base, raw_body=KEY)}
        response = handle_request(canonical(request_fixture()), client_factory=Mock(return_value=client))
        self.assertIs(response["ok"], True)
        self.assertNotIn(KEY, canonical(response).decode())

    def test_errors_never_reflect_exception_code_or_message(self):
        for exception in (RuntimeError(KEY), DreamError("provider_error", KEY), DreamError(KEY, KEY),
                          DreamError([KEY], KEY)):
            response = handle_request(canonical(request_fixture()), client_factory=Mock(side_effect=exception))
            self.assertIs(response["ok"], False)
            self.assertNotIn(KEY, canonical(response).decode())
            self.assertIn(response["error"]["code"], ("worker_error", "provider_error"))

    def test_transport_and_http_errors_sanitized(self):
        for transport in (FakeTransport(error=RuntimeError(KEY)), FakeTransport(error=DreamError(KEY, KEY)),
                          FakeTransport(body={"error": KEY}, status=401),
                          FakeTransport(body={"error": KEY}, status=302)):
            response, transport, _ = self.run_request(transport=transport)
            self.assertIs(response["ok"], False)
            self.assertEqual(len(transport.calls), 1)
            self.assertNotIn(KEY, canonical(response).decode())

    def test_schema_and_unknown_envelope_fields_rejected(self):
        request = request_fixture()
        request["schema"] = "recallcard.memory-provider-request/2"
        self.assert_rejected_before_call(request, "invalid_schema")
        request = request_fixture()
        request["api_key"] = KEY
        self.assert_rejected_before_call(request, "invalid_fields")
        for field in ("schema", "config", "job"):
            request = request_fixture()
            del request[field]
            self.assert_rejected_before_call(request, "invalid_fields")

    def test_duplicate_keys_at_any_depth_rejected(self):
        data = canonical(request_fixture())
        for duplicate in (
            data.replace(b'"schema":', b'"schema":"other","schema":', 1),
            data.replace(b'"accepted_at":', b'"accepted_at":null,"accepted_at":', 1),
            data.replace(b'"model":', b'"model":"other","model":', 1),
        ):
            self.assert_rejected_before_call(duplicate, "invalid_json")

    def test_invalid_utf8_nonfinite_trailing_and_deep_json_rejected(self):
        for data in (b"", b"\xff", b"{} {}", b'{"secret":NaN}', b'{"secret":Infinity}',
                     b'{"secret":"\\ud800"}', b"{", b"[" * 100 + b"0" + b"]" * 100):
            with self.subTest(data=data[:40]):
                self.assert_rejected_before_call(data)
        for data in (b"[]", b"null", b"true", b'"secret"'):
            self.assert_rejected_before_call(data, "invalid_fields")

    def test_input_size_is_bounded_before_client(self):
        self.assert_rejected_before_call(b" " * (MAX_INPUT_BYTES + 1), "input_limit")

    def test_main_single_json_stdout_success(self):
        transport = FakeTransport()
        factory = lambda *args, **kwargs: DreamClient(*args, transport=transport, **kwargs)
        stdin, stdout, stderr = io.BytesIO(canonical(request_fixture())), io.BytesIO(), io.StringIO()
        with patch.object(sys, "stderr", stderr):
            code = main([], stdin=stdin, stdout=stdout, client_factory=factory)
        self.assertEqual(code, 0)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(stdout.getvalue().count(b"\n"), 1)
        self.assertLessEqual(len(stdout.getvalue()), MAX_OUTPUT_BYTES)
        self.assertIs(json.loads(stdout.getvalue())["ok"], True)

    def test_main_errors_use_stdout_and_nonzero_status(self):
        for argv, data, expected in (([], b"{", "invalid_json"), (["--key=" + KEY], b"", "invalid_arguments")):
            stdout, stderr = io.BytesIO(), io.StringIO()
            with patch.object(sys, "stderr", stderr):
                code = main(argv, stdin=io.BytesIO(data), stdout=stdout)
            self.assertEqual(code, 2)
            self.assertEqual(stderr.getvalue(), "")
            self.assertEqual(json.loads(stdout.getvalue())["error"]["code"], expected)
            self.assertNotIn(KEY.encode(), stdout.getvalue())

    def test_main_reads_only_limit_plus_one_bytes(self):
        source = io.BytesIO(b" " * (MAX_INPUT_BYTES + 100))
        stdout = io.BytesIO()
        self.assertEqual(main([], stdin=source, stdout=stdout), 2)
        self.assertEqual(source.tell(), MAX_INPUT_BYTES + 1)
        self.assertEqual(json.loads(stdout.getvalue())["error"]["code"], "input_limit")

    def test_io_errors_are_sanitized_and_write_failure_does_not_raise(self):
        source, stdout = Mock(), io.BytesIO()
        source.read.side_effect = OSError(KEY)
        self.assertEqual(main([], stdin=source, stdout=stdout), 2)
        self.assertEqual(json.loads(stdout.getvalue())["error"]["code"], "io_error")
        self.assertNotIn(KEY.encode(), stdout.getvalue())
        broken_output = Mock()
        broken_output.write.side_effect = OSError(KEY)
        self.assertEqual(main([], stdin=io.BytesIO(b"{"), stdout=broken_output), 2)

    def test_module_launch_no_network_when_consent_absent(self):
        request = request_fixture()
        request["config"]["consent"] = None
        environment = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1]))
        process = subprocess.run(
            [sys.executable, "-m", "recallcard_dream.runtime"], input=canonical(request),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment, timeout=5, check=False,
        )
        self.assertEqual(process.returncode, 2)
        self.assertEqual(process.stderr, b"")
        self.assertEqual(process.stdout.count(b"\n"), 1)
        self.assertEqual(json.loads(process.stdout)["error"]["code"], "approval_required")


if __name__ == "__main__":
    unittest.main()
