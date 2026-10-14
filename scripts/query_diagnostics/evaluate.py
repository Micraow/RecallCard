#!/usr/bin/env python3
"""独立合成查询诊断：实际生产 Rust 排名核、冻结消融与未参与调参的留出集。"""
from __future__ import annotations
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from candidates import MODES, han, projection, search, visible_sources
from production import ProductionRank


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def validate(fixture):
    sources, queries = fixture["sources"], fixture["queries"]
    assert 1 <= len(sources) <= 256 and 1 <= len(queries) <= 512
    by_id = {}
    for source in sources:
        assert re.fullmatch(r"[A-Za-z0-9_-]{1,80}", source["source_id"])
        assert source["source_id"] not in by_id
        assert source["kind"] in {"event", "attachment"}
        assert type(source["revoked"]) is bool and isinstance(source["scope"], str)
        assert isinstance(source["text"], str) and 0 < len(source["text"].encode()) <= 32768
        for key in ("title", "filename"):
            assert source.get(key) is None or isinstance(source[key], str) and len(source[key].encode()) <= 1024
        by_id[source["source_id"]] = source
    seen = set()
    for query in queries:
        assert query["id"] not in seen
        seen.add(query["id"])
        assert isinstance(query["query"], str) and len(query["query"].encode()) <= 4096
        assert query["intent"] in {"exact", "question"} and isinstance(query["category"], str)
        expected = query["expected_sources"]
        assert isinstance(expected, list) and len(expected) == len(set(expected))
        assert set(expected) <= {source["source_id"] for source in visible_sources(sources, query)}
    return fixture


def known_fixture(repo):
    directory = repo / "crates/recallcard/tests/fixtures/short-question"
    original = json.loads((directory / "corpus.json").read_text())
    return validate({"schema": "recallcard.query-diagnostic-fixture/1", "synthetic_only": True,
        "sources": [{**source, "kind": "event", "scope": "personal", "revoked": False,
                     "title": "", "filename": "", "role": "user"} for source in original],
        "queries": json.loads((directory / "queries.json").read_text())})


def measure(rows):
    positive = [row for row in rows if row["expected_sources"]]
    negative = [row for row in rows if not row["expected_sources"]]
    result = {"queries": len(rows), "positive_queries": len(positive), "no_answer_queries": len(negative)}
    for k in (1, 3, 5):
        positives_hit = sum(bool(set(row["returned"][:k]) & set(row["expected_sources"])) for row in positive)
        returned = sum(len(row["returned"][:k]) for row in rows)
        relevant = sum(len(set(row["returned"][:k]) & set(row["expected_sources"])) for row in rows)
        result[f"source_hit@{k}"] = positives_hit
        result[f"returned@{k}"] = returned
        result[f"non_expected_candidates@{k}"] = returned - relevant
        result[f"annotated_source_precision@{k}"] = relevant / returned if returned else None
        result[f"no_answer_nonempty@{k}"] = sum(bool(row["returned"][:k]) for row in negative)
    return result


def evaluate_fixture(rank, fixture):
    strategies = {}
    for mode in MODES:
        rows = []
        for case in fixture["queries"]:
            ranked, explanation = search(rank, mode, fixture["sources"], case)
            returned = [row["id"] for row in ranked]
            permitted = {source["source_id"] for source in visible_sources(fixture["sources"], case)}
            assert set(returned) <= permitted
            rows.append({"id": case["id"], "category": case["category"], "intent": case["intent"],
                         "expected_sources": case["expected_sources"], "returned": returned,
                         "scores": [row["score"] for row in ranked], "explanation": explanation})
        strategies[mode] = {"metrics": measure(rows), "by_category": {category: measure([row for row in rows if row["category"] == category])
                             for category in sorted({row["category"] for row in rows})}, "results": rows}
    # 成对变化防止相同总命中掩盖“救回一题、丢掉另一题”。
    baseline = strategies["production_question_intent"]["results"]
    for mode, result in strategies.items():
        deltas = {"new_source_hits_at5": [], "lost_source_hits_at5": [],
                  "introduced_no_answer_candidates": [], "removed_no_answer_candidates": []}
        for before, after in zip(baseline, result["results"], strict=True):
            positive = bool(before["expected_sources"])
            left = bool(set(before["returned"][:5]) & set(before["expected_sources"])) if positive else bool(before["returned"])
            right = bool(set(after["returned"][:5]) & set(after["expected_sources"])) if positive else bool(after["returned"])
            if left != right:
                key = ("new_source_hits_at5" if right else "lost_source_hits_at5") if positive else ("introduced_no_answer_candidates" if right else "removed_no_answer_candidates")
                deltas[key].append(before["id"])
        result["paired_deltas_vs_question_intent"] = deltas
    # exact 输入保持原生产行为。所有实验仅在显式 question 上有意义。
    for mode in MODES[2:]:
        for before, after in zip(baseline, strategies[mode]["results"], strict=True):
            if before["intent"] == "exact":
                assert before["returned"] == after["returned"], (mode, before["id"], "exact changed")
    # 字段/意图只能改变已有候选顺序，不可凭提示制造来源。
    for before, after in zip(strategies["normalized_question"]["results"], strategies["normalized_body_intent_weighted"]["results"], strict=True):
        assert set(before["returned"]) == set(after["returned"])
    return strategies


def overlap_report(rank, fixture):
    by_id = {source["source_id"]: source for source in fixture["sources"]}
    rows = []
    for case in fixture["queries"]:
        if case["category"] != "synonym_question":
            continue
        terms = rank.terms(case["query"], "natural_question")
        for ident in case["expected_sources"]:
            fields = [by_id[ident]["text"], *projection(by_id[ident])["entities"]]
            shared = [term for term in terms if len(term) == 2 and all(han(c) for c in term) and any(term in field for field in fields)]
            rows.append({"query_id": case["id"], "source_id": ident, "shared_han_bigrams": shared,
                         "query_terms": terms, "zero_overlap": not shared})
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--out", type=Path, required=True, help="全新输出目录，必须位于/tmp")
    parser.add_argument("--holdout", type=Path, default=Path(__file__).with_name("holdout.json"))
    parser.add_argument("--rustc", default="rustc")
    args = parser.parse_args()
    repo, output = args.repo.resolve(), args.out.resolve()
    assert output.is_relative_to(Path("/tmp")), "诊断只在/tmp创建全新目录"
    output.mkdir(parents=True, exist_ok=False)
    holdout_bytes = args.holdout.read_bytes()
    lock = json.loads(Path(__file__).with_name("experiment-lock.json").read_text())
    assert digest(Path(__file__).with_name("candidates.py").read_bytes()) == lock["candidate_sha256"], "候选已改变：须建立新的实验版本与留出集"
    assert digest(holdout_bytes) == lock["holdout_sha256"], "留出集身份不匹配"

    holdout = json.loads(holdout_bytes)
    for case in holdout["queries"]:
        case["semantic_intent"] = case["intent"]
        case["intent"] = case["lexical_intent"]
    holdout = validate(holdout)
    known = known_fixture(repo)
    rank = ProductionRank(repo, output / "rank", rustc=args.rustc)
    results = {name: evaluate_fixture(rank, fixture) for name, fixture in [("known_short_questions", known), ("independent_holdout", holdout)]}
    # 复现已有实际 CLI 诊断的完整可见来源数量，不把默认短句与显式意图混为一谈。
    known_results = results["known_short_questions"]
    for policy, expected in [("production_default", 22), ("production_question_intent", 31)]:
        hits = sum(bool(set(row["returned"]) & set(row["expected_sources"])) for row in known_results[policy]["results"])
        assert hits == expected, (policy, hits, "production replay drift")
    code = {path.name: digest(path.read_bytes()) for path in Path(__file__).parent.glob("*.py")}
    summary = {"schema": "recallcard.query-diagnostics-report/1", "synthetic_only": True,
        "model_used": False, "production_defaults_changed": False,
        "scope": "排名核回放和独立合成投影；不等于完整权限、分块、最终字节预算或真实用户验收",
        "precision_note": "非预期候选包括同主题但无答案的来源，不等于系统生成错误答案；标注不是穷尽语义相关性",
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=repo, text=True).strip()),
        "rank_identity": {key: value for key, value in rank.identity.items()
                          if key not in {"binary", "generated_rust", "compile_command"}},
        "experiment_lock": lock, "candidate_and_driver_sha256": code,
        "fixture_sha256": {"known": digest(canonical(known)), "holdout_file": digest(holdout_bytes)},
        "known_synonym_overlap": overlap_report(rank, known),
        "sets": {name: {mode: {key: value for key, value in result.items() if key != "results"}
                         for mode, result in strategies.items()} for name, strategies in results.items()}}
    for name, value in [("summary.json", summary), ("results.json", results)]:
        (output / name).write_bytes(canonical(value) + b"\n")
    print(json.dumps({"known": summary["sets"]["known_short_questions"], "holdout": summary["sets"]["independent_holdout"]}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
