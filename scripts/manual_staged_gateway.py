#!/usr/bin/env python3
"""Synthetic-only loopback HTTPS operator gateway using the production Rust staged runtime and Python provider.

The certificate is ephemeral, explicitly trusted by the child client, and never
added to the system trust store. No real credentials or private corpus allowed.
"""
from __future__ import annotations
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import shlex
import threading

from manual_model_gateway import ManualGateway, ENDPOINT, SYNTHETIC_KEY, MAX_BYTES, read, write


def run(binary, queue_path, output_path, timeout=1800, *, runtime_root=None, state_dir=None, service_mode=False):
    # All inputs are synthetic; the Rust harness rejects existing vaults.
    if bool(runtime_root) != bool(state_dir):
        raise ValueError('Existing runtime and state directories must be selected together')
    output = Path(output_path).absolute()
    if any(path.exists() for path in (output, output.with_suffix('.stdout'), output.with_suffix('.stderr'), output.with_suffix('.receipt.json'))):
        raise FileExistsError('Use a new output path; prior evidence is immutable')
    queue = ManualGateway(queue_path)
    posts = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            if (self.path != '/v1/chat/completions'
                    or self.headers.get('Authorization') != 'Bearer ' + SYNTHETIC_KEY):
                self.send_error(403)
                return
            value = self.headers.get('Content-Length', '')
            if not value.isascii() or not value.isdigit() or not 0 < int(value) <= MAX_BYTES:
                self.send_error(413)
                return
            payload = self.rfile.read(int(value))
            if len(payload) != int(value):
                self.send_error(400)
                return
            # Queue is an operator interface behind the actual TLS server, not
            # an injected production client transport. Never persist headers.
            posts.append({'request_sha256': hashlib.sha256(payload).hexdigest(),
                          'request_bytes': len(payload), 'tls_version': self.connection.version()})
            try:
                response = queue(ENDPOINT, payload, SYNTHETIC_KEY, timeout, 2 * 1024 * 1024)
                self.send_response(response.status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(response.body)))
                self.end_headers()
                self.wfile.write(response.body)
            except Exception as error:
                posts[-1]['error_type'] = type(error).__name__
                self.send_error(503)

    with tempfile.TemporaryDirectory(prefix='recallcard-synthetic-tls-') as temporary:
        temporary = Path(temporary)
        cert, key = temporary / 'server.crt', temporary / 'server.key'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
                        '-keyout', str(key), '-out', str(cert), '-days', '1',
                        '-subj', '/CN=RecallCard synthetic loopback',
                        '-addext', 'subjectAltName=IP:127.0.0.1'],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        key.chmod(0o600)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        server.daemon_threads = True
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        endpoint = f'https://127.0.0.1:{server.server_port}/v1/chat/completions'
        launcher = temporary / 'python-with-synthetic-ca'
        launcher.write_text('#!/bin/sh\nexport SSL_CERT_FILE=' + shlex.quote(str(cert))
                            + '\nexec ' + shlex.quote(os.sys.executable) + ' "$@"\n')
        launcher.chmod(0o700)
        if service_mode:
            service_python = temporary / 'python3'
            service_python.write_bytes(launcher.read_bytes())
            service_python.chmod(0o700)
        root = Path(runtime_root).resolve() if runtime_root else output.parent / 'runtime'
        environment = dict(os.environ, RECALLCARD_DREAM_API_KEY=SYNTHETIC_KEY,
                           RECALLCARD_STATE_DIR=str(Path(state_dir).resolve() if state_dir else output.parent / 'state'))
        if service_mode:
            environment.pop('RECALLCARD_DREAM_API_KEY', None)
            environment['PATH'] = str(temporary) + os.pathsep + environment.get('PATH', '')
        command = [str(Path(binary).resolve()), 'continue' if runtime_root else 'start', str(root), endpoint, str(launcher),
                   str(Path(__file__).resolve().parents[1] / 'python'), str(output.parent / 'runtime-evidence')]
        try:
            process = subprocess.run(command, env=environment, capture_output=True, timeout=2 * timeout + 60)
            output.with_suffix('.stdout').write_bytes(process.stdout)
            output.with_suffix('.stderr').write_bytes(process.stderr)
            receipt = {'schema': 'recallcard.manual-staged-https/1', 'synthetic_only': True,
                       'production_core': ('DesktopSession model setup + independent service + PythonMemoryProvider' if service_mode else 'MemoryRuntime::tick + PythonMemoryProvider + recallcard_dream.runtime'),
                       'test_launcher': 'child-process-only ephemeral CA; system trust untouched', 'transport_injected': False,
                       'loopback_only': True, 'tls_verification_enabled': True,
                       'ephemeral_certificate_sha256': hashlib.sha256(cert.read_bytes()).hexdigest(),
                       'credential_persisted': False, 'exit_code': process.returncode,
                       'posts': posts, 'http_verified': process.returncode == 0 and len(posts) == 2,
                       'rust_review_apply_verified': False}
            write(output.with_suffix('.receipt.json'), receipt)
            if process.returncode or len(posts) != 2:
                raise RuntimeError('Staged runtime did not complete two provider requests; see preserved stderr')
            print(json.dumps({'ok': True, 'receipt': str(output.with_suffix('.receipt.json')), 'http_verified': True}), flush=True)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--queue', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=int, default=900)
    parser.add_argument('--synthetic-only', action='store_true', required=True)
    parser.add_argument('--service-mode', action='store_true')
    parser.add_argument('--runtime-root')
    parser.add_argument('--state-dir')
    args = parser.parse_args()
    run(args.binary, args.queue, args.output, args.timeout, runtime_root=args.runtime_root, state_dir=args.state_dir, service_mode=args.service_mode)
