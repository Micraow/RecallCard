"""Safe diagnostics never substitute for the real GUI journey assertions."""
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import Mock, patch
import connected_context_native as journey_module
from native_smoke import DriverError, wait_for
from connected_context_native import ConnectedJourney, ORIGINAL, UPDATE, ATTACHMENT_QUERY, ATTACHMENT_TEXT


class CanonicalEvidenceTest(unittest.TestCase):
    def test_attachment_row_wait_uses_one_complete_exact_message_snapshot(self):
        journey = object.__new__(ConnectedJourney)
        journey.driver = Mock()
        selector = ".source-message[data-reference='event:synthetic-present']"
        required = ("原件已保留", "合成图表.pdf", "合成归档别名.pdf", "application/pdf")
        complete = "\n".join(required)
        journey.driver.text.side_effect = [DriverError('{"value":{"error":"no such element"}}'),
                                           required[0], complete]
        with patch("native_smoke.time.sleep"):
            result = journey.wait_for_message_text(selector, required, ("合成未附原件.pdf",), "合成刷新恢复")
        self.assertEqual(result, complete)
        self.assertEqual([call.args for call in journey.driver.text.call_args_list], [(selector,)] * 3)
        journey.driver.click.assert_not_called()
        journey.driver.command.assert_not_called()

    def test_attachment_row_wait_rejects_wrong_association_immediately(self):
        journey = object.__new__(ConnectedJourney)
        journey.driver = Mock()
        journey.driver.text.return_value = "原件已保留 合成图表.pdf 合成未附原件.pdf"
        with self.assertRaisesRegex(AssertionError, "混入"):
            journey.wait_for_message_text(".exact-synthetic-message", ("原件已保留", "合成图表.pdf"),
                                          ("合成未附原件.pdf",), "合成错误归属")
        self.assertEqual(journey.driver.text.call_count, 1)

    def test_attachment_row_wait_does_not_accept_persistent_missing_or_incomplete_data(self):
        required = ("原件已保留", "合成图表.pdf", "合成归档别名.pdf", "application/pdf")
        samples = [None, DriverError('{"value":{"error":"no such element"}}')]
        samples.extend("\n".join(value for value in required if value != omitted) for omitted in required)
        for sample in samples:
            journey = object.__new__(ConnectedJourney)
            journey.driver = Mock()
            if isinstance(sample, DriverError):
                journey.driver.text.side_effect = sample
            else:
                journey.driver.text.return_value = sample
            def bounded(check, description):
                return wait_for(check, description, timeout=0.001)
            with self.subTest(sample=sample), patch.object(journey_module, "wait_for", side_effect=bounded), patch("native_smoke.time.sleep"):
                with self.assertRaisesRegex(AssertionError, "等待超时"):
                    journey.wait_for_message_text(".exact-synthetic-message", required,
                                                  ("合成未附原件.pdf",), "合成持久缺失")
            journey.driver.click.assert_not_called()

    def test_native_attachment_fixture_has_opaque_text_payloads_and_one_explicit_gap(self):
        with tempfile.TemporaryDirectory() as directory:
            journey = object.__new__(ConnectedJourney)
            journey.temporary = Path(directory)
            with zipfile.ZipFile(journey.create_archive()) as archive:
                record = json.loads(archive.read("conversations.json"))[0]
                self.assertEqual(len(record["mapping"]), 4)
                self.assertTrue(all("children" not in node for node in record["mapping"].values()))
                upload = record["mapping"]["u1"]["message"]
                self.assertEqual(upload["content"]["parts"], [""])
                self.assertEqual(upload["metadata"]["attachments"][0]["id"], "file_SYNTHETIC_PRESENT")
                self.assertEqual(record["mapping"]["u2"]["message"]["content"]["parts"], [ORIGINAL])
                self.assertIn("file_SYNTHETIC_PRESENT.dat", archive.namelist())
                self.assertNotIn("file_SYNTHETIC_MISSING.dat", archive.namelist())
                payload = archive.read("file_SYNTHETIC_PRESENT.dat")
                self.assertTrue(payload.startswith(b"%PDF-"))
                self.assertIn(b"SYNTHETIC", payload)
                library = json.loads(archive.read("library_files.json"))
                self.assertEqual(library[0]["file_size_bytes"], len(payload))
                self.assertEqual(len(json.loads(archive.read("export_manifest.json"))["logical_files"]), 3)
                text_file = record["mapping"]["a1"]["message"]["metadata"]["attachments"][0]
                self.assertEqual(text_file["id"], "file_SYNTHETIC_TEXT")
                self.assertEqual(text_file["mime_type"], "text/plain")
                self.assertEqual(archive.read(text_file["id"] + ".dat"), ATTACHMENT_TEXT.encode("utf-8"))
                self.assertEqual(text_file["size"], len(ATTACHMENT_TEXT.encode("utf-8")))
                self.assertNotIn(ATTACHMENT_QUERY, json.dumps(record, ensure_ascii=False))
                self.assertNotIn(ATTACHMENT_QUERY, json.dumps(library, ensure_ascii=False))

    def test_portable_visible_text_requires_the_entire_text_and_no_clipping(self):
        journey = object.__new__(ConnectedJourney)
        journey.driver = Mock()
        expected = "合成完整回答，包括结尾"
        for sample in [None, {"text": expected[:-1], "visible": True, "unclipped": True},
                       {"text": expected, "visible": False, "unclipped": True},
                       {"text": expected, "visible": True, "unclipped": False}]:
            journey.driver.observe.return_value = sample
            self.assertFalse(journey.exact_visible_text(".synthetic", expected))
        journey.driver.observe.return_value = {"text": expected, "visible": True, "unclipped": True}
        self.assertTrue(journey.exact_visible_text(".synthetic", expected))

    def test_missing_vault_is_not_import_success(self):
        with tempfile.TemporaryDirectory() as directory:
            journey = object.__new__(ConnectedJourney)
            journey.vault = Path(directory) / 'absent'
            result = journey.canonical_summary()
            self.assertFalse(result['vault_exists'])
            self.assertFalse(result['original_present'])
            self.assertEqual(result['events'], 0)

    def test_reports_counts_without_serializing_messages_or_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            journey = object.__new__(ConnectedJourney)
            journey.vault = Path(directory)
            journey.events = Mock(return_value=[{'content': ORIGINAL}, {'content': UPDATE}])
            journey.memories = Mock(return_value=[])
            result = journey.canonical_summary()
            self.assertEqual(result['events'], 2)
            self.assertTrue(result['original_present'])
            self.assertTrue(result['update_present'])
            self.assertNotIn(ORIGINAL, json.dumps(result, ensure_ascii=False))
            journey.events.side_effect = ValueError('untrusted message content')
            self.assertEqual(journey.canonical_summary(), {'readable': False, 'error_type': 'ValueError'})


if __name__ == '__main__':
    unittest.main()
