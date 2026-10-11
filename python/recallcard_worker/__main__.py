"""JSONL stdio 与显式文件入口；授权只来自启动参数，绝不来自请求正文。"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import sys
import tempfile

from .embedding import (
    MAX_JSON_BYTES, EmbeddingClient, EmbeddingError, EmbeddingSpace, NetworkApproval,
    _fields, _string, canonical, fail, index_corpus, load_json, scopes,
    search_index, validate_index, text_hash,
)
from .local_query import LocalQueryClient, LocalIndexClient
from .managed_cloud import ManagedCloudClient


class Worker:
    """每个可信宿主独占一个进程；不要把 stdin 直接暴露给模型或网页。"""

    def __init__(self, client: EmbeddingClient | None = None):
        self.client = client or EmbeddingClient()

    def handle(self, request: dict) -> dict:
        ident = None
        try:
            if not isinstance(request, dict):
                fail("invalid_request", "请求必须为 JSON 对象")
            ident = request.get("id")
            if isinstance(ident, str):
                _string(ident, 128, "id")
            elif type(ident) is not int or not 0 <= ident <= 2**53 - 1:
                ident = None
                fail("invalid_request", "id 必须为有界字符串或非负整数")
            result = self.dispatch(request)
            return {"id": ident, "ok": True, "result": result}
        except EmbeddingError as error:
            result = {"id": ident, "ok": False, "error": {"code": error.code, "message": str(error)},
                      "diagnostics": self.client.diagnostics()}
            if error.partial_index is not None:
                result["partial_index"] = error.partial_index
            return result
        except (TypeError, ValueError, OverflowError, RecursionError):
            return {"id": None, "ok": False, "error": {"code": "invalid_request", "message": "请求字段类型或大小无效"}}

        except Exception:
            return {"id": ident, "ok": False, "error": {"code": "worker_error", "message": "worker 内部异常；未记录原始异常或私密数据"}}

    def dispatch(self, request: dict) -> dict:
        op = request.get("op")
        if op == "embed_documents":
            _fields(request, {"id", "op", "space", "texts", "allowed_scopes"})
            if not isinstance(self.client, (LocalIndexClient, ManagedCloudClient)):
                fail("index_not_approved", "未配置独立当前 Memory 编码批准")
            space = EmbeddingSpace.from_dict(request["space"])
            vectors = self.client.embed_documents(space, request["texts"], request["allowed_scopes"])
            return {"space_signature": space.signature,
                    "encoded": [{"input_hash": text_hash(text), "vector": vector} for text, vector in zip(request["texts"], vectors)],
                    "diagnostics": self.client.diagnostics()}
        if op == "space_signature":
            _fields(request, {"id", "op", "space"})
            space = EmbeddingSpace.from_dict(request["space"])
            return {"space": asdict(space), "space_signature": space.signature}
        if op == "validate_index":
            _fields(request, {"id", "op", "index"})
            space, documents = validate_index(request["index"])
            return {"space_signature": space.signature, "documents": len(documents),
                    "complete": request["index"].get("complete", True)}
        if op == "index":
            _fields(request, {"id", "op", "space", "corpus"}, {"previous_index", "batch_size"})
            space = EmbeddingSpace.from_dict(request["space"])
            return index_corpus(space, request["corpus"], self.client, request.get("previous_index"),
                                batch_size=request.get("batch_size", 64))
        if op not in ("query", "query_vector"):
            fail("unknown_operation", "不支持此 worker 操作")
        payload_field = "query" if op == "query" else "query_vector"
        required = {"id", "op", "index", payload_field, "allowed_scopes", "allowed_refs"}
        if op == "query_vector":
            required.add("space_signature")
        _fields(request, required, {"limit", "lexical_refs", "expected_generation"})
        index = request["index"]
        space, _ = validate_index(index)
        if "expected_generation" in request and request["expected_generation"] != index["generation"]:
            fail("generation_mismatch", "当前语料 generation 与索引不一致，请回退文本检索")
        arguments = {"space_signature": space.signature if op == "query" else request["space_signature"],
                     "allowed_scopes": request["allowed_scopes"], "allowed_refs": request["allowed_refs"],
                     "limit": request.get("limit", 10), "lexical_refs": request.get("lexical_refs")}
        if op == "query_vector":
            return search_index(index, request["query_vector"], **arguments)
        # 联网前把全部 scope/ref、完整性、候选列表、预算字段检查完。
        space.prepare(request["query"], "query")
        initial = search_index(index, [1.0] + [0.0] * (space.dimensions - 1), **arguments)
        if not initial["results"]:
            return {**initial, "query_cache_hit": False, "diagnostics": self.client.diagnostics()}
        query_vector, cache_hit = self.client.query_vector(space, request["query"], request["allowed_scopes"])
        result = {**search_index(index, query_vector, **arguments), "query_cache_hit": cache_hit,
                  "diagnostics": self.client.diagnostics()}
        if isinstance(self.client, (LocalQueryClient, ManagedCloudClient)):
            result['encoded_query'] = {'vector': query_vector, 'input_hash': text_hash(space.prepare(request['query'], 'query'))}
        return result


def serve(worker: Worker, source, destination):
    """二进制流，一行一请求；超长行耗尽后恢复下一条，不保留整行。"""
    while True:
        line = source.readline(MAX_JSON_BYTES + 1)
        if not line:
            return
        if len(line) > MAX_JSON_BYTES:
            while line and not line.endswith(b"\n"):
                line = source.readline(65536)
            result = {"id": None, "ok": False, "error": {"code": "input_limit", "message": "请求行超过字节上限"}}
        else:
            try:
                result = worker.handle(load_json(line))
            except EmbeddingError as error:
                result = {"id": None, "ok": False, "error": {"code": error.code, "message": str(error)}}
        destination.write(canonical(result) + b"\n")
        destination.flush()


def read_json_file(path: str):
    if path == "-":
        data = sys.stdin.buffer.read(MAX_JSON_BYTES + 1)
    else:
        with open(path, "rb") as source:
            data = source.read(MAX_JSON_BYTES + 1)
    return load_json(data)


def atomic_json_file(path: str, value: dict):
    target = Path(path)
    if target.is_symlink():
        fail("unsafe_path", "拒绝覆盖符号链接缓存")
    parent = target.parent
    if not parent.is_dir():
        fail("invalid_path", "缓存目录不存在，请先建立私有本机目录")
    encoded = canonical(value) + b"\n"
    if len(encoded) > MAX_JSON_BYTES:
        fail("output_limit", "缓存超过文件字节上限")
    descriptor, temp_path = tempfile.mkstemp(prefix=".recallcard-embedding-", dir=parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp_path, target)
    finally:
        if os.path.exists(temp_path):
            os.unlink(temp_path)


class ChineseParser(argparse.ArgumentParser):
    def __init__(self, *args, **kwargs):
        kwargs.setdefault("add_help", False)
        kwargs.setdefault("prog", "recallcard-worker")
        super().__init__(*args, **kwargs)
        self._positionals.title = "位置参数"
        self._optionals.title = "选项"
        self.add_argument("-h", "--help", action="help", help="显示帮助并退出")

    def format_help(self):
        return super().format_help().replace("usage:", "用法:", 1)

    def error(self, message):
        self.exit(2, "参数无效；请使用 --help 检查所需参数与允许值\n")


def parser() -> argparse.ArgumentParser:
    result = ChineseParser(description="RecallCard 可选向量 worker；默认禁止联网，全部配置留在进程外")
    result.add_argument("--stdio", action="store_true", help="使用有界 JSONL stdin/stdout 协议")
    result.add_argument("--managed-cloud-once", action="store_true", help="本机宿主已持久预留的单请求编码；仍须独立目的地/范围/数据批准")
    result.add_argument("--allow-network", action="store_true", help="明确允许发送已选数据；仍须指定目的地、scope 和数据类型")
    result.add_argument("--approve-endpoint", help="本进程唯一允许接收数据的完整 HTTPS 接口地址")
    result.add_argument("--approve-scope", action="append", default=[], help="明确批准发送的 scope，可重复")
    result.add_argument("--approve-data", action="append", choices=["corpus", "query"], default=[], help="明确批准发送语料或查询，可重复")
    result.add_argument("--local-query-endpoint", help="显式配置无凭据的数字回环 embedding 查询端点")
    result.add_argument("--local-index-endpoint", help="独立批准的数字回环语料编码端点")
    result.add_argument("--local-index-scope", action="append", default=[], help="独立批准本机语料编码范围")
    result.add_argument("--local-query-scope", action="append", default=[], help="允许本机按需编码的 scope")
    result.add_argument("--max-local-calls", type=int, default=20, help="本机查询编码次数预算")
    result.add_argument("--max-local-bytes", type=int, default=65536, help="本机查询累计文本字节预算")
    result.add_argument("--key-env", default="RECALLCARD_EMBEDDING_API_KEY", help="只读取此环境变量的 key；不接收明文 key 参数")
    result.add_argument("--max-retries", type=int, default=2, help="单批最多重试次数，默认 2、最高 5")
    result.add_argument("--max-network-calls", type=int, default=100, help="整个进程的请求次数预算，包含重试")
    result.add_argument("--max-transmitted-bytes", type=int, default=1_048_576, help="整个进程累计发送文本 UTF-8 字节预算，包含重试")
    result.add_argument("--timeout", type=float, default=30.0, help="单次请求超时秒数，最高 120")
    result.add_argument("--price-per-million-tokens", type=float, help="用户提供的每百万 token 价格快照，仅用于成功响应用量估算")
    result.add_argument("--pricing-date", help="价格快照日期 YYYY-MM-DD")
    result.add_argument("--pricing-currency", help="价格快照的三位货币代码，例如 USD；有单价时必须提供")
    sub = result.add_subparsers(dest="command", parser_class=ChineseParser)
    build = sub.add_parser("index", help="从显式导出的语料文件构建本机派生索引")
    build.add_argument("--corpus", required=True, help="已审阅的语料 JSON 文件；- 表示 stdin")
    build.add_argument("--space", required=True, help="本机空间配置 JSON 文件；不得包含 key")
    build.add_argument("--cache", required=True, help="派生索引文件；请放在不参与同步的本机私有目录")
    build.add_argument("--batch-size", type=int, default=64, help="每批最多文档数，范围 1–64")
    return result


def main(argv=None) -> int:
    arguments = parser().parse_args(argv)
    try:
        if arguments.stdio == (arguments.command == "index"):
            fail("invalid_config", "请选择 --stdio 或 index 子命令之一")
        approved_scopes = scopes(arguments.approve_scope, empty=True)
        approval = NetworkApproval(arguments.allow_network, arguments.approve_endpoint,
                                   tuple(approved_scopes), tuple(set(arguments.approve_data)))
        client = EmbeddingClient(approval, key_env=arguments.key_env, max_retries=arguments.max_retries,
                                 max_network_calls=arguments.max_network_calls, max_transmitted_bytes=arguments.max_transmitted_bytes,
                                 timeout=arguments.timeout, price_per_million_tokens=arguments.price_per_million_tokens,
                                 pricing_date=arguments.pricing_date, pricing_currency=arguments.pricing_currency)
        if arguments.managed_cloud_once:
            if not arguments.stdio or not arguments.allow_network or arguments.local_query_endpoint or arguments.local_index_endpoint or arguments.local_query_scope or arguments.local_index_scope:
                fail("invalid_config", "单次云编码需独立启动，不能继承或混用本机配置")
            client = ManagedCloudClient(approval, key_env=arguments.key_env, timeout=arguments.timeout)
        if arguments.local_query_endpoint and arguments.local_index_endpoint:
            fail("invalid_config", "查询与语料编码使用独立 worker")
        if arguments.local_index_endpoint:
            if not arguments.stdio or arguments.allow_network or arguments.approve_endpoint or approved_scopes or arguments.approve_data or arguments.local_query_scope:
                fail("invalid_config", "本机语料配置不能混用其他批准")
            client = LocalIndexClient(arguments.local_index_endpoint, arguments.local_index_scope, max_calls=arguments.max_local_calls, max_bytes=arguments.max_local_bytes, timeout=arguments.timeout)
        elif arguments.local_index_scope:
            fail("invalid_config", "语料范围需要独立本机端点")
        if arguments.local_query_endpoint:
            if not arguments.stdio or arguments.allow_network or arguments.approve_endpoint or approved_scopes or arguments.approve_data:
                fail('invalid_config', '本机 query-only 与云批准/语料编码配置不能混用')
            client = LocalQueryClient(arguments.local_query_endpoint, arguments.local_query_scope,
                                      max_calls=arguments.max_local_calls, max_bytes=arguments.max_local_bytes,
                                      timeout=arguments.timeout)
        elif arguments.local_query_scope:
            fail('invalid_config', '本机查询 scope 需要明确的本机端点')
        if arguments.stdio:
            serve(Worker(client), sys.stdin.buffer, sys.stdout.buffer)
        else:
            # --space/--corpus 仅由用户显式指定；stdio 请求不能指定或枚举路径。
            space = EmbeddingSpace.from_dict(read_json_file(arguments.space))
            corpus = read_json_file(arguments.corpus)
            pending_path = arguments.cache + ".pending"
            previous = None
            if Path(pending_path).exists():
                candidate = read_json_file(pending_path)
                old_space, _ = validate_index(candidate)
                if old_space.signature == space.signature and candidate["generation"] == corpus.get("generation"):
                    previous = candidate
            if previous is None and Path(arguments.cache).exists():
                previous = read_json_file(arguments.cache)
            result = index_corpus(space, corpus, client, previous,
                                  checkpoint=lambda index: atomic_json_file(pending_path, index), batch_size=arguments.batch_size)
            atomic_json_file(arguments.cache, result["index"])
            if Path(pending_path).exists():
                Path(pending_path).unlink()
            sys.stdout.buffer.write(canonical({"ok": True, "diagnostics": result["diagnostics"]}) + b"\n")
        return 0
    except EmbeddingError as error:
        sys.stderr.write(json.dumps({"ok": False, "error": {"code": error.code, "message": str(error)}}, ensure_ascii=False) + "\n")
        return 2
    except (OSError, ValueError, RecursionError):
        sys.stderr.write('{"ok":false,"error":{"code":"io_error","message":"输入输出失败或配置无效；未记录路径、正文或凭据"}}\n')
        return 2
    except Exception:
        sys.stderr.write('{"ok":false,"error":{"code":"worker_error","message":"worker 内部异常；未记录原始异常或私密数据"}}\n')
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
