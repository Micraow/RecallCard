"""可选、显式授权的向量计算；本模块不会扫描或写入 Vault。"""
from __future__ import annotations

from collections import OrderedDict
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from http.client import HTTPException
import hashlib
import json
import math
import os
import re
import ssl
import time
from typing import Callable, Mapping
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, HTTPSHandler, ProxyHandler, Request, build_opener

CORPUS_SCHEMA = "recallcard.embedding-corpus/1"
INDEX_SCHEMA = "recallcard.embedding-index/1"
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_DOCUMENTS = 10_000
MAX_DIMENSIONS = 8192
MAX_VECTOR_VALUES = 500_000
MAX_TEXT_BYTES = 8192
MAX_CORPUS_BYTES = 4 * 1024 * 1024
MAX_BATCH_BYTES = 131_072
MAX_BATCH_SIZE = 64
_HASH = re.compile(r"[0-9a-f]{64}\Z")
_ENV = re.compile(r"[A-Za-z_][A-Za-z0-9_]{0,127}\Z")


class EmbeddingError(Exception):
    """只携带可向宿主显示的固定错误；不保留正文、凭据或供应商响应。"""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.partial_index = None


def fail(code: str, message: str):
    raise EmbeddingError(code, message)


def canonical(value) -> bytes:
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")


def text_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _integer(value, low: int, high: int, label: str) -> int:
    if type(value) is not int or not low <= value <= high:
        fail("invalid_input", f"{label}必须为范围内的整数")
    return value


def _string(value, limit: int, label: str, empty=False) -> str:
    if not isinstance(value, str):
        fail("invalid_input", f"{label}必须为字符串")
    try:
        size = len(value.encode("utf-8"))
    except UnicodeError:
        fail("invalid_input", f"{label}必须为有效 UTF-8")
    if size > limit or (not empty and not value.strip()) or "\x00" in value:
        fail("invalid_input", f"{label}为空、过长或含有无效字符")
    return value


def _hash(value) -> str:
    if not isinstance(value, str) or not _HASH.fullmatch(value):
        fail("invalid_hash", "哈希必须为小写 SHA-256 十六进制字符串")
    return value


def _scope(value) -> str:
    _string(value, 128, "scope")
    if not all(c.isalnum() or c in ":-_." for c in value):
        fail("invalid_scope", "scope 含有无效字符")
    return value


def scopes(value, *, empty=False) -> list[str]:
    if not isinstance(value, list) or len(value) > 256 or (not value and not empty):
        fail("invalid_scope", "scope 必须为有界列表")
    result = [_scope(item) for item in value]
    if len(set(result)) != len(result):
        fail("invalid_scope", "scope 不允许重复")
    return sorted(result)


def _ref(value) -> str:
    _string(value, 512, "ref")
    if not (value.startswith("event:") or value.startswith("memory:") or value.startswith("document:")):
        fail("invalid_ref", "ref 类型不受支持")
    if any(c.isspace() or ord(c) < 32 for c in value) or any(c in value for c in "/\\") or ".." in value:
        fail("invalid_ref", "ref 含有路径或无效字符")
    return value


def _fields(value, required: set[str], optional: set[str] | None = None):
    if not isinstance(value, dict) or not required <= value.keys() or value.keys() - required - (optional or set()):
        fail("invalid_input", "字段缺失或含有未知字段")


def _endpoint(value, *, offline_loopback=False) -> str:
    _string(value, 2048, "endpoint")
    try:
        url = urlsplit(value)
        port = url.port
    except ValueError:
        fail("invalid_endpoint", "endpoint 格式无效")
    local_metadata = offline_loopback and url.scheme == "http" and url.hostname in ("127.0.0.1", "::1")
    if ((url.scheme != "https" and not local_metadata) or not url.hostname or url.username is not None or url.password is not None
            or url.query or url.fragment or not url.path or url.path.endswith("/")
            or any(c.isspace() or ord(c) < 33 for c in value) or "\\" in value
            or (port is not None and port < 1)):
        fail("invalid_endpoint", "endpoint 必须是无凭据、查询参数及片段的完整 HTTPS 接口地址")
    try:
        value.encode("ascii")
    except UnicodeError:
        fail("invalid_endpoint", "endpoint 请使用 ASCII 或 IDNA 域名")
    return value


@dataclass(frozen=True)
class EmbeddingSpace:
    provider: str
    endpoint: str
    model: str
    dimensions: int
    revision: str
    document_instruction: str
    query_instruction: str
    normalization: str
    preprocessing_version: str

    @classmethod
    def from_dict(cls, value: dict) -> "EmbeddingSpace":
        _fields(value, set(cls.__dataclass_fields__))
        _string(value["provider"], 128, "provider")
        _endpoint(value["endpoint"], offline_loopback=True)
        _string(value["model"], 256, "model")
        _integer(value["dimensions"], 1, MAX_DIMENSIONS, "dimensions")
        _string(value["revision"], 256, "revision")
        for key in ("document_instruction", "query_instruction"):
            _string(value[key], 2048, key, empty=True)
        if value["normalization"] not in ("l2", "none"):
            fail("invalid_space", "normalization 只支持 l2 或 none")
        if value["preprocessing_version"] != "utf8-v1":
            fail("invalid_space", "不支持此文本预处理版本")
        return cls(**value)

    @property
    def signature(self) -> str:
        return hashlib.sha256(canonical(asdict(self))).hexdigest()

    def prepare(self, text: str, kind: str) -> str:
        _string(text, MAX_TEXT_BYTES, "文本")
        if kind not in ("corpus", "query"):
            fail("invalid_input", "不支持此输入类型")
        prefix = self.document_instruction if kind == "corpus" else self.query_instruction
        effective = prefix + "\n" + text if prefix else text
        _string(effective, MAX_TEXT_BYTES, "预处理文本")
        return effective


def vector(value, dimensions: int, normalization="none") -> list[float]:
    if not isinstance(value, list) or len(value) != dimensions:
        fail("invalid_vector", "向量维度与空间不一致")
    if any(type(x) not in (int, float) for x in value):
        fail("invalid_vector", "向量元素必须为有限数字")
    try:
        result = [float(x) for x in value]
    except (OverflowError, ValueError):
        fail("invalid_vector", "向量元素超出有效范围")
    if any(not math.isfinite(x) for x in result):
        fail("invalid_vector", "向量含有 NaN 或无穷值")
    norm = math.hypot(*result)
    if not math.isfinite(norm) or norm == 0:
        fail("invalid_vector", "向量范数为零或超出有效范围")
    return [x / norm for x in result] if normalization == "l2" else result


def validate_corpus(value: dict, space: EmbeddingSpace) -> dict:
    _fields(value, {"schema", "generation", "scope", "documents"})
    if value["schema"] != CORPUS_SCHEMA:
        fail("invalid_schema", "不支持此语料 schema")
    _string(value["generation"], 256, "generation")
    allowed = scopes(value["scope"], empty=True)
    docs = value["documents"]
    if not isinstance(docs, list) or len(docs) > MAX_DOCUMENTS or len(docs) * space.dimensions > MAX_VECTOR_VALUES:
        fail("input_limit", "文档或向量总量超过上限")
    seen = set()
    total = 0
    result = []
    for doc in docs:
        _fields(doc, {"ref", "content_hash", "text", "scope"})
        ref = _ref(doc["ref"])
        if ref in seen:
            fail("duplicate_ref", "语料包含重复 ref")
        seen.add(ref)
        scope = _scope(doc["scope"])
        if scope not in allowed:
            fail("scope_denied", "文档 scope 不在语料声明范围内")
        _string(doc["text"], MAX_TEXT_BYTES, "文本")
        if _hash(doc["content_hash"]) != text_hash(doc["text"]):
            fail("hash_mismatch", "文本与 content_hash 不一致")
        effective = space.prepare(doc["text"], "corpus")
        total += len(effective.encode("utf-8"))
        if total > MAX_CORPUS_BYTES:
            fail("input_limit", "语料文本总量超过上限")
        result.append({"ref": ref, "scope": scope, "content_hash": doc["content_hash"],
                       "input_hash": text_hash(effective), "effective": effective})
    return {"generation": value["generation"], "scope": allowed, "documents": sorted(result, key=lambda d: d["ref"])}


def validate_index(value: dict) -> tuple[EmbeddingSpace, list[dict]]:
    _fields(value, {"schema", "space", "space_signature", "generation", "scope", "documents"}, {"complete"})
    if value["schema"] != INDEX_SCHEMA:
        fail("invalid_schema", "不支持此索引 schema")
    space = EmbeddingSpace.from_dict(value["space"])
    if _hash(value["space_signature"]) != space.signature:
        fail("signature_mismatch", "索引的向量空间签名不一致")
    _string(value["generation"], 256, "generation")
    allowed = scopes(value["scope"], empty=True)
    if type(value.get("complete", True)) is not bool:
        fail("invalid_input", "complete 必须为布尔值")
    docs = value["documents"]
    if not isinstance(docs, list) or len(docs) > MAX_DOCUMENTS or len(docs) * space.dimensions > MAX_VECTOR_VALUES:
        fail("input_limit", "索引文档或向量总量超过上限")
    seen = set()
    by_input = {}
    result = []
    for doc in docs:
        _fields(doc, {"ref", "content_hash", "input_hash", "scope", "vector"})
        ref = _ref(doc["ref"])
        if ref in seen:
            fail("duplicate_ref", "索引包含重复 ref")
        seen.add(ref)
        _hash(doc["content_hash"])
        input_hash = _hash(doc["input_hash"])
        scope = _scope(doc["scope"])
        if scope not in allowed:
            fail("scope_denied", "索引文档越出声明范围")
        values = vector(doc["vector"], space.dimensions)
        if space.normalization == "l2" and not math.isclose(math.hypot(*values), 1.0, rel_tol=1e-6, abs_tol=1e-6):
            fail("invalid_vector", "索引向量未遵循声明的归一化方式")
        if input_hash in by_input and by_input[input_hash] != values:
            fail("cache_conflict", "同一输入哈希的缓存向量不一致")
        by_input[input_hash] = values
        result.append({**doc, "vector": values})
    return space, result


@dataclass(frozen=True)
class NetworkApproval:
    enabled: bool = False
    endpoint: str | None = None
    scopes: tuple[str, ...] = ()
    data: tuple[str, ...] = ()

    def check(self, space: EmbeddingSpace, requested_scopes: list[str], kind: str):
        if not self.enabled or self.endpoint != space.endpoint or kind not in self.data:
            fail("approval_required", "联网前必须明确批准接收地址与待发送的数据类型")
        if not requested_scopes or not set(requested_scopes) <= set(self.scopes):
            fail("scope_denied", "联网所需 scope 未获明确批准")


@dataclass(frozen=True)
class TransportResponse:
    status: int
    body: bytes
    headers: Mapping[str, str]


class _NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def https_transport(endpoint: str, payload: bytes, key: str, timeout: float, max_response_bytes: int) -> TransportResponse:
    """固定目的地、验证 TLS、禁用重定向与隐式代理；不打印响应正文。"""
    opener = build_opener(ProxyHandler({}), _NoRedirect(), HTTPSHandler(context=ssl.create_default_context()))
    request = Request(endpoint, payload, {"Authorization": "Bearer " + key, "Content-Type": "application/json"}, method="POST")
    try:
        with opener.open(request, timeout=timeout) as response:
            body = response.read(max_response_bytes + 1)
            if len(body) > max_response_bytes:
                fail("response_limit", "供应商响应超过上限")
            return TransportResponse(response.status, body, dict(response.headers))
    except HTTPError as error:
        # 错误正文可能复述私密输入或 key，故从不读取、返回或记录。
        status, headers = error.code, dict(error.headers)
        error.close()
        return TransportResponse(status, b"", headers)


class EmbeddingClient:
    def __init__(self, approval: NetworkApproval | None = None, *, transport: Callable = https_transport,
                 key_env="RECALLCARD_EMBEDDING_API_KEY", max_retries=2, timeout=30.0,
                 max_network_calls=100, max_transmitted_bytes=1_048_576, sleep: Callable = time.sleep,
                 price_per_million_tokens: float | None = None, pricing_date: str | None = None,
                 pricing_currency: str | None = None):
        self.approval = approval or NetworkApproval()
        self.transport = transport
        if not isinstance(key_env, str) or not _ENV.fullmatch(key_env):
            fail("invalid_config", "密钥环境变量名称无效")
        self.key_env = key_env
        self.max_retries = _integer(max_retries, 0, 5, "max_retries")
        if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 0 < timeout <= 120:
            fail("invalid_config", "timeout 必须在 0 到 120 秒之间")
        self.timeout = timeout
        self.max_network_calls = _integer(max_network_calls, 1, 1000, "max_network_calls")
        self.max_transmitted_bytes = _integer(max_transmitted_bytes, 1, 64 * 1024 * 1024, "max_transmitted_bytes")
        self.sleep = sleep
        if price_per_million_tokens is not None:
            if type(price_per_million_tokens) not in (int, float) or not math.isfinite(price_per_million_tokens) or not 0 <= price_per_million_tokens <= 1_000_000:
                fail("invalid_config", "价格快照无效")
            try:
                datetime.strptime(pricing_date, "%Y-%m-%d")
            except (TypeError, ValueError):
                fail("invalid_config", "价格快照需要 YYYY-MM-DD 日期")
            if not isinstance(pricing_currency, str) or not re.fullmatch(r"[A-Z]{3}", pricing_currency):
                fail("invalid_config", "价格快照需要明确的三位货币代码")
        self.pricing_currency = pricing_currency if price_per_million_tokens is not None else None
        self.price = price_per_million_tokens
        self.pricing_date = pricing_date
        self.calls = self.retries = self.transmitted_bytes = self.input_tokens = 0
        self.usage_missing = False
        self.query_cache: OrderedDict[tuple[str, str], list[float]] = OrderedDict()

    def diagnostics(self) -> dict:
        return {"network_calls": self.calls, "retries": self.retries, "transmitted_utf8_bytes": self.transmitted_bytes,
                "input_tokens": self.input_tokens if not self.usage_missing else None,
                "reported_input_tokens": self.input_tokens, "output_tokens": 0,
                "cache_read_tokens": None, "cache_write_tokens": None,
                "estimated_cost": self.input_tokens * self.price / 1_000_000 if self.price is not None and not self.usage_missing else None,
                "pricing_date": self.pricing_date, "pricing_currency": self.pricing_currency,
                "cost_note": "仅按成功响应的供应商用量与用户价格快照估算；失败请求及重试可能另行计费，非账单或费用保证"}

    def _reserve(self, size: int):
        if self.calls >= self.max_network_calls or self.transmitted_bytes + size > self.max_transmitted_bytes:
            fail("network_budget_exhausted", "本进程联网次数或累计发送文本字节预算已用尽")
        self.calls += 1
        self.transmitted_bytes += size

    @staticmethod
    def _retry_delay(headers: Mapping[str, str], attempt: int) -> float:
        raw = next((str(v) for k, v in headers.items() if k.lower() == "retry-after"), "")
        try:
            seconds = float(raw)
            if not math.isfinite(seconds):
                raise ValueError
        except ValueError:
            try:
                date = parsedate_to_datetime(raw)
                seconds = (date - datetime.now(timezone.utc)).total_seconds()
            except (TypeError, ValueError, OverflowError):
                seconds = 0.25 * (2 ** attempt)
        return min(5.0, max(0.0, seconds))

    def embed(self, space: EmbeddingSpace, texts: list[str], requested_scopes: list[str], kind: str) -> list[list[float]]:
        # texts 已由 EmbeddingSpace.prepare 处理；此处再次检查，公开接口不信任调用者。
        space = EmbeddingSpace.from_dict(asdict(space))
        # 本机 HTTP 只用于已计算向量的空间身份；绝不由此放开携带凭据的网络传输。
        _endpoint(space.endpoint)
        self.approval.check(space, scopes(requested_scopes), kind)
        if not isinstance(texts, list) or not 1 <= len(texts) <= MAX_BATCH_SIZE:
            fail("input_limit", "每批文本数量超过上限或为空")
        size = sum(len(_string(t, MAX_TEXT_BYTES, "文本").encode("utf-8")) for t in texts)
        if size > MAX_BATCH_BYTES:
            fail("input_limit", "每批文本总字节超过上限")
        key = os.environ.get(self.key_env)
        if not key or len(key) > 8192 or any(ord(c) < 33 or ord(c) > 126 for c in key):
            fail("missing_key", "请在指定环境变量中提供有效 API key")
        payload = canonical({"model": space.model, "input": texts, "dimensions": space.dimensions, "encoding_format": "float"})
        response_limit = min(MAX_JSON_BYTES, max(65536, len(texts) * space.dimensions * 40 + 65536))
        for attempt in range(self.max_retries + 1):
            self._reserve(size)
            if attempt:
                self.retries += 1
            try:
                response = self.transport(space.endpoint, payload, key, self.timeout, response_limit)
            except (URLError, OSError, TimeoutError, HTTPException):
                if attempt == self.max_retries:
                    fail("network_error", "连接供应商失败，已达到重试上限")
                self.sleep(self._retry_delay({}, attempt))
                continue
            if response.status == 200:
                if not isinstance(response.body, bytes) or len(response.body) > response_limit:
                    fail("response_limit", "供应商响应超过上限或格式错误")
                return self._parse_response(response.body, space, len(texts))
            retryable = response.status in (408, 429, 500, 502, 503, 504)
            if retryable and attempt < self.max_retries:
                self.sleep(self._retry_delay(response.headers, attempt))
                continue
            if response.status == 429:
                fail("rate_limited", "供应商限流，已达到重试上限；可稍后重试")
            if response.status in (401, 403):
                fail("provider_auth", "供应商拒绝身份验证或访问权限")
            if 300 <= response.status < 400:
                fail("redirect_blocked", "供应商重定向已阻止，请重新确认目的地")
            fail("provider_error", "供应商请求失败；未记录错误正文")
        fail("network_error", "请求未完成")

    def _parse_response(self, body: bytes, space: EmbeddingSpace, count: int) -> list[list[float]]:
        response = load_json(body)
        if not isinstance(response, dict) or response.get("model") != space.model:
            fail("model_mismatch", "供应商响应 model 与所选空间不一致")
        data = response.get("data")
        if not isinstance(data, list) or len(data) != count:
            fail("invalid_response", "供应商返回的向量数量不匹配")
        ordered = {}
        for item in data:
            if not isinstance(item, dict):
                fail("invalid_response", "供应商向量格式无效")
            i = item.get("index")
            if type(i) is not int or not 0 <= i < count or i in ordered:
                fail("invalid_response", "供应商向量序号无效或重复")
            ordered[i] = vector(item.get("embedding"), space.dimensions, space.normalization)
        usage = response.get("usage")
        token_count = usage.get("prompt_tokens") if isinstance(usage, dict) else None
        if type(token_count) is int and 0 <= token_count <= 1_000_000_000:
            self.input_tokens += token_count
        else:
            self.usage_missing = True
        return [ordered[i] for i in range(count)]

    def query_vector(self, space: EmbeddingSpace, query: str, requested_scopes: list[str]) -> tuple[list[float], bool]:
        effective = space.prepare(query, "query")
        key = (space.signature, text_hash(effective))
        # 只缓存编码结果；每次检索仍重新做当前 scope/ref 过滤。
        if key in self.query_cache:
            self.query_cache.move_to_end(key)
            return self.query_cache[key][:], True
        result = self.embed(space, [effective], requested_scopes, "query")[0]
        self.query_cache[key] = result
        if len(self.query_cache) > 32:
            self.query_cache.popitem(last=False)
        return result, False


def load_json(data: bytes | str):
    if isinstance(data, str):
        try:
            data = data.encode("utf-8")
        except UnicodeError:
            fail("invalid_json", "JSON 不是有效 UTF-8")
    if not isinstance(data, bytes) or len(data) > MAX_JSON_BYTES:
        fail("input_limit", "JSON 数据超过上限")

    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                fail("invalid_json", "JSON 不允许重复字段")
            result[key] = value
        return result

    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=pairs,
                          parse_constant=lambda _: fail("invalid_json", "JSON 不允许 NaN 或无穷值"))
    except (ValueError, UnicodeError, RecursionError):
        fail("invalid_json", "JSON 无效或嵌套过深")


def index_corpus(space: EmbeddingSpace, corpus: dict, client: EmbeddingClient, previous_index: dict | None = None,
                 *, checkpoint: Callable[[dict], None] | None = None, batch_size=MAX_BATCH_SIZE) -> dict:
    """按空间签名及实际编码输入哈希增量构建；不会读取任何额外文件。"""
    space = EmbeddingSpace.from_dict(asdict(space))
    batch_size = _integer(batch_size, 1, MAX_BATCH_SIZE, "batch_size")
    source = validate_corpus(corpus, space)
    reusable = {}
    if previous_index is not None:
        old_space, old_docs = validate_index(previous_index)
        if old_space.signature == space.signature:
            reusable = {doc["input_hash"]: doc["vector"] for doc in old_docs}
    ready = {}
    pending = OrderedDict()
    reused = 0
    for doc in source["documents"]:
        if doc["input_hash"] in reusable:
            ready[doc["input_hash"]] = reusable[doc["input_hash"]]
            reused += 1
        else:
            pending.setdefault(doc["input_hash"], doc["effective"])

    def snapshot(complete: bool) -> dict:
        return {"schema": INDEX_SCHEMA, "space": asdict(space), "space_signature": space.signature,
                "generation": source["generation"], "scope": source["scope"], "complete": complete,
                "documents": [{"ref": d["ref"], "content_hash": d["content_hash"], "input_hash": d["input_hash"],
                               "scope": d["scope"], "vector": ready[d["input_hash"]]}
                              for d in source["documents"] if d["input_hash"] in ready]}

    started = time.monotonic()
    missing = list(pending.items())
    offset = 0
    try:
        # 在第一次 API 调用前校验整个待发送范围，避免混合范围部分发送。
        if missing:
            client.approval.check(space, source["scope"], "corpus")
        while offset < len(missing):
            batch = []
            byte_count = 0
            while offset + len(batch) < len(missing) and len(batch) < batch_size:
                item = missing[offset + len(batch)]
                size = len(item[1].encode("utf-8"))
                if batch and byte_count + size > MAX_BATCH_BYTES:
                    break
                batch.append(item)
                byte_count += size
            values = client.embed(space, [text for _, text in batch], source["scope"], "corpus")
            ready.update((item[0], value) for item, value in zip(batch, values))
            offset += len(batch)
            if checkpoint:
                checkpoint(snapshot(False))
    except EmbeddingError as error:
        error.partial_index = snapshot(False)
        raise
    diagnostics = {**client.diagnostics(), "documents": len(source["documents"]), "reused_documents": reused,
                   "embedded_unique_inputs": len(pending), "deduplicated_documents": len(source["documents"]) - reused - len(pending),
                   "latency_ms": round((time.monotonic() - started) * 1000), "usage_scope": "当前 worker 进程累计"}
    return {"index": snapshot(True), "diagnostics": diagnostics}


def reciprocal_rank_fusion(rankings: list[list[str]], allowed_refs: set[str], *, limit=10, k=60) -> list[dict]:
    """先移除无权限及重复项再计算排名；不会把外部 lexical refs 当授权。"""
    _integer(limit, 1, 100, "limit")
    _integer(k, 1, 1000, "k")
    if not isinstance(rankings, list) or len(rankings) > 8:
        fail("input_limit", "融合排名列表超过上限")
    scores = {}
    for ranking in rankings:
        if not isinstance(ranking, list) or len(ranking) > MAX_DOCUMENTS:
            fail("input_limit", "融合候选数量超过上限")
        seen = set()
        rank = 0
        for ref in ranking:
            _ref(ref)
            if ref not in allowed_refs or ref in seen:
                continue
            seen.add(ref)
            rank += 1
            scores[ref] = scores.get(ref, 0.0) + 1.0 / (k + rank)
    return [{"ref": ref, "score": score} for ref, score in sorted(scores.items(), key=lambda x: (-x[1], x[0]))[:limit]]


def search_index(index: dict, query_vector: list[float], *, space_signature: str, allowed_scopes: list[str],
                 allowed_refs: list[str], limit=10, lexical_refs: list[str] | None = None) -> dict:
    space, documents = validate_index(index)
    if not index.get("complete", True):
        fail("index_incomplete", "增量检查点尚未完成，不能作为可检索索引")
    if _hash(space_signature) != space.signature:
        fail("signature_mismatch", "查询编码器与索引空间不一致")
    _integer(limit, 1, 100, "limit")
    visible_scopes = set(scopes(allowed_scopes, empty=True))
    if not isinstance(allowed_refs, list) or len(allowed_refs) > MAX_DOCUMENTS:
        fail("input_limit", "当前允许读取的 ref 列表无效或过长")
    permitted_refs = {_ref(ref) for ref in allowed_refs}
    q = vector(query_vector, space.dimensions, "l2")
    rows = []
    for doc in documents:
        if doc["scope"] not in visible_scopes or doc["ref"] not in permitted_refs:
            continue
        normalized = vector(doc["vector"], space.dimensions, "l2")
        score = max(-1.0, min(1.0, math.fsum(a * b for a, b in zip(q, normalized))))
        rows.append({"ref": doc["ref"], "score": score})
    rows.sort(key=lambda row: (-row["score"], row["ref"]))
    if lexical_refs is not None:
        # 融合只覆盖当前可见且有向量的记录；宿主仍可单独返回纯文本结果。
        visible = {row["ref"] for row in rows}
        result = reciprocal_rank_fusion([[row["ref"] for row in rows], lexical_refs], visible, limit=limit)
        return {"results": result, "ranking": "rrf", "space_signature": space.signature, "generation": index["generation"]}
    return {"results": rows[:limit], "ranking": "cosine", "space_signature": space.signature, "generation": index["generation"]}
