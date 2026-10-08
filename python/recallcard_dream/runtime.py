"""本机常驻服务调用的一次性 stdio 适配器；只返回未信任提议，不访问 Vault。

stdin 是一个最多 2 MiB、以 EOF 结束的 JSON 对象；stdout 恰好一个有界 JSON 行。
授权记录由本机服务持久化；普通 provider 配置本身不能授权发送数据。
Rust 仍须在提交前核对当前来源、角色、摘要、修订和保护规则。
供应商 token 用量可缺失，只用于诊断，不构成账单或硬性费用保证。
"""
from __future__ import annotations

import sys

from .client import (
    MAX_REQUEST_BYTES, MAX_RESULT_BYTES, DreamClient, DreamError, NetworkApproval,
    _date, _fields, _integer, _scope, _text, canonical, load_json, validate_endpoint,
    validate_job, validate_result,
)

REQUEST_SCHEMA = "recallcard.memory-provider-request/1"
MAX_INPUT_BYTES = 2 * 1024 * 1024
MAX_OUTPUT_BYTES = 2 * 1024 * 1024

# 从固定白名单重新构造错误，不回显异常文本、路径、供应商正文或凭据。
_ERROR_MESSAGES = {
    "approval_required": "发送来源与旧记忆前，需要完整、匹配的明确授权记录",
    "incomplete_response": "供应商响应不完整；不会自动重试",
    "input_limit": "输入超过允许的字节、数量或嵌套上限",
    "invalid_arguments": "本适配器不接受命令行参数；请通过标准输入提供一个请求",
    "invalid_config": "供应商配置或单次预算无效",
    "invalid_endpoint": "供应商地址必须是无凭据、查询参数和片段的完整 HTTPS 接口地址",
    "invalid_field": "请求或结果字段无效",
    "invalid_fields": "请求或结果字段缺失或含有未知字段",
    "invalid_json": "必须提供一个无重复字段的完整 UTF-8 JSON 对象",
    "invalid_reference": "请求或结果引用无效或超出任务范围",
    "invalid_response": "供应商结果或用量格式无效",
    "invalid_schema": "请求或结果协议版本不受支持",
    "invalid_scope": "记忆范围格式无效",
    "invalid_time": "授权或数据时间必须为有效的带时区 RFC3339",
    "io_error": "标准输入输出失败；未记录路径、正文或凭据",
    "job_mismatch": "结果与当前任务或输入摘要不匹配",
    "missing_key": "请通过 RECALLCARD_DREAM_API_KEY 环境变量提供有效密钥",
    "network_budget_exhausted": "单次网络请求或发送字节预算已用尽",
    "output_limit": "结果超过输出字节上限",
    "provider_error": "供应商返回失败状态；未记录错误正文，不会自动重试",
    "redirect_denied": "拒绝供应商重定向；不会向另一个地址发送数据或凭据",
    "request_limit": "完整供应商请求超过单次发送字节预算",
    "response_limit": "供应商响应超过字节上限",
    "scope_denied": "任务、数据与明确授权的记忆范围不一致",
    "timeout": "供应商请求超时；不会自动重试",
    "transport_error": "供应商传输失败；未记录原始异常，不会自动重试",
    "worker_error": "适配器执行失败；未记录原始异常或私密数据",
}


def _fail(code):
    raise DreamError(code, _ERROR_MESSAGES[code]) from None


def _error(code):
    if not isinstance(code, str) or code not in _ERROR_MESSAGES:
        code = "worker_error"
    return {"ok": False, "error": {"code": code, "message": _ERROR_MESSAGES[code]}}


def _config(value):
    if not isinstance(value, dict):
        _fail("invalid_config")
    # 调度器字段由 Rust 解释；此适配器只识别下列配置，不能由额外字段获得权限。
    if not isinstance(value.get("consent"), dict):
        _fail("approval_required")
    if not {"provider", "scope", "budget"} <= value.keys():
        _fail("invalid_config")
    provider = value["provider"]
    _fields(provider, {"endpoint", "model"}, {"kind"})
    if "kind" in provider and provider["kind"] != "openai_compatible":
        _fail("invalid_config")
    endpoint = validate_endpoint(provider["endpoint"])
    model = _text(provider["model"], 256)
    scope = _scope(value["scope"])
    consent = value["consent"]
    required = {"endpoint", "model", "scope", "send_source_snapshots", "send_memory_snapshots",
                "auto_apply", "accepted_at"}
    if consent.keys() != required:
        _fail("approval_required")
    if (consent["endpoint"] != endpoint or consent["model"] != model or consent["scope"] != scope
            or consent["send_source_snapshots"] is not True or consent["send_memory_snapshots"] is not True
            or consent["auto_apply"] is not True):
        _fail("approval_required")
    _date(consent["accepted_at"], nullable=False)
    budget = value["budget"]
    if not isinstance(budget, dict) or not {"max_output_tokens_per_call", "max_request_bytes_per_call"} <= budget.keys():
        _fail("invalid_config")
    output_tokens = _integer(budget["max_output_tokens_per_call"], 1, 131072)
    request_bytes = _integer(budget["max_request_bytes_per_call"], 1, MAX_REQUEST_BYTES)
    return endpoint, model, scope, output_tokens, request_bytes


def _usage(diagnostics, max_request_bytes):
    if not isinstance(diagnostics, dict):
        _fail("invalid_response")
    usage = {}
    for name in ("input_tokens", "output_tokens"):
        value = diagnostics.get(name)
        if value is not None and (type(value) is not int or not 0 <= value <= 2**63 - 1):
            _fail("invalid_response")
        usage[name] = value
    request_bytes = diagnostics.get("request_bytes")
    calls = diagnostics.get("network_calls")
    if (type(request_bytes) is not int or not 1 <= request_bytes <= max_request_bytes
            or type(calls) is not int or calls != 1):
        _fail("invalid_response")
    usage.update(request_bytes=request_bytes, network_calls=calls)
    return usage


def execute_request(data: bytes, *, client_factory=None):
    """最多执行一次网络请求；注入 client_factory 只用于纯合成、无网络测试。"""
    request = load_json(data, MAX_INPUT_BYTES)
    _fields(request, {"schema", "config", "job"})
    if request["schema"] != REQUEST_SCHEMA:
        _fail("invalid_schema")
    endpoint, model, scope, output_tokens, request_bytes = _config(request["config"])
    job = validate_job(request["job"])
    if job["allowed_scope"] != scope:
        _fail("scope_denied")
    approval = NetworkApproval(enabled=True, endpoint=endpoint, scopes=(scope,), dream_data=True)
    client = (client_factory or DreamClient)(
        endpoint, model, approval, max_requests=1, max_output_tokens=output_tokens,
        max_request_bytes=request_bytes, max_total_request_bytes=request_bytes,
    )
    completed = client.execute(job)
    if not isinstance(completed, dict) or "result" not in completed:
        _fail("invalid_response")
    result = validate_result(completed["result"], job, max_result_bytes=MAX_RESULT_BYTES)
    response = {"ok": True, "result": result, "usage": _usage(completed.get("diagnostics"), request_bytes),
                "verification": "provider_reported"}
    if len(canonical(response)) + 1 > MAX_OUTPUT_BYTES:
        _fail("output_limit")
    return response


def handle_request(data: bytes, *, client_factory=None):
    """协议边界只公开固定错误；供应商或注入执行器的任意异常正文都不会外泄。"""
    try:
        return execute_request(data, client_factory=client_factory)
    except DreamError as error:
        return _error(error.code)
    except Exception:
        return _error("worker_error")


def main(argv=None, *, stdin=None, stdout=None, client_factory=None):
    """stdout 同时承载成功和失败；stderr 不写入请求、结果或异常。"""
    argv = sys.argv[1:] if argv is None else argv
    stdin = sys.stdin.buffer if stdin is None else stdin
    stdout = sys.stdout.buffer if stdout is None else stdout
    if argv:
        response = _error("invalid_arguments")
    else:
        try:
            data = stdin.read(MAX_INPUT_BYTES + 1)
            response = handle_request(data, client_factory=client_factory)
        except Exception:
            response = _error("io_error")
    try:
        encoded = canonical(response) + b"\n"
        if len(encoded) > MAX_OUTPUT_BYTES:
            encoded = canonical(_error("output_limit")) + b"\n"
            response = _error("output_limit")
        stdout.write(encoded)
        stdout.flush()
    except Exception:
        return 2
    return 0 if response["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
