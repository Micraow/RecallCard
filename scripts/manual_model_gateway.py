#!/usr/bin/env python3
"""Human-driven, explicitly scoped local Chat Completions queue; never opens a network socket.

Uses the production DreamClient request builder, parser and result validator.
The operator reads requests and supplies replies; no successful reply is generated.
This transport is not evidence of HTTP, Rust review/apply, MCP or desktop acceptance.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python'))
from recallcard_dream.client import DreamClient, DreamError, NetworkApproval, TransportResponse, canonical, load_json
from recallcard_dream.__main__ import atomic_result_file

ENDPOINT = 'https://manual-gateway.invalid/v1/chat/completions'
SYNTHETIC_KEY = 'synthetic-manual-gateway-only'
MAX_BYTES = 4 * 1024 * 1024


def read(path, limit=MAX_BYTES):
    path = Path(path).absolute()
    if any(part.is_symlink() for part in (path, *path.parents)):
        raise ValueError('Symlink paths are not allowed')
    with path.open('rb') as stream:
        return load_json(stream.read(limit + 1), limit)


def write(path, value):
    atomic_result_file(path, value, max_bytes=MAX_BYTES)


class ManualGateway:
    def __init__(self, root, *, create=True, private_local_only=False):
        self.root = Path(root).absolute()
        if any(part.is_symlink() for part in (self.root, *self.root.parents)):
            raise ValueError('Symlink queue paths are not allowed')
        if not create and not (self.root / 'control.json').is_file():
            raise PermissionError('Existing queue control is required')
        self.root.mkdir(parents=True, exist_ok=True)
        for part in ('requests', 'responses', 'receipts'):
            (self.root / part).mkdir(exist_ok=True)
        control = self.root / 'control.json'
        if not control.exists():
            write(control, {'enabled': True, 'revision': 1, 'synthetic_only': not private_local_only, 'private_local_only': private_local_only})
        self.private_local_only = read(control).get('private_local_only', False)
        if create and read(control).get('private_local_only', False) != private_local_only:
            raise PermissionError('Queue data mode cannot change')

    def control(self):
        value = read(self.root / 'control.json')
        modes = (value.get('synthetic_only') is True, value.get('private_local_only') is True)
        if value.get('enabled') is not True or sum(modes) != 1 or modes[1] != self.private_local_only:
            raise PermissionError('Manual local test permission revoked')
        if type(value.get('revision')) is not int or value['revision'] < 1:
            raise ValueError('Invalid permission revision')
        return value

    def __call__(self, endpoint, payload, key, timeout, response_limit):
        if endpoint != ENDPOINT or key != SYNTHETIC_KEY:
            raise PermissionError('Only the fixed local queue destination is allowed')
        revision = self.control()['revision']
        body = load_json(payload, MAX_BYTES)
        ident = uuid.uuid4().hex
        digest = hashlib.sha256(payload).hexdigest()
        request = {'schema': 'recallcard.manual-model-request/1', 'id': ident,
                   'request_sha256': digest, 'permission_revision': revision,
                   'transport': 'file-queue-no-network', 'data_mode': ('private_local_only' if self.control().get('private_local_only') else 'synthetic_only'), 'body': body}
        write(self.root / 'requests' / (ident + '.json'), request)
        print(json.dumps({'waiting_for_operator': ident, 'request': str(self.root / 'requests' / (ident + '.json'))}), flush=True)
        started = time.monotonic()
        response = self.root / 'responses' / (ident + '.json')
        try:
            while True:
                if self.control()['revision'] != revision:
                    raise PermissionError('Permission changed while waiting')
                if time.monotonic() - started >= timeout:
                    raise TimeoutError('Operator response timed out')
                if response.exists():
                    value = read(response, response_limit + 4096)
                    if (value.get('id') != ident or value.get('request_sha256') != digest
                            or value.get('permission_revision') != revision):
                        raise ValueError('Response does not match the pending request')
                    if type(value.get('status')) is not int or not 100 <= value['status'] <= 599:
                        raise ValueError('Invalid response status')
                    encoded = canonical(value.get('body'))
                    if len(encoded) > response_limit:
                        raise ValueError('Response exceeds the provider byte limit')
                    if self.control()['revision'] != revision:
                        raise PermissionError('Permission changed before response delivery')
                    write(self.root / 'receipts' / (ident + '.json'), {'id': ident, 'request_sha256': digest,
                          'status': value['status'], 'response_sha256': hashlib.sha256(encoded).hexdigest(),
                          'transport': 'file-queue-no-network', 'delivered_to_client': True})
                    return TransportResponse(value['status'], encoded)
                time.sleep(0.05)
        except Exception as exc:
            write(self.root / 'receipts' / (ident + '.json'), {'id': ident,
                  'request_sha256': digest, 'delivered_to_client': False, 'error_type': type(exc).__name__})
            raise


def reply(root, ident, body_file, status):
    if len(ident) != 32 or any(c not in '0123456789abcdef' for c in ident):
        raise ValueError('Invalid queue request id')
    queue = ManualGateway(root, create=False)
    request = read(queue.root / 'requests' / (ident + '.json'))
    if (queue.root / 'receipts' / (ident + '.json')).exists():
        raise ValueError('Request has already finished; create a new request')
    if queue.control()['revision'] != request['permission_revision']:
        raise PermissionError('Pending request permission is stale')
    write(queue.root / 'responses' / (ident + '.json'), {k: request[k] for k in ('id', 'request_sha256', 'permission_revision')} | {'status': status, 'body': read(body_file)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    dream = sub.add_parser('dream')
    dream.add_argument('--queue', required=True)
    dream.add_argument('--job', required=True)
    dream.add_argument('--output', required=True)
    dream.add_argument('--timeout', type=float, default=900)
    mode = dream.add_mutually_exclusive_group(required=True)
    mode.add_argument('--synthetic-only', action='store_true')
    mode.add_argument('--private-local-only', action='store_true', help='Explicitly authorized, sanitized private material; file queue only, never upload queue files')
    response = sub.add_parser('reply')
    response.add_argument('--queue', required=True)
    response.add_argument('--id', required=True)
    response.add_argument('--body', required=True, help='Actual Chat Completions JSON envelope supplied by the operator')
    response.add_argument('--status', type=int, default=200)
    args = parser.parse_args()
    if args.command == 'reply':
        reply(args.queue, args.id, args.body, args.status)
        return 0
    job = read(args.job)
    os.environ['RECALLCARD_DREAM_API_KEY'] = SYNTHETIC_KEY
    queue = ManualGateway(args.queue, private_local_only=args.private_local_only)
    client = DreamClient(ENDPOINT, 'manual-operator', NetworkApproval(True, ENDPOINT, (job['allowed_scope'],), True), transport=queue, timeout=args.timeout)
    try:
        result = client.execute(job)
        result['verification_boundary'] = {'real_dream_client': True, 'model_response': 'manual_operator',
            'transport': 'file-queue-no-network', 'http_verified': False, 'rust_review_apply_verified': False,
            'mcp_agent_verified': False, 'requires_review': True}
        write(args.output, result)
        print(json.dumps({'ok': True, 'output': args.output, 'requires_review': True}), flush=True)
        return 0
    except DreamError as exc:
        write(args.output, {'ok': False, 'error': {'code': exc.code, 'message': str(exc)}, 'transport': 'file-queue-no-network'})
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
