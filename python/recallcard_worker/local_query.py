"""已配置本机 embedding 服务的 query-only 适配器；无凭据、代理、DNS 或重定向。"""
from dataclasses import asdict
from http.client import HTTPConnection, HTTPException
from urllib.parse import urlsplit

from .embedding import (
    EmbeddingClient, EmbeddingSpace, TransportResponse, MAX_JSON_BYTES,
    _endpoint, _integer, _string, canonical, fail, scopes,
)


def local_endpoint(value):
    _endpoint(value, offline_loopback=True)
    url = urlsplit(value)
    if (url.scheme != 'http' or url.hostname not in ('127.0.0.1', '::1')
            or url.port is None or url.path != '/v1/embeddings'):
        fail('invalid_local_endpoint', '本机查询只接受带明确端口的数字回环 /v1/embeddings 地址')
    return url


def local_transport(endpoint, payload, _key, timeout, limit):
    url = local_endpoint(endpoint)
    connection = HTTPConnection(url.hostname, url.port, timeout=timeout)
    try:
        connection.request('POST', url.path, body=payload, headers={'Content-Type': 'application/json', 'Accept': 'application/json'})
        response = connection.getresponse()
        # HTTPConnection 不读取代理环境、不跟随重定向，也没有认证头。
        if response.status != 200:
            return TransportResponse(response.status, b'', {})
        body = response.read(limit + 1)
        if len(body) > limit:
            fail('response_limit', '本机 embedding 响应超过上限')
        return TransportResponse(response.status, body, {})
    finally:
        connection.close()


class LocalQueryClient(EmbeddingClient):
    def __init__(self, endpoint, allowed_scopes, *, max_calls=20, max_bytes=65536, timeout=30.0, transport=local_transport):
        local_endpoint(endpoint)
        self.local_endpoint = endpoint
        self.local_scopes = scopes(allowed_scopes)
        super().__init__(max_retries=0, max_network_calls=_integer(max_calls, 1, 1000, '本机请求预算'),
                         max_transmitted_bytes=_integer(max_bytes, 1, 4 * 1024 * 1024, '本机文本字节预算'),
                         timeout=timeout, transport=transport)

    def _reserve(self, size):
        if self.calls >= self.max_network_calls or self.transmitted_bytes + size > self.max_transmitted_bytes:
            fail('local_budget_exhausted', '本机查询次数或文本字节预算已用尽；未自动重试')
        self.calls += 1
        self.transmitted_bytes += size

    def embed(self, space, texts, requested_scopes, kind):
        space = EmbeddingSpace.from_dict(asdict(space))
        if kind != 'query':
            fail('local_query_only', '本机查询配置不批准语料编码或索引重建')
        if space.endpoint != self.local_endpoint or any(scope not in self.local_scopes for scope in scopes(requested_scopes)):
            fail('query_not_approved', '本机查询目的地或范围与配置不一致')
        if not isinstance(texts, list) or len(texts) != 1:
            fail('input_limit', '本机查询每次仅编码一个完整查询')
        size = len(_string(texts[0], 8192, '本机查询').encode('utf-8'))
        self._reserve(size)
        payload = canonical({'model': space.model, 'input': texts, 'dimensions': space.dimensions, 'encoding_format': 'float'})
        limit = min(MAX_JSON_BYTES, space.dimensions * 40 + 65536)
        try:
            response = self.transport(self.local_endpoint, payload, None, self.timeout, limit)
        except (OSError, TimeoutError, HTTPException):
            fail('local_provider_unavailable', '本机 embedding 服务不可用或超时；未自动重试')
        if response.status != 200:
            if 300 <= response.status < 400:
                fail('redirect_blocked', '本机 embedding 重定向已拒绝')
            fail('local_provider_error', '本机 embedding 服务未返回成功结果')
        if not isinstance(response.body, bytes) or len(response.body) > limit:
            fail('response_limit', '本机 embedding 响应超过上限')
        return self._parse_response(response.body, space, 1)

    def diagnostics(self):
        return {'local_calls': self.calls, 'network_calls': 0, 'retries': 0,
                'transmitted_utf8_bytes': self.transmitted_bytes,
                'input_tokens': self.input_tokens if not self.usage_missing else None,
                'external_calls': 0}

class LocalIndexClient(LocalQueryClient):
    """独立本机语料批准；查询端口配置本身永远不能构造此客户端。"""
    def embed_documents(self, space, texts, requested_scopes):
        space = EmbeddingSpace.from_dict(asdict(space))
        if space.endpoint != self.local_endpoint or any(scope not in self.local_scopes for scope in scopes(requested_scopes)):
            fail('index_not_approved', '本机语料目的地或范围与独立配置不一致')
        if not isinstance(texts, list) or not 1 <= len(texts) <= 8 or len(texts) * space.dimensions > 8192:
            fail('input_limit', '本机语料批次或向量总维度超过上限')
        for text in texts:
            _string(text, 262144, '本机语料')
        size = sum(len(text.encode('utf-8')) for text in texts)
        self._reserve(size)
        payload = canonical({'model': space.model, 'input': texts, 'dimensions': space.dimensions, 'encoding_format': 'float'})
        limit = 512 * 1024
        try:
            response = self.transport(self.local_endpoint, payload, None, self.timeout, limit)
        except (OSError, TimeoutError, HTTPException):
            fail('local_provider_unavailable', '本机 embedding 服务不可用或超时；未自动重试')
        if response.status != 200:
            fail('redirect_blocked' if 300 <= response.status < 400 else 'local_provider_error', '本机语料编码未成功；未跟随重定向')
        if not isinstance(response.body, bytes) or len(response.body) > limit:
            fail('response_limit', '本机语料响应超过上限')
        return self._parse_response(response.body, space, len(texts))

    def embed(self, *_):
        fail('local_index_only', '语料编码批准不批准查询或外部网络')
