"""真实模型验收的纯函数；导入不下载、不加载模型、不启动监听。"""
import hashlib
import json
import os
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
FIXTURES = REPO / "crates/recallcard/tests/fixtures/short-question"
HOLDOUT = REPO / "scripts/query_diagnostics/holdout.json"
HOLDOUT_SHA256 = "c2c130e503bc8e753884865f48bed5fddc2dc95c1ddb0f6947b5a9e131883515"
HOLDOUT_VARIANTS = ("events_semantic_attachment_lexical", "events_and_attachments_semantic")
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


def canonical_digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                    separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def holdout_datasets():
    # 此文件已经观察过：只固定回放，不再称为新的盲测或改写原始标注。
    if digest(HOLDOUT) != HOLDOUT_SHA256:
        raise ValueError("fixed_holdout_identity_mismatch")
    original = read_json(HOLDOUT)
    corpus, queries = original["sources"], original["queries"]
    if (original["schema_version"] != "recall-query-holdout-v1"
            or len(corpus) != 24 or len(queries) != 64):
        raise ValueError("fixed_holdout_schema_mismatch")
    by_id = {source["source_id"]: source for source in corpus}
    scopes = {source["scope"] for source in corpus}
    if len(by_id) != len(corpus) or len(scopes) != 6:
        raise ValueError("fixed_holdout_sources_invalid")
    for source in corpus:
        if (source["kind"] not in {"event", "attachment"} or type(source["revoked"]) is not bool
                or not isinstance(source["text"], str) or not 0 < len(source["text"].encode()) <= 1024
                or (source["kind"] == "attachment" and source["role"] is not None)
                or (source["kind"] == "event" and source["role"] not in {"user", "assistant"})):
            raise ValueError("fixed_holdout_source_invalid")
    seen = set()
    for query in queries:
        if (query["id"] in seen or query["lexical_intent"] not in {"exact", "question"}
                or not isinstance(query["query"], str) or len(query["query"].encode()) > 4096
                or not set(query["visible_scopes"]) <= scopes):
            raise ValueError("fixed_holdout_query_invalid")
        seen.add(query["id"])
        permitted = {key for key, source in by_id.items()
                     if not source["revoked"] and source["scope"] in query["visible_scopes"]}
        if not set(query["expected_sources"]) <= permitted:
            raise ValueError("fixed_holdout_qrels_invalid")
    return [{"name": "independent-synthetic-attachment-holdout", "suite": "attachment-holdout",
             "synthetic_only": True, "variants": list(HOLDOUT_VARIANTS),
             "corpus": corpus, "queries": queries}]


def datasets(suite="legacy"):
    if suite == "attachment-holdout":
        return holdout_datasets()
    if suite != "legacy":
        raise ValueError("unknown_synthetic_suite")
    primary = {"name": "original-synthetic-12-source-52-question",
               "corpus": read_json(FIXTURES / "corpus.json"), "queries": read_json(FIXTURES / "queries.json")}
    assert len(primary["corpus"]) == 12 and len(primary["queries"]) == 52
    assert sum(q["category"] == "synonym_question" for q in primary["queries"]) == 9
    extra = read_json(HERE / "edge-cases.json")
    return [primary, {"name": "separate-new-negation-correction-diagnostic", **extra}]


def allowed_inputs(fixture_sets, query_instruction):
    return ({source["text"] for data in fixture_sets for source in data["corpus"]
             if not source.get("revoked", False)}
            | {query_instruction + "\n" + row["query"] for data in fixture_sets for row in data["queries"]
               if data.get("suite") != "attachment-holdout"
               or (row["query"].strip() and len(row["visible_scopes"]) <= 4)})


def holdout_request_budget(data):
    # 一scope一SQLite，最多缓存32条query。两variant保持space与原问题不变。
    scoped_queries = {(scope, row["query"]) for row in data["queries"]
                      if row["query"].strip() and len(set(row["visible_scopes"])) <= 4
                      for scope in row["visible_scopes"]}
    by_scope = {scope: sum(s == scope for s, _ in scoped_queries)
                for scope in sorted({source["scope"] for source in data["corpus"]})}
    documents = sum(not source["revoked"] for source in data["corpus"])
    # 本固定集正文均为一个1024-byte chunk；每来源最多一次编码。
    bound = documents + len(scoped_queries)
    if max(by_scope.values(), default=0) > 32 or bound > 128:
        raise ValueError("paired_holdout_exceeds_existing_request_budget")
    return {"non_revoked_document_inputs": documents, "unique_scoped_queries": len(scoped_queries),
            "unique_queries_per_scope": by_scope, "paired_embedding_request_upper_bound": bound,
            "paired_embedded_input_upper_bound": bound,
            "assumption": "同Vault同scope同embedding space；第二variant复用query与Event向量，仅新增附件向量。"}


def summarize(rows):
    def group(items):
        positives = [r for r in items if r["expected_sources"]]
        negatives = [r for r in items if not r["expected_sources"]]
        result = {"queries": len(items), "positive_queries": len(positives), "negative_queries": len(negatives),
                  "negative_queries_with_any_candidate": sum(bool(r["returned_sources"]) for r in negatives),
                  "non_expected_candidates_all": sum(len(set(r["returned_sources"]) - set(r["expected_sources"])) for r in items)}
        result["negative_nonempty_rate"] = (result["negative_queries_with_any_candidate"] / len(negatives)
                                            if negatives else None)
        returned_kinds = [kind for row in items for kind in row.get("returned_kinds", [])]
        result["returned_candidates_by_kind"] = {kind: returned_kinds.count(kind) for kind in sorted(set(returned_kinds))}
        for k in (1, 3, 5):
            result[f"positive_source_hit_at_{k}"] = sum(bool(set(r["returned_sources"][:k]) & set(r["expected_sources"])) for r in positives)
            result[f"positive_source_hit_rate_at_{k}"] = (result[f"positive_source_hit_at_{k}"] / len(positives)
                                                         if positives else None)
            returned = sum(len(r["returned_sources"][:k]) for r in items)
            relevant = sum(len(set(r["returned_sources"][:k]) & set(r["expected_sources"])) for r in items)
            result[f"returned_candidates_at_{k}"] = returned
            result[f"annotated_source_precision_at_{k}"] = relevant / returned if returned else None
            result[f"non_expected_candidates_at_{k}"] = sum(len(set(r["returned_sources"][:k]) - set(r["expected_sources"])) for r in items)
            result[f"obsolete_or_conflicting_at_{k}"] = sum(bool(set(r["returned_sources"][:k]) & set(r.get("obsolete_or_conflicting_sources", []))) for r in items)
        return result
    if any(len(r["returned_sources"]) != len(set(r["returned_sources"])) for r in rows):
        raise ValueError("duplicate_candidate_source")
    def kind(row):
        kinds = row.get("source_kind_expected")
        if isinstance(kinds, str):
            return kinds
        return "+".join(sorted(set(kinds))) if kinds else ("no_answer" if not row["expected_sources"] else "unspecified")
    return {"all": group(rows),
            "by_category": {c: group([r for r in rows if r["category"] == c]) for c in sorted({r["category"] for r in rows})},
            "by_expected_source_kind": {k: group([r for r in rows if kind(r) == k]) for k in sorted({kind(r) for r in rows})},
            "by_execution": {"executed": group([r for r in rows if r.get("route_executed", True)]),
                             "not_executed": group([r for r in rows if not r.get("route_executed", True)])},
            "by_query_status": {s: group([r for r in rows if r.get("query_status", "unspecified") == s])
                                for s in sorted({r.get("query_status", "unspecified") for r in rows})}}


def paired_deltas(before, after):
    if len(before) != len(after) or len({row["id"] for row in before}) != len(before):
        raise ValueError("paired_query_set_mismatch")
    right = {row["id"]: row for row in after}
    if len(right) != len(after) or right.keys() != {row["id"] for row in before}:
        raise ValueError("paired_query_set_mismatch")
    result = {f"{change}_source_hits_at_{k}": [] for k in (1, 3, 5) for change in ("new", "lost")}
    result.update(introduced_no_answer_candidates=[], removed_no_answer_candidates=[], changed_candidate_order=[],
                  route_status_changed=[])
    for left in before:
        row = right[left["id"]]
        if (left["expected_sources"] != row["expected_sources"]
                or left.get("visible_scopes") != row.get("visible_scopes")):
            raise ValueError("paired_query_annotation_mismatch")
        ident, expected = left["id"], set(left["expected_sources"])
        if expected:
            for k in (1, 3, 5):
                old, new = bool(set(left["returned_sources"][:k]) & expected), bool(set(row["returned_sources"][:k]) & expected)
                if old != new:
                    result[f"{'new' if new else 'lost'}_source_hits_at_{k}"].append(ident)
        elif bool(left["returned_sources"]) != bool(row["returned_sources"]):
            result["introduced_no_answer_candidates" if row["returned_sources"] else "removed_no_answer_candidates"].append(ident)
        if left["returned_sources"] != row["returned_sources"]:
            result["changed_candidate_order"].append(ident)
        if (left.get("route_executed"), left.get("query_status")) != (row.get("route_executed"), row.get("query_status")):
            result["route_status_changed"].append(ident)
    return result
