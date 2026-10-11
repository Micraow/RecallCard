"""Safe diagnostics never substitute for the real GUI journey assertions."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
from connected_context_native import ConnectedJourney, ORIGINAL, UPDATE


class CanonicalEvidenceTest(unittest.TestCase):
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
