"""Opt-in SQLite vector store; never loads models, sends data, or scans a Vault.

The trusted host supplies current canonical source states under its read lock.
This is an independent v2 building block, not an enabled product search path.
"""
from __future__ import annotations
from contextlib import contextmanager
from dataclasses import dataclass, field
import hashlib
import heapq
import json
import math
from pathlib import Path
import sqlite3
import struct
from typing import Iterable, Mapping

MAX_DIMENSIONS = 8192
MAX_BATCH = 64
QUERY_PAGE = 256
MAX_RESULTS = 100
MAX_SOURCES = 100_000
MAX_MANIFEST_BYTES = 32 * 1024 * 1024
MAX_RELATIONS = 16
DEFAULT_DISK_CAP = 512 * 1024 * 1024


class IndexError(ValueError):
    """Fixed diagnostics only; no private source text or provider responses."""


def canonical(value) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def require(condition, message):
    if not condition:
        raise IndexError(message)


def valid_hash(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def bounded_text(value, limit=512):
    return isinstance(value, str) and 0 < len(value.encode()) <= limit and "\x00" not in value


@dataclass(frozen=True)
class SourceState:
    ref: str
    kind: str
    scope: str
    content_hash: str
    evidence: Mapping[str, str] = field(default_factory=dict)
    # Canonical, authorized host hints, not inferred from timestamp ordering.
    relations: Mapping[str, tuple[str, ...]] = field(default_factory=dict)
    session_ref: str | None = None
    role: str | None = None
    occurred_at: str | None = None

    def validate(self):
        require(self.kind in ("event", "memory"), "invalid_source_kind")
        require(bounded_text(self.ref) and self.ref.startswith(self.kind + ":"), "invalid_source_ref")
        require(bounded_text(self.scope, 128) and valid_hash(self.content_hash), "invalid_source_identity")
        require(len(self.evidence) <= MAX_SOURCES, "evidence_limit")
        for ref, hash_ in self.evidence.items():
            require(bounded_text(ref) and ref.startswith("event:") and valid_hash(hash_), "invalid_evidence")
        require(self.kind != "memory" or bool(self.evidence), "memory_requires_sources")
        require(self.session_ref is None or bounded_text(self.session_ref), "invalid_session")
        require(self.role is None or self.role in ("user", "assistant", "system", "tool", "unknown"), "invalid_role")
        require(self.occurred_at is None or bounded_text(self.occurred_at, 64), "invalid_time")
        require(set(self.relations) <= {"previous", "next", "corrects", "branch_choices"}, "invalid_relation_kind")
        require(sum(len(v) for v in self.relations.values()) <= MAX_RELATIONS, "relation_limit")
        for values in self.relations.values():
            require(isinstance(values, (tuple, list)), "invalid_relation_refs")
            require(all(bounded_text(v) and v.startswith(("event:", "memory:")) for v in values), "invalid_relation_refs")
        return self

    def identity(self):
        # Navigation hints may change without changing a source's embedding.
        return {"ref": self.ref, "kind": self.kind, "scope": self.scope,
                "content_hash": self.content_hash, "evidence": dict(self.evidence),
                "session_ref": self.session_ref, "role": self.role, "occurred_at": self.occurred_at}


@dataclass(frozen=True)
class EncodedChunk:
    source_ref: str
    input_hash: str
    start_byte: int
    end_byte: int
    vector: tuple[float, ...]


def vector_bytes(values, dimensions):
    require(len(values) == dimensions, "dimension_mismatch")
    require(all(type(v) in (int, float) and math.isfinite(v) for v in values), "invalid_vector")
    norm = math.hypot(*values)
    require(math.isfinite(norm) and norm > 0, "invalid_vector_norm")
    try:
        return struct.pack("<" + "f" * dimensions, *(v / norm for v in values))
    except (OverflowError, struct.error):
        raise IndexError("invalid_float32_vector") from None


class BoundedIndex:
    def __init__(self, path: Path, space: dict, *, disk_cap=DEFAULT_DISK_CAP):
        # Paths/configuration belong to trusted startup, never an untrusted query.
        self.path = Path(path)
        require(type(space.get("dimensions")) is int and 1 <= space["dimensions"] <= MAX_DIMENSIONS, "invalid_dimensions")
        require({"provider", "model", "revision", "dimensions", "preprocessing"} <= space.keys(), "incomplete_space")
        require(len(canonical(space)) <= 16384, "space_limit")
        require(type(disk_cap) is int and 65536 <= disk_cap <= 4 * 1024**3, "invalid_disk_cap")
        self.space = json.loads(canonical(space))
        self.signature = digest(canonical(space))
        self.dimensions = space["dimensions"]
        self.disk_cap = disk_cap
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.connection() as db:
            db.executescript("""
            CREATE TABLE IF NOT EXISTS spaces(signature TEXT PRIMARY KEY, config BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS generations(id TEXT PRIMARY KEY, signature TEXT NOT NULL,
              source_manifest TEXT NOT NULL, expected INTEGER NOT NULL, cursor INTEGER NOT NULL DEFAULT 0,
              status TEXT NOT NULL DEFAULT 'building');
            CREATE TABLE IF NOT EXISTS sources(generation TEXT NOT NULL, ref TEXT NOT NULL,
              identity BLOB NOT NULL, PRIMARY KEY(generation,ref));
            CREATE TABLE IF NOT EXISTS chunks(generation TEXT NOT NULL, ordinal INTEGER NOT NULL,
              ref TEXT NOT NULL, input_hash TEXT NOT NULL, start_byte INTEGER NOT NULL,
              end_byte INTEGER NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(generation,ordinal));
            CREATE TABLE IF NOT EXISTS batches(generation TEXT NOT NULL, start INTEGER NOT NULL,
              count INTEGER NOT NULL, digest TEXT NOT NULL, PRIMARY KEY(generation,start));
            CREATE TABLE IF NOT EXISTS published(signature TEXT PRIMARY KEY, generation TEXT NOT NULL);
            """)
            db.execute("INSERT OR IGNORE INTO spaces VALUES (?,?)", (self.signature, canonical(space)))
            actual = db.execute("SELECT config FROM spaces WHERE signature=?", (self.signature,)).fetchone()[0]
            require(actual == canonical(space), "space_signature_conflict")

    @contextmanager
    def connection(self):
        db = sqlite3.connect(self.path, timeout=1.0, isolation_level=None)
        try:
            db.execute("PRAGMA trusted_schema=OFF")
            db.execute("PRAGMA temp_store=MEMORY")
            db.execute("PRAGMA cache_size=-2048")
            page_size = db.execute("PRAGMA page_size").fetchone()[0]
            db.execute(f"PRAGMA max_page_count={self.disk_cap // page_size}")
            yield db
        finally:
            db.close()

    @staticmethod
    def source_map(sources: Iterable[SourceState]):
        result = {}
        for source in sources:
            source.validate()
            require(source.ref not in result, "duplicate_source")
            # Seal caller-owned mappings for this operation's snapshot.
            result[source.ref] = SourceState(source.ref, source.kind, source.scope, source.content_hash,
                dict(source.evidence), {k: tuple(v) for k,v in source.relations.items()},
                source.session_ref, source.role, source.occurred_at)
            require(len(result) <= MAX_SOURCES, "source_limit")
        identities = {r: s.identity() for r, s in sorted(result.items())}
        require(len(canonical(identities)) <= MAX_MANIFEST_BYTES, "manifest_limit")
        return result, identities

    def begin(self, sources: Iterable[SourceState], expected_chunks: int) -> str:
        _, identities = self.source_map(sources)
        require(type(expected_chunks) is int and 0 <= expected_chunks <= 1_000_000, "chunk_count_limit")
        require(expected_chunks * self.dimensions * 4 <= self.disk_cap, "vector_disk_limit")
        manifest = digest(canonical(identities))
        generation = digest(canonical([self.signature, manifest, expected_chunks]))
        with self.connection() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT signature,source_manifest,expected FROM generations WHERE id=?", (generation,)).fetchone()
                if row:
                    require(row == (self.signature, manifest, expected_chunks), "generation_conflict")
                else:
                    db.execute("INSERT INTO generations(id,signature,source_manifest,expected) VALUES (?,?,?,?)", (generation, self.signature, manifest, expected_chunks))
                    db.executemany("INSERT INTO sources VALUES (?,?,?)", [(generation, ref, canonical(value)) for ref, value in identities.items()])
                db.commit()
            except Exception:
                db.rollback()
                raise
        return generation

    def checkpoint(self, generation):
        with self.connection() as db:
            row = db.execute("SELECT signature,cursor,expected,status FROM generations WHERE id=?", (generation,)).fetchone()
        require(row and row[0] == self.signature, "unknown_generation")
        return {"completed": row[1], "expected": row[2], "status": row[3]}

    def append(self, generation: str, start: int, chunks: list[EncodedChunk]):
        # The caller performs model inference BEFORE entering this operation.
        require(type(start) is int and start >= 0, "invalid_cursor")
        require(0 < len(chunks) <= MAX_BATCH, "batch_limit")
        encoded = []
        for c in chunks:
            require(bounded_text(c.source_ref) and valid_hash(c.input_hash), "invalid_chunk_identity")
            require(type(c.start_byte) is int and type(c.end_byte) is int and 0 <= c.start_byte < c.end_byte <= 2**53, "invalid_span")
            encoded.append((c.source_ref, c.input_hash, c.start_byte, c.end_byte, vector_bytes(c.vector, self.dimensions)))
        receipt = digest(canonical([[*c[:4], digest(c[4])] for c in encoded]))
        with self.connection() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                gen = db.execute("SELECT signature,cursor,expected,status FROM generations WHERE id=?", (generation,)).fetchone()
                require(gen and gen[0] == self.signature, "unknown_generation")
                old = db.execute("SELECT count,digest FROM batches WHERE generation=? AND start=?", (generation, start)).fetchone()
                if old:
                    require(old == (len(chunks), receipt), "checkpoint_conflict")
                    db.commit()
                    return gen[1]
                require(gen[3] == "building" and gen[1] == start and start + len(chunks) <= gen[2], "cursor_mismatch")
                for ref in {c[0] for c in encoded}:
                    require(db.execute("SELECT 1 FROM sources WHERE generation=? AND ref=?", (generation, ref)).fetchone(), "missing_source")
                db.executemany("INSERT INTO chunks VALUES (?,?,?,?,?,?,?)", [(generation, start + i, *c) for i, c in enumerate(encoded)])
                db.execute("INSERT INTO batches VALUES (?,?,?,?)", (generation, start, len(chunks), receipt))
                db.execute("UPDATE generations SET cursor=? WHERE id=?", (start + len(chunks), generation))
                db.commit()
            except Exception:
                db.rollback()
                raise
        return start + len(chunks)

    def publish(self, generation):
        with self.connection() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                g = db.execute("SELECT signature,cursor,expected FROM generations WHERE id=?", (generation,)).fetchone()
                require(g and g[0] == self.signature and g[1] == g[2], "incomplete_generation")
                require(db.execute("SELECT COUNT(*) FROM chunks WHERE generation=?", (generation,)).fetchone()[0] == g[2], "incomplete_generation")
                db.execute("UPDATE generations SET status='complete' WHERE id=?", (generation,))
                db.execute("INSERT INTO published VALUES (?,?) ON CONFLICT(signature) DO UPDATE SET generation=excluded.generation", (self.signature, generation))
                db.commit()
            except Exception:
                db.rollback()
                raise

    @staticmethod
    def visible(source, current, allowed_scopes):
        if source.scope not in allowed_scopes:
            return False
        if source.kind == "memory":
            for ref, hash_ in source.evidence.items():
                event = current.get(ref)
                if not event or event.kind != "event" or event.scope not in allowed_scopes or event.content_hash != hash_:
                    return False
        return True

    def query(self, vector, current_sources: Iterable[SourceState], allowed_scopes: set[str], *, limit=10,
              kind=None, max_chunks_per_source=None, per_session_limit=None, include_candidate_pool=False,
              target_refs=None, backend="python"):
        require(type(limit) is int and 1 <= limit <= MAX_RESULTS, "result_limit")
        require(kind in (None, "event", "memory"), "invalid_source_kind")
        for value in (max_chunks_per_source, per_session_limit):
            require(value is None or type(value) is int and 1 <= value <= MAX_RESULTS, "invalid_diversity_limit")
        require(type(include_candidate_pool) is bool, "invalid_candidate_pool")
        require(target_refs is None or isinstance(target_refs, set) and len(target_refs) <= MAX_RESULTS, "invalid_target_refs")
        require(backend in ("python", "numpy"), "invalid_backend")
        np = None
        if backend == "numpy":
            try:
                import numpy as np
            except ImportError:
                raise IndexError("numpy_not_installed") from None
        current, identities = self.source_map(current_sources)
        identity_bytes = {r: canonical(identities[r]) for r,s in current.items()
                          if (not kind or s.kind == kind) and (target_refs is None or r in target_refs)
                          and self.visible(s, current, allowed_scopes)}
        q = struct.unpack("<" + "f" * self.dimensions, vector_bytes(vector, self.dimensions))
        heap = []; selected = {}; raw_heap = []; eligible_sources = set(); scanned = 0; generation = None
        ranked_chunks = 0
        with self.connection() as db:
            db.execute("BEGIN")
            row = db.execute("SELECT generation FROM published WHERE signature=?", (self.signature,)).fetchone()
            if not row:
                return {"results": [], "coverage": "unavailable", "space_signature": self.signature}
            generation = row[0]
            cursor = db.execute("SELECT c.ordinal,c.ref,c.start_byte,c.end_byte,c.vector,s.identity FROM chunks c JOIN sources s ON s.generation=c.generation AND s.ref=c.ref WHERE c.generation=? ORDER BY c.ordinal", (generation,))
            while page := cursor.fetchmany(QUERY_PAGE):
                filtered = []
                for ordinal, ref, start, end, blob, old_identity in page:
                    scanned += 1
                    if ref not in identity_bytes or old_identity != identity_bytes[ref]:
                        continue
                    require(len(blob) == self.dimensions * 4, "corrupt_vector_size")
                    filtered.append((ordinal,ref,start,end,blob))
                if np is not None and filtered:
                    # At most one authorized page is materialized, never the whole index.
                    matrix = np.frombuffer(b"".join(r[4] for r in filtered), dtype="<f4").reshape(-1,self.dimensions).astype("float64")
                    require(bool(np.isfinite(matrix).all()), "corrupt_vector")
                    norms = np.sqrt((matrix*matrix).sum(axis=1))
                    require(bool(np.isclose(norms,1.,rtol=1e-5,atol=1e-5).all()), "corrupt_vector_norm")
                    scores = (matrix*np.asarray(q,dtype="float64")).sum(axis=1).tolist()
                else:
                    scores = []
                    for row_ in filtered:
                        values = struct.unpack("<" + "f" * self.dimensions,row_[4])
                        require(all(math.isfinite(v) for v in values), "corrupt_vector")
                        require(math.isclose(math.hypot(*values),1.,rel_tol=1e-5,abs_tol=1e-5), "corrupt_vector_norm")
                        scores.append(sum(a*b for a,b in zip(q,values)))
                for (ordinal,ref,start,end,_),score in zip(filtered,scores):
                    source = current[ref]
                    eligible_sources.add(ref)
                    ranked_chunks += 1
                    item = (score, -ordinal, ref, start, end)
                    if include_candidate_pool:
                        if len(raw_heap) < MAX_RESULTS:heapq.heappush(raw_heap,item)
                        elif item > raw_heap[0]:heapq.heapreplace(raw_heap,item)
                    group = source.session_ref or ref
                    same_source = [v for v in selected.values() if v[2] == ref]
                    same_group = [v for v in selected.values() if (current[v[2]].session_ref or v[2]) == group]
                    constrained = (same_source if max_chunks_per_source is not None and len(same_source) >= max_chunks_per_source
                                   else same_group if per_session_limit is not None and len(same_group) >= per_session_limit else [])
                    if constrained:
                        worst = min(constrained)
                        if item > worst:
                            del selected[-worst[1]]
                            selected[ordinal] = item
                            heap = list(selected.values())
                            heapq.heapify(heap)
                        continue
                    if len(heap) < limit:
                        heapq.heappush(heap, item)
                        selected[ordinal] = item
                    elif item > heap[0]:
                        removed = heapq.heapreplace(heap, item)
                        del selected[-removed[1]]
                        selected[ordinal] = item
            db.commit()
        def output_row(item):
            score, negative_ordinal, ref, start, end = item
            source = current[ref]
            relations = {k: [r for r in refs if r in current and current[r].scope == source.scope and self.visible(current[r], current, allowed_scopes)] for k,refs in source.relations.items()}
            return {"ref": ref, "kind": source.kind, "scope": source.scope, "content_hash": source.content_hash,
                         "score": score, "range": [start,end], "ordinal": -negative_ordinal,
                         "evidence": dict(source.evidence), "relations": relations,
                         "session_ref": source.session_ref, "role": source.role, "occurred_at": source.occurred_at}
        rows = [output_row(item) for item in sorted(heap, reverse=True)]
        wanted = set(identity_bytes)
        return {"results": rows, "space_signature": self.signature, "generation": generation,
                "coverage": "complete" if eligible_sources == wanted else "partial", "indexed_current_sources": len(eligible_sources),
                "eligible_current_sources": len(wanted), "scanned_chunks": scanned, "page_rows": QUERY_PAGE,
                "max_chunks_per_source": max_chunks_per_source, "per_session_limit": per_session_limit,
                "ranked_chunks_before_diversity": ranked_chunks,
                "candidate_pool": [output_row(item) for item in sorted(raw_heap,reverse=True)],
                "candidate_pool_truncated": include_candidate_pool and ranked_chunks > MAX_RESULTS,
                "backend": backend}
