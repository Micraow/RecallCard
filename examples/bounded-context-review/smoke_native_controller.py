"""Opt-in synthetic MCP + native-file controller smoke, using an existing binary.

Native judgments here are explicitly scripted toys, not measured model quality.
No network, full-original verifier, compiler or provider request is invoked.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time
from judgment_contract import Scope, encode
from native_review_controller import Reply
from review_journal import FileSession, write_once


def script(packet, select=False):
    row = packet['candidates'][0]; span = row['context'][-1 if select else 0]
    return encode({'packet_id':packet['packet_id'], 'assessments':[{
        'candidate':row['candidate'], 'source_relevance':'relevant' if select else 'uncertain',
        'answer_coverage':'partial', 'support':[{'evidence':span['evidence'],'quote':span['text'][:30]}]
            + ([{'evidence':span['evidence'],'quote':span['text'][-15:]}] if select else []),
        'unresolved':[], 'source_reason':'The invented source clue links the subject.' if select else 'Need another bounded original clue.',
        'coverage_reason':'The answer is still incomplete.'}], 'selected':row['candidate'] if select else None,
        'inspect':[] if select else [{'candidate':row['candidate'],
            'action_id':next(a['action_id'] for a in row['legal_actions'] if a['action']=='continue'),
            'missing_evidence_reason':'Need the immediately following bounded source clue.'}],
        'stop_reason':'selected' if select else 'need_inspection', 'abstention_reason':None})


def run(binary, output):
    output.mkdir(exist_ok=False)
    records=[]
    with tempfile.TemporaryDirectory(prefix='synthetic-',dir=output) as directory:
        root=Path(directory); vault=root/'vault'
        env={**os.environ,'RECALLCARD_STATE_DIR':str(root/'state')}
        subprocess.run([binary,'--vault',str(vault),'init'],env=env,check=True,capture_output=True)
        for i,role in enumerate(('user','assistant')):
            event={'role':role,'origin':'native','scope':'personal','session_id':'controller-toy',
                'occurred_at':'2026-01-02T00:00:00Z','content':('beacon '+'An invented constraint α. '*200) if i==0 else 'An invented later reply.',
                'source':{'platform':'controller-toy','account_namespace':'synthetic-owner','conversation_id':'toy-session','message_id':str(i)},
                'metadata':{'qwen':{'node_id':str(i),'parent_id':None if i==0 else '0','on_current_path':True}}}
            file=root/f'event-{i}.json';file.write_bytes(encode(event))
            subprocess.run([binary,'--vault',str(vault),'capture','--file',str(file)],env=env,check=True,capture_output=True)
        proc=subprocess.Popen([binary,'--vault',str(vault),'mcp','--scope','personal'],
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env)
        sequence=0; buffer=b''
        def call(name,args):
            nonlocal sequence,buffer
            sequence+=1; start=time.monotonic()
            method='tools/call' if name in ('search','read') else name
            request={'jsonrpc':'2.0','method':method}
            notify=method=='notifications/initialized'
            if not notify:request['id']=sequence
            if args is not None:request['params']={'name':name,'arguments':args} if method=='tools/call' else args
            raw=encode(request)+b'\n';proc.stdin.write(raw);proc.stdin.flush()
            if notify:return Reply(b'',len(raw),0,round((time.monotonic()-start)*1000))
            selector=selectors.DefaultSelector();selector.register(proc.stdout,selectors.EVENT_READ)
            while b'\n' not in buffer:
                assert selector.select(15),'MCP timeout'
                chunk=os.read(proc.stdout.fileno(),65536);assert chunk,'closed MCP';buffer+=chunk
            selector.close();wire,buffer=buffer.split(b'\n',1);wire+=b'\n'
            response=json.loads(wire);assert response.get('id')==sequence and not response.get('error'),response
            result=response['result']
            if method=='tools/call':
                assert not result.get('isError') and len(result['content'])==1,result
                body=result['content'][0]['text'].encode()
            else:body=encode(result)
            records.append({'request':request,'response':response})
            return Reply(body,len(raw),len(wire),round((time.monotonic()-start)*1000))
        try:
            scope=Scope.create('beacon',['personal'],{'role':'user'},'2026-02-03T00:00:00Z')
            session=FileSession(output/'session',scope,call)
            session.perform('setup',{'method':'initialize','params':{'protocolVersion':'2025-11-25'}})
            session.perform('setup',{'method':'notifications/initialized'})
            session.perform('setup',{'method':'tools/list'})
            packet=session.perform('discovery',dict(query='beacon',target='events',detail='session_discovery_compact',
                limit=3,budget_bytes=32768,event_filter={'role':'user'},as_of=scope.as_of))
            write_once(output/'native-01.json',script(packet))
            following=session.native((output/'native-01.json').read_bytes())
            # Resume reconstructs the complete session without issuing any tools.
            before=len(records);session=FileSession.restore(output/'session',call);assert len(records)==before
            write_once(output/'native-02.json',script(following,True))
            session.native((output/'native-02.json').read_bytes())
            raw=session.perform('read',None)
            import base64
            page=json.loads(base64.b64decode(raw))
            assert page['relevance']=='caller_assessed' and page['evidence_binding']=='verified' and page['results']
            before=len(records);FileSession.restore(output/'session');assert len(records)==before
            receipt={'schema':'recallcard.native-controller-synthetic-mcp/1','passed':True,
                'synthetic_only':True,'native_judgments':'scripted_not_model_quality',
                'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                'model_or_provider_requests':0,'rust_builds':0,'unbounded_original_fetches':0,
                'extra_original_verification_fetches':0,'resume_dispatched_tools':0,
                'source_relevance':'relevant','answer_coverage':'partial','server_verified_read':True,
                'multiple_quotes_from_same_visible_span':True,
                'unique_proof_spans':len(session.controller.workflow.selected_read_args()['context_review']['candidate']['evidence']),
                'costs':session.controller.costs.summary()}
            write_once(output/'receipt.json',encode(receipt))
            print(json.dumps(receipt))
        finally:
            write_once(output/'wire.json',encode(records))
            proc.stdin.close();code=proc.wait(timeout=15);stderr=proc.stderr.read()
            assert code==0 and not stderr,(code,stderr)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True);parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args();run(args.binary,args.output)
