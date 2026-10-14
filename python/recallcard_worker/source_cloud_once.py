"""可信宿主启动的一次性来源编码器；一次HTTPS、零重试、无落盘与原文日志。"""
from __future__ import annotations

import argparse
from dataclasses import fields
import hashlib
import sys

from .embedding import (
    EmbeddingClient, EmbeddingError, EmbeddingSpace, NetworkApproval,
    _endpoint, canonical, https_transport, scopes,
)
from .host_encoding import (
    CLOUD_PROVIDER, ERROR_CODES, MAX_FRAME, MAX_RESPONSE_FRAME, HostEncodingError,
    frame, require, strict_json, valid_hash, valid_id, validate_inputs, validate_vectors,
)

SCHEMA = "recallcard.source-cloud-once/1"
KEY_ENV = "RECALLCARD_SOURCE_EMBEDDING_API_KEY"
TIMEOUT = 5.0


def validate_request(request, *, endpoint, scope, kind):
    require(isinstance(request, dict) and set(request) == {
        "schema", "id", "scope", "kind", "space", "space_signature", "inputs"})
    require(request["schema"] == SCHEMA and valid_id(request["id"]))
    require(kind in ("query", "event", "attachment") and request["kind"] == kind)
    require(request["scope"] == scope and scopes([scope]) == [scope], "scope_denied")
    _endpoint(endpoint)  # 默认安全HTTPS，不接受本机HTTP。
    description = request["space"]
    base_fields = {field.name for field in fields(EmbeddingSpace)}
    require(isinstance(description, dict) and set(description) == base_fields | {"tokenizer", "chunking_version"})
    for name in ("tokenizer", "chunking_version"):
        value = description[name]
        require(isinstance(value, str) and bool(value.strip()) and "\x00" not in value)
        require(len(value.encode("utf-8")) <= 256)
    space = EmbeddingSpace.from_dict({key: description[key] for key in base_fields})
    require(space.provider == CLOUD_PROVIDER and space.normalization == "l2", "invalid_config")
    require(space.endpoint == endpoint, "approval_required")
    require(valid_hash(request["space_signature"]) and request["space_signature"] ==
            hashlib.sha256(canonical(description)).hexdigest(), "invalid_config")
    validate_inputs(request["inputs"], space.dimensions, kind)
    # EmbeddingClient.embed 接收的已是prepare结果，绝不再次插入instruction。
    payload = canonical({"model": space.model, "input": request["inputs"],
                         "dimensions": space.dimensions, "encoding_format": "float"})
    require(len(payload) <= MAX_FRAME, "input_limit")
    return space, payload


def run_once(source, destination, *, endpoint, scope, kind, key_env=KEY_ENV, transport=https_transport):
    """流/transport注入供合成测试使用；正式CLI不能更换transport。"""
    ident = None
    try:
        require(key_env == KEY_ENV, "invalid_config")
        line = source.readline(MAX_FRAME + 1)
        require(bool(line) and len(line) <= MAX_FRAME and line.endswith(b"\n"), "input_limit")
        # 调用者必须关闭stdin；拒绝第二行、尾部空白或任何其他请求，然后才读密钥联网。
        require(source.read(1) == b"", "invalid_request")
        request = strict_json(line)
        if isinstance(request, dict) and valid_id(request.get("id")):
            ident = request["id"]
        space, expected_payload = validate_request(request, endpoint=endpoint, scope=scope, kind=kind)
        calls = 0
        def bounded_transport(actual_endpoint, payload, key, timeout, response_limit):
            nonlocal calls
            require(calls == 0, "network_budget_exhausted")
            require(actual_endpoint == endpoint and payload == expected_payload, "approval_required")
            require(len(payload) <= MAX_FRAME, "input_limit")
            calls += 1
            response = transport(actual_endpoint, payload, key, timeout, min(response_limit, MAX_RESPONSE_FRAME))
            require(isinstance(response.body, bytes) and len(response.body) <= MAX_RESPONSE_FRAME, "response_limit")
            return response
        approval = NetworkApproval(enabled=True, endpoint=endpoint, scopes=(scope,), data=(kind,))
        client = EmbeddingClient(approval, key_env=key_env, max_retries=0, max_network_calls=1,
                                 max_transmitted_bytes=MAX_FRAME, timeout=TIMEOUT, transport=bounded_transport)
        vectors = client.embed(space, request["inputs"], [scope], kind)
        validate_vectors(vectors, len(request["inputs"]), space.dimensions)
        output = frame({"schema": SCHEMA, "id": ident, "ok": True, "vectors": vectors})
        result = 0
    except (HostEncodingError, EmbeddingError) as error:
        code = error.code if error.code in ERROR_CODES else "host_encoding_failed"
        output = frame({"schema": SCHEMA, "id": ident, "ok": False, "error": {"code": code}})
        result = 2
    except Exception:
        output = frame({"schema": SCHEMA, "id": ident, "ok": False,
                        "error": {"code": "host_encoding_failed"}})
        result = 2
    destination.write(output)
    destination.flush()
    return result


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        # argparse默认错误会复述参数；一次性协议不能输出原始参数或路径。
        raise HostEncodingError("invalid_config")


def main(argv=None):
    parser = _Parser(add_help=False, allow_abbrev=False)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--scope", required=True)
    parser.add_argument("--kind", required=True, choices=("query", "event", "attachment"))
    parser.add_argument("--key-env", required=True)
    try:
        arguments = parser.parse_args(argv)
    except HostEncodingError:
        sys.stdout.buffer.write(frame({"schema": SCHEMA, "id": None, "ok": False,
                                       "error": {"code": "invalid_config"}}))
        sys.stdout.buffer.flush()
        return 2
    return run_once(sys.stdin.buffer, sys.stdout.buffer, endpoint=arguments.endpoint,
                    scope=arguments.scope, kind=arguments.kind, key_env=arguments.key_env)


if __name__ == "__main__":
    raise SystemExit(main())
