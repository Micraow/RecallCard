"""无凭据宿主编码通道：只交换有界向量，绝不连接网络或读取密钥。"""
from __future__ import annotations

import json
import math

from .embedding import canonical

SCHEMA = "recallcard.host-encode/1"
CLOUD_PROVIDER = "host-cloud-openai-compatible"
MAX_FRAME = 64 * 1024
MAX_RESPONSE_FRAME = 256 * 1024
MAX_INPUTS = 4
MAX_VALUES = 8192
MAX_INPUT_BYTES = 8192
KINDS = ("query", "event", "attachment")
ERROR_CODES = frozenset({
    "configuration_changed", "approval_required", "scope_denied", "events_not_approved",
    "attachments_not_approved", "query_not_approved", "missing_key", "provider_auth",
    "rate_limited", "redirect_blocked", "network_error", "provider_error", "model_mismatch",
    "invalid_response", "invalid_vector", "response_limit", "input_limit", "invalid_request",
    "invalid_config", "network_budget_exhausted", "host_encoding_failed", "host_response_limit",
    "host_protocol_error", "host_encoding_budget", "host_encoder_required",
})


class HostEncodingError(Exception):
    """错误只允许固定码，不能带回输入、凭据、路径或供应商正文。"""
    def __init__(self, code):
        self.code = code if code in ERROR_CODES else "host_encoding_failed"
        super().__init__(self.code)


def require(condition, code="host_protocol_error"):
    if not condition:
        raise HostEncodingError(code)


def valid_id(value):
    if type(value) is int:
        return 0 <= value <= 2**53 - 1
    if not isinstance(value, str) or not value.strip() or "\x00" in value:
        return False
    try:
        return len(value.encode("utf-8")) <= 128
    except UnicodeError:
        return False


def valid_hash(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result
    def constant(_):
        raise HostEncodingError("host_protocol_error")
    try:
        return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise HostEncodingError("host_protocol_error") from None


def validate_inputs(inputs, dimensions, kind):
    require(kind in KINDS)
    require(type(dimensions) is int and 1 <= dimensions <= MAX_VALUES)
    require(isinstance(inputs, list) and 1 <= len(inputs) <= MAX_INPUTS, "input_limit")
    require(kind != "query" or len(inputs) == 1, "input_limit")
    require(len(inputs) * dimensions <= MAX_VALUES, "input_limit")
    size = 0
    for value in inputs:
        require(isinstance(value, str) and bool(value.strip()) and "\x00" not in value)
        try:
            count = len(value.encode("utf-8"))
        except UnicodeError:
            raise HostEncodingError("host_protocol_error") from None
        require(count <= MAX_INPUT_BYTES, "input_limit")
        size += count
    require(size <= MAX_FRAME, "input_limit")


def validate_vectors(vectors, count, dimensions):
    require(isinstance(vectors, list) and len(vectors) == count, "invalid_vector")
    require(count * dimensions <= MAX_VALUES, "invalid_vector")
    for values in vectors:
        require(isinstance(values, list) and len(values) == dimensions, "invalid_vector")
        try:
            require(all(type(value) in (int, float) and math.isfinite(value) for value in values), "invalid_vector")
            norm = math.hypot(*values)
            require(math.isfinite(norm) and norm > 0, "invalid_vector")
        except (OverflowError, TypeError, ValueError):
            raise HostEncodingError("invalid_vector") from None
    return vectors


def frame(value, code="host_response_limit", *, limit=MAX_RESPONSE_FRAME):
    try:
        raw = canonical(value) + b"\n"
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise HostEncodingError("host_protocol_error") from None
    require(len(raw) <= limit, code)
    return raw


class HostEncoder:
    """只能由可信serve绑定流与空间；每个已验证作业最多交换一次。"""
    def __init__(self, source, destination, *, dimensions, space_signature):
        require(type(dimensions) is int and 1 <= dimensions <= MAX_VALUES, "invalid_config")
        require(valid_hash(space_signature), "invalid_config")
        self.source = source
        self.destination = destination
        self.dimensions = dimensions
        self.space_signature = space_signature
        self._active = False
        self._used = False
        self._id = None
        self.failed = False

    def begin_job(self, ident):
        require(not self.failed and not self._active and valid_id(ident))
        self._id, self._active, self._used = ident, True, False

    def end_job(self):
        self._id, self._active = None, False

    def encode(self, kind, inputs):
        require(self._active and not self.failed, "host_encoder_required")
        require(not self._used, "host_encoding_budget")
        validate_inputs(inputs, self.dimensions, kind)
        request = {"schema": SCHEMA, "id": self._id, "kind": kind,
                   "space_signature": self.space_signature, "inputs": inputs}
        raw = frame(request, "input_limit", limit=MAX_FRAME)
        self._used = True
        try:
            self.destination.write(raw)
            self.destination.flush()
            line = self.source.readline(MAX_RESPONSE_FRAME + 1)
            require(bool(line) and len(line) <= MAX_RESPONSE_FRAME and line.endswith(b"\n"))
            response = strict_json(line)
            require(isinstance(response, dict) and response.get("schema") == SCHEMA)
            require(type(response.get("id")) is type(self._id) and response["id"] == self._id)
            require(type(response.get("ok")) is bool)
            if response["ok"]:
                require(set(response) == {"schema", "id", "ok", "vectors"})
                return validate_vectors(response["vectors"], len(inputs), self.dimensions)
            require(set(response) == {"schema", "id", "ok", "error"})
            error = response["error"]
            require(isinstance(error, dict) and set(error) == {"code"})
            require(isinstance(error["code"], str) and error["code"] in ERROR_CODES)
        except HostEncodingError:
            # 失败后不能再把未消费/乱序回调误当作下一作业。
            self.failed = True
            raise
        except (OSError, ValueError, TypeError, OverflowError, RecursionError):
            self.failed = True
            raise HostEncodingError("host_protocol_error") from None
        raise HostEncodingError(error["code"])
