"""Synthetic local queue contracts. No HTTP, MCP substitute or real account access."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from manual_model_gateway import ManualGateway, ENDPOINT, SYNTHETIC_KEY, canonical, read, write, reply
from manual_mcp_agent import load_agent_entry


class ManualGatewayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name) / 'queue'
        self.gateway = ManualGateway(self.root)
        self.pool = ThreadPoolExecutor(max_workers=1)

    def tearDown(self):
        self.pool.shutdown(wait=True)
        self.temp.cleanup()

    def pending(self, timeout=2):
        future = self.pool.submit(self.gateway, ENDPOINT,
                                  canonical({'messages': [{'role': 'user', 'content': 'synthetic input'}]}),
                                  SYNTHETIC_KEY, timeout, 4096)
        deadline = time.monotonic() + 1
        while not list((self.root / 'requests').glob('*.json')):
            if time.monotonic() > deadline:
                self.fail('No real request appeared in the queue')
            time.sleep(0.005)
        path = next((self.root / 'requests').glob('*.json'))
        return future, read(path)

    def respond(self, request, body=None, **changes):
        value = {k: request[k] for k in ('id', 'request_sha256', 'permission_revision')}
        value.update(status=200, body={'choices': []} if body is None else body)
        value.update(changes)
        write(self.root / 'responses' / (request['id'] + '.json'), value)

    def test_request_body_is_visible_but_key_is_never_recorded(self):
        future, request = self.pending()
        self.assertEqual(request['body']['messages'][0]['content'], 'synthetic input')
        self.assertNotIn(SYNTHETIC_KEY, json.dumps(request))
        self.respond(request, {'choices': []})
        response = future.result(timeout=2)
        self.assertEqual(response.status, 200)
        self.assertEqual(json.loads(response.body), {'choices': []})
        self.assertTrue(read(self.root / 'receipts' / (request['id'] + '.json'))['delivered_to_client'])

    def test_no_response_times_out_without_inventing_success(self):
        future, request = self.pending(timeout=0.1)
        with self.assertRaises(TimeoutError):
            future.result(timeout=2)
        self.assertFalse(read(self.root / 'receipts' / (request['id'] + '.json'))['delivered_to_client'])

    def test_revocation_during_wait_rejects_late_response(self):
        future, request = self.pending()
        temp = self.root / 'new-control.json'
        write(temp, {'enabled': False, 'revision': 2, 'synthetic_only': True})
        temp.replace(self.root / 'control.json')
        self.respond(request)
        with self.assertRaises(PermissionError):
            future.result(timeout=2)

    def test_response_for_other_request_is_not_accepted(self):
        future, request = self.pending()
        self.respond(request, request_sha256='0' * 64)
        with self.assertRaises(ValueError):
            future.result(timeout=2)

    def test_response_limit_is_enforced_without_truncation(self):
        future, request = self.pending()
        self.respond(request, {'text': 'x' * 5000})
        with self.assertRaises(ValueError):
            future.result(timeout=2)

    def test_reply_cannot_recreate_deleted_permission(self):
        self.gateway.control()
        (self.root / 'control.json').unlink()
        with self.assertRaises(PermissionError):
            reply(self.root, '1' * 32, 'missing.json', 200)
        self.assertFalse((self.root / 'control.json').exists())

    def test_completed_request_cannot_receive_another_reply(self):
        future, request = self.pending()
        self.respond(request)
        future.result(timeout=2)
        with self.assertRaisesRegex(ValueError, 'finished'):
            reply(self.root, request['id'], 'missing.json', 200)

    def test_non_synthetic_destination_or_key_is_rejected_before_recording(self):
        for endpoint, key in [('https://example.test/chat', SYNTHETIC_KEY), (ENDPOINT, 'not-a-test-key')]:
            with self.assertRaises(PermissionError):
                self.gateway(endpoint, b'{}', key, 1, 4096)
        self.assertEqual(list((self.root / 'requests').iterdir()), [])


class ManagedEntryContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = {'binary': '/synthetic/recallcard', 'vault': str(self.root / 'vault')}
        self.path = self.root / 'vault/generated/bootstrap/conn_synthetic/context.json'
        self.path.parent.mkdir(parents=True)
        self.entry = {'schema': 'recallcard.agent-entry/1', 'reference_data': True,
                      'connection_id': 'conn_synthetic', 'scopes': ['project:synthetic'],
                      'file_path': str(self.path), 'profile': [], 'directory': []}
        write(self.path, self.entry)

    def tearDown(self):
        self.temp.cleanup()

    def result(self, entry=None, returncode=0):
        return SimpleNamespace(returncode=returncode, stdout=canonical({'ok': True, 'result': entry or self.entry}), stderr=b'')

    def test_exact_refreshed_file_is_loaded(self):
        with patch('manual_mcp_agent.subprocess.run', return_value=self.result()) as runner:
            result = load_agent_entry(self.config, 'conn_synthetic', 'project:synthetic', {}, self.root)
        self.assertEqual(result, self.entry)
        self.assertIn('connection-context', runner.call_args.args[0])

    def test_denied_entry_never_falls_back_to_direct_file_read(self):
        with patch('manual_mcp_agent.subprocess.run', return_value=self.result(returncode=1)), patch('manual_mcp_agent.read') as reader:
            with self.assertRaises(RuntimeError):
                load_agent_entry(self.config, 'conn_synthetic', 'project:synthetic', {}, self.root)
        reader.assert_not_called()

    def test_wrong_path_or_scope_is_rejected(self):
        for change in [{'file_path': str(self.root / 'elsewhere.json')}, {'scopes': ['private']}]:
            with patch('manual_mcp_agent.subprocess.run', return_value=self.result(self.entry | change)):
                with self.assertRaises(ValueError):
                    load_agent_entry(self.config, 'conn_synthetic', 'project:synthetic', {}, self.root)

    def test_changed_file_is_rejected_after_refresh(self):
        self.path.write_bytes(canonical(self.entry | {'profile': ['newer']}))
        with patch('manual_mcp_agent.subprocess.run', return_value=self.result()):
            with self.assertRaisesRegex(ValueError, 'changed'):
                load_agent_entry(self.config, 'conn_synthetic', 'project:synthetic', {}, self.root)


if __name__ == '__main__':
    unittest.main()
