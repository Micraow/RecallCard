"""Create-only local controller journal; replay never reissues a tool or model call.

Incomplete intent files are an explicit interrupted state. They cannot be
silently discarded, repaired, or automatically retried by this module.
"""
from __future__ import annotations
import base64
from dataclasses import asdict
import hashlib
from pathlib import Path
from bounded_span_adapter import parse_result
from judgment_contract import ContractError, Scope, _require, encode
from native_review_controller import Controller, Reply


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def write_once(path, raw):
    with Path(path).open('xb') as stream:
        stream.write(raw)
        stream.flush()
        # A journal record is recovery metadata, never an authorization token.
        import os
        os.fsync(stream.fileno())


class TransportJournal:
    def __init__(self, root, transport=None, replay=False):
        self.root, self.transport, self.replay, self.index = Path(root), transport, replay, 0
        if not replay:
            self.root.mkdir(exist_ok=False)
        self.records = sorted(self.root.glob('*.receipt.json'))
        if replay:
            _require(len(self.records) == len(list(self.root.glob('*.request.json'))), 'interrupted transport intent; no automatic retry')

    def __call__(self, name, args):
        self.index += 1
        stem = self.root/('%04d'%self.index)
        request = encode({'name': name, 'args': args})
        req, body, receipt = (Path(str(stem)+suffix) for suffix in ('.request.json','.result.bin','.receipt.json'))
        if self.replay:
            _require(req.read_bytes() == request, 'replay request changed')
            record = parse_result(receipt.read_bytes())
            raw = body.read_bytes()
            _require(record['request_sha256'] == sha(request) and record['result_sha256'] == sha(raw), 'journal hash mismatch')
            return Reply(raw, record['request_bytes'], record['wire_bytes'], record['elapsed_ms'], record['error'], record['costs_complete'])
        write_once(req, request)
        # A transport exception leaves an incomplete intent. An upstream adapter
        # with known partial bytes should instead return Reply(error=...).
        reply = self.transport(name, args)
        _require(type(reply) is Reply, 'transport must return Reply')
        write_once(body, reply.raw)
        record = asdict(reply); record.pop('raw')
        record.update(request_sha256=sha(request), result_sha256=sha(reply.raw))
        write_once(receipt, encode(record))
        return reply

    def assert_drained(self):
        if self.replay:
            _require(self.index == len(self.records), 'unreplayed transport records')


class FileSession:
    """Explicit file handoff: perform accepts native bytes, never generates them."""
    def __init__(self, root, scope, transport):
        self.root = Path(root)
        self.root.mkdir(exist_ok=False)
        self.controller = Controller(scope)
        self.transport = TransportJournal(self.root/'transport', transport)
        self.index = 0
        write_once(self.root/'scope.json', encode(scope.payload()))

    @staticmethod
    def _apply(controller, transport, kind, payload):
        if kind == 'setup':
            controller.setup(transport, payload['method'], payload.get('params'))
            return None
        if kind == 'protocol_check':
            return controller.check_unassessed_gate(transport)
        if kind == 'discovery':
            return controller.discover(payload, transport)
        if kind == 'native':
            return controller.accept_native(base64.b64decode(payload['raw_base64'], validate=True), transport, payload['elapsed_ms'])
        if kind == 'read':
            raw = controller.read_selected(transport) if payload is None else controller.read_request(payload, transport)
            return base64.b64encode(raw).decode()
        raise ContractError('unknown session event kind')

    def perform(self, kind, payload):
        self.index += 1
        stem = self.root/('%04d'%self.index)
        intent = encode({'kind': kind, 'payload': payload})
        write_once(Path(str(stem)+'.intent.json'), intent)
        try:
            output = self._apply(self.controller, self.transport, kind, payload)
        except Exception as error:
            write_once(Path(str(stem)+'.failure.json'), encode({'exception': type(error).__name__,
                'message': str(error), 'costs': self.controller.costs.summary(), 'automatic_retry': False}))
            raise
        result = encode({'output': output, 'costs': self.controller.costs.summary()})
        write_once(Path(str(stem)+'.result.json'), result)
        write_once(Path(str(stem)+'.complete.json'), encode({'intent_sha256': sha(intent), 'result_sha256': sha(result)}))
        return output

    def native(self, raw, elapsed_ms=0):
        return self.perform('native', {'raw_base64': base64.b64encode(raw).decode(), 'elapsed_ms': elapsed_ms})

    @classmethod
    def restore(cls, root, transport=None):
        root = Path(root)
        intents = sorted(root.glob('*.intent.json'))
        _require(len(intents) == len(list(root.glob('*.complete.json'))) and not list(root.glob('*.failure.json')),
                 'interrupted or failed native session; no automatic retry')
        scope = parse_result((root/'scope.json').read_bytes())
        obj = cls.__new__(cls)
        obj.root = root
        obj.controller = Controller(Scope.create(scope['query'], scope['scopes'], scope['event_filter'], scope['as_of']))
        replay = TransportJournal(root/'transport', replay=True)
        for number, path in enumerate(intents, 1):
            _require(path.name == '%04d.intent.json'%number, 'noncontiguous session journal')
            stem = root/('%04d'%number)
            receipt = parse_result(Path(str(stem)+'.complete.json').read_bytes())
            raw, result = path.read_bytes(), Path(str(stem)+'.result.json').read_bytes()
            _require(receipt['intent_sha256'] == sha(raw) and receipt['result_sha256'] == sha(result), 'session hash changed')
            event = parse_result(raw)
            output = cls._apply(obj.controller, replay, event['kind'], event['payload'])
            _require(encode({'output': output, 'costs': obj.controller.costs.summary()}) == result, 'controller replay result changed')
        replay.assert_drained()
        obj.index = len(intents)
        # Continue only after every old event has replayed without any I/O.
        replay.replay, replay.transport = False, transport
        obj.transport = replay
        return obj
