"""冻结的无模型查询消融；只供诊断，不修改生产默认或补写语义同义词。"""
from __future__ import annotations
import re

VERSION = "recallcard.query-diagnostics/1"
# 通用会话框架与疑问语法，不含领域名词、来源 ID 或问题到答案的词表。
PREFIXES = tuple(sorted(("请帮我查一下", "能帮我查一下", "请帮我找一下", "我想了解一下",
                        "请问一下", "我想知道", "我想查一下", "请告诉我", "帮我找一下",
                        "麻烦查一下", "请问", "我之前", "之前", "关于"), key=lambda x: (-len(x), x)))
QUESTIONS = tuple(sorted(("是什么时候", "什么时候", "什么时间", "什么地方", "多长时间",
                         "哪个时候", "在哪里", "在哪儿", "多少次", "多少天", "多少度", "多少钱",
                         "为什么", "星期几", "有没有", "是什么", "如何", "怎么", "何时", "多久",
                         "几天", "几次", "几点", "几度", "为何", "是否"), key=lambda x: (-len(x), x)))
QUESTION_RE = re.compile("|".join(map(re.escape, QUESTIONS)))
QUOTES = frozenset('\"“”\'‘’')
ASCII = re.compile(r"[A-Za-z0-9_./:+#-]+")
TIME_QUERY = re.compile(r"何时|什么时候|什么时间|星期几|多久|多长时间|几天|几点|多少天")
TIME_VALUE = re.compile(r"(?:周|星期)[一二三四五六日天]|[0-9零一二三四五六七八九十百两]+(?:年|月|日|天|周|小时|分钟|秒)")
CAUSE_QUERY = re.compile(r"为什么|为何")
CAUSE_VALUE = re.compile(r"因为|由于|原因|因此|为避免")
MODES = ("production_default", "production_question_intent", "normalized_question",
         "normalized_body_intent_weighted", "unigram_relaxation_negative_control")


def han(char):
    return "\u3400" <= char <= "\u9fff" or "\uf900" <= char <= "\ufaff"


def normalize(query, intent):
    if intent != "question" or any(char in QUOTES for char in query):
        return query
    result = query.strip()
    for _ in range(2):
        prefix = next((prefix for prefix in PREFIXES if result.startswith(prefix)), None)
        if prefix is None:
            break
        result = result[len(prefix):].lstrip()
    # 留空格，不把删除前后汉字拼成原查询不存在的新词组。
    result = QUESTION_RE.sub(" ", result)
    result = re.sub(r"[，。？！；、,?!;]+", " ", result)
    return " ".join(result.split())


def projection(source):
    # 对应当前 Document 投影：标题不进普通 Event 词法字段，附件名是 entities。
    return {"source_id": source["source_id"], "text": source["text"],
            "entities": [source["filename"]] if source.get("filename") else [], "labels": [],
            "evidence": "Attachment/Content" if source["kind"] == "attachment" else
                        "Assistant/Native" if source.get("role") == "assistant" else "User/Native",
            "state": "captured"}


def visible_sources(sources, case):
    allowed = case.get("visible_scopes", ["personal"])
    return [source for source in sources if source["scope"] in allowed and not source["revoked"]]


def matches_identifier(text, token):
    # 仅负对照需要；正例策略仍完全由生产 Rust 匹配完整标识符。
    word = lambda char: (not han(char) and char.isalnum()) or char == "_"
    lowered = text.lower()
    for match in re.finditer(re.escape(token.lower()), lowered):
        if (match.start() == 0 or not word(lowered[match.start()-1])) and (match.end() == len(lowered) or not word(lowered[match.end()])):
            return True
    return False


def search(rank, mode, sources, case):
    assert mode in MODES
    selected = visible_sources(sources, case)
    docs = [projection(source) for source in selected]
    query, intent = case["query"], case["intent"]
    lexical = "natural_question" if intent == "question" and mode != "production_default" else "exact"
    rewritten = normalize(query, intent) if mode.startswith("normalized_") else query
    terms = rank.terms(rewritten, lexical)
    rows = rank.search(rewritten, lexical, docs) if terms else []
    explanation = {"query": rewritten, "changed": rewritten != query, "terms": terms,
                   "scope_filtered_before_ranking": True, "production_eligibility": True}
    if mode == "normalized_body_intent_weighted" and intent == "question" and not any(c in QUOTES for c in query):
        by_id = {source["source_id"]: source for source in selected}
        for row in rows:
            text = by_id[row["id"]]["text"].lower()
            body_fraction = sum(term in text for term in terms) / max(1, len(terms))
            factor = 0.35 + 0.65 * body_fraction
            # 只改已进入候选的排序，意图提示不能创造缺失的词法证据或断言可回答。
            if TIME_QUERY.search(query) and TIME_VALUE.search(text):
                factor *= 1.15
            if CAUSE_QUERY.search(query) and CAUSE_VALUE.search(text):
                factor *= 1.15
            row["score"] *= factor
        explanation["weighting"] = "body_fraction:0.35+0.65*f; explicit_time_or_cause:1.15"
    elif mode == "unigram_relaxation_negative_control" and intent == "question" and not any(c in QUOTES for c in query):
        chars = {char for char in query if han(char)}
        identifiers = [token for token in terms if not all(han(c) for c in token)]
        ids = {row["id"] for row in rows}
        for doc in docs:
            fields = [doc["text"], *doc["entities"]]
            if any(not any(matches_identifier(field, token) for field in fields) for token in identifiers):
                continue
            matched = {char for char in chars if any(char in field for field in fields)}
            if doc["source_id"] not in ids and len(matched) >= 2 and len(matched) / max(1, len(chars)) >= 0.2:
                rows.append({"id": doc["source_id"], "score": len(matched) / max(1, len(chars))})
        explanation["production_eligibility"] = False
        explanation["warning"] = "negative_control_only: two shared Han characters; not semantic understanding"
    rows.sort(key=lambda row: (-row["score"], row["id"]))
    return rows, explanation
