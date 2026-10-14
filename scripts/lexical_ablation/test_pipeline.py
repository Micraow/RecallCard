"""临时BM25 worker与原始合成附件的局部合同；完整Rust链单独运行。"""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
import evaluate
import evaluate_pipeline
import pipeline_worker

REPO=Path(os.environ.get('RECALLCARD_EVAL_REPO',Path(__file__).resolve().parents[2]))
sys.path.insert(0,str(REPO/'python'))
sys.path.insert(0,str(REPO/'python/tests'))
from recallcard_worker import event_index, event_lexical
from test_event_index import settings, Encoder

class PipelineMethods(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.original=(event_lexical.candidates,event_index.EventIndex.upsert,event_index.EventIndex.query)
    def tearDown(self):
        event_lexical.candidates,event_index.EventIndex.upsert,event_index.EventIndex.query=self.original
        self.temp.cleanup()
    def worker(self,mode):
        bodies={'a':'rare rare common','b':'rare common'}
        fixture={'schema':'recallcard.synthetic-lexical-pipeline/1','private_data':False,'documents':[{'text':body,'content_hash':evaluate.digest(body.encode())} for body in bodies.values()]}
        fp=self.root/'fixture.json';fp.write_text(json.dumps(fixture))
        (self.root/'pipeline_experiment.json').write_text(json.dumps({'fixture':str(fp),'mode':mode}))
        pipeline_worker.install(event_index,self.root)
        worker=event_index.EventIndex(settings(self.root/'index.sqlite'),encoder=Encoder())
        def call(op,**kw):
            r=worker.execute({'schema':event_index.SCHEMA,'id':1,'op':op,**kw});self.assertTrue(r['ok'],r);return r['result']
        build=call('begin',source_epoch='1'*64,lexical_version=event_lexical.VERSION)
        for name,body in bodies.items():
            cuts=[0,9,len(body)] if name=='a' else [0,len(body)]
            chunks=[]
            for ordinal,(start,end) in enumerate(zip(cuts,cuts[1:])):
                text=body[start:end]
                chunks.append(dict(ordinal=ordinal,start_byte=start,end_byte=end,text=text,text_sha256=evaluate.digest(text.encode()),lexical_before=body[max(0,start-256):start],lexical_after=body[end:end+256]))
            result=call('upsert',build_id=build['build_id'],batch_id=name,source={'ref':'event:'+name,'scope':'synthetic','content_hash':evaluate.digest(body.encode()),'role':'user','session_ref':None,'occurred_at':None,'text_bytes':len(body)},source_chunk_count=len(chunks),chunks=chunks)
        call('finish',**{k:result[k] for k in ('build_id','source_epoch','source_count','chunk_count','receipt')})
        return worker,call
    def test_bm25_uses_actual_chunk_terms_and_keeps_event_aggregation(self):
        worker,call=self.worker('bm25_same_b0')
        result=call('query',query='rare')
        byref={row['ref']:row for row in result['results']}
        self.assertEqual(len(byref),2)
        self.assertEqual(byref['event:a']['retrieval']['lexical_rank'],1)
        self.assertEqual(byref['event:a']['start_byte'],0)
        self.assertEqual(byref['event:a']['end_byte'],9)
        self.assertEqual(byref['event:a']['retrieval']['lexical_matched_terms'],1)
        self.assertEqual(result['coverage']['fusion'],event_lexical.FUSION)
        self.assertEqual(len((self.root/'chunks.jsonl').read_text().splitlines()),2)
        self.assertEqual(len((self.root/'queries.jsonl').read_text().splitlines()),1)
    def test_bm25_candidate_window_counts_equal_production(self):
        worker,call=self.worker('bm25_same')
        result=call('query',query='rare common')
        self.assertEqual(result['coverage']['lexical']['candidate_chunks'],3)
        self.assertEqual(result['coverage']['lexical']['postings_scanned'],4)
        self.assertEqual(result['coverage']['lexical_candidates'],2)
    def test_synthetic_attachment_keeps_body_distinct_from_message(self):
        path=self.root/'attachment.zip';evaluate_pipeline.attachment_archive(path)
        with zipfile.ZipFile(path) as z:
            messages=json.loads(z.read('export/conversations.json'))
            m=messages[0]['mapping']['pipeline-upload']['message']
            self.assertEqual(m['content']['parts'],[])
            self.assertEqual(m['metadata']['attachments'][0]['name'],'pipeline-attachment-note.txt')
            self.assertIn(b'ATTACHMENT_BODY_ONLY_834',z.read('export/file_PIPELINE_SYNTHETIC.dat'))
            self.assertNotIn('ATTACHMENT_BODY_ONLY_834',json.dumps(messages))
    def test_fixture_does_not_use_labels_or_qrels_to_generate_numeric_vectors(self):
        data=evaluate_pipeline.fixture()
        self.assertTrue(data['replaced_ids'])
        self.assertEqual(sum('DISTRACTOR_SEMANTIC' in d['text'] for d in data['documents']),206)
        self.assertEqual(len({d['id'] for d in data['documents']}),len(data['documents']))
        self.assertEqual(len({q['id'] for q in data['queries']}),len(data['queries']))
        self.assertTrue(all(set(q['qrels'])<={d['id'] for d in data['documents']} for q in data['queries']))

if __name__=='__main__':unittest.main()
