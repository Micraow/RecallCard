"""独立本机 Event 索引；只接收可信宿主的有界 JSONL，不扫描 Vault。

原文只在小批本机编码期间存在。SQLite 保存来源身份、范围和可重建向量。
候选不是权限证明；宿主必须在显示或发送前核验当前正本及整个来源快照。
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from dataclasses import fields
import hashlib
import heapq
import json
import math
import os
from pathlib import Path
import sqlite3
import stat
import struct
import sys

from .embedding import EmbeddingError, EmbeddingSpace, canonical, scopes
from .local_query import LocalIndexClient, LocalQueryClient, local_endpoint

SCHEMA = "recallcard.local-event-index/1"
CONFIG_SCHEMA = "recallcard.local-event-config/1"
MAX_LINE = 256 * 1024
MAX_RESPONSE = 64 * 1024
MAX_CONFIG = 64 * 1024
MAX_BATCH = 8
MAX_BATCH_BYTES = 64 * 1024
MAX_CHUNK_BYTES = 8192
MAX_SOURCES = 100_000
MAX_CHUNKS = 1_000_000
QUERY_PAGE = 256
MAX_RESULTS = 100
# 单个合法SQLite行远小于此值（最大向量为8192*4字节）。必须在取行前
# 限制SQLite自身分配，不能依赖fetchmany之后的Python字段检查。
MAX_SQLITE_VALUE = 64 * 1024
DEFAULT_DISK_CAP = 256 * 1024 * 1024
APPLICATION_ID = 0x52434549  # RCEI：只允许管理专用派生数据库
SIDECARS = ("-journal", "-wal", "-shm")


class EventIndexError(Exception):
    """只保留固定错误码，绝不附加正文、路径或提供器原始错误。"""
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise EventIndexError(code)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def valid_hash(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def text(value, limit, *, empty=False):
    require(isinstance(value, str), "invalid_request")
    try:
        encoded = value.encode("utf-8")
    except UnicodeError:
        raise EventIndexError("invalid_request") from None
    require(len(encoded) <= limit and "\x00" not in value and (empty or bool(value.strip())), "invalid_request")
    return value


def integer(value, low, high):
    require(type(value) is int and low <= value <= high, "invalid_request")
    return value


def shape(value, required, optional=()):
    require(isinstance(value, dict) and set(required) <= value.keys()
            and not (value.keys() - set(required) - set(optional)), "invalid_request")


def _private_path(path, *, missing=False):
    path = Path(path)
    require(path.is_absolute() and ".." not in path.parts, "unsafe_path")
    for part in [*reversed(path.parents), path]:
        try:
            info = part.lstat()
        except FileNotFoundError:
            require(missing and part == path, "unsafe_path")
            continue
        require(not stat.S_ISLNK(info.st_mode), "unsafe_path")
        if part != path:
            require(stat.S_ISDIR(info.st_mode), "unsafe_path")
        else:
            require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "unsafe_path")
            if os.name == "posix":
                require(info.st_uid == os.getuid() and info.st_mode & 0o077 == 0, "unsafe_permissions")
    return path


def read_config(path):
    path = _private_path(Path(path))
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(info.st_size <= MAX_CONFIG and stat.S_ISREG(info.st_mode), "config_limit")
        raw = stream.read(MAX_CONFIG + 1)
    require(len(raw) <= MAX_CONFIG, "config_limit")
    return strict_json(raw)


def strict_json(raw):
    def pairs(items):
        output = {}
        for key, value in items:
            require(key not in output, "invalid_request")
            output[key] = value
        return output
    def constant(_):
        raise EventIndexError("invalid_request")
    try:
        return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise EventIndexError("invalid_request") from None


def vector_blob(values, dimensions):
    require(isinstance(values, (list, tuple)) and len(values) == dimensions, "invalid_vector")
    require(all(type(v) in (int, float) and math.isfinite(v) for v in values), "invalid_vector")
    norm = math.hypot(*values)
    require(math.isfinite(norm) and norm > 0, "invalid_vector")
    try:
        return struct.pack("<" + "f" * dimensions, *(v / norm for v in values))
    except (ValueError, OverflowError, struct.error):
        raise EventIndexError("invalid_vector") from None


def decode_blob(blob, dimensions):
    require(isinstance(blob, bytes) and len(blob) == dimensions * 4, "corrupt_vector")
    values = struct.unpack("<" + "f" * dimensions, blob)
    require(all(math.isfinite(v) for v in values)
            and math.isclose(math.hypot(*values), 1.0, rel_tol=1e-5, abs_tol=1e-5), "corrupt_vector")
    return values


class EventIndex:
    """config只来自可信启动文件；encoder注入仅用于测试，CLI没有此开关。"""
    def __init__(self, config, *, encoder=None):
        require(_supported_python(), "python_version_unsupported")
        shape(config, {"schema", "database", "scope", "space", "events_approved", "query_approved"},
              {"disk_cap_bytes", "backend"})
        require(config["schema"] == CONFIG_SCHEMA, "invalid_config")
        require(type(config["events_approved"]) is bool and type(config["query_approved"]) is bool, "invalid_config")
        require(config.get("backend", "python") == "python", "unsupported_backend")
        self.events_approved = config["events_approved"]
        self.query_approved = config["query_approved"]
        try:
            self.scope = scopes([config["scope"]])[0]
        except EmbeddingError:
            raise EventIndexError("invalid_config") from None
        self.space = json.loads(canonical(config["space"]))
        base_fields = {f.name for f in fields(EmbeddingSpace)}
        shape(self.space, base_fields | {"tokenizer", "chunking_version"})
        require(self.space["provider"] == "local-openai-compatible" and self.space["normalization"] == "l2", "invalid_config")
        text(self.space["tokenizer"], 256)
        text(self.space["chunking_version"], 256)
        try:
            local_endpoint(self.space["endpoint"])
            self.embedding_space = EmbeddingSpace.from_dict({k: self.space[k] for k in base_fields})
        except EmbeddingError:
            raise EventIndexError("invalid_local_config") from None
        self.dimensions = self.embedding_space.dimensions
        self.signature = digest(canonical(self.space))
        self.disk_cap = integer(config.get("disk_cap_bytes", DEFAULT_DISK_CAP), 1024 * 1024, 4 * 1024**3)
        self.path = _private_path(Path(text(config["database"], 4096)), missing=True)
        self.encoder = encoder
        self._prepare_database_file()
        with self.connection(write=True) as db:
            existing_tables = db.execute("SELECT COUNT(*) FROM sqlite_master WHERE type='table'").fetchone()[0]
            if existing_tables:
                require(db.execute("PRAGMA application_id").fetchone()[0] == APPLICATION_ID, "foreign_database")
                require(db.execute("PRAGMA user_version").fetchone()[0] == 1, "database_version")
            else:
                db.execute(f"PRAGMA application_id={APPLICATION_ID}")
                db.execute("PRAGMA user_version=1")
            for statement in (
                "CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
                "CREATE TABLE IF NOT EXISTS generations(id TEXT PRIMARY KEY,source_epoch TEXT NOT NULL,state TEXT NOT NULL,receipt TEXT NOT NULL,accepted_batches INTEGER NOT NULL DEFAULT 0,source_count INTEGER NOT NULL DEFAULT 0,chunk_count INTEGER NOT NULL DEFAULT 0)",
                "CREATE TABLE IF NOT EXISTS sources(generation TEXT NOT NULL,ref TEXT NOT NULL,content_hash TEXT NOT NULL,scope TEXT NOT NULL,role TEXT NOT NULL,session_ref TEXT,occurred_at TEXT,text_bytes INTEGER NOT NULL,expected_chunks INTEGER NOT NULL,received_chunks INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(generation,ref))",
                "CREATE TABLE IF NOT EXISTS vectors(input_hash TEXT PRIMARY KEY,vector BLOB NOT NULL)",
                "CREATE TABLE IF NOT EXISTS chunks(generation TEXT NOT NULL,source_ref TEXT NOT NULL,ordinal INTEGER NOT NULL,start_byte INTEGER NOT NULL,end_byte INTEGER NOT NULL,text_sha256 TEXT NOT NULL,input_hash TEXT NOT NULL,PRIMARY KEY(generation,source_ref,ordinal))",
                "CREATE INDEX IF NOT EXISTS chunks_vector ON chunks(input_hash)",
                "CREATE TABLE IF NOT EXISTS batches(generation TEXT NOT NULL,batch_id TEXT NOT NULL,payload_hash TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(generation,batch_id))",
                "CREATE TABLE IF NOT EXISTS queries(input_hash TEXT PRIMARY KEY,vector BLOB NOT NULL,used INTEGER NOT NULL)",
            ):
                db.execute(statement)
            identity = canonical({"schema": CONFIG_SCHEMA, "space": self.space, "scope": self.scope}).decode()
            previous = db.execute("SELECT value FROM metadata WHERE key='identity'").fetchone()
            require(previous is None or previous[0] == identity, "space_mismatch")
            db.execute("INSERT OR IGNORE INTO metadata VALUES ('identity',?)", (identity,))
            db.execute("INSERT OR IGNORE INTO metadata VALUES ('query_clock','0')")
            if not db.execute("SELECT 1 FROM metadata WHERE key='vector_count'").fetchone():
                db.execute("INSERT INTO metadata VALUES ('vector_count',?)", (str(db.execute("SELECT COUNT(*) FROM vectors").fetchone()[0]),))
            # 兼容同一新模块早期fixture；仅缺失时补派生计数，不逐batch重扫来源表。
            for generation, in db.execute("SELECT id FROM generations"):
                key = f"pending:{generation}"
                if not db.execute("SELECT 1 FROM metadata WHERE key=?", (key,)).fetchone():
                    pending = db.execute("SELECT COUNT(*) FROM sources WHERE generation=? AND received_chunks != expected_chunks", (generation,)).fetchone()[0]
                    db.execute("INSERT INTO metadata VALUES (?,?)", (key, str(pending)))

    def _prepare_database_file(self):
        for suffix in SIDECARS:
            _private_path(Path(str(self.path) + suffix), missing=True)
        try:
            fd = os.open(self.path, os.O_RDWR | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
        except FileExistsError:
            _private_path(self.path)
        else:
            os.close(fd)

    def _disk_bytes(self):
        total = 0
        for p in [self.path, *(Path(str(self.path) + s) for s in SIDECARS)]:
            _private_path(p, missing=True)
            try:
                total += p.stat().st_size
            except FileNotFoundError:
                pass
        return total

    @contextmanager
    def connection(self, *, write=False):
        _private_path(self.path)
        require(self._disk_bytes() <= self.disk_cap, "disk_limit")
        identity = self.path.stat()
        db = sqlite3.connect(self.path, timeout=1.0, isolation_level=None)
        try:
            db.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, MAX_SQLITE_VALUE)
            require((identity.st_dev, identity.st_ino) == (self.path.stat().st_dev, self.path.stat().st_ino), "unsafe_path")
            db.execute("PRAGMA trusted_schema=OFF")
            require(db.execute("PRAGMA journal_mode").fetchone()[0].lower() in ("delete", "off"), "unsupported_journal")
            db.execute("PRAGMA journal_mode=DELETE")
            db.execute("PRAGMA synchronous=FULL")
            db.execute("PRAGMA cache_size=-2048")
            db.execute("PRAGMA mmap_size=0")
            db.execute("PRAGMA temp_store=FILE")
            page = db.execute("PRAGMA page_size").fetchone()[0]
            # GC可能修改整库；不能只为journal预留1/4，否则满库连清空代次
            # 都无法提交。DB最多占总预算1/3，余量覆盖全库回滚与页/扇区开销。
            maximum_pages = max(1, self.disk_cap // 3 // page)
            db.execute(f"PRAGMA max_page_count={maximum_pages}")
            if write:
                require(db.execute("PRAGMA page_count").fetchone()[0] <= maximum_pages, "disk_limit")
            db.execute("BEGIN IMMEDIATE" if write else "BEGIN")
            yield db
            require(self._disk_bytes() <= self.disk_cap, "disk_limit")
            _private_path(self.path)
            require((identity.st_dev, identity.st_ino) == (self.path.stat().st_dev, self.path.stat().st_ino), "unsafe_path")
            db.commit()
        except Exception:
            if db.in_transaction:
                db.rollback()
            raise
        finally:
            db.close()

    @staticmethod
    def _published(db):
        row = db.execute("SELECT value FROM metadata WHERE key='published'").fetchone()
        return row[0] if row else None

    def _stats(self, db, build_id=None):
        published = self._published(db)
        if build_id is None:
            row = db.execute("SELECT id FROM generations WHERE state='building' LIMIT 1").fetchone()
            build_id = row[0] if row else published
        row = db.execute("SELECT id,source_epoch,state,receipt,accepted_batches,source_count,chunk_count FROM generations WHERE id=?", (build_id,)).fetchone()
        result = {"build_id": build_id, "published_generation": published, "space_signature": self.signature,
                  "receipt": None, "source_count": 0, "chunk_count": 0, "pending_sources": 0, "accepted_batches": 0,
                  "source_epoch": None, "state": "unconfigured"}
        if row:
            result.update(zip(("build_id", "source_epoch", "state", "receipt", "accepted_batches", "source_count", "chunk_count"), row))
            pending = db.execute("SELECT value FROM metadata WHERE key=?", (f"pending:{build_id}",)).fetchone()
            require(pending is not None, "index_counter_missing")
            result["pending_sources"] = int(pending[0])
        result["vector_count"] = int(db.execute("SELECT value FROM metadata WHERE key='vector_count'").fetchone()[0])
        return result

    @staticmethod
    def _gc(db, keep):
        # 只触及这个专用派生数据库；没有外部文件路径或事实源删除操作。
        for table in ("chunks", "sources", "batches"):
            db.execute(f"DELETE FROM {table} WHERE generation NOT IN (SELECT id FROM generations WHERE id IN (?,?))", keep)
        db.execute("DELETE FROM generations WHERE id NOT IN (?,?)", keep)
        removed = db.execute("DELETE FROM vectors WHERE NOT EXISTS (SELECT 1 FROM chunks WHERE chunks.input_hash=vectors.input_hash)").rowcount
        db.execute("UPDATE metadata SET value=CAST(value AS INTEGER)-? WHERE key='vector_count'", (removed,))
        db.execute("DELETE FROM metadata WHERE key LIKE 'pending:%' AND key NOT IN (?,?)", tuple(f"pending:{g}" for g in keep))

    def begin(self, request):
        require(self.events_approved, "events_not_approved")
        epoch = request["source_epoch"]
        require(valid_hash(epoch), "invalid_source_epoch")
        build_id = digest(canonical([SCHEMA, self.signature, self.scope, epoch]))
        with self.connection(write=True) as db:
            old = db.execute("SELECT source_epoch FROM generations WHERE id=?", (build_id,)).fetchone()
            if old is None:
                self._gc(db, (self._published(db) or "", build_id))
                receipt = digest(canonical(["begin", build_id]))
                db.execute("INSERT INTO generations(id,source_epoch,state,receipt) VALUES (?,?,'building',?)", (build_id, epoch, receipt))
                db.execute("INSERT INTO metadata VALUES (?, '0')", (f"pending:{build_id}",))
            else:
                require(old[0] == epoch, "generation_conflict")
            result = self._stats(db, build_id)
        return result

    def _source(self, value, count):
        shape(value, {"ref", "content_hash", "scope", "role", "session_ref", "occurred_at", "text_bytes"})
        ref = text(value["ref"], 512)
        require(ref.startswith("event:") and not any(c.isspace() or ord(c) < 32 or c in "/\\" for c in ref)
                and ".." not in ref, "invalid_source")
        require(valid_hash(value["content_hash"]) and value["scope"] == self.scope, "scope_or_source_mismatch")
        require(value["role"] in ("user", "assistant", "system", "tool", "unknown"), "invalid_source")
        if value["session_ref"] is not None:
            text(value["session_ref"], 512)
        if value["occurred_at"] is not None:
            text(value["occurred_at"], 64)
        integer(value["text_bytes"], 1, 2**53-1)
        integer(count, 1, MAX_CHUNKS)
        return tuple(value[k] for k in ("ref", "content_hash", "scope", "role", "session_ref", "occurred_at", "text_bytes")) + (count,)

    def _prepared(self, value, *, query=False):
        text(value, 4096 if query else MAX_CHUNK_BYTES)
        prefix = self.space["query_instruction" if query else "document_instruction"]
        output = f"{prefix}\n{value}" if prefix else value
        text(output, 8192 if query else MAX_BATCH_BYTES)
        return output

    def _encode(self, kind, texts):
        require(self.query_approved if kind == "query" else self.events_approved,
                "query_not_approved" if kind == "query" else "events_not_approved")
        if self.encoder is not None:
            output = self.encoder(kind, texts)
        elif kind == "query":
            client = LocalQueryClient(self.space["endpoint"], [self.scope], max_calls=1, max_bytes=MAX_BATCH_BYTES)
            output = client.embed(self.embedding_space, texts, [self.scope], "query")
        else:
            client = LocalIndexClient(self.space["endpoint"], [self.scope], max_calls=1, max_bytes=MAX_BATCH_BYTES)
            output = client.embed_documents(self.embedding_space, texts, [self.scope])
        require(isinstance(output, list) and len(output) == len(texts), "invalid_vector")
        return [vector_blob(v, self.dimensions) for v in output]

    def upsert(self, request):
        require(self.events_approved, "events_not_approved")
        build_id = request["build_id"]
        require(valid_hash(build_id), "invalid_generation")
        batch_id = text(request["batch_id"], 128)
        count = request["source_chunk_count"]
        source = self._source(request["source"], count)
        chunks = request["chunks"]
        require(isinstance(chunks, list) and 1 <= len(chunks) <= MAX_BATCH, "batch_limit")
        require(len(chunks) * self.dimensions <= 8192, "batch_limit")
        rows = []
        prepared = {}
        size = 0
        for c in chunks:
            shape(c, {"ordinal", "start_byte", "end_byte", "text_sha256", "text"})
            ordinal = integer(c["ordinal"], 0, count-1)
            start = integer(c["start_byte"], 0, source[6]-1)
            end = integer(c["end_byte"], start+1, source[6])
            raw = text(c["text"], MAX_CHUNK_BYTES).encode()
            require(len(raw) == end-start and digest(raw) == c["text_sha256"], "chunk_hash_or_range_mismatch")
            item = self._prepared(c["text"])
            size += len(item.encode())
            input_hash = digest(item.encode())
            prepared[input_hash] = item
            rows.append((ordinal, start, end, c["text_sha256"], input_hash))
        require(len({r[0] for r in rows}) == len(rows) and size <= MAX_BATCH_BYTES, "batch_limit")
        rows.sort()
        payload = digest(canonical([source, rows]))
        with self.connection() as db:
            gen = db.execute("SELECT state FROM generations WHERE id=?", (build_id,)).fetchone()
            require(gen is not None, "unknown_generation")
            prior = db.execute("SELECT payload_hash FROM batches WHERE generation=? AND batch_id=?", (build_id, batch_id)).fetchone()
            if prior:
                require(prior[0] == payload, "batch_conflict")
                return {**self._stats(db, build_id), "encoded_chunks": 0, "reused_chunks": len(rows), "already_applied": True}
            require(gen[0] == "building", "generation_complete")
            existing = db.execute("SELECT ref,content_hash,scope,role,session_ref,occurred_at,text_bytes,expected_chunks FROM sources WHERE generation=? AND ref=?", (build_id, source[0])).fetchone()
            require(existing is None or tuple(existing) == source, "source_conflict")
            for ordinal, start, end, chunk_hash, input_hash in rows:
                previous = db.execute("SELECT start_byte,end_byte,text_sha256,input_hash FROM chunks WHERE generation=? AND source_ref=? AND ordinal=?", (build_id, source[0], ordinal)).fetchone()
                require(previous is None or tuple(previous) == (start, end, chunk_hash, input_hash), "chunk_conflict")
            cached = {h: row[0] for h in prepared if (row := db.execute("SELECT vector FROM vectors WHERE input_hash=?", (h,)).fetchone())}
        for blob in cached.values():
            decode_blob(blob, self.dimensions)
        missing = [h for h in prepared if h not in cached]
        # 唯一可能接触提供器的步骤在数据库事务外；只允许固定本机端点。
        encoded = self._encode("corpus", [prepared[h] for h in missing]) if missing else []
        cached.update(zip(missing, encoded))
        with self.connection(write=True) as db:
            gen = db.execute("SELECT state,receipt,source_count,chunk_count FROM generations WHERE id=?", (build_id,)).fetchone()
            require(gen is not None and gen[0] == "building", "generation_changed")
            prior = db.execute("SELECT payload_hash FROM batches WHERE generation=? AND batch_id=?", (build_id, batch_id)).fetchone()
            if prior:
                require(prior[0] == payload, "batch_conflict")
                return {**self._stats(db, build_id), "encoded_chunks": len(missing), "reused_chunks": len(rows)-len(missing), "already_applied": True}
            existing = db.execute("SELECT ref,content_hash,scope,role,session_ref,occurred_at,text_bytes,expected_chunks FROM sources WHERE generation=? AND ref=?", (build_id, source[0])).fetchone()
            require(existing is None or tuple(existing) == source, "source_conflict")
            if existing is None:
                require(gen[2] < MAX_SOURCES, "source_limit")
                db.execute("INSERT INTO sources(generation,ref,content_hash,scope,role,session_ref,occurred_at,text_bytes,expected_chunks) VALUES (?,?,?,?,?,?,?,?,?)", (build_id, *source))
                db.execute("UPDATE generations SET source_count=source_count+1 WHERE id=?", (build_id,))
                db.execute("UPDATE metadata SET value=CAST(value AS INTEGER)+1 WHERE key=?", (f"pending:{build_id}",))
            old_received = db.execute("SELECT received_chunks FROM sources WHERE generation=? AND ref=?", (build_id, source[0])).fetchone()[0]
            added = 0
            for ordinal, start, end, chunk_hash, input_hash in rows:
                previous = db.execute("SELECT start_byte,end_byte,text_sha256,input_hash FROM chunks WHERE generation=? AND source_ref=? AND ordinal=?", (build_id, source[0], ordinal)).fetchone()
                require(previous is None or tuple(previous) == (start, end, chunk_hash, input_hash), "chunk_conflict")
                if previous is not None:
                    continue
                require(gen[3] + added < MAX_CHUNKS, "chunk_limit")
                # 顺序ordinal的范围必须单调；允许相邻chunk重叠，禁止反转/重复范围。
                before = db.execute("SELECT start_byte,end_byte FROM chunks WHERE generation=? AND source_ref=? AND ordinal<? ORDER BY ordinal DESC LIMIT 1", (build_id, source[0], ordinal)).fetchone()
                after = db.execute("SELECT start_byte,end_byte FROM chunks WHERE generation=? AND source_ref=? AND ordinal>? ORDER BY ordinal LIMIT 1", (build_id, source[0], ordinal)).fetchone()
                require((before is None or before[0] < start and before[1] <= end)
                        and (after is None or start < after[0] and end <= after[1]), "chunk_order_invalid")
                inserted = db.execute("INSERT OR IGNORE INTO vectors VALUES (?,?)", (input_hash, cached[input_hash])).rowcount
                if inserted:
                    db.execute("UPDATE metadata SET value=CAST(value AS INTEGER)+1 WHERE key='vector_count'")
                db.execute("INSERT INTO chunks VALUES (?,?,?,?,?,?,?)", (build_id, source[0], ordinal, start, end, chunk_hash, input_hash))
                added += 1
            db.execute("UPDATE sources SET received_chunks=received_chunks+? WHERE generation=? AND ref=?", (added, build_id, source[0]))
            if old_received < count and old_received + added == count:
                db.execute("UPDATE metadata SET value=CAST(value AS INTEGER)-1 WHERE key=?", (f"pending:{build_id}",))
            receipt = digest(canonical([gen[1], batch_id, payload]))
            db.execute("INSERT INTO batches VALUES (?,?,?,?)", (build_id, batch_id, payload, receipt))
            db.execute("UPDATE generations SET receipt=?,accepted_batches=accepted_batches+1,chunk_count=chunk_count+? WHERE id=?", (receipt, added, build_id))
            result = {**self._stats(db, build_id), "encoded_chunks": len(missing), "reused_chunks": len(rows)-len(missing), "already_applied": False}
        return result

    def finish(self, request):
        require(self.events_approved, "events_not_approved")
        for name in ("build_id", "source_epoch", "receipt"):
            require(valid_hash(request[name]), "invalid_generation")
        integer(request["source_count"], 0, MAX_SOURCES)
        integer(request["chunk_count"], 0, MAX_CHUNKS)
        with self.connection(write=True) as db:
            result = self._stats(db, request["build_id"])
            require(result["source_epoch"] == request["source_epoch"] and result["receipt"] == request["receipt"], "snapshot_mismatch")
            require(result["source_count"] == request["source_count"] and result["chunk_count"] == request["chunk_count"], "snapshot_count_mismatch")
            require(result["pending_sources"] == 0, "incomplete_sources")
            require(not db.execute("SELECT 1 FROM sources WHERE generation=? AND received_chunks != expected_chunks LIMIT 1", (request["build_id"],)).fetchone(), "incomplete_sources")
            actual = db.execute("SELECT COUNT(*) FROM chunks WHERE generation=?", (request["build_id"],)).fetchone()[0]
            require(actual == result["chunk_count"], "snapshot_count_mismatch")
            require(not db.execute("SELECT 1 FROM chunks c LEFT JOIN vectors v ON v.input_hash=c.input_hash WHERE c.generation=? AND v.input_hash IS NULL LIMIT 1", (request["build_id"],)).fetchone(), "incomplete_vectors")
            db.execute("UPDATE generations SET state='complete' WHERE id=?", (request["build_id"],))
            db.execute("INSERT INTO metadata VALUES ('published',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (request["build_id"],))
            self._gc(db, (request["build_id"], request["build_id"]))
            self._verified_published(db, request["build_id"])
            result = {**self._stats(db, request["build_id"]), "generation": request["build_id"]}
        return result

    def _query_vector(self, query):
        require(self.query_approved, "query_not_approved")
        prepared = self._prepared(query, query=True)
        input_hash = digest(prepared.encode())
        with self.connection() as db:
            row = db.execute("SELECT vector FROM queries WHERE input_hash=?", (input_hash,)).fetchone()
        blob = row[0] if row else self._encode("query", [prepared])[0]
        values = decode_blob(blob, self.dimensions)
        mode = "cache" if row else "local"
        try:
            with self.connection(write=True) as db:
                used = int(db.execute("SELECT value FROM metadata WHERE key='query_clock'").fetchone()[0]) + 1
                db.execute("UPDATE metadata SET value=? WHERE key='query_clock'", (str(used),))
                db.execute("INSERT INTO queries VALUES (?,?,?) ON CONFLICT(input_hash) DO UPDATE SET used=excluded.used", (input_hash, blob, used))
                db.execute("DELETE FROM queries WHERE input_hash IN (SELECT input_hash FROM queries ORDER BY used DESC LIMIT -1 OFFSET 32)")
        except EventIndexError as error:
            if error.code != "disk_limit":
                raise
            mode = "cache" if row else "local_uncached"
        except sqlite3.Error as error:
            if getattr(error, "sqlite_errorcode", None) != sqlite3.SQLITE_FULL:
                raise
            mode = "cache" if row else "local_uncached"
        # 缓存只是可选优化。磁盘满时事务已回滚，仍可使用此次已验证的本机向量；
        # 权限、损坏和其他数据库错误不能按此路径忽略。
        return values, mode

    def _verified_published(self, db, generation):
        """只在同一读事务中使用；派生计数、关系和字段均不能代替实际行。"""
        require(valid_hash(generation), "corrupt_snapshot")
        row = db.execute("SELECT source_epoch,source_count,chunk_count,state,receipt FROM generations WHERE id=?", (generation,)).fetchone()
        require(row is not None and valid_hash(row[0]) and row[3] == "complete"
                and valid_hash(row[4]) and type(row[1]) is int and 0 <= row[1] <= MAX_SOURCES
                and type(row[2]) is int and 0 <= row[2] <= MAX_CHUNKS, "corrupt_snapshot")
        require(generation == digest(canonical([SCHEMA, self.signature, self.scope, row[0]])), "corrupt_snapshot")
        require(db.execute("SELECT COUNT(*) FROM sources WHERE generation=?", (generation,)).fetchone()[0] == row[1]
                and db.execute("SELECT COUNT(*) FROM chunks WHERE generation=?", (generation,)).fetchone()[0] == row[2], "corrupt_snapshot")
        # SQL只返回常量；在读取TEXT/BLOB到Python之前检查类型和字节界限。
        invalid_source = db.execute("""SELECT 1 FROM sources s WHERE generation=? AND (
            typeof(ref) != 'text' OR length(CAST(ref AS BLOB)) NOT BETWEEN 7 AND 512
            OR substr(ref,1,6) != 'event:' OR instr(ref,char(0)) != 0
            OR typeof(content_hash) != 'text' OR length(content_hash) != 64 OR content_hash GLOB '*[^0-9a-f]*'
            OR typeof(scope) != 'text' OR scope != ? OR role NOT IN ('user','assistant','system','tool','unknown')
            OR (session_ref IS NOT NULL AND (typeof(session_ref) != 'text' OR length(CAST(session_ref AS BLOB)) NOT BETWEEN 1 AND 512 OR instr(session_ref,char(0)) != 0))
            OR (occurred_at IS NOT NULL AND (typeof(occurred_at) != 'text' OR length(CAST(occurred_at AS BLOB)) NOT BETWEEN 1 AND 64 OR instr(occurred_at,char(0)) != 0))
            OR typeof(text_bytes) != 'integer' OR text_bytes NOT BETWEEN 1 AND 9007199254740991
            OR typeof(expected_chunks) != 'integer' OR expected_chunks NOT BETWEEN 1 AND ?
            OR typeof(received_chunks) != 'integer' OR received_chunks != expected_chunks
            OR expected_chunks != (SELECT COUNT(*) FROM chunks c WHERE c.generation=s.generation AND c.source_ref=s.ref)
        ) LIMIT 1""", (generation, self.scope, MAX_CHUNKS)).fetchone()
        require(invalid_source is None, "corrupt_snapshot")
        invalid_chunk = db.execute("""SELECT 1 FROM chunks c
            LEFT JOIN sources s ON s.generation=c.generation AND s.ref=c.source_ref
            LEFT JOIN vectors v ON v.input_hash=c.input_hash
            WHERE c.generation=? AND (
                s.ref IS NULL OR v.input_hash IS NULL
                OR typeof(c.source_ref) != 'text' OR length(CAST(c.source_ref AS BLOB)) NOT BETWEEN 7 AND 512
                OR typeof(c.ordinal) != 'integer' OR c.ordinal < 0 OR c.ordinal >= s.expected_chunks
                OR typeof(c.start_byte) != 'integer' OR typeof(c.end_byte) != 'integer'
                OR c.start_byte < 0 OR c.end_byte <= c.start_byte OR c.end_byte > s.text_bytes
                OR c.end_byte-c.start_byte > ?
                OR typeof(c.text_sha256) != 'text' OR length(c.text_sha256) != 64 OR c.text_sha256 GLOB '*[^0-9a-f]*'
                OR typeof(c.input_hash) != 'text' OR length(c.input_hash) != 64 OR c.input_hash GLOB '*[^0-9a-f]*'
                OR typeof(v.vector) != 'blob' OR length(v.vector) != ?
            ) LIMIT 1""", (generation, MAX_CHUNK_BYTES, self.dimensions * 4)).fetchone()
        require(invalid_chunk is None, "corrupt_snapshot")
        source_hash = hashlib.sha256()
        source_hash.update(b"[")
        first = True
        for ref, in db.execute("SELECT ref FROM sources WHERE generation=? ORDER BY ref", (generation,)):
            if not first:
                source_hash.update(b",")
            source_hash.update(canonical(ref))
            first = False
        source_hash.update(b"]")
        return row[:3], source_hash.hexdigest()

    def query(self, request):
        require(self.query_approved, "query_not_approved")
        query = text(request["query"], 4096)
        limit = integer(request.get("limit", 10), 1, MAX_RESULTS)
        expected = request.get("expected_generation")
        require(expected is None or valid_hash(expected), "invalid_generation")
        with self.connection() as db:
            initial = self._published(db)
            require(initial is not None, "index_unavailable")
            require(expected is None or expected == initial, "generation_mismatch")
            self._verified_published(db, initial)
        vector, mode = self._query_vector(query)
        heap = []
        scanned = 0
        with self.connection() as db:
            generation = self._published(db)
            require(generation == initial, "generation_changed")
            row, source_set_hash = self._verified_published(db, generation)
            cursor = db.execute("SELECT c.source_ref,s.content_hash,c.start_byte,c.end_byte,c.text_sha256,v.vector,c.ordinal,s.role,s.session_ref,s.occurred_at FROM chunks c JOIN sources s ON s.generation=c.generation AND s.ref=c.source_ref JOIN vectors v ON v.input_hash=c.input_hash WHERE c.generation=? AND s.scope=? ORDER BY c.source_ref,c.ordinal", (generation, self.scope))
            best = None
            previous_range = None
            def add_candidate(item):
                if len(heap) < limit:
                    heapq.heappush(heap, item)
                elif item[:3] > heap[0][:3]:
                    heapq.heapreplace(heap, item)
            while page := cursor.fetchmany(QUERY_PAGE):
                for ref, source_hash, start, end, chunk_hash, blob, ordinal, role, session, occurred in page:
                    values = decode_blob(blob, self.dimensions)
                    score = sum(a*b for a, b in zip(vector, values))
                    require(math.isfinite(score) and -1.00001 <= score <= 1.00001, "corrupt_vector")
                    score = max(-1.0, min(1.0, score))
                    # ref+ordinal是稳定且唯一的并列顺序，不混用词法分数。
                    item = (score, ref, -ordinal, source_hash, start, end, chunk_hash, role, session, occurred)
                    scanned += 1
                    if best is None or ref != best[1]:
                        if best is not None:
                            add_candidate(best)
                        best = item
                        previous_range = None
                    require(previous_range is None or previous_range[0] < start and previous_range[1] <= end, "corrupt_snapshot")
                    previous_range = (start, end)
                    if item[:3] > best[:3]:
                        best = item
            if best is not None:
                add_candidate(best)
            require(scanned == row[2], "corrupt_snapshot")
            results = []
            response_bytes = 2048  # 为envelope、generation和coverage留出余量。
            response_limited = False
            for r in sorted(heap, key=lambda r:r[:3], reverse=True):
                candidate = {"ref": r[1], "content_hash": r[3], "start_byte": r[4], "end_byte": r[5], "text_sha256": r[6], "score": r[0],
                             "role": r[7], "session_ref": r[8], "occurred_at": r[9]}
                size = len(canonical(candidate)) + 1
                if response_bytes + size > MAX_RESPONSE:
                    response_limited = True
                    break
                results.append(candidate)
                response_bytes += size
        return {"generation": generation, "source_epoch": row[0], "space_signature": self.signature, "source_set_hash": source_set_hash, "results": results,
                "next_cursor": None, "coverage": {"status": "indexed_snapshot_requires_host_revalidation", "indexed_sources": row[1],
                "indexed_chunks": row[2], "scanned_chunks": scanned, "query_encoding": mode, "candidate_limit": limit,
                "candidates_truncated": row[1] > len(results), "response_limited": response_limited,
                "chunk_selection": "per_event_best_chunk",
                "local_only": True, "external_calls": 0, "page_rows": QUERY_PAGE}}

    def status(self, request):
        with self.connection() as db:
            result = self._stats(db)
        result["database_bytes"] = self.path.stat().st_size
        result["database_and_journal_bytes"] = self._disk_bytes()
        return result

    def dispatch(self, request):
        common = {"schema", "id", "op"}
        require(request.get("schema") == SCHEMA, "invalid_schema")
        op = request.get("op")
        required = {
            "begin": {"source_epoch"},
            "upsert": {"build_id", "batch_id", "source", "source_chunk_count", "chunks"},
            "finish": {"build_id", "source_epoch", "source_count", "chunk_count", "receipt"},
            "query": {"query"},
            "status": set(),
        }
        require(op in required, "unknown_operation")
        shape(request, common | required[op], {"limit", "expected_generation"} if op == "query" else ())
        return getattr(self, op)(request)

    def execute(self, request):
        ident = None
        try:
            require(isinstance(request, dict) and len(canonical(request)) <= MAX_LINE, "input_limit")
            ident = request.get("id")
            require(type(ident) is int and 0 <= ident <= 2**53-1 or isinstance(ident, str) and bool(text(ident, 128)), "invalid_request")
            result = {"schema": SCHEMA, "id": ident, "ok": True, "result": self.dispatch(request)}
            require(len(canonical(result)) <= MAX_RESPONSE, "output_limit")
            return result
        except EventIndexError as error:
            code = error.code
        except EmbeddingError as error:
            code = error.code if error.code in {"local_provider_unavailable", "local_provider_error", "redirect_blocked", "response_limit", "local_budget_exhausted"} else "local_encoding_failed"
        except sqlite3.Error as error:
            code = {sqlite3.SQLITE_FULL: "disk_limit", sqlite3.SQLITE_BUSY: "index_busy", sqlite3.SQLITE_LOCKED: "index_busy"}.get(getattr(error, "sqlite_errorcode", None), "index_storage_error")
        except (OSError, ValueError, TypeError, OverflowError, RecursionError):
            code = "invalid_request_or_storage"
        except Exception:
            code = "worker_error"
        return {"schema": SCHEMA, "id": ident if type(ident) is int or isinstance(ident, str) and len(ident) <= 128 else None,
                "ok": False, "error": {"code": code}}


def serve(worker, source, destination):
    while True:
        line = source.readline(MAX_LINE + 1)
        if not line:
            return
        if len(line) > MAX_LINE:
            while line and not line.endswith(b"\n"):
                line = source.readline(65536)
            result = {"schema": SCHEMA, "id": None, "ok": False, "error": {"code": "input_limit"}}
        else:
            try:
                result = worker.execute(strict_json(line))
            except EventIndexError as error:
                result = {"schema": SCHEMA, "id": None, "ok": False, "error": {"code": error.code}}
        destination.write(canonical(result) + b"\n")
        destination.flush()


def _supported_python():
    return sys.version_info >= (3, 11) and callable(getattr(sqlite3.Connection, "setlimit", None))


class _UnsupportedPython:
    """旧解释器仍能读有界协议并说明缺少能力；不打开配置、数据库或提供器。"""
    @staticmethod
    def execute(request):
        ident = request.get("id") if isinstance(request, dict) else None
        if type(ident) is int:
            if not 0 <= ident <= 2**53-1:
                ident = None
        elif isinstance(ident, str):
            try:
                text(ident, 128)
            except EventIndexError:
                ident = None
        else:
            ident = None
        return {"schema": SCHEMA, "id": ident, "ok": False,
                "error": {"code": "python_version_unsupported"}}


def main(argv=None):
    parser = argparse.ArgumentParser(description="RecallCard 独立本机 Event 索引；只有可信启动配置授予权限")
    parser.add_argument("--config", required=True, help="私有本机配置JSON；拒绝符号链接、广泛权限及超过64KiB")
    arguments = parser.parse_args(argv)
    try:
        if not _supported_python():
            serve(_UnsupportedPython(), sys.stdin.buffer, sys.stdout.buffer)
            return 0
        worker = EventIndex(read_config(arguments.config))
        serve(worker, sys.stdin.buffer, sys.stdout.buffer)
        return 0
    except Exception:
        # 启动失败不输出异常字符串（可能包含私有路径、配置或SQLite内容）。
        sys.stderr.write("本机 Event 索引启动失败，请检查私有配置、权限和磁盘。\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
