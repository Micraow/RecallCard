"""Safe diagnostics never substitute for the real GUI journey assertions."""
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import Mock
from connected_context_native import ConnectedJourney, ORIGINAL, UPDATE


class CanonicalEvidenceTest(unittest.TestCase):
    def test_native_attachment_fixture_has_one_opaque_payload_and_one_explicit_gap(self):
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
                self.assertEqual(len(json.loads(archive.read("export_manifest.json"))["logical_files"]), 2)

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
