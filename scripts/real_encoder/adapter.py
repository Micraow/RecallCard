"""CI 专用合成白名单适配器；不是用户可配置的模型服务。"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import importlib.metadata
import ipaddress
import json
import math
import os
import sys
import time

from common import HERE, RUNTIME, read_json, require_ci, verify_file


def deny_external_network(event, args):
    # Python audit 是本进程防线，不冒充整个 runner 的网络沙箱。
    if event in {"socket.connect", "socket.sendto"}:
        address = args[-1]
        if not isinstance(address, tuple) or len(address) < 2:
            raise RuntimeError("禁止编码进程对外联网")
        try:
            allowed = ipaddress.ip_address(address[0]).is_loopback
        except ValueError:
            allowed = False
        if not allowed:
            raise RuntimeError("禁止编码进程对外联网")
    elif event == "socket.getaddrinfo":
        if args[0] not in ("127.0.0.1", "::1", None):
            raise RuntimeError("禁止编码进程外部 DNS")


def check_lengths(tokenizer, inputs, maximum=512):
    lengths = [len(tokenizer.encode(text, add_special_tokens=True).ids) for text in inputs]
    if not lengths or max(lengths) > maximum:
        raise ValueError("input_token_limit")
    return lengths


def check_vectors(vectors, count, dimensions=512):
    output = []
    for vector in vectors:
        values = [float(value) for value in vector]
        if len(values) != dimensions or not all(math.isfinite(value) for value in values):
            raise ValueError("invalid_vector")
        if not math.isclose(sum(value * value for value in values), 1.0, abs_tol=0.002):
            raise ValueError("vector_not_normalized")
        output.append(values)
    if len(output) != count:
        raise ValueError("vector_count_mismatch")
    return output


class Encoder:
    def __init__(self, model_dir, allowed_inputs):
        require_ci()
        self.lock = read_json(HERE / "model-lock.json")
        self.files = {name: verify_file(model_dir / name, expected) for name, expected in self.lock["files"].items()}
        self.runtime = {name: importlib.metadata.version(name) for name in RUNTIME}
        if self.runtime != RUNTIME:
            raise RuntimeError("编码运行时版本不符")
        self.allowed_inputs = frozenset(allowed_inputs)
        self.requests = 0
        self.inputs = 0
        self.input_bytes = 0
        self.max_tokens_seen = 0
        self.encoding_seconds = []
        self.max_rss_kib = 0
        os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1",
                          TOKENIZERS_PARALLELISM="false", OMP_NUM_THREADS="2", OPENBLAS_NUM_THREADS="2")
        sys.addaudithook(deny_external_network)
        started = time.monotonic()
        from fastembed import TextEmbedding
        from fastembed.common.preprocessor_utils import load_tokenizer
        # 独立 tokenizer，先按相同配置注册 special tokens，再彻底关闭截断和 padding。
        self.tokenizer, _ = load_tokenizer(model_dir)
        self.tokenizer.no_truncation()
        self.tokenizer.no_padding()
        check_lengths(self.tokenizer, list(self.allowed_inputs), self.lock["max_tokens"])
        try:
            check_lengths(self.tokenizer, ["合成测试" * 600], self.lock["max_tokens"])
        except ValueError:
            self.oversize_rejected_before_embedding = True
        else:
            raise RuntimeError("token preflight 未能拒绝过长输入")
        self.model = TextEmbedding(model_name=self.lock["model"], threads=2,
                                   providers=["CPUExecutionProvider"], local_files_only=True,
                                   specific_model_path=str(model_dir), cache_dir=str(model_dir))
        # 双重防线：模型 tokenizer 也关闭截断，每次批处理前仍执行独立 preflight。
        self.model.model.tokenizer.no_truncation()
        self.load_seconds = time.monotonic() - started
        self.first_embedding_seconds = None

    def encode(self, payload):
        if (not isinstance(payload, dict) or set(payload) != {"model", "input", "dimensions", "encoding_format"}
                or payload["model"] != self.lock["model"] or payload["dimensions"] != 512
                or payload["encoding_format"] != "float"):
            raise ValueError("invalid_request")
        texts = payload["input"]
        if (not isinstance(texts, list) or not 1 <= len(texts) <= 8
                or not all(isinstance(text, str) and text in self.allowed_inputs for text in texts)):
            raise ValueError("synthetic_allowlist_required")
        lengths = check_lengths(self.tokenizer, texts, self.lock["max_tokens"])
        if self.requests >= 128 or self.inputs + len(texts) > 256:
            raise ValueError("request_budget_exceeded")
        self.requests += 1
        self.inputs += len(texts)
        self.input_bytes += sum(len(text.encode()) for text in texts)
        self.max_tokens_seen = max(self.max_tokens_seen, *lengths)
        started = time.monotonic()
        # RecallCard 已对查询加一次指令；禁止 query_embed 或再次拼接 prefix。
        vectors = check_vectors(self.model.embed(texts, batch_size=8, parallel=None), len(texts))
        elapsed = time.monotonic() - started
        self.encoding_seconds.append(elapsed)
        if self.first_embedding_seconds is None:
            self.first_embedding_seconds = elapsed
        return {"object": "list", "model": self.lock["model"],
                "data": [{"index": i, "object": "embedding", "embedding": row} for i, row in enumerate(vectors)],
                "usage": {"prompt_tokens": sum(lengths), "total_tokens": sum(lengths)}}


def make_server(encoder):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            self.connection.settimeout(15)
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if self.path != "/v1/embeddings" or not 0 < size <= 65536:
                    raise ValueError("invalid_request")
                raw = self.rfile.read(size)
                if len(raw) != size:
                    raise ValueError("incomplete_request")
                result = encoder.encode(json.loads(raw))
                status = 200
            except Exception:
                # 不记录正文、模型异常或路径。任何失败由生产 client fail-closed。
                result, status = {"error": "synthetic_encoder_request_rejected"}, 400
            data = json.dumps(result, allow_nan=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
    return HTTPServer(("127.0.0.1", 0), Handler)
