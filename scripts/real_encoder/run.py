"""CI-only：真实小编码器经过现有只读 facade 的固定合成验收。"""
import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import resource
import re
import signal
import statistics
import subprocess
import sys
import threading
import time

from common import (FIXTURES, HERE, HOLDOUT, HOLDOUT_VARIANTS, REPO, allowed_inputs, canonical_digest,
                    datasets, digest, holdout_request_budget, paired_deltas, read_json, require_ci,
                    save_json, summarize)

RSS_CAP = 1280 * 1024 * 1024
TIMEOUT = 240


def rss_tree_bytes(root):
    """Linux RSS 求和有共享页重复；仅统计本次进程及子孙，不导出进程信息。"""
    records = {}
    for status in Path("/proc").glob("[0-9]*/status"):
        try:
            data = dict(line.split(":", 1) for line in status.read_text().splitlines() if ":" in line)
            records[int(status.parent.name)] = (int(data["PPid"]), int(data.get("VmRSS", "0 kB").split()[0]) * 1024)
        except (OSError, ValueError, KeyError):
            continue
    family = {root}
    while True:
        grown = family | {pid for pid, (parent, _) in records.items() if parent in family}
        if grown == family:
            return sum(records.get(pid, (0, 0))[1] for pid in family)
        family = grown


def runtime_lock(pip_report):
    report = read_json(pip_report)
    rows = []
    for item in report["install"]:
        rows.append({"name": item["metadata"]["name"], "version": item["metadata"]["version"],
                     "archive_hashes": item["download_info"]["archive_info"]["hashes"]})
    return {"python": platform.python_version(), "platform": "linux-x86_64", "packages": sorted(rows, key=lambda x: x["name"].lower()),
            "note": "本轮完整安装解析与官方包摘要；模型空间关键运行时另有固定版本断言。"}


def verify_rows(rows, fixture, *, strict_route=False):
    """绑定原始qrels与来源身份；不按返回候选修订标注或把拒绝当已执行。"""
    queries = {row["id"]: row for row in fixture["queries"]}
    sources = {row["source_id"]: row for row in fixture["corpus"]}
    # 词法路径可能返回空正文父Event的文件名别名；它不是标注附件正文来源。
    sources.update({"parent:" + source["source_id"]: {**source, "kind": "attachment_parent"}
                    for source in fixture["corpus"] if source.get("kind") == "attachment"})
    if len(rows) != len(queries) or {row["id"] for row in rows} != queries.keys():
        raise ValueError("result_query_set_mismatch")
    for row in rows:
        query = queries[row["id"]]
        if row["expected_sources"] != query["expected_sources"] or row["category"] != query["category"]:
            raise ValueError("result_annotation_mismatch")
        visible = query.get("visible_scopes", ["personal"])
        if "visible_scopes" in row and row["visible_scopes"] != visible:
            raise ValueError("result_visible_scope_mismatch")
        row["visible_scopes"] = visible
        expected_kinds = sorted({sources[key].get("kind", "event") for key in query["expected_sources"]})
        returned = row["returned_sources"]
        if not isinstance(returned, list) or len(returned) != len(set(returned)):
            raise ValueError("result_candidate_set_invalid")
        permitted = {key for key, source in sources.items()
                     if not source.get("revoked", False) and source.get("scope", "personal") in visible}
        if not set(returned) <= permitted:
            raise ValueError("result_scope_or_revocation_violation")
        returned_kinds = [sources[key].get("kind", "event") for key in returned]
        if "source_kind_expected" in row and row["source_kind_expected"] != expected_kinds:
            raise ValueError("result_expected_kind_mismatch")
        if "returned_kinds" in row and row["returned_kinds"] != returned_kinds:
            raise ValueError("result_returned_kind_mismatch")
        row["source_kind_expected"], row["returned_kinds"] = expected_kinds, returned_kinds
        if strict_route and (type(row.get("route_executed")) is not bool
                             or not isinstance(row.get("query_status"), str)):
            raise ValueError("result_route_status_required")
        if strict_route:
            rejected = ("input_rejected" if not query["query"].strip()
                        else "scope_limit" if len(set(visible)) > 4 else None)
            if row["route_executed"] != (rejected is None) or row["query_status"] != (rejected or "semantic"):
                raise ValueError("result_facade_admission_mismatch")
        if row.get("route_executed") is False and returned:
            raise ValueError("unexecuted_query_returned_candidates")


def evaluate(binary, model_dir, output, pip_report, suite="legacy"):
    require_ci()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("验收只支持固定 Linux x86_64 runner")
    output.mkdir(parents=True, exist_ok=False)
    work = output.parent / (output.name + "-private-runtime")
    work.mkdir(exist_ok=False)
    home = work / "home"
    home.mkdir()
    lock = read_json(HERE / "model-lock.json")
    fixture_sets = datasets(suite)
    # 使用生产 EmbeddingSpace.prepare 的原合同：prefix + 换行 + query，且只加一次。
    allowlist = allowed_inputs(fixture_sets, lock["query_instruction"])
    source_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    if source_sha != os.environ.get("GITHUB_SHA"):
        raise RuntimeError("源码与触发 CI SHA 不一致")
    subprocess.run(["git", "diff", "--exit-code"], cwd=REPO, check=True, capture_output=True)
    summary = {"schema": "recallcard.real-encoder-synthetic-ci/1", "status": "started", "source_sha": source_sha, "suite": suite,
               "synthetic_only": True, "private_data_used": False, "external_inference_used": False,
               "model_revision": lock["revision"], "model_lock_sha256": digest(HERE / "model-lock.json"),
               "harness_sha256": digest(HERE / "harness.rs"), "binary_sha256": digest(binary),
               "driver_sha256": {name: digest(HERE / name) for name in
                                 ("common.py", "run.py", "adapter.py", "download.py", "requirements.txt")},
               "adapted_datasets_canonical_sha256": canonical_digest(fixture_sets),
               "fixture_sha256": {"corpus.json": digest(FIXTURES / "corpus.json"), "queries.json": digest(FIXTURES / "queries.json"),
                                  "separate_edge_cases.json": digest(HERE / "edge-cases.json")},
               "quality_gate": "固定输入回放：输出真实排名，不事后调整阈值或把工作流绿色称为生产语义准确率达标",
               "precision_note": "annotated_source_precision只按冻结expected_sources计量；非预期候选包括同主题但无答案来源，不等于生成错误答案；标注不是穷尽语义相关性的真值。",
               "limits": {"runtime_seconds": TIMEOUT, "sampled_process_tree_rss_bytes": RSS_CAP,
                          "max_embedding_requests": 128, "max_embedded_inputs": 256, "threads": 2,
                          "batch_size": 8, "max_tokens_including_prefix_and_special_tokens": 512,
                          "model_download_bytes": 100 * 1024**2, "index_disk_cap_bytes_per_scope": 16 * 1024**2},
               "limitations": ["原12源52问为原创合成小集，不是旧gold或用户资料。",
                               "@k仅为候选source-hit；召回原文不等于答案正确。",
                               "没有语义拒答阈值；无关问、近似错误陈述也可能返回高分候选。",
                               "否定与更正使用单独新增小集；不会把过时/冲突来源自动当作已解决。",
                               "RSS为每50ms采样的进程树RSS之和，有共享页重复且可能遗漏短峰；另记录编码进程自身ru_maxrss。",
                               "只执行 Linux hosted runner CPU；不证明用户机器、iGPU或大规模性能。"]}
    if suite == "attachment-holdout":
        summary["fixture_sha256"] = {"original_holdout.json": digest(HOLDOUT)}
        summary["fixture_projection"] = {"annotated_sources": 24, "synthetic_metadata_only_attachment_parents": 12,
                                         "attachment_payload": "synthetic UTF-8 text/plain",
                                         "source_title": "preserved metadata, not indexed by current production route"}
        summary["paired_request_budget"] = holdout_request_budget(fixture_sets[0])
        summary["limitations"] = [
            "冻结的24源64问独立合成集已在先前词法诊断中观察过，本次为固定真实模型回放，不是新的盲测。",
            "同Vault同scope同模型空间，附件始终可词法检索；只比较独立附件编码批准false→true，不更改原文、查询或qrels。",
            "所有附件均为合成UTF-8 text/plain payload；保留包括.pdf在内的原始filename字段，不代表PDF解析或OCR。",
            "另有12个空正文附件父Event；词法命中parent:ATT-*只计独立非预期候选，不冒充附件正文来源。title保留为元数据，但当前生产路线不索引title。",
            "42个正例、22个无答案；来源命中@k只衡量候选来源，不证明答案正确或能正确拒答。",
            "输入拒绝与scope_limit保留在全量分母并单列；它们没有执行语义路线，不能算作成功语义拒答。",
            "没有语义拒答阈值；非预期候选/有标注precision不等于完整相关性真值。",
            "RSS为每50ms采样的进程树RSS之和，有共享页重复且可能遗漏短峰；另记录编码进程自身ru_maxrss。",
            "只执行Linux hosted runner CPU；配对第二轮复用缓存，耗时不是两轮独立冷启动性能比较。",
        ]
    save_json(output / "summary.json", summary)
    save_json(output / "dependency-lock.json", runtime_lock(pip_report))
    peak = [0]
    stop = threading.Event()

    def monitor():
        while not stop.wait(0.05):
            peak[0] = max(peak[0], rss_tree_bytes(os.getpid()))
            if peak[0] > RSS_CAP:
                os.kill(os.getpid(), signal.SIGUSR1)
                return

    def expired(*_):
        raise TimeoutError("真实编码验收超时")

    def oversized(*_):
        raise MemoryError("真实编码验收超过 RSS 预算")

    signal.signal(signal.SIGALRM, expired)
    signal.signal(signal.SIGUSR1, oversized)
    signal.alarm(TIMEOUT)
    watcher = threading.Thread(target=monitor, daemon=True)
    watcher.start()
    process = None
    server = None
    started = time.monotonic()
    try:
        from adapter import Encoder, make_server
        encoder = Encoder(model_dir, allowlist)
        server = make_server(encoder)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        identity = hashlib.sha256(json.dumps({"lock": lock, "runtime": encoder.runtime, "preprocessing": "prefix-newline-once;utf8-v1"},
                                            sort_keys=True, ensure_ascii=False).encode()).hexdigest()
        config = {"datasets": fixture_sets, "suite": suite, "work": str(work), "endpoint": f"http://127.0.0.1:{server.server_port}/v1/embeddings",
                  "model": lock["model"], "space_revision": "real-ci-v1:" + identity,
                  "tokenizer_identity": "sha256:" + encoder.files["tokenizer.json"]["sha256"],
                  "query_instruction": lock["query_instruction"], "python": sys.executable, "python_path": str(REPO / "python")}
        save_json(work / "harness-input.json", config)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home), "LANG": "C.UTF-8", "TZ": "UTC",
               "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "data"),
               "RECALLCARD_STATE_DIR": str(work / "state")}
        process = subprocess.Popen([str(binary), str(work / "harness-input.json")], cwd=work, env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        stdout, stderr = process.communicate(timeout=180)
        if process.returncode or len(stdout.encode()) > 1024 * 1024:
            code = re.fullmatch(r"recallcard_synthetic_error:([a-z_]{1,96})\n?", stderr)
            if code:
                summary["harness_error_code"] = code.group(1)
            raise RuntimeError("真实 facade 验收失败")
        result = json.loads(stdout)
        fixtures_by_name = {data["name"]: data for data in fixture_sets}
        if suite == "attachment-holdout":
            if [data.get("variant") for data in result["datasets"]] != list(HOLDOUT_VARIANTS):
                raise ValueError("paired_variant_set_mismatch")
        for data in result["datasets"]:
            fixture = fixtures_by_name[data["name"]]
            verify_rows(data["semantic"], fixture, strict_route=suite == "attachment-holdout")
            for rows in data["baselines"].values():
                verify_rows(rows, fixture)
            data["metrics"] = {"semantic": summarize(data["semantic"]),
                               **{name: summarize(rows) for name, rows in data["baselines"].items()}}
            durations = [row["seconds"] for row in data["semantic"]]
            data["query_seconds"] = {"first": durations[0], "median": statistics.median(durations), "max": max(durations)}
        if suite == "attachment-holdout":
            summary["paired_deltas"] = paired_deltas(result["datasets"][0]["semantic"], result["datasets"][1]["semantic"])
            bound = summary["paired_request_budget"]
            if (encoder.requests > bound["paired_embedding_request_upper_bound"]
                    or encoder.inputs > bound["paired_embedded_input_upper_bound"]):
                raise RuntimeError("paired_embedding_budget_exceeded")
        sqlite_files = sorted((work / "state").rglob("*.sqlite"))
        expected_indexes = (len({source["scope"] for source in fixture_sets[0]["corpus"]})
                            if suite == "attachment-holdout" else len(fixture_sets))
        if len(sqlite_files) != expected_indexes:
            raise RuntimeError("未观察到实际 SQLite 索引")
        summary.update(status="passed_contract_and_measured_quality", datasets=result["datasets"],
                       model_files=encoder.files, runtime=encoder.runtime, model_space_identity=identity,
                       measurements={"load_from_verified_files_seconds": encoder.load_seconds,
                           "first_embedding_seconds": encoder.first_embedding_seconds,
                           "embedding_request_seconds_total": sum(encoder.encoding_seconds),
                           "embedding_requests": encoder.requests, "encoded_inputs": encoder.inputs,
                           "input_utf8_bytes": encoder.input_bytes, "max_tokens_seen": encoder.max_tokens_seen,
                           "sqlite_index_bytes": [path.stat().st_size for path in sqlite_files],
                           "synthetic_runtime_directory_bytes": sum(p.stat().st_size for p in work.rglob("*") if p.is_file()),
                           "verified_model_bytes": sum(v["bytes"] for v in encoder.files.values()),
                           "dependency_environment_bytes": sum(p.stat().st_size for p in Path(sys.prefix).rglob("*") if p.is_file())},
                       token_preflight_oversize_rejected=encoder.oversize_rejected_before_embedding,
                       model_network_policy="explicit verified local files + local_files_only + offline env + Python external socket/DNS audit denial",
                       model_download=read_json(model_dir / "verified-download.json"),
                       billing={"paid_inference_calls": 0, "runner_cost": "未从 GitHub 账单读取；只记录实际耗时，不将 Actions 分钟声称免费"})
    except Exception as error:
        summary.update(status="failed", failure_class=type(error).__name__, failure_details="未输出配置、原文或异常路径；检查工程链和预算")
        raise
    finally:
        signal.alarm(0)
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        stop.set()
        watcher.join(timeout=1)
        if server is not None:
            server.shutdown()
            server.server_close()
        summary["runtime_seconds"] = time.monotonic() - started
        summary["peak_sampled_process_tree_rss_bytes"] = peak[0]
        summary["encoder_process_ru_maxrss_bytes"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
        save_json(output / "summary.json", summary)
    print(json.dumps({"status": summary["status"], "source_sha": source_sha, "suite": suite,
                      "datasets": [{"name": data["name"], "variant": data.get("variant"),
                                    "metrics": data["metrics"]["semantic"]["all"]}
                                   for data in summary["datasets"]]}, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pip-report", type=Path, required=True)
    parser.add_argument("--suite", choices=("legacy", "attachment-holdout"), default="legacy")
    args = parser.parse_args()
    try:
        evaluate(args.binary.resolve(), args.model.resolve(), args.output.resolve(), args.pip_report.resolve(), args.suite)
    except Exception:
        print("真实模型合成验收未通过；可用脱敏 summary 中的状态区分失败与未运行。", file=sys.stderr)
        raise SystemExit(1) from None
