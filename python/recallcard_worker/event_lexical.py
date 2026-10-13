"""有界、可重建的 SQLite 词项通道；哈希只避免落原词，不构成隐私加密。

词项精确区分 ASCII 大小写，保留标识符连接符；连续汉字使用双字组，
独立单字使用单字词项。无停用词、前缀匹配或相关性阈值。所有限制均为
资源边界，不是召回质量保证。相邻上下文只由可信宿主从当前正本提供。
"""
from __future__ import annotations

import hashlib
import json
import re

VERSION = "recallcard.lexical/ascii-id-cjk-bigram/1"
CONTEXT_BYTES = 256
MAX_TERM_BYTES = 128
MAX_CHUNK_TERMS = 4096
MAX_QUERY_TERMS = 32
MAX_TOTAL_TERMS = 2_000_000
MAX_POSTINGS = 8192
MAX_CANDIDATE_CHUNKS = 1024
CHANNEL_LIMIT = 100
FUSION = "rrf/1:k=60"
ASCII = re.compile(r"[A-Za-z0-9_./:+#-]+")


class LexicalError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code="corrupt_lexical_index"):
    if not condition:
        raise LexicalError(code)


def hashed(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def _han(char):
    return "\u3400" <= char <= "\u4dbf" or "\u4e00" <= char <= "\u9fff" or "\uf900" <= char <= "\ufaff" or "\U00020000" <= char <= "\U000323af"


def terms(value, *, start=0, end=None, before="", after="", total=None, query=False):
    """返回hash->(正本start,end,原词hash)；窗口边缘碎ASCII词永不独立入库。"""
    raw = value.encode()
    end = start + len(raw) if end is None else end
    total = end if total is None else total
    left, right = before.encode(), after.encode()
    require(len(left) <= CONTEXT_BYTES and len(right) <= CONTEXT_BYTES
            and len(left) <= start and end + len(right) <= total,
            "lexical_context_invalid")
    # 有中间chunk而无足够邻接字符时不能声称完整覆盖。每侧至少一个字符；
    # 词项在窗口边缘被截断时跳过，完整长词超过显式最大长度也不会变成短词。
    require((start == 0 or bool(left)) and (end == total or bool(right))
            and len(left) >= min(start, CONTEXT_BYTES) - 3
            and len(right) >= min(total - end, CONTEXT_BYTES) - 3, "lexical_context_required")
    combined = before + value + after
    window_start = start - len(left)
    offsets = [window_start]
    for char in combined:
        offsets.append(offsets[-1] + len(char.encode()))
    found = {}

    def add(kind, a, b):
        term = combined[a:b]
        span_start, span_end = offsets[a], offsets[b]
        if span_start >= end or span_end <= start:
            return
        if len(term.encode()) > MAX_TERM_BYTES:
            require(not query, "lexical_query_term_length_limit")
            return
        key = hashed(canonical([VERSION, kind, term]))
        span = (span_start, span_end, hashed(term.encode()))
        if key not in found or span < found[key]:
            found[key] = span
        require(len(found) <= (MAX_QUERY_TERMS if query else MAX_CHUNK_TERMS),
                "lexical_query_term_limit" if query else "lexical_chunk_term_limit")

    for match in ASCII.finditer(combined):
        a, b = match.span()
        if not any(c.isascii() and c.isalnum() for c in match[0]):
            continue
        if a == 0 and window_start != 0 or b == len(combined) and offsets[-1] != total:
            continue
        add("ascii", a, b)
    i = 0
    while i < len(combined):
        if not _han(combined[i]):
            i += 1
            continue
        j = i + 1
        while j < len(combined) and _han(combined[j]):
            j += 1
        if j - i == 1:
            # 只有两侧都确认非汉字/源边界，才把此字符认作独立单字。
            if (i > 0 or window_start == 0) and (j < len(combined) or offsets[-1] == total):
                add("han1", i, j)
        else:
            for at in range(i, j - 1):
                add("han2", at, at + 2)
        i = j
    return sorted((key, *span) for key, span in found.items())


def initialize(db):
    for statement in (
        "CREATE TABLE IF NOT EXISTS lexical_generations(generation TEXT PRIMARY KEY,version TEXT NOT NULL,chunk_count INTEGER NOT NULL DEFAULT 0,term_count INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE IF NOT EXISTS lexical_chunks(generation TEXT NOT NULL,source_ref TEXT NOT NULL,ordinal INTEGER NOT NULL,term_count INTEGER NOT NULL,terms_hash TEXT NOT NULL,PRIMARY KEY(generation,source_ref,ordinal))",
        "CREATE TABLE IF NOT EXISTS lexical_terms(generation TEXT NOT NULL,term_hash TEXT NOT NULL,source_ref TEXT NOT NULL,ordinal INTEGER NOT NULL,start_byte INTEGER NOT NULL,end_byte INTEGER NOT NULL,text_sha256 TEXT NOT NULL,PRIMARY KEY(generation,term_hash,source_ref,ordinal))",
        "CREATE INDEX IF NOT EXISTS lexical_by_chunk ON lexical_terms(generation,source_ref,ordinal,term_hash)",
    ):
        db.execute(statement)


def exists(db):
    return db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='lexical_generations'").fetchone() is not None


def version(db, generation):
    if not exists(db):
        return None
    row = db.execute("SELECT version FROM lexical_generations WHERE generation=?", (generation,)).fetchone()
    if row is None:
        return None
    require(row[0] == VERSION, "lexical_version_unsupported")
    return row[0]


def gc(db, keep):
    if not exists(db):
        return
    for table in ("lexical_terms", "lexical_chunks", "lexical_generations"):
        db.execute(f"DELETE FROM {table} WHERE generation NOT IN (?,?)", keep)


def insert_chunk(db, generation, ref, ordinal, entries):
    require(len(entries) <= MAX_CHUNK_TERMS, "lexical_chunk_term_limit")
    row = db.execute("SELECT term_count FROM lexical_generations WHERE generation=?", (generation,)).fetchone()
    require(row is not None and type(row[0]) is int and 0 <= row[0] <= MAX_TOTAL_TERMS)
    require(row[0] + len(entries) <= MAX_TOTAL_TERMS, "lexical_index_term_limit")
    db.execute("INSERT INTO lexical_chunks VALUES (?,?,?,?,?)", (generation, ref, ordinal, len(entries), hashed(canonical(entries))))
    db.executemany("INSERT INTO lexical_terms VALUES (?,?,?,?,?,?,?)", ((generation, key, ref, ordinal, start, end, sha) for key, start, end, sha in entries))
    db.execute("UPDATE lexical_generations SET chunk_count=chunk_count+1,term_count=term_count+? WHERE generation=?", (len(entries), generation))


def verify(db, generation, expected_chunks, scope):
    """SQL先检查字节大小，再逐chunk增量校验manifest；从不加载整词表。"""
    if not exists(db):
        return False
    if version(db, generation) is None:
        require(not db.execute("SELECT 1 FROM lexical_chunks WHERE generation=? LIMIT 1", (generation,)).fetchone()
                and not db.execute("SELECT 1 FROM lexical_terms WHERE generation=? LIMIT 1", (generation,)).fetchone())
        return False
    count = db.execute("SELECT chunk_count,term_count FROM lexical_generations WHERE generation=?", (generation,)).fetchone()
    require(type(count[0]) is int and count[0] == expected_chunks
            and type(count[1]) is int and 0 <= count[1] <= MAX_TOTAL_TERMS)
    require(db.execute("SELECT COUNT(*) FROM lexical_chunks WHERE generation=?", (generation,)).fetchone()[0] == expected_chunks
            and db.execute("SELECT COUNT(*) FROM lexical_terms WHERE generation=?", (generation,)).fetchone()[0] == count[1])
    invalid = db.execute("""SELECT 1 FROM lexical_chunks l LEFT JOIN chunks c
        ON c.generation=l.generation AND c.source_ref=l.source_ref AND c.ordinal=l.ordinal
        WHERE l.generation=? AND (c.source_ref IS NULL OR typeof(l.source_ref)!='text'
        OR length(CAST(l.source_ref AS BLOB)) NOT BETWEEN 7 AND 512 OR typeof(l.ordinal)!='integer'
        OR typeof(l.term_count)!='integer' OR l.term_count NOT BETWEEN 0 AND ?
        OR typeof(l.terms_hash)!='text' OR length(l.terms_hash)!=64 OR l.terms_hash GLOB '*[^0-9a-f]*') LIMIT 1""", (generation, MAX_CHUNK_TERMS)).fetchone()
    require(invalid is None)
    invalid = db.execute("""SELECT 1 FROM lexical_terms t
        LEFT JOIN lexical_chunks l ON l.generation=t.generation AND l.source_ref=t.source_ref AND l.ordinal=t.ordinal
        LEFT JOIN chunks c ON c.generation=t.generation AND c.source_ref=t.source_ref AND c.ordinal=t.ordinal
        LEFT JOIN sources s ON s.generation=t.generation AND s.ref=t.source_ref
        WHERE t.generation=? AND (l.source_ref IS NULL OR c.source_ref IS NULL OR s.ref IS NULL OR s.scope!=?
        OR typeof(t.source_ref)!='text' OR length(CAST(t.source_ref AS BLOB)) NOT BETWEEN 7 AND 512
        OR typeof(t.ordinal)!='integer' OR typeof(t.term_hash)!='text' OR length(t.term_hash)!=64 OR t.term_hash GLOB '*[^0-9a-f]*'
        OR typeof(t.start_byte)!='integer' OR typeof(t.end_byte)!='integer'
        OR t.start_byte<0 OR t.end_byte<=t.start_byte OR t.end_byte>s.text_bytes OR t.end_byte-t.start_byte>?
        OR t.start_byte>=c.end_byte OR t.end_byte<=c.start_byte
        OR t.start_byte<c.start_byte-? OR t.end_byte>c.end_byte+?
        OR typeof(t.text_sha256)!='text' OR length(t.text_sha256)!=64 OR t.text_sha256 GLOB '*[^0-9a-f]*') LIMIT 1""", (generation, scope, MAX_TERM_BYTES, CONTEXT_BYTES, CONTEXT_BYTES)).fetchone()
    require(invalid is None)
    for ref, ordinal, term_count, manifest in db.execute("SELECT source_ref,ordinal,term_count,terms_hash FROM lexical_chunks WHERE generation=? ORDER BY source_ref,ordinal", (generation,)):
        entries = db.execute("SELECT term_hash,start_byte,end_byte,text_sha256 FROM lexical_terms WHERE generation=? AND source_ref=? AND ordinal=? ORDER BY term_hash LIMIT ?", (generation, ref, ordinal, MAX_CHUNK_TERMS + 1)).fetchall()
        require(len(entries) == term_count and hashed(canonical(entries)) == manifest)
    return True


def candidates(db, generation, scope, query_terms):
    """独立倒排查找，最多读取8192 posting、保留1024 chunk，再选100 Event。"""
    found = {}
    scanned = 0
    truncated = False
    for key, *_ in query_terms:
        cursor = db.execute("""SELECT t.source_ref,t.ordinal,t.start_byte,t.end_byte,t.text_sha256
            FROM lexical_terms t JOIN sources s ON s.generation=t.generation AND s.ref=t.source_ref
            WHERE t.generation=? AND t.term_hash=? AND s.scope=? ORDER BY t.source_ref,t.ordinal LIMIT ?""",
            (generation, key, scope, MAX_POSTINGS - scanned + 1))
        for ref, ordinal, start, end, sha in cursor:
            if scanned == MAX_POSTINGS:
                truncated = True
                break
            scanned += 1
            identifier = (ref, ordinal)
            if identifier not in found:
                if len(found) == MAX_CANDIDATE_CHUNKS:
                    truncated = True
                    continue
                found[identifier] = [0, (start, end, sha)]
            found[identifier][0] += 1
            found[identifier][1] = min(found[identifier][1], (start, end, sha))
        if scanned == MAX_POSTINGS:
            # 一旦资源预算用完，明确报告窗口限制；不声称全部词项已扫描。
            truncated = True
            break
    per_event = {}
    for (ref, ordinal), (matched, evidence) in found.items():
        item = (matched, -ordinal, evidence)
        if ref not in per_event or item[:2] > per_event[ref][:2]:
            per_event[ref] = item
    ordered = sorted(per_event.items(), key=lambda r: (-r[1][0], r[0], -r[1][1]))
    truncated = truncated or len(ordered) > CHANNEL_LIMIT
    return [(ref, -item[1], item[0], item[2]) for ref, item in ordered[:CHANNEL_LIMIT]], {
        "status": "available" if query_terms else "unsupported_query", "version": VERSION, "query_terms": len(query_terms),
        "postings_scanned": scanned, "postings_limit": MAX_POSTINGS,
        "candidate_chunks": len(found), "candidates_limit": MAX_CANDIDATE_CHUNKS,
        "candidates_truncated": truncated, "channel_limit": CHANNEL_LIMIT,
        "case_policy": "ascii_case_sensitive", "match_policy": "whole_ascii_or_han_bigram",
        "term_max_bytes": MAX_TERM_BYTES, "query_term_limit": MAX_QUERY_TERMS,
    }
