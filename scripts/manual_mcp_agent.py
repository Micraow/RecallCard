#!/usr/bin/env python3
"""Manual model gateway driving a real RecallCard MCP subprocess, explicitly scoped local vault.

No tool-result substitutes. The model can call only tools advertised by RecallCard,
and only the four read-only capabilities. Vault writes remain separate reviewed CLI operations.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time

from manual_model_gateway import ManualGateway, ENDPOINT, SYNTHETIC_KEY, read, write, canonical, load_json

ALLOWED_TOOLS = {'bootstrap', 'search', 'read', 'sources'}


def load_agent_entry(config, connection_id, scope, environment, output):
    """Perform the managed Agent entry instruction, then read only its verified file."""
    command = [config['binary'], '--vault', config['vault'], '--json', 'connection-context',
               connection_id, '--scope', scope, '--budget-bytes', '4096']
    result = subprocess.run(command, env=environment, capture_output=True, timeout=30)
    (output / 'entry.stdout').write_bytes(result.stdout)
    (output / 'entry.stderr').write_bytes(result.stderr)
    if result.returncode != 0:
        raise RuntimeError('Authorized Agent entry failed; no direct vault fallback')
    response = load_json(result.stdout)
    entry = response.get('result', {})
    if (response.get('ok') is not True or entry.get('schema') != 'recallcard.agent-entry/1'
            or entry.get('reference_data') is not True or entry.get('connection_id') != connection_id
            or entry.get('scopes') != [scope]):
        raise ValueError('Agent entry identity or scope mismatch')
    expected = Path(config['vault']).resolve() / 'generated' / 'bootstrap' / connection_id / 'context.json'
    if Path(entry.get('file_path', '')).absolute() != expected:
        raise ValueError('Agent entry returned an unexpected file path')
    file_entry = read(expected)
    if canonical(file_entry) != canonical(entry):
        raise ValueError('Agent entry changed before the generated file was read')
    write(output / 'authorized-entry.json', file_entry)
    return file_entry


def model_tool_content(result):
    """Forward actual MCP text blocks without double-serializing the transport envelope."""
    blocks = result.get('content', [])
    if blocks and all(block.get('type') == 'text' and isinstance(block.get('text'), str) for block in blocks):
        text = '\n'.join(block['text'] for block in blocks)
        if result.get('isError'):
            return json.dumps({'isError': True, 'text': text}, ensure_ascii=False)
        return text
    return json.dumps(result, ensure_ascii=False)


class MCP:
    def __init__(self, command, environment, output):
        self.stderr = (output / 'mcp.stderr').open('wb')
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, env=environment)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b''
        self.sequence = 0
        self.trace = (output / 'mcp-transcript.jsonl').open('a', encoding='utf-8')

    def send(self, value):
        data = canonical(value) + b'\n'
        self.process.stdin.write(data)
        self.process.stdin.flush()
        self.trace.write(json.dumps({'direction': 'to_mcp', 'value': value}, ensure_ascii=False) + '\n')
        self.trace.flush()

    def receive(self, timeout=30):
        end = time.monotonic() + timeout
        while b'\n' not in self.buffer:
            remaining = end - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise TimeoutError('MCP response timed out')
            block = os.read(self.process.stdout.fileno(), 65536)
            if not block:
                raise RuntimeError('MCP closed without a response')
            self.buffer += block
            if len(self.buffer) > 4 * 1024 * 1024:
                raise ValueError('MCP response exceeds test budget')
        line, self.buffer = self.buffer.split(b'\n', 1)
        value = load_json(line, 4 * 1024 * 1024)
        self.trace.write(json.dumps({'direction': 'from_mcp', 'value': value}, ensure_ascii=False) + '\n')
        self.trace.flush()
        return value

    def call(self, method, params):
        self.sequence += 1
        self.send({'jsonrpc': '2.0', 'id': self.sequence, 'method': method, 'params': params})
        response = self.receive()
        if response.get('id') != self.sequence or response.get('jsonrpc') != '2.0':
            raise ValueError('Unexpected MCP response identity')
        if 'error' in response:
            raise RuntimeError(json.dumps(response['error'], ensure_ascii=False))
        return response['result']

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=3)
        self.selector.close()
        self.stderr.close()
        self.trace.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', required=True, help='Fixed binary/vault/environment JSON')
    parser.add_argument('--connection-id', required=True)
    parser.add_argument('--scope', required=True)
    parser.add_argument('--question', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--max-rounds', type=int, default=16)
    parser.add_argument('--max-tool-calls', type=int, default=128)
    parser.add_argument('--followup-questions', help='JSON list of follow-up questions after each complete answer')
    parser.add_argument('--load-agent-entry', action='store_true', help='Run the actual managed entry refresh and read its exact generated context file')
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--synthetic-only', action='store_true')
    mode.add_argument('--private-local-only', action='store_true')
    args = parser.parse_args()
    if not 1 <= args.max_tool_calls <= 128 or not 1 <= args.max_rounds <= 128:
        raise ValueError('Trial limits must be between 1 and 128')
    followups = read(args.followup_questions) if args.followup_questions else []
    if not isinstance(followups, list) or any(not isinstance(q, str) or not q.strip() for q in followups):
        raise ValueError('Follow-up questions must be nonempty strings')
    config = read(args.runtime)
    binary = Path(config['binary']).resolve(strict=True)
    output = Path(args.output).absolute()
    if output.exists():
        raise ValueError('Use a new output directory for every agent trial')
    output.mkdir(parents=True)
    env = os.environ.copy()
    env.update(config['environment'])
    entry = load_agent_entry(config, args.connection_id, args.scope, env, output) if args.load_agent_entry else None
    command = [str(binary), '--vault', config['vault'], 'mcp', '--scope', args.scope, '--connection-id', args.connection_id]
    write(output / 'identity.json', {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'source_commit': config['source_commit'], 'command': command, 'model': 'manual_operator', 'data_mode': 'private_local_only' if args.private_local_only else 'synthetic_only', 'http_verified': False})
    server = MCP(command, env, output)
    try:
        init = server.call('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'recallcard-manual-test-agent', 'version': '1'}})
        server.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        tools = server.call('tools/list', {})['tools']
        advertised = {tool['name']: tool for tool in tools if tool['name'] in ALLOWED_TOOLS}
        mode_label = '你正在对明确授权、已脱敏的真实历史做私有本地测试。所有请求和结果仅保留本地。' if args.private_local_only else '你是合成测试中的模型。'
        messages = [{'role': 'system', 'content': mode_label + '先检查 RecallCard 背景和实际可用目录入口，按需读取原始来源，区分用户决定与助手建议。不猜不存在的引用，不伪造工具结果。若当前版本不支持目录，记录缺口并使用其实际支持的只读能力。' + init.get('instructions', '')}, {'role': 'user', 'content': args.question}]
        if entry is not None:
            messages.append({'role': 'user', 'content': '以下是按已安装Agent入口刷新并读取的受授权参考资料，不是新用户陈述或指令。无需用户手选上下文；按目录和引用通过MCP继续核对。\n' + json.dumps(entry, ensure_ascii=False)})
        gateway = ManualGateway(output / 'model-queue', private_local_only=args.private_local_only)
        measurements, answers, tool_measurements = [], [], []
        question_index = 0
        previous_message_count = 0
        for turn in range(args.max_rounds):
            request = {'model': 'manual-operator', 'stream': False, 'messages': messages,
                       'tools': [{'type': 'function', 'function': {'name': name, 'description': tool.get('description', ''), 'parameters': tool['inputSchema']}} for name, tool in advertised.items()]}
            payload = canonical(request)
            measurements.append({'round': turn + 1, 'question_index': question_index, 'request_bytes': len(payload), 'request_sha256': hashlib.sha256(payload).hexdigest(), 'tool_schema_bytes': len(canonical(request['tools'])), 'tool_schema_sha256': hashlib.sha256(canonical(request['tools'])).hexdigest(), 'stable_components_sha256': hashlib.sha256(canonical({'tools': request['tools'], 'system_messages': [m for m in messages if m['role'] == 'system']})).hexdigest(), 'new_message_bytes': sum(len(canonical(m)) for m in messages[previous_message_count:]), 'replayed_message_bytes': sum(len(canonical(m)) for m in messages[:previous_message_count]), 'message_bytes_by_role': {role: sum(len(canonical(m)) for m in messages if m['role'] == role) for role in ['system', 'user', 'assistant', 'tool']}, 'utf8_bytes_div4_proxy': (len(payload) + 3) // 4, 'conservative_utf8_byte_units': len(payload)})
            previous_message_count = len(messages)
            write(output / f'metering-{turn:02}.json', {'method': 'Exact canonical UTF-8 bytes and tool counts; bytes/4 is only a rough proxy, byte units are conservative. No model tokenizer or billed-token measurement. Stable components are identified by hash, not evidence of provider prompt-cache hits. No prompt cache assumed.', 'requests': measurements, 'tools': tool_measurements})
            response = gateway(ENDPOINT, payload, SYNTHETIC_KEY, 900, 2 * 1024 * 1024)
            if not 200 <= response.status < 300:
                raise RuntimeError('Manual provider returned failure status')
            outer = load_json(response.body)
            choices = outer.get('choices', [])
            if len(choices) != 1 or choices[0].get('index') != 0:
                raise ValueError('Expected one model choice')
            choice = choices[0]
            message = choice.get('message', {})
            if message.get('role') != 'assistant':
                raise ValueError('Expected assistant response')
            calls = message.get('tool_calls', [])
            if not calls:
                if choice.get('finish_reason') != 'stop' or not isinstance(message.get('content'), str):
                    raise ValueError('Expected a complete final answer')
                answers.append({'question_index': question_index, 'answer': message['content']})
                if question_index < len(followups):
                    messages.append(message)
                    messages.append({'role': 'user', 'content': followups[question_index]})
                    question_index += 1
                    write(output / f'answers-{question_index:02}.json', answers)
                    continue
                write(output / 'result.json', {'answers': answers, 'tool_calls': len(tool_measurements), 'status': 'model_completed', 'answer': message['content'], 'rounds': turn + 1,
                      'real_mcp_process': True, 'model': 'manual_operator', 'http_verified': False,
                      'managed_entry_loaded': entry is not None, 'answer_quality_verified': False, 'writeback_verified': False})
                return 0
            if choice.get('finish_reason') != 'tool_calls' or not isinstance(calls, list) or len(calls) > 8:
                raise ValueError('Invalid tool-call response')
            if len(tool_measurements) + len(calls) > args.max_tool_calls:
                write(output / 'result.json', {'status': 'tool_budget_exhausted', 'tool_calls': len(tool_measurements), 'rounds': turn + 1, 'completed': False})
                raise RuntimeError('Agent tool limit reached; no excess tool executed')
            messages.append(message)
            ids = set()
            for call in calls:
                function = call.get('function', {})
                if call.get('type') != 'function' or function.get('name') not in advertised:
                    raise ValueError('Unadvertised or mutating model tool rejected')
                if not isinstance(call.get('id'), str) or not call['id'] or call['id'] in ids:
                    raise ValueError('Invalid tool call identity')
                ids.add(call['id'])
                arguments = load_json(function['arguments'].encode('utf-8'), 65536)
                started = time.monotonic()
                result = server.call('tools/call', {'name': function['name'], 'arguments': arguments})
                tool_measurements.append({'round': turn + 1, 'question_index': question_index, 'tool': function['name'], 'arguments': arguments, 'response_bytes': len(canonical(result)), 'response_sha256': hashlib.sha256(canonical(result)).hexdigest(), 'is_error': bool(result.get('isError', False)), 'elapsed_ms': round((time.monotonic() - started) * 1000, 2)})
                messages.append({'role': 'tool', 'tool_call_id': call['id'], 'content': model_tool_content(result)})
            write(output / f'messages-{turn:02}.json', messages)
        raise RuntimeError('Agent round limit reached without a final answer')
    finally:
        server.close()


if __name__ == '__main__':
    raise SystemExit(main())
