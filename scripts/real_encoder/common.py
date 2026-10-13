"""真实模型验收的纯函数；导入不下载、不加载模型、不启动监听。"""
import hashlib
import json
import os
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
FIXTURES = REPO / "crates/recallcard/tests/fixtures/short-question"
RUNTIME = {"fastembed": "0.9.0", "numpy": "2.2.6", "onnxruntime": "1.23.2",
           "tokenizers": "0.22.1", "huggingface-hub": "0.36.0"}


def require_ci():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("仅允许显式触发的官方 GitHub hosted runner；本地不执行模型验收")


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def verify_file(path, expected):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != expected["bytes"]:
        raise ValueError("模型文件长度或类型不符")
    sha256 = digest(path)
    if "sha256" in expected and sha256 != expected["sha256"]:
        raise ValueError("模型 SHA256 不符")
    if "git_blob_sha1" in expected:
        data = path.read_bytes()
        actual = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if actual != expected["git_blob_sha1"]:
            raise ValueError("官方 Git blob hash 不符")
    if not ({"sha256", "git_blob_sha1"} & expected.keys()):
        raise ValueError("禁止无 hash 的模型文件")
    return {"bytes": expected["bytes"], "sha256": sha256,
            **({"verified_git_blob_sha1": expected["git_blob_sha1"]} if "git_blob_sha1" in expected else {})}


def datasets():
    primary = {"name": "original-synthetic-12-source-52-question",
               "corpus": read_json(FIXTURES / "corpus.json"), "queries": read_json(FIXTURES / "queries.json")}
    assert len(primary["corpus"]) == 12 and len(primary["queries"]) == 52
    assert sum(q["category"] == "synonym_question" for q in primary["queries"]) == 9
    extra = read_json(HERE / "edge-cases.json")
    return [primary, {"name": "separate-new-negation-correction-diagnostic", **extra}]


def summarize(rows):
    def group(items):
        positives = [r for r in items if r["expected_sources"]]
        negatives = [r for r in items if not r["expected_sources"]]
        result = {"queries": len(items), "positive_queries": len(positives), "negative_queries": len(negatives),
                  "negative_queries_with_any_candidate": sum(bool(r["returned_sources"]) for r in negatives),
                  "non_expected_candidates_all": sum(len(set(r["returned_sources"]) - set(r["expected_sources"])) for r in items)}
        for k in (1, 3, 5):
            result[f"positive_source_hit_at_{k}"] = sum(bool(set(r["returned_sources"][:k]) & set(r["expected_sources"])) for r in positives)
            result[f"non_expected_candidates_at_{k}"] = sum(len(set(r["returned_sources"][:k]) - set(r["expected_sources"])) for r in items)
            result[f"obsolete_or_conflicting_at_{k}"] = sum(bool(set(r["returned_sources"][:k]) & set(r.get("obsolete_or_conflicting_sources", []))) for r in items)
        return result
    return {"all": group(rows), "by_category": {c: group([r for r in rows if r["category"] == c]) for c in sorted({r["category"] for r in rows})}}
