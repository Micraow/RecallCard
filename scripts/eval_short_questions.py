#!/usr/bin/env python3
"""固定原创合成集的真实 CLI 来源命中诊断；只创建新 Vault，不使用模型或用户资料。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def evaluate(binary: Path, output: Path, allow_dirty: bool = False):
    binary = binary.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=False)
    home = output / "isolated-home"
    home.mkdir()
    vault = output / "vault"
    receipts = output / "receipts"
    receipts.mkdir()
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home),
           "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "data"),
           "LANG": "C.UTF-8", "TZ": "UTC"}

    def save(path, value):
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")

    def run(name, args):
        command = [str(binary), "--vault", str(vault), "--json", *args]
        result = subprocess.run(command, env=env, cwd=output, capture_output=True, text=True, timeout=30)
        save(receipts / f"{name}.json", {"command": command, "returncode": result.returncode,
                                        "stdout": result.stdout, "stderr": result.stderr})
        if result.returncode:
            raise RuntimeError(f"合成 CLI 验收失败：{name}，详见独立收据")
        return json.loads(result.stdout)

    fixtures = Path(__file__).resolve().parents[1] / "crates/recallcard/tests/fixtures/short-question"
    corpus = json.loads((fixtures / "corpus.json").read_text())
    queries = json.loads((fixtures / "queries.json").read_text())
    run("setup", ["setup"])
    status = run("status", ["status"])["result"]
    build = status["build"]
    if not allow_dirty and (build["dirty"] or build["commit"] == "unknown"):
        raise RuntimeError("正式对照必须使用可识别的干净构建；开发诊断可显式使用 --allow-dirty")
    assert not status["service"]["running"]
    assert status["background"]["config"]["provider"] is None
    assert status["background"]["config"]["consent"] is None
    source_file = output / "synthetic.jsonl"
    with source_file.open("w") as stream:
        for source in corpus:
            stream.write(json.dumps({"scope": "personal", "role": "user", "origin": "user_input",
                "occurred_at": "2026-10-13T00:00:00Z", "content": source["text"],
                "source": {"platform": "manual-web", "conversation_id": "synthetic-short-question",
                           "message_id": source["source_id"]},
                "capture": {"completeness": "partial", "reason": "original_synthetic_fixture"}}, ensure_ascii=False) + "\n")
    imported = run("import", ["import", "--format", "manual-jsonl", "--file", str(source_file), "--scope", "personal"])
    assert imported["events_added"] == len(corpus) == 12
    refs = dict(zip(imported["refs"], [source["source_id"] for source in corpus], strict=True))
    for reference, source in zip(imported["refs"], corpus, strict=True):
        original = run(source["source_id"] + "_source", ["sources", reference])
        assert isinstance(original, list) and len(original) == 1
        assert "event:" + original[0]["id"] == reference
        assert original[0]["source"]["message_id"] == source["source_id"]
        assert original[0]["content"] == source["text"]
    all_results = {}
    summaries = {}
    for policy in ["default", "question_intent"]:
        results = []
        for case in queries:
            mode = "natural_question" if policy == "question_intent" and case["intent"] == "question" else "exact"
            result = run(policy + "_" + case["id"], ["search", case["query"], "--target", "events",
                "--limit", "20", "--budget-bytes", "32768", "--lexical-match", mode])
            assert not result["truncated"]
            assert result["coverage"]["semantic_search"] == "unavailable"
            returned = [refs[row["ref"]] for row in result["results"]]
            expected = case["expected_sources"]
            hits = sorted(set(returned) & set(expected))
            results.append({**case, "mode": mode, "returned_sources": returned, "source_hits": hits,
                "missing_sources": sorted(set(expected) - set(returned)),
                "non_expected_sources": sorted(set(returned) - set(expected))})
        summaries[policy] = {"positive_queries": sum(bool(r["expected_sources"]) for r in results),
            "positive_queries_with_source_hit": sum(bool(r["source_hits"]) for r in results),
            "returned_sources": sum(len(r["returned_sources"]) for r in results),
            "non_expected_sources": sum(len(r["non_expected_sources"]) for r in results),
            "by_category": {category: {"queries": sum(r["category"] == category for r in results),
                "source_hits": sum(bool(r["source_hits"]) for r in results if r["category"] == category),
                "nonempty_negatives": sum(bool(r["returned_sources"]) and not r["expected_sources"] for r in results if r["category"] == category)}
                for category in sorted({r["category"] for r in results})}}
        all_results[policy] = results
    summary = {"schema": "recallcard.synthetic-short-question/1", "build": build,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "fixture_sha256": {name: hashlib.sha256((fixtures / name).read_bytes()).hexdigest() for name in ["corpus.json", "queries.json"]},
        "corpus_sources": len(corpus), "queries": len(queries), "synthetic_only": True,
        "private_data_used": False, "provider_configured": False, "semantic_search": "unavailable",
        "metrics": summaries, "scope": "固定小样本诊断，不是生产召回率或原旧gold；候选命中不等于答案正确"}
    save(output / "summary.json", summary)
    save(output / "results.json", all_results)
    assert summaries["question_intent"]["by_category"]["short_unpunctuated_question"]["source_hits"] == 9
    assert summaries["question_intent"]["by_category"]["exact_near_miss"]["nonempty_negatives"] == 0
    assert summaries["question_intent"]["by_category"]["negative_unrelated"]["nonempty_negatives"] == 0
    for before, after in zip(all_results["default"], all_results["question_intent"], strict=True):
        if before["intent"] == "exact":
            assert before["returned_sources"] == after["returned_sources"]
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="全新目录；已有目录拒绝覆盖")
    parser.add_argument("--allow-dirty", action="store_true", help="仅开发诊断，不得声称是干净发布构建")
    options = parser.parse_args()
    evaluate(options.binary, options.output.resolve(), options.allow_dirty)
