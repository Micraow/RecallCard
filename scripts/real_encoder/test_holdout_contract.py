"""冻结附件混合回放的纯stdlib合同；不下载、加载或调用模型。"""
from copy import deepcopy
import unittest
from unittest.mock import patch

from adapter import Encoder
from common import (HERE, HOLDOUT, HOLDOUT_SHA256, HOLDOUT_VARIANTS, allowed_inputs, canonical_digest,
                    datasets, digest, holdout_request_budget, paired_deltas, read_json, summarize)
from run import verify_rows


class HoldoutContracts(unittest.TestCase):
    def setUp(self):
        self.data = datasets("attachment-holdout")[0]

    def rows(self):
        return [{"id": query["id"], "category": query["category"],
                 "expected_sources": deepcopy(query["expected_sources"]), "returned_sources": [],
                 "visible_scopes": deepcopy(query["visible_scopes"]),
                 "route_executed": bool(query["query"].strip()) and len(set(query["visible_scopes"])) <= 4,
                 "query_status": "semantic" if query["query"].strip() and len(set(query["visible_scopes"])) <= 4
                                 else "input_rejected" if not query["query"].strip() else "scope_limit"}
                for query in self.data["queries"]]

    def test_default_suite_preserves_original_fixtures(self):
        legacy = datasets()
        self.assertEqual([len(data["queries"]) for data in legacy], [52, 3])
        self.assertEqual([len(data["corpus"]) for data in legacy], [12, 4])
        with self.assertRaises(ValueError):
            datasets("unreviewed-suite")

    def test_original_holdout_bytes_queries_qrels_and_metadata_are_unchanged(self):
        original = read_json(HOLDOUT)
        self.assertEqual(digest(HOLDOUT), HOLDOUT_SHA256)
        self.assertEqual(self.data["corpus"], original["sources"])
        self.assertEqual(self.data["queries"], original["queries"])
        self.assertEqual(self.data["variants"], list(HOLDOUT_VARIANTS))
        self.assertEqual(sum(source["kind"] == "event" for source in self.data["corpus"]), 12)
        self.assertEqual(sum(source["kind"] == "attachment" for source in self.data["corpus"]), 12)
        self.assertEqual(sum(source["revoked"] for source in self.data["corpus"]), 2)
        self.assertEqual(sum(bool(query["expected_sources"]) for query in self.data["queries"]), 42)
        # 按解码后原文判定，不依赖category，也不把JSON转义当成字面反斜线。
        self.assertEqual([ord(char) for char in self.data["queries"][60]["query"]], [9, 32, 10, 12288])
        self.assertFalse(self.data["queries"][60]["query"].strip())
        self.assertEqual(self.data["queries"][0]["lexical_intent"], "question")
        self.assertNotEqual(self.data["queries"][0]["intent"], "question")

    def test_changed_holdout_identity_fails_closed(self):
        with patch("common.digest", return_value="0" * 64), self.assertRaisesRegex(ValueError, "identity"):
            datasets("attachment-holdout")

    def test_adapted_hash_is_canonical_and_sensitive_to_original_metadata(self):
        reordered = {key: self.data[key] for key in reversed(self.data)}
        self.assertEqual(canonical_digest(self.data), canonical_digest(reordered))
        changed = deepcopy(self.data)
        changed["corpus"][0]["title"] += " changed"
        self.assertNotEqual(canonical_digest(self.data), canonical_digest(changed))

    def test_allowlist_includes_only_admitted_original_queries_and_non_revoked_source_text(self):
        prefix = read_json(HERE / "model-lock.json")["query_instruction"]
        actual = allowed_inputs([self.data], prefix)
        for source in self.data["corpus"]:
            self.assertEqual(source["text"] in actual, not source["revoked"])
        for query in self.data["queries"]:
            admitted = bool(query["query"].strip()) and len(query["visible_scopes"]) <= 4
            self.assertEqual(prefix + "\n" + query["query"] in actual, admitted)
        self.assertNotIn("", actual)
        self.assertNotIn(prefix + "\n" + prefix + "\n" + self.data["queries"][0]["query"], actual)

    def test_legacy_allowlist_does_not_acquire_holdout_admission_rules(self):
        prefix = "prefix"
        legacy = {"corpus": [{"text": "body"}],
                  "queries": [{"query": ""}, {"query": " \t\n"},
                              {"query": "legacy", "visible_scopes": [str(number) for number in range(5)]}]}
        self.assertEqual(allowed_inputs([legacy], prefix), {"body", "prefix\n", "prefix\n \t\n", "prefix\nlegacy"})

    def test_adapter_rejects_all_five_unadmitted_holdout_queries_before_model_use(self):
        encoder = Encoder.__new__(Encoder)
        encoder.lock = read_json(HERE / "model-lock.json")
        prefix = encoder.lock["query_instruction"]
        encoder.allowed_inputs = frozenset(allowed_inputs([self.data], prefix))
        rejected = [query for query in self.data["queries"]
                    if not query["query"].strip() or len(query["visible_scopes"]) > 4]
        self.assertEqual(len(rejected), 5)
        for query in rejected:
            payload = {"model": encoder.lock["model"], "input": [prefix + "\n" + query["query"]],
                       "dimensions": 512, "encoding_format": "float"}
            with self.subTest(id=query["id"]), self.assertRaisesRegex(ValueError, "synthetic_allowlist_required"):
                encoder.encode(payload)

    def test_same_space_paired_budget_fits_original_caps_without_category_shortcuts(self):
        result = holdout_request_budget(self.data)
        self.assertEqual(result["non_revoked_document_inputs"], 22)
        self.assertEqual(result["unique_scoped_queries"], 59)
        self.assertEqual(result["paired_embedding_request_upper_bound"], 81)
        self.assertEqual(max(result["unique_queries_per_scope"].values()), 13)
        self.assertLessEqual(result["paired_embedding_request_upper_bound"], 128)
        self.assertLessEqual(result["paired_embedded_input_upper_bound"], 256)
        changed = deepcopy(self.data)
        for query in changed["queries"]:
            query["category"] = "empty_input"
        self.assertEqual(result, holdout_request_budget(changed))
        for number in range(33):
            changed["queries"].append({"query": f"extra {number}", "visible_scopes": ["workshop"]})
        with self.assertRaisesRegex(ValueError, "budget"):
            holdout_request_budget(changed)

    def test_metrics_keep_all_denominators_and_separate_route_rejections(self):
        rows = self.rows()
        verify_rows(rows, self.data, strict_route=True)
        result = summarize(rows)
        self.assertEqual(result["all"]["queries"], 64)
        self.assertEqual(result["all"]["positive_queries"], 42)
        self.assertEqual(result["all"]["negative_queries"], 22)
        self.assertEqual(result["by_execution"]["executed"]["queries"], 59)
        self.assertEqual(result["by_execution"]["not_executed"]["queries"], 5)
        self.assertEqual(result["by_query_status"]["scope_limit"]["queries"], 3)
        self.assertEqual(result["by_query_status"]["input_rejected"]["queries"], 2)
        self.assertEqual(result["by_expected_source_kind"]["event"]["queries"], 20)
        self.assertEqual(result["by_expected_source_kind"]["attachment"]["queries"], 20)
        self.assertEqual(result["by_expected_source_kind"]["attachment+event"]["queries"], 2)
        self.assertEqual(result["by_expected_source_kind"]["no_answer"]["queries"], 22)
        self.assertEqual(result["all"]["positive_source_hit_rate_at_1"], 0)
        self.assertIsNone(result["all"]["annotated_source_precision_at_1"])

    def test_source_hit_precision_and_no_answer_nonempty_are_distinct(self):
        rows = [{"id": "positive", "category": "test", "expected_sources": ["right"],
                 "returned_sources": ["wrong", "right"], "returned_kinds": ["attachment_parent", "attachment"]},
                {"id": "negative", "category": "test", "expected_sources": [],
                 "returned_sources": ["wrong"], "returned_kinds": ["event"]}]
        result = summarize(rows)["all"]
        self.assertEqual(result["positive_source_hit_at_1"], 0)
        self.assertEqual(result["positive_source_hit_rate_at_3"], 1)
        self.assertEqual(result["non_expected_candidates_at_3"], 2)
        self.assertEqual(result["annotated_source_precision_at_3"], 1 / 3)
        self.assertEqual(result["negative_queries_with_any_candidate"], 1)
        self.assertEqual(result["negative_nonempty_rate"], 1)
        self.assertEqual(result["returned_candidates_by_kind"]["attachment_parent"], 1)
        rows[0]["returned_sources"] = ["right", "right"]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            summarize(rows)

    def test_returned_attachment_parent_is_not_relabelled_as_expected_attachment(self):
        rows = self.rows()
        row = next(row for row in rows if row["expected_sources"] == ["ATT-07"])
        row["returned_sources"] = ["parent:ATT-07", "ATT-07"]
        verify_rows(rows, self.data, strict_route=True)
        self.assertEqual(row["returned_kinds"], ["attachment_parent", "attachment"])
        metric = summarize([row])["all"]
        self.assertEqual(metric["positive_source_hit_at_1"], 0)
        self.assertEqual(metric["positive_source_hit_at_3"], 1)
        self.assertEqual(metric["non_expected_candidates_at_3"], 1)

    def test_result_scope_revocation_annotation_kind_and_execution_checks(self):
        for returned in (["unknown"], ["ATT-07"], ["EVT-23"], ["parent:ATT-24"]):
            rows = self.rows()
            rows[0]["returned_sources"] = returned
            with self.assertRaises(ValueError):
                verify_rows(rows, self.data, strict_route=True)
        for field, value in (("expected_sources", []), ("visible_scopes", ["release"]),
                             ("source_kind_expected", ["attachment"]), ("route_executed", "yes")):
            rows = self.rows()
            rows[0][field] = value
            with self.assertRaises(ValueError):
                verify_rows(rows, self.data, strict_route=True)
        rows = self.rows()
        rows[0]["returned_sources"] = rows[0]["expected_sources"]
        rows[0]["route_executed"] = False
        with self.assertRaisesRegex(ValueError, "admission"):
            verify_rows(rows, self.data, strict_route=True)

    def test_strict_route_status_matches_actual_admission_exactly(self):
        for ident, statuses in (("Q01", ("lexical_only", "input_rejected", "scope_limit", "")),
                               ("Q59", ("semantic", "scope_limit")),
                               ("Q61", ("semantic", "scope_limit")),
                               ("Q60", ("semantic", "input_rejected"))):
            for status in statuses:
                rows = self.rows()
                next(row for row in rows if row["id"] == ident)["query_status"] = status
                with self.subTest(id=ident, status=status), self.assertRaisesRegex(ValueError, "admission"):
                    verify_rows(rows, self.data, strict_route=True)
        rows = self.rows()
        for row in rows:
            if row["route_executed"]:
                row["query_status"] = "lexical_only"
        verify_rows(rows, self.data, strict_route=False)

    def test_pairing_uses_ids_not_array_order_and_preserves_qrels(self):
        before = self.rows()
        after = deepcopy(before)
        after[0]["returned_sources"] = after[0]["expected_sources"]
        negative = next(row for row in after if row["route_executed"] and not row["expected_sources"])
        negative["returned_sources"] = ["nonexpected"]
        result = paired_deltas(before, list(reversed(after)))
        self.assertEqual(result["new_source_hits_at_1"], [before[0]["id"]])
        self.assertEqual(result["introduced_no_answer_candidates"], [negative["id"]])
        self.assertEqual(result["lost_source_hits_at_5"], [])
        after[0]["expected_sources"] = []
        with self.assertRaisesRegex(ValueError, "annotation"):
            paired_deltas(before, after)
        with self.assertRaisesRegex(ValueError, "set"):
            paired_deltas(before, before[:-1])


if __name__ == "__main__":
    unittest.main()
