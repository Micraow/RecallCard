"""原样提取生产 Rust 排名核，使用标准库协议回放；不模拟评分、不调用 Cargo。

仅回放 tokenize/ranking_terms/rank，不覆盖 Vault 权限、抑制、时间过滤、
附件入库、来源合并、摘要预算或完整 CLI/MCP 链路。编译产物只写入 /tmp。
"""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
from typing import Iterable, Mapping


_MAGIC = b"RCRANK1\0"
_MAX_BYTES = 128 * 1024 * 1024
_MAX_ITEMS = 1_000_000
_MODES = {"exact", "identifier_prefix", "natural_question"}
_CONTEXT = "crates/recallcard/src/context.rs"
_SEARCH = "crates/recallcard/src/context/search.rs"
_SIGNATURES = {
    "tokenize": "pub fn tokenize(text: &str) -> Vec<String>",
    "rank": "fn rank<'a>(docs: &'a [Document], query: &[String], raw: &str, mode: &str) -> Vec<(f64, &'a Document)>",
    "han": "fn han(c: char) -> bool",
    "query_terms": "pub(super) fn query_terms(text: &str) -> Vec<String>",
    "natural_han_query": "pub(super) fn natural_han_query(text: &str, mode: &str) -> bool",
    "ranking_terms": "pub(super) fn ranking_terms(text: &str, mode: &str) -> Vec<String>",
    "han_term": "pub(super) fn han_term(term: &str) -> bool",
    "coherent_han_overlap": "pub(super) fn coherent_han_overlap(query: &str, fields: &[String]) -> bool",
    "term_matches": "pub(super) fn term_matches(field: &str, term: &str) -> bool",
    "term_matches_mode": "pub(super) fn term_matches_mode(field: &str, term: &str, mode: &str) -> bool",
    "evidence_weight": "pub(super) fn evidence_weight(doc: &Document) -> f64",
}
_FUNCTIONS = {
    _CONTEXT: ("tokenize", "rank"),
    _SEARCH: tuple(name for name in _SIGNATURES if name not in {"tokenize", "rank"}),
}
_DOCUMENT = """struct Document {
    reference: String,
    text: String,
    entities: Vec<String>,
    labels: Vec<String>,
    evidence: String,
    state: String,
}
"""


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _tokens(source: str) -> list[tuple[str, int, int]]:
    """只解析提取所需词法边界；未知或不完整字面量直接失败。

    保留源文本偏移；字符串、字符、原始字符串与嵌套注释中的花括号
    不参与语法计数，生命周期与字符字面量分别处理。
    """
    tokens = []
    i, size = 0, len(source)
    while i < size:
        start, char = i, source[i]
        if char.isspace():
            i += 1
            continue
        if source.startswith("//", i):
            end = source.find("\n", i + 2)
            i = size if end == -1 else end + 1
            continue
        if source.startswith("/*", i):
            i += 2
            depth = 1
            while i < size and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise ValueError("Rust 提取失败：未闭合的块注释")
            continue
        raw = re.match(r'(?:br|cr|r)(#{0,255})"', source[i:])
        if raw:
            close = '"' + raw.group(1)
            end = source.find(close, i + len(raw.group(0)))
            if end == -1:
                raise ValueError("Rust 提取失败：未闭合的原始字符串")
            i = end + len(close)
            tokens.append(("<literal>", start, i))
            continue
        quote_at = i + 1 if char in "bc" and source[i + 1:i + 2] == '"' else i
        if source[quote_at:quote_at + 1] == '"':
            i = quote_at + 1
            while i < size:
                if source[i] == "\\":
                    i += 2
                elif source[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            else:
                raise ValueError("Rust 提取失败：未闭合的字符串")
            if i > size:
                raise ValueError("Rust 提取失败：字符串转义不完整")
            tokens.append(("<literal>", start, i))
            continue
        quote_at = i + 1 if char == "b" and source[i + 1:i + 2] == "'" else i
        if source[quote_at:quote_at + 1] == "'":
            end = quote_at + 1
            if source[end:end + 1] == "\\":
                end += 2
                if source[quote_at + 2:quote_at + 3] == "u":
                    if source[end:end + 1] != "{":
                        raise ValueError("Rust 提取失败：无效的 Unicode 字符转义")
                    close = source.find("}", end + 1)
                    if close == -1:
                        raise ValueError("Rust 提取失败：未闭合的 Unicode 字符转义")
                    end = close + 1
                elif source[quote_at + 2:quote_at + 3] == "x":
                    end += 2
            else:
                end += 1
            if source[end:end + 1] == "'":
                i = end + 1
                tokens.append(("<literal>", start, i))
                continue
            lifetime = re.match(r"'[A-Za-z_][A-Za-z_0-9]*", source[quote_at:])
            if quote_at != start or not lifetime:
                raise ValueError("Rust 提取失败：无法识别的字符或生命周期")
            i = quote_at + len(lifetime.group(0))
            tokens.append((lifetime.group(0), start, i))
            continue
        if char.isalpha() or char == "_":
            i += 1
            while i < size and (source[i].isalnum() or source[i] == "_"):
                i += 1
            tokens.append((source[start:i], start, i))
            continue
        tokens.append((char, start, i + 1))
        i += 1
    return tokens


def _extract_functions(source: str, names: Iterable[str]) -> dict[str, str]:
    """按模块顶层函数与平衡分隔符提取，拒绝重复、缺失或签名/属性漂移。"""
    wanted = set(names)
    tokens = _tokens(source)
    result = {}
    stack = []
    item_start = 0
    active = None
    pairs = {"}": "{", ")": "(", "]": "["}
    for index, (token, start, end) in enumerate(tokens):
        if not stack and token == "fn":
            if index + 1 >= len(tokens):
                raise ValueError("Rust 提取失败：函数声明不完整")
            name = tokens[index + 1][0]
            if name in wanted:
                if name in result or active is not None:
                    raise ValueError(f"Rust 提取失败：顶层函数重复：{name}")
                active = (name, item_start)
        if token in {"{", "(", "["}:
            if not stack and token == "{" and active is not None:
                name, first = active
                actual = [part[0] for part in tokens[first:index]]
                # Rust 允许参数列表最后一个逗号，除此之外必须精确匹配已审阅接口。
                actual = [part for n, part in enumerate(actual)
                          if not (part == "," and n + 1 < len(actual) and actual[n + 1] == ")")]
                expected = [part[0] for part in _tokens(_SIGNATURES[name])]
                if actual != expected:
                    raise ValueError(f"Rust 提取失败：函数签名或属性已改变：{name}")
            stack.append(token)
        elif token in pairs:
            if not stack or stack.pop() != pairs[token]:
                raise ValueError("Rust 提取失败：分隔符不平衡")
            if not stack and token == "}":
                if active is not None:
                    name, first = active
                    result[name] = source[tokens[first][1]:end]
                    active = None
                item_start = index + 1
        elif not stack and token == ";":
            if active is not None:
                raise ValueError("Rust 提取失败：目标函数没有函数体")
            item_start = index + 1
    if stack or active is not None:
        raise ValueError("Rust 提取失败：文件语法边界不完整")
    missing = wanted - result.keys()
    if missing:
        raise ValueError(f"Rust 提取失败：未找到唯一顶层函数：{', '.join(sorted(missing))}")
    return result


def _frame(value: str) -> bytes:
    if not isinstance(value, str):
        raise TypeError("排名回放字段必须是字符串")
    data = value.encode("utf-8")
    if len(data) > _MAX_BYTES:
        raise ValueError("排名回放字段超过协议限制")
    return struct.pack("<I", len(data)) + data


def _count(value: int) -> bytes:
    if value > _MAX_ITEMS:
        raise ValueError("排名回放条目数超过协议限制")
    return struct.pack("<I", value)


class ProductionRank:
    """从当前检出源码编译一次，多次使用真实排名核。

    output 必须在 /tmp，内部创建独立子目录，保留源码、二进制和身份收据。
    docs 使用 source_id/text，entities/labels 默认为空列表，evidence/state
    默认为空字符串（中性权重）。不会读取或修改任何 Vault 或测试样本。
    """

    def __init__(self, repo: Path | str, output: Path | str, rustc: str = "rustc"):
        self.repo = Path(repo).resolve(strict=True)
        output = Path(output).resolve()
        if not output.is_relative_to(Path("/tmp").resolve()):
            raise ValueError("排名回放编译目录必须位于 /tmp")
        if output.is_relative_to(self.repo):
            raise ValueError("排名回放编译产物不能写入项目仓库")
        compiler = shutil.which(os.fspath(rustc))
        if compiler is None:
            raise FileNotFoundError("未找到 rustc；请先配置已有 Rust 工具链，不会自动下载依赖")
        sources, functions = {}, {}
        for filename, names in _FUNCTIONS.items():
            data = (self.repo / filename).read_bytes()
            sources[filename] = data
            functions[filename] = _extract_functions(data.decode("utf-8"), names)
        io_path = Path(__file__).with_name("rank_io.rs")
        io_bytes = io_path.read_bytes()
        generated = (
            "// 由当前生产源码逐字提取；仅用于离线排名核回放。\n"
            "use std::collections::{BTreeMap, BTreeSet};\n\n"
            + _DOCUMENT + "\n"
            + "\n\n".join(functions[_CONTEXT].values())
            + "\n\nmod search {\nuse super::*;\n"
            + "\n\n".join(functions[_SEARCH].values())
            + "\n}\n\n" + io_bytes.decode("utf-8")
        ).encode("utf-8")
        output.mkdir(parents=True, exist_ok=True)
        self.output = Path(tempfile.mkdtemp(prefix="production-rank-", dir=output))
        source_path = self.output / "production_rank.rs"
        self.binary = self.output / "production_rank"
        source_path.write_bytes(generated)
        version = subprocess.run([compiler, "--version"], check=True, capture_output=True,
                                 text=True, timeout=30).stdout.strip()
        command = [compiler, "--edition=2021", "-C", "opt-level=1", "-C", "debuginfo=0",
                   "-C", "strip=debuginfo",
                   f"--remap-path-prefix={self.output}=/synthetic-query-diagnostics",
                   str(source_path), "-o", str(self.binary)]
        compiled = subprocess.run(command, capture_output=True, text=True, timeout=120,
                                  cwd=self.output)
        if compiled.returncode:
            raise RuntimeError(f"生产排名核编译失败（未使用替代评分）：\n{compiled.stderr}")
        self.identity = {
            "schema": "recallcard.production-rank-replay/1",
            "scope": "当前生产 Rust 排名核回放；不是完整 Vault 权限或端到端验收",
            "full_vault_e2e": False,
            "not_covered": ["Vault 授权与抑制", "时间过滤", "附件入库", "来源合并",
                            "语义融合", "摘要与预算", "CLI/MCP 端到端"],
            "source_sha256": {name: _sha256(data) for name, data in sources.items()},
            "function_sha256": {f"{filename}::{name}": _sha256(body.encode("utf-8"))
                                for filename, entries in functions.items()
                                for name, body in entries.items()},
            "generated_rust_sha256": _sha256(generated),
            "rank_io_sha256": _sha256(io_bytes),
            "binary_sha256": _sha256(self.binary.read_bytes()),
            "rustc_version": version,
            "compile_command": command,
            "generated_rust": str(source_path),
            "binary": str(self.binary),
            "protocol": "RCRANK1; little-endian u32 UTF-8 lengths/counts; u32 index + f64 score",
            "ordering": "score descending (f64::total_cmp), source_id ascending",
            "reference_mapping": "Document.reference 直接使用传入 source_id；不是 Event 正本生成的哈希 ref",
        }
        (self.output / "identity.json").write_text(
            json.dumps(self.identity, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def _request(self, query: str, mode: str, docs: Iterable[Mapping]) -> tuple[list[str], list[dict]]:
        if mode not in _MODES:
            raise ValueError(f"不支持的 lexical_match：{mode}")
        documents = list(docs)
        parts = [_MAGIC, _frame(query), _frame(mode), _count(len(documents))]
        ids = []
        for document in documents:
            source_id = document["source_id"]
            parts.extend((_frame(source_id), _frame(document["text"])))
            ids.append(source_id)
            for field in ("entities", "labels"):
                values = document.get(field, [])
                if not isinstance(values, (list, tuple)):
                    raise TypeError(f"排名回放 {field} 必须是字符串列表")
                parts.append(_count(len(values)))
                parts.extend(_frame(value) for value in values)
            parts.extend((_frame(document.get("evidence", "")), _frame(document.get("state", ""))))
        if len(set(ids)) != len(ids):
            raise ValueError("排名回放 source_id 必须唯一，以保证稳定排序和来源映射")
        if sum(map(len, parts)) > _MAX_BYTES:
            raise ValueError("排名回放请求超过协议限制")
        completed = subprocess.run([str(self.binary)], input=b"".join(parts), capture_output=True,
                                   timeout=120, cwd=self.output)
        if completed.returncode:
            raise RuntimeError("生产排名核回放失败：" + completed.stderr.decode("utf-8", errors="replace"))
        data, position = completed.stdout, 0

        def read(size):
            nonlocal position
            if size < 0 or position + size > len(data):
                raise RuntimeError("排名回放响应截断")
            result = data[position:position + size]
            position += size
            return result

        def count():
            value = struct.unpack("<I", read(4))[0]
            if value > _MAX_ITEMS:
                raise RuntimeError("排名回放响应条目数超限")
            return value

        if read(len(_MAGIC)) != _MAGIC:
            raise RuntimeError("排名回放响应协议不匹配")
        terms = []
        for _ in range(count()):
            length = struct.unpack("<I", read(4))[0]
            if length > _MAX_BYTES:
                raise RuntimeError("排名回放响应字符串超限")
            terms.append(read(length).decode("utf-8"))
        rows, seen = [], set()
        for _ in range(count()):
            index = struct.unpack("<I", read(4))[0]
            score = struct.unpack("<d", read(8))[0]
            if index >= len(ids) or index in seen or not math.isfinite(score) or score <= 0:
                raise RuntimeError("排名回放响应包含无效文档序号或分数")
            seen.add(index)
            rows.append({"id": ids[index], "score": score})
        if position != len(data):
            raise RuntimeError("排名回放响应含多余字节")
        if rows != sorted(rows, key=lambda row: (-row["score"], row["id"])):
            raise RuntimeError("排名回放响应未按分数和来源稳定排序")
        if not terms and rows:
            raise RuntimeError("排名回放空词查询必须返回空结果")
        return terms, rows

    def search(self, query: str, mode: str, docs: Iterable[Mapping]) -> list[dict]:
        """返回按真实分数降序、source_id 升序排列的 {id, score}。"""
        return self._request(query, mode, docs)[1]

    def terms(self, query: str, mode: str) -> list[str]:
        """返回生产 ranking_terms 的实际结果，不使用 Python 近似分词。"""
        return self._request(query, mode, [])[0]
