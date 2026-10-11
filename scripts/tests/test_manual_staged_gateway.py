"""Operator harness preflight must not overwrite evidence or change test vault state."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import manual_staged_gateway as gateway


class StagedGatewayPreflightTests(unittest.TestCase):
    def test_existing_receipt_is_rejected_before_network_or_process(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'transport.json'
            output.with_suffix('.receipt.json').write_text('{}')
            with patch.object(gateway.subprocess, 'run', side_effect=AssertionError('must not spawn')):
                with self.assertRaises(FileExistsError):
                    gateway.run('/synthetic/binary', root / 'queue', output)
            self.assertFalse((root / 'queue').exists())

    def test_resume_requires_both_existing_state_and_vault(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for args in ({'runtime_root':root/'runtime'}, {'state_dir':root/'state'}):
                with self.subTest(args=args), self.assertRaises(ValueError):
                    gateway.run('/synthetic/binary', root/'queue', root/'result.json', **args)
            self.assertFalse((root/'queue').exists())


if __name__ == '__main__':
    unittest.main()
