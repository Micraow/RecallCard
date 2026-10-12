"""Toy file handoff/recovery; exact bytes and incomplete states remain visible."""
import copy
import tempfile
import unittest
from pathlib import Path
from judgment_contract import ContractError, encode
from review_journal import FileSession
from test_native_review_controller import LocalToy, fixture, arguments, decision


class JournalTests(unittest.TestCase):
    def test_freeze_native_file_restore_without_tool_replay_then_continue(self):
        with tempfile.TemporaryDirectory() as directory:
            scope, page = fixture(count=1); toy = LocalToy(page)
            session = FileSession(Path(directory)/'session', scope, toy); toy.controller = session.controller
            p = session.perform('discovery', arguments(scope))
            p2 = session.native(encode(decision(p)), 17)
            self.assertEqual(len(toy.requests), 2)
            calls = []
            restored = FileSession.restore(session.root, lambda *args: calls.append(args))
            self.assertEqual(calls, [])
            self.assertEqual(restored.controller.packet_value, p2)
            self.assertEqual(restored.controller.costs.summary(), session.controller.costs.summary())
            toy.controller = restored.controller; restored.transport.transport = toy
            restored.native(encode(decision(p2, 'select')), 9)
            restored.perform('read', None)
            self.assertEqual(len(toy.requests), 3)
            again = FileSession.restore(session.root, lambda *args: calls.append(args))
            self.assertEqual(calls, [])
            self.assertEqual(again.controller.costs.summary()['native_elapsed_ms'], 26)

    def test_pending_intent_is_not_automatically_reissued(self):
        with tempfile.TemporaryDirectory() as directory:
            scope, page = fixture(count=1); toy = LocalToy(page)
            session = FileSession(Path(directory)/'session', scope, toy); toy.controller = session.controller
            session.perform('discovery', arguments(scope))
            (session.root/'0002.intent.json').write_bytes(encode({'kind':'native','payload':{}}))
            calls = []
            with self.assertRaisesRegex(ContractError, 'interrupted'): FileSession.restore(session.root, lambda *args: calls.append(args))
            self.assertEqual(calls, [])

    def test_changed_result_or_intent_hash_rejects_without_io(self):
        for name in ('0001.result.json','0001.intent.json','transport/0001.result.bin'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                scope, page = fixture(count=1); toy = LocalToy(page)
                session = FileSession(Path(directory)/'session', scope, toy); toy.controller = session.controller
                session.perform('discovery', arguments(scope))
                path = session.root/name; path.write_bytes(path.read_bytes()+b' ')
                calls = []
                with self.assertRaises(ContractError): FileSession.restore(session.root, lambda *args: calls.append(args))
                self.assertEqual(calls, [])

    def test_invalid_native_receipt_preserves_failed_cost_and_cannot_resume(self):
        with tempfile.TemporaryDirectory() as directory:
            scope, page = fixture(count=1); toy = LocalToy(page)
            session = FileSession(Path(directory)/'session', scope, toy); toy.controller = session.controller
            session.perform('discovery', arguments(scope))
            with self.assertRaises(ContractError): session.native(b'{}')
            self.assertTrue((session.root/'0002.failure.json').is_file())
            self.assertEqual(session.controller.costs.summary()['native_output_bytes'], 2)
            with self.assertRaises(ContractError): FileSession.restore(session.root)

    def test_existing_directory_cannot_overwrite_completed_session(self):
        with tempfile.TemporaryDirectory() as directory:
            scope, page = fixture(count=1); path = Path(directory)/'session'
            FileSession(path, scope, LocalToy(page))
            with self.assertRaises(FileExistsError): FileSession(path, scope, LocalToy(page))
