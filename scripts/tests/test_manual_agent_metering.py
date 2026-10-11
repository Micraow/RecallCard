"""Harness bookkeeping only; no claim of real MCP or model acceptance."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import manual_mcp_agent as agent


class MeteringTests(unittest.TestCase):
    def test_mcp_text_forwarding_preserves_data_and_errors_without_envelope(self):
        raw = '{"text":"合成原话","snapshot":"abc"}'
        self.assertEqual(agent.model_tool_content({'content':[{'type':'text','text':raw}],'isError':False}),raw)
        error = json.loads(agent.model_tool_content({'content':[{'type':'text','text':'拒绝越权'}],'isError':True}))
        self.assertTrue(error['isError'])
        self.assertEqual(error['text'],'拒绝越权')

    def test_followup_keeps_immutable_per_round_metering(self):
        class Server:
            def __init__(self, *_): pass
            def call(self, method, params):
                return {'tools': []} if method == 'tools/list' else {}
            def send(self, *_): pass
            def close(self): pass
        class Gateway:
            def __init__(self, *_): pass
            def __call__(self, *args):
                return SimpleNamespace(status=200, body=json.dumps({'choices':[{'index':0,'message':{'role':'assistant','content':'synthetic answer'},'finish_reason':'stop'}]}).encode())
        with tempfile.TemporaryDirectory() as t:
            root=Path(t); binary=root/'binary';binary.write_bytes(b'synthetic')
            config=root/'runtime.json';config.write_text(json.dumps({'binary':str(binary),'vault':str(root/'vault'),'environment':{},'source_commit':'test'}))
            followups=root/'followups.json';followups.write_text('["Second question"]')
            output=root/'output'
            argv=['manual_mcp_agent.py','--runtime',str(config),'--connection-id','synthetic','--scope','personal','--question','First question','--followup-questions',str(followups),'--output',str(output),'--synthetic-only']
            with patch.object(sys,'argv',argv),patch.object(agent,'MCP',Server),patch.object(agent,'ManualGateway',Gateway):
                self.assertEqual(agent.main(),0)
            first=json.loads((output/'metering-00.json').read_text())
            second=json.loads((output/'metering-01.json').read_text())
            self.assertEqual(len(first['requests']),1)
            self.assertEqual(len(second['requests']),2)
            self.assertEqual(second['requests'][1]['question_index'],1)
            self.assertGreater(second['requests'][1]['request_bytes'],first['requests'][0]['request_bytes'])
            result=json.loads((output/'result.json').read_text())
            self.assertEqual(len(result['answers']),2)
            self.assertEqual(result['tool_calls'],0)


if __name__=='__main__':unittest.main()
