"""显式 opt-in 的 Extract → 授权候选查找 → Consolidate；不访问 Vault、不发布。

lookup 是可信宿主提供的本机只读回调，收到完整提议及明确 scope，返回当前可见候选 ref。
export_job 必须由同一宿主重新从正本导出，不能在 Python 猜测 Rust 的 hash。
默认 CLI、后台调度和网络授权不改变；调用方必须显式给 client 两次请求的预算与授权。
"""
from __future__ import annotations

from copy import deepcopy
import re
import time

from .client import canonical, fail, validate_job, validate_result, _fields, _integer

_MEMORY_REF = re.compile(r"memory:mem_[0-9a-f]{32}@[1-9][0-9]*\Z")


def validate_extraction_context(context, consolidation_job):
    _fields(context, {"job", "result"})
    extraction_job = validate_job(context["job"])
    result = validate_result(context["result"], extraction_job)
    if extraction_job["memory_read_set"]:
        fail("invalid_fields", "Extraction 必须只读取本次 Event，不能预选旧记忆")
    if (extraction_job["allowed_scope"] != consolidation_job["allowed_scope"]
            or canonical(extraction_job["source_refs"]) != canonical(consolidation_job["source_refs"])):
        fail("job_mismatch", "提取与整合的完整来源、范围或版本不一致；请重新提取")
    # An empty read set already forbids modifications; keep an explicit stage contract.
    if any(p["operation"] not in {"add", "noop", "conflict"} for p in result["proposals"]):
        fail("invalid_fields", "Extraction 只允许新增假设、无需变更或冲突")
    return {"job": extraction_job, "result": result}


def execute_staged(source_job, *, client, lookup, export_job, max_candidates=32, checkpoint=None):
    """只返回未信任的最终提议。回调不能自动发送/发布；错误不会自动重试或截断。"""
    source_job = validate_job(source_job)
    if source_job["memory_read_set"]:
        fail("invalid_fields", "分阶段整理必须从没有旧记忆的来源 Job 开始")
    _integer(max_candidates, 1, 32)
    started = time.monotonic()
    extracted = client.execute(source_job)
    result = validate_result(extracted["result"], source_job)
    context = validate_extraction_context({"job": source_job, "result": result}, source_job)
    if checkpoint is not None:
        checkpoint("extraction", deepcopy(extracted))
    references = []
    lookup_calls = 0
    for proposal in result["proposals"]:
        # Conflicts remain in the full extraction context; none are silently dropped.
        if proposal["operation"] != "add":
            continue
        candidates = lookup(deepcopy(proposal), source_job["allowed_scope"])
        lookup_calls += 1
        if not isinstance(candidates, list) or len(candidates) > max_candidates:
            fail("input_limit", "候选查找超过明确上限；请缩小来源，不会截断候选")
        for reference in candidates:
            if not isinstance(reference, str) or not _MEMORY_REF.fullmatch(reference):
                fail("invalid_reference", "候选查找必须返回带版本的 Memory 引用")
            if reference not in references:
                references.append(reference)
        if len(references) > max_candidates:
            fail("input_limit", "合并候选超过明确上限；不会预先裁掉旧记忆")
    # Host revalidates authorization, source suppressions, and exact revisions now.
    job = validate_job(export_job(deepcopy(source_job), list(references)))
    validate_extraction_context(context, job)
    if {m["ref"] for m in job["memory_read_set"]} != set(references):
        fail("job_mismatch", "重新导出的旧记忆与检索候选不一致；请重新查找")
    if checkpoint is not None:
        checkpoint("candidate_job", deepcopy(job))
    consolidated = client.execute(job, extraction=context)
    final = validate_result(consolidated["result"], job)
    return {"result": final, "job": job, "extraction": context,
            "requires_review": True,
            "diagnostics": {"pipeline": "opt-in-extract-lookup-consolidate/1",
                            "lookup_calls": lookup_calls, "candidate_count": len(references),
                            "elapsed_seconds": time.monotonic() - started,
                            "extraction": extracted.get("diagnostics", {}),
                            "consolidation": consolidated.get("diagnostics", {})}}
