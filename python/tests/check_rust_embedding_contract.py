"""已编译 Rust CLI → 合成 Vault → 导出 → fake 向量 → 离线 stdio 检索。"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recallcard_worker.embedding import (
    EmbeddingClient, EmbeddingSpace, NetworkApproval, TransportResponse,
    canonical, index_corpus, validate_corpus,
)


def check(binary):
    binary = str(Path(binary).resolve(strict=True))
    space = EmbeddingSpace("fixture", "https://embedding.invalid/v1/embeddings", "fixture-model", 3,
                           "fixture-revision-1", "", "", "l2", "utf8-v1")
    with tempfile.TemporaryDirectory(prefix="recallcard-contract-") as directory:
        environment = {**os.environ, "RECALLCARD_STATE_DIR": directory + "/state",
                       "PYTHONPATH": str(Path(__file__).resolve().parents[1])}

        def run(*arguments, data=None):
            result = subprocess.run([binary, "--vault", directory + "/vault", *arguments],
                                    input=None if data is None else canonical(data), capture_output=True,
                                    env=environment, timeout=10)
            if result.returncode:
                raise AssertionError("Rust 合同测试调用失败：" + result.stderr.decode("utf-8", errors="replace"))
            return json.loads(result.stdout)

        run("init")
        event = run("capture", "--file", "-", data={
            "role": "user", "origin": "native", "scope": "project:fixture", "content": "用于接口验证的虚构记忆",
            "source": {"platform": "manual-web", "conversation_id": "fixture", "message_id": "fixture-1"},
        })
        run("memory", "add", "--file", "-", data={
            "content": "用于接口验证的虚构记忆", "source_refs": [event["id"]],
            "evidence": "user_explicit", "scope": "project:fixture",
        })
        source = run("embedding-export", "--scope", "project:fixture")
        assert len(validate_corpus(source, space)["documents"]) == 1

        requests = []

        def fake_transport(endpoint, body, key, timeout, response_limit):
            payload = json.loads(body)
            requests.append(payload)
            return TransportResponse(200, canonical({"model": space.model, "usage": {"prompt_tokens": 1},
                                     "data": [{"index": i, "embedding": [1, 0, 0]} for i, _ in enumerate(payload["input"])]}), {})

        client = EmbeddingClient(NetworkApproval(True, space.endpoint, ("project:fixture",), ("corpus",)),
                                 transport=fake_transport)
        with patch.dict(os.environ, {"RECALLCARD_EMBEDDING_API_KEY": "synthetic-in-memory-key"}):
            built = index_corpus(space, source, client)
        assert len(requests) == 1
        rebuilt = index_corpus(space, source, EmbeddingClient(), built["index"])
        assert rebuilt["diagnostics"]["network_calls"] == 0
        index = built["index"]
        request = {"id": 1, "op": "query_vector", "index": index, "query_vector": [1, 0, 0],
                   "space_signature": space.signature, "allowed_scopes": ["project:fixture"],
                   "allowed_refs": [d["ref"] for d in source["documents"]], "expected_generation": source["generation"]}
        denied = {**request, "id": 2, "allowed_refs": []}
        outdated = {**request, "id": 3, "expected_generation": "stale-fixture-generation"}
        environment.pop("RECALLCARD_EMBEDDING_API_KEY", None)
        result = subprocess.run([sys.executable, "-m", "recallcard_worker", "--stdio"],
                                input=b"\n".join(canonical(r) for r in [request, denied, outdated]) + b"\n",
                                capture_output=True, env=environment, timeout=10)
        assert result.returncode == 0 and not result.stderr
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        assert responses[0]["ok"] and responses[0]["result"]["results"][0]["ref"] == source["documents"][0]["ref"]
        assert responses[0]["result"]["results"][0]["score"] == 1.0
        assert responses[1]["ok"] and not responses[1]["result"]["results"]
        assert not responses[2]["ok"] and responses[2]["error"]["code"] == "generation_mismatch"
    print("通过：Rust 合成 Memory 导出、Python 哈希合同、fake 增量构建、离线缓存复用、JSONL query_vector、撤权过滤和过期 generation 拒绝；没有联网")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("用法：python3 python/tests/check_rust_embedding_contract.py <已编译的 recallcard 路径>")
    check(sys.argv[1])
