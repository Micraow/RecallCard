"""仅测试诊断方法合同，不把 fake 排名器当生产召回结果。"""
import copy
import unittest
from candidates import MODES, normalize, projection, search, visible_sources


class FakeRank:
    def __init__(self):
        self.seen = None
    def terms(self, query, mode):
        return query.split()
    def search(self, query, mode, docs):
        self.seen = copy.deepcopy(docs)
        return [{"id": doc["source_id"], "score": float(i + 1)} for i, doc in enumerate(docs)]


def source(identifier="source", **extra):
    return {"source_id": identifier, "kind": "event", "scope": "personal", "revoked": False,
            "title": "不进入生产词法字段的标题", "filename": "", "text": "设备检查已登记。", **extra}


class CandidateContractTest(unittest.TestCase):
    def test_exact_and_quoted_input_are_byte_preserved(self):
        for query in ["  ABc_42 /tmp/notes-v2  ", '请问“合成精确词”什么时候', "'synthetic phrase' 怎么处理"]:
            self.assertEqual(normalize(query, "exact"), query)
            if any(char in query for char in '\"“”\'‘’'):
                self.assertEqual(normalize(query, "question"), query)

    def test_question_removal_never_joins_han_fragments(self):
        self.assertEqual(normalize("请问海藻多久检查？", "question"), "海藻 检查")
        self.assertNotIn("藻检", normalize("海藻多久检查", "question"))

    def test_identifier_bytes_are_not_rewritten_by_frames(self):
        query = "请问 ABc_42 /opt/x-v2 v2.1 怎么检查？"
        result = normalize(query, "question")
        for identifier in ["ABc_42", "/opt/x-v2", "v2.1"]:
            self.assertIn(identifier, result)

    def test_negation_is_not_a_stopword(self):
        self.assertEqual(normalize("不要删除合成记录", "question"), "不要删除合成记录")
        self.assertIn("不能", normalize("请问为什么不能删除合成记录？", "question"))

    def test_empty_normalization_is_an_empty_query_not_an_all_document_query(self):
        rank = FakeRank()
        case = {"query": "请问多久？", "intent": "question"}
        rows, explanation = search(rank, "normalized_question", [source()], case)
        self.assertEqual(rows, [])
        self.assertEqual(explanation["terms"], [])
        self.assertIsNone(rank.seen)

    def test_scope_and_revocation_are_filtered_before_rank(self):
        rank = FakeRank()
        sources = [source("visible"), source("hidden", scope="project:hidden"), source("revoked", revoked=True)]
        case = {"query": "设备", "intent": "exact", "visible_scopes": ["personal"]}
        for mode in MODES:
            rows, _ = search(rank, mode, sources, case)
            self.assertEqual([row["id"] for row in rows], ["visible"])
            self.assertEqual([row["source_id"] for row in rank.seen], ["visible"])
        self.assertEqual(len(visible_sources(sources, {"visible_scopes": []})), 0)

    def test_attachment_is_independent_evidence_and_title_is_not_promoted(self):
        doc = projection(source(kind="attachment", filename="合成材料.txt"))
        self.assertEqual(doc["evidence"], "Attachment/Content")
        self.assertEqual(doc["entities"], ["合成材料.txt"])
        self.assertNotIn("不进入生产词法字段的标题", str(doc))
        self.assertEqual(projection(source(role="assistant"))["evidence"], "Assistant/Native")

    def test_exact_rank_order_is_unchanged_by_experiments(self):
        sources = [source("a", text="设备"), source("b", text="其他", filename="设备.txt")]
        case = {"query": "设备", "intent": "exact"}
        baseline, _ = search(FakeRank(), "production_question_intent", sources, case)
        for mode in MODES[2:]:
            rows, _ = search(FakeRank(), mode, sources, case)
            self.assertEqual(rows, baseline)

    def test_field_weighting_does_not_add_candidates(self):
        class OneResult(FakeRank):
            def search(self, query, mode, docs):
                return [{"id": docs[0]["source_id"], "score": 1.0}]
        case = {"query": "设备何时检查", "intent": "question"}
        rows, explanation = search(OneResult(), "normalized_body_intent_weighted", [source("a"), source("b")], case)
        self.assertEqual([row["id"] for row in rows], ["a"])
        self.assertTrue(explanation["production_eligibility"])

    def test_single_character_relaxation_is_explicit_negative_control(self):
        class NoResults(FakeRank):
            def terms(self, query, mode):
                return ["海藻", "藻检"]
            def search(self, query, mode, docs):
                return []
        rows, explanation = search(NoResults(), "unigram_relaxation_negative_control",
                                   [source(text="海风中检查记录")], {"query": "海藻检查", "intent": "question"})
        self.assertEqual([row["id"] for row in rows], ["source"])
        self.assertFalse(explanation["production_eligibility"])
        self.assertIn("negative_control_only", explanation["warning"])


if __name__ == "__main__":
    unittest.main()
