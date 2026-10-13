"""CI-only：真实小编码器经过现有只读 facade 的固定合成验收。"""
import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import resource
import signal
import statistics
import subprocess
import sys
import threading
import time

from common import FIXTURES, HERE, REPO, datasets, digest, read_json, require_ci, save_json, summarize

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


def evaluate(binary, model_dir, output, pip_report):
    require_ci()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("验收只支持固定 Linux x86_64 runner")
    output.mkdir(parents=True, exist_ok=False)
    work = output.parent / (output.name + "-private-runtime")
    work.mkdir(exist_ok=False)
    home = work / "home"
    home.mkdir()
    lock = read_json(HERE / "model-lock.json")
    fixture_sets = datasets()
    # 使用生产 EmbeddingSpace.prepare 的原合同：prefix + 换行 + query，且只加一次。
    allowlist = {row["text"] for data in fixture_sets for row in data["corpus"]}
    allowlist |= {lock["query_instruction"] + "\n" + row["query"] for data in fixture_sets for row in data["queries"]}
    source_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    if source_sha != os.environ.get("GITHUB_SHA"):
        raise RuntimeError("源码与触发 CI SHA 不一致")
    subprocess.run(["git", "diff", "--exit-code"], cwd=REPO, check=True, capture_output=True)
    summary = {"schema": "recallcard.real-encoder-synthetic-ci/1", "status": "started", "source_sha": source_sha,
               "synthetic_only": True, "private_data_used": False, "external_inference_used": False,
               "model_revision": lock["revision"], "model_lock_sha256": digest(HERE / "model-lock.json"),
               "harness_sha256": digest(HERE / "harness.rs"), "binary_sha256": digest(binary),
               "fixture_sha256": {"corpus.json": digest(FIXTURES / "corpus.json"), "queries.json": digest(FIXTURES / "queries.json"),
                                  "separate_edge_cases.json": digest(HERE / "edge-cases.json")},
               "quality_gate": "首次固定基线：输出真实排名，不事后调整阈值或把工作流绿色称为生产语义准确率达标",
               "limits": {"runtime_seconds": TIMEOUT, "sampled_process_tree_rss_bytes": RSS_CAP,
                          "max_embedding_requests": 128, "max_embedded_inputs": 256, "threads": 2,
                          "batch_size": 8, "max_tokens_including_prefix_and_special_tokens": 512,
                          "model_download_bytes": 100 * 1024**2, "index_disk_cap_bytes_per_vault": 16 * 1024**2},
               "limitations": ["原12源52问为原创合成小集，不是旧gold或用户资料。",
                               "@k仅为候选source-hit；召回原文不等于答案正确。",
                               "没有语义拒答阈值；无关问、近似错误陈述也可能返回高分候选。",
                               "否定与更正使用单独新增小集；不会把过时/冲突来源自动当作已解决。",
                               "RSS为每50ms采样的进程树RSS之和，有共享页重复且可能遗漏短峰；另记录编码进程自身ru_maxrss。",
                               "只执行 Linux hosted runner CPU；不证明用户机器、iGPU或大规模性能。"]}
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
        config = {"datasets": fixture_sets, "work": str(work), "endpoint": f"http://127.0.0.1:{server.server_port}/v1/embeddings",
                  "model": lock["model"], "space_revision": "real-ci-v1:" + identity,
                  "tokenizer_identity": "sha256:" + encoder.files["tokenizer.json"]["sha256"],
                  "query_instruction": lock["query_instruction"], "python": sys.executable, "python_path": str(REPO / "python")}
        save_json(work / "harness-input.json", config)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home), "LANG": "C.UTF-8", "TZ": "UTC",
               "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "data"),
               "RECALLCARD_STATE_DIR": str(work / "state")}
        process = subprocess.Popen([str(binary), str(work / "harness-input.json")], cwd=work, env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        stdout, _ = process.communicate(timeout=180)
        if process.returncode or len(stdout.encode()) > 1024 * 1024:
            raise RuntimeError("真实 facade 验收失败")
        result = json.loads(stdout)
        for data in result["datasets"]:
            data["metrics"] = {"semantic": summarize(data["semantic"]),
                               **{name: summarize(rows) for name, rows in data["baselines"].items()}}
            durations = [row["seconds"] for row in data["semantic"]]
            data["query_seconds"] = {"first": durations[0], "median": statistics.median(durations), "max": max(durations)}
        sqlite_files = sorted((work / "state").rglob("*.sqlite"))
        if len(sqlite_files) != len(fixture_sets):
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
    print(json.dumps({"status": summary["status"], "source_sha": source_sha,
                      "synonym_question": summary["datasets"][0]["metrics"]["semantic"]["by_category"]["synonym_question"]}, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pip-report", type=Path, required=True)
    args = parser.parse_args()
    try:
        evaluate(args.binary.resolve(), args.model.resolve(), args.output.resolve(), args.pip_report.resolve())
    except Exception:
        print("真实模型合成验收未通过；可用脱敏 summary 中的状态区分失败与未运行。", file=sys.stderr)
        raise SystemExit(1) from None
