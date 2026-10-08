"""显式文件 CLI：默认不联网；只原子保存未信任的 DreamResult。"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
import tempfile

from .client import (
    MAX_JOB_BYTES, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, MAX_RESULT_BYTES,
    DreamClient, DreamError, NetworkApproval, canonical, fail, load_json, validate_job,
)


def _safe_target(path, overwrite=False):
    target = Path(path).absolute()
    for component in (target, *target.parents):
        if component.is_symlink():
            fail("unsafe_path", "拒绝符号链接输出路径；请选择私有本机普通文件")
    if not target.parent.is_dir():
        fail("invalid_path", "输出父目录不存在；请先建立私有本机目录")
    if target.exists():
        if not target.is_file():
            fail("unsafe_path", "输出目标必须为普通文件")
        if not overwrite:
            fail("output_exists", "输出文件已存在；覆盖必须明确指定 --overwrite")
    return target


def atomic_result_file(path, value, *, overwrite=False, max_bytes=MAX_RESULT_BYTES):
    """同目录临时文件先 fsync；默认用排他硬链接提交，防止检查后的覆盖竞争。"""
    target = _safe_target(path, overwrite)
    encoded = canonical(value) + b"\n"
    if len(encoded) > max_bytes:
        fail("output_limit", "结果文件超过字节预算")
    descriptor, temporary = tempfile.mkstemp(prefix=".recallcard-dream-", dir=target.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        _safe_target(target, overwrite)
        if overwrite:
            os.replace(temporary, target)
        else:
            try:
                os.link(temporary, target)
            except FileExistsError:
                fail("output_exists", "输出文件已存在；未覆盖已有结果")
        if os.name == "posix":
            directory = os.open(target.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def read_job(path, limit=MAX_JOB_BYTES):
    if path == "-":
        data = sys.stdin.buffer.read(limit + 1)
    else:
        with open(path, "rb") as stream:
            data = stream.read(limit + 1)
    return load_json(data, limit)


class ChineseParser(argparse.ArgumentParser):
    def __init__(self, *args, **kwargs):
        kwargs.setdefault("add_help", False)
        kwargs.setdefault("prog", "recallcard-dream")
        super().__init__(*args, **kwargs)
        self._positionals.title = "位置参数"
        self._optionals.title = "选项"
        self.add_argument("-h", "--help", action="help", help="显示帮助并退出")

    def format_help(self):
        return super().format_help().replace("usage:", "用法:", 1)

    def error(self, message):
        self.exit(2, '{"ok":false,"error":{"code":"invalid_arguments","message":"参数无效；请使用 --help 检查所需参数和允许值"}}\n')


def parser():
    result = ChineseParser(description="RecallCard 可选 Dream API；默认禁止联网，不批准或发布 Memory")
    result.add_argument("--job", required=True, help="已由 Rust 导出并经人工检查的 Job JSON；- 表示 stdin")
    result.add_argument("--output", help="未信任的 Result 输出文件；保存到私有本机目录")
    result.add_argument("--validate-only", action="store_true", help="仅离线校验 Job 结构与完整范围，不调用 API、不保存 Result")
    result.add_argument("--endpoint", help="供应商完整 HTTPS Chat Completions 地址；不自动拼接路径")
    result.add_argument("--model", help="用户选择的供应商模型标识；没有默认模型")
    result.add_argument("--allow-network", action="store_true", help="允许本次单条网络请求；还需要下列三项精确批准")
    result.add_argument("--approve-endpoint", help="明确批准的完整地址，必须与 --endpoint 完全相同")
    result.add_argument("--approve-scope", action="append", default=[], help="明确批准发送的 scope，可重复")
    result.add_argument("--approve-dream-data", action="store_true", help="批准发送完整 Job 所选来源及旧记忆快照")
    result.add_argument("--overwrite", action="store_true", help="明确允许原子替换已有 Result；不会覆盖输入 Job")
    result.add_argument("--max-output-tokens", type=int, default=4096, help="传给供应商的生成 token 上限，范围 1–131072；默认 4096")
    result.add_argument("--max-request-bytes", type=int, default=MAX_REQUEST_BYTES, help="完整请求的 UTF-8 字节上限，最多 4 MiB")
    result.add_argument("--max-job-bytes", type=int, default=MAX_JOB_BYTES, help="输入 Job 字节上限，最多 1 MiB")
    result.add_argument("--max-result-bytes", type=int, default=MAX_RESULT_BYTES, help="Result 文件字节上限，最多 1 MiB")
    result.add_argument("--max-response-bytes", type=int, default=MAX_RESPONSE_BYTES, help="供应商完整响应字节上限，最多 2 MiB")
    result.add_argument("--timeout", type=float, default=30, help="整个请求的超时秒数，大于 0 且最多 1800；默认 30")
    result.add_argument("--connect-timeout", type=float, help="连接超时秒数，最多120且不超过整个请求；默认最多30")
    result.add_argument("--read-timeout", type=float, help="单次读写等待秒数，不超过整个请求；默认与请求时限相同")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if type(args.max_job_bytes) is not int or not 1 <= args.max_job_bytes <= MAX_JOB_BYTES:
            fail("invalid_config", "Job 字节预算必须在 1 到 1 MiB 之间")
        job = validate_job(read_job(args.job, args.max_job_bytes), max_job_bytes=args.max_job_bytes)
        if args.validate_only:
            if args.allow_network or args.output:
                fail("invalid_config", "离线校验不能同时要求联网或输出 Result")
            response = {"ok": True, "validated": True, "network_calls": 0,
                        "job_id": job["job_id"], "scope": job["allowed_scope"],
                        "sources": len(job["source_refs"]), "memories": len(job["memory_read_set"]),
                        "note": "仅校验导出结构与范围；原始摘要、当前来源、suppression 和发布权限仍由 Rust 核对"}
        else:
            if not args.output or not args.endpoint or not args.model:
                fail("invalid_config", "执行 Dream 需要 --output、--endpoint 和 --model")
            target = _safe_target(args.output, args.overwrite)
            if args.job != "-":
                source = Path(args.job)
                if source.resolve() == target.resolve() or (target.exists() and os.path.samefile(source, target)):
                    fail("unsafe_path", "Result 输出不能覆盖输入 Job")
            approval = NetworkApproval(args.allow_network, args.approve_endpoint,
                                       tuple(args.approve_scope), args.approve_dream_data)
            client = DreamClient(args.endpoint, args.model, approval, timeout=args.timeout, connect_timeout=args.connect_timeout, read_timeout=args.read_timeout,
                                 max_output_tokens=args.max_output_tokens, max_request_bytes=args.max_request_bytes,
                                 max_total_request_bytes=args.max_request_bytes, max_job_bytes=args.max_job_bytes,
                                 max_result_bytes=args.max_result_bytes, max_response_bytes=args.max_response_bytes)
            completed = client.execute(job)
            atomic_result_file(target, completed["result"], overwrite=args.overwrite, max_bytes=args.max_result_bytes)
            response = {"ok": True, "requires_review": True, "diagnostics": completed["diagnostics"]}
        sys.stdout.buffer.write(canonical(response) + b"\n")
        return 0
    except DreamError as error:
        sys.stderr.write(json.dumps({"ok": False, "error": {"code": error.code, "message": str(error)}}, ensure_ascii=False) + "\n")
    except (OSError, ValueError, RecursionError):
        sys.stderr.write('{"ok":false,"error":{"code":"io_error","message":"输入输出失败或配置无效；未记录路径、正文或凭据"}}\n')
    except Exception:
        sys.stderr.write('{"ok":false,"error":{"code":"worker_error","message":"执行器内部异常；未记录原始异常或私密数据"}}\n')
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
