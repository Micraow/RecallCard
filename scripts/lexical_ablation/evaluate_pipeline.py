#!/usr/bin/env python3
"""生产Rust→Python→数值HTTP→SQLite→RRF→正本/共享预算的原创合成试验。"""
import argparse
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import zipfile
import evaluate

MODES=('count_production','bm25_same','bm25_same_b0')

def fixture():
    data=evaluate.fixture(); data['schema']='recallcard.synthetic-lexical-pipeline/1'
    # 固定数值干扰通道，与真实模型质量无关；保证精确目标在semantic top100外。
    for d in data['documents']:
        if d['label'].startswith('background-'):
            d['text']='DISTRACTOR_SEMANTIC '+d['text']
            d['content_hash']=evaluate.digest(d['text'].encode())
    def doc(label,text,**extra):
        ident=evaluate.source_id('pipeline-'+label)
        data['documents'].append(dict(id=ident,label=label,text=text,scope='personal',active=True,content_hash=evaluate.digest(text.encode()),**extra))
        return ident
    def query(label,text,ids,category='pipeline_boundary'):
        data['queries'].append(dict(id=label,text=text,scope='personal',category=category,qrels={i:1 for i in ids},expectation='source'))
    edge=doc('boundary-exact',' '*247+'EDGE_CODE_710 已处理。'+('合成正文。'*100))
    doc('boundary-neighbor','EDGE_CODE_7100 尚未处理。')
    query('boundary-exact','EDGE_CODE_710',[edge])
    query('boundary-prefix-absent','EDGE_CODE_71',[],category='negative')
    long=doc('multi-chunk',('合成前文记录。'*500)+'TAIL_ANCHOR_83 末尾证据。')
    query('multi-chunk','TAIL_ANCHOR_83',[long])
    old=doc('revision-old','REVISION_MARKER_81 旧内容已过时。')
    new=doc('revision-new','REVISION_MARKER_81 现在改为另一方案。',replaces='revision-old')
    query('revision-current','REVISION_MARKER_81',[new])
    data['replaced_ids']=[old]
    promoted=doc('anchor-promotion','BOLT_CODE_81 NUT_CODE_92 完整原始编号。')
    for i in range(6):
        doc(f'anchor-decoy-{i}','DISTRACTOR_SEMANTIC BOLT_CODE_81 记录检修清单，但是缺少另一个编号。')
    query('anchor-promotion','BOLT_CODE_81 NUT_CODE_92 记录检修清单',[promoted])
    assistant=doc('assistant-negation','助手仅建议 ASSISTANT_OPTION_44，用户尚未同意。',role='assistant')
    query('assistant-negation','ASSISTANT_OPTION_44',[assistant],category='negation_evidence')
    return data

def attachment_archive(path):
    body=b'ATTACHMENT_BODY_ONLY_834 synthetic opaque payload'
    message={'id':'pipeline-upload','author':{'role':'user'},'content':{'content_type':'text','parts':[]},'metadata':{'attachments':[{'id':'file_PIPELINE_SYNTHETIC','name':'pipeline-attachment-note.txt','mime_type':'text/plain','size':len(body)}]}}
    conversation={'id':'pipeline-attachment','title':'合成附件边界','current_node':'pipeline-upload','mapping':{'root':{'id':'root','parent':None,'message':None},'pipeline-upload':{'id':'pipeline-upload','parent':'root','message':message}}}
    manifest={'version':1,'manifest_file':'export_manifest.json','export_files':[{'path':'file_PIPELINE_SYNTHETIC.dat','size_bytes':len(body)}],'logical_files':{'file_PIPELINE_SYNTHETIC.dat':{'files':['file_PIPELINE_SYNTHETIC.dat'],'sharded':False}}}
    with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_STORED) as z:
        for name,value in [('conversations.json',[conversation]),('export_manifest.json',manifest),('conversation_asset_file_names.json',{'file_PIPELINE_SYNTHETIC.dat':'pipeline-attachment-note.txt'})]:
            z.writestr('export/'+name,json.dumps(value,ensure_ascii=False))
        z.writestr('export/file_PIPELINE_SYNTHETIC.dat',body)

@contextmanager
def provider():
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*_): pass
        def do_POST(self):
            assert self.path=='/v1/embeddings'
            data=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            assert data['model']=='synthetic-pipeline-fixed-vectors'
            vectors=[]
            for value in data['input']:
                vectors.append([1.,0.] if value.startswith('synthetic-query\n') or 'DISTRACTOR_SEMANTIC' in value else [0.,1.])
            encoded=json.dumps({'model':data['model'],'data':[{'index':i,'embedding':v} for i,v in enumerate(vectors)]}).encode()
            self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(encoded))); self.end_headers(); self.wfile.write(encoded)
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True); thread.start()
    try: yield f'http://127.0.0.1:{server.server_port}/v1/embeddings'
    finally: server.shutdown(); server.server_close(); thread.join()

def worker_copy(repo,path,fixture_path,mode):
    path.mkdir()
    shutil.copytree(repo/'python/recallcard_worker',path/'recallcard_worker',ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    for name in ('evaluate.py','pipeline_worker.py'):
        shutil.copyfile(Path(__file__).with_name(name),path/name)
    (path/'pipeline_experiment.json').write_text(json.dumps({'fixture':str(fixture_path),'mode':mode}))
    with (path/'recallcard_worker/__init__.py').open('a') as f:
        f.write('\n# 只在临时合成副本安装实验排序。\nfrom pathlib import Path as _Path\nfrom . import event_index as _event_index\nfrom pipeline_worker import install as _install\n_install(_event_index, _Path(__file__).resolve().parent.parent)\n')

def analyze(data,host,worker_root,chunk_bytes):
    refs=host['references']; reverse={r:ident for ident,r in refs.items()}; docs={d['id']:d for d in data['documents']}
    forbidden={refs[d['id']] for d in data['documents'] if not d['active'] or d['scope']!='personal'}|{refs[i] for i in data['replaced_ids']}
    chunks={}
    for line in (worker_root/'chunks.jsonl').read_text().splitlines():
        item=json.loads(line); ref=item['source']['ref']; source=docs[reverse[ref]]['text']; raw=source.encode()
        assert ref not in forbidden
        for c in item['chunks']:
            start,end=c['start_byte'],c['end_byte']; assert 0<end-start<=chunk_bytes
            assert raw[start:end].decode()==c['text'] and evaluate.digest(raw[start:end])==c['text_sha256']
            assert source.encode()[max(0,start-len(c['lexical_before'].encode())):start]==c['lexical_before'].encode()
            chunks[(ref,c['ordinal'])]=(start,end)
    raw_queries={}
    for line in (worker_root/'queries.jsonl').read_text().splitlines():
        item=json.loads(line); raw_queries.setdefault(item['query'],item['result'])
    details=[]; quality={stage:[] for stage in ('worker_rrf','host_anchor','shared_budget')}
    for q,page in zip(data['queries'],host['pages']):
        assert q['id']==page['query']
        worker=raw_queries[q['text']]
        for r in worker['results']:
            evidence=r['retrieval']; expected=sum(1/(60+rank) for rank in (evidence['semantic_rank'],evidence['lexical_rank']) if rank is not None)
            assert abs(expected-evidence['fusion_score'])<1e-12
        stage_rows={'worker_rrf':worker['results'],'host_anchor':page['host']['results'],'shared_budget':page['shared']['results']}
        stages={}
        for name,rows in stage_rows.items():
            ids=[r['ref'] for r in rows]
            assert len(set(ids))==len(ids) and not (set(ids)&forbidden)
            mapped=[{'id':reverse[r['ref']]} for r in rows]
            metric=evaluate.metrics(q,mapped)
            quality[name].append(dict(query=q['id'],category=q['category'],positive=bool(q['qrels']),coverage={'status':worker['coverage']['lexical']['status'],'truncated':bool(worker['coverage']['candidates_truncated'] or worker['coverage']['lexical'].get('candidates_truncated')) if name=='worker_rrf' else bool(page['host' if name=='host_anchor' else 'shared']['truncated'])},metrics=metric))
            stages[name]=dict(metrics=metric,top5=[dict(label=docs[reverse[r['ref']]]['label'],semantic_rank=r.get('semantic_rank',r.get('retrieval',{}).get('semantic_rank')),lexical_rank=r.get('retrieval',{}).get('lexical_rank'),fused_rank=r.get('retrieval',{}).get('fused_rank'),protected=r.get('retrieval_support',{}).get('protected'),matched_chunk=r.get('matched_chunk'),lexical_evidence=r.get('retrieval',{}).get('lexical_evidence')) for r in rows[:5]])
            if name!='worker_rrf':
                flags=[r['retrieval_support']['protected'] for r in rows]
                assert flags==sorted(flags,reverse=True),'精确锚点优先层发生倒置'
                for r in rows:
                    assert r['role']==docs[reverse[r['ref']]].get('role','user')
                    assert r['relevance']=='unassessed'
                    source=docs[reverse[r['ref']]]['text'].encode(); a,b=r['text_range']['start_byte'],r['text_range']['end_byte']
                    assert source[a:b].decode()==r['text'] and evaluate.digest(source[a:b])==r['text_hash']
        # Rust内已验证共享8KiB；快照必须与通过真实recall_fetch读回的来源相同。
        if page['shared']['results']:
            assert page['first_fetch']['results'][0]['snapshot']==page['shared']['results'][0]['snapshot']
        details.append(dict(query=q['id'],category=q['category'],stages=stages,shared_truncated=page['shared']['truncated']))
    assert host['revoked_search_rejected'] and host['revoked_fetch_rejected']
    attachment=host['attachment']; assert attachment['event_text']=='' and attachment['excluded_from_event_embeddings']
    assert len(attachment['name_search']['results'])==2
    assert len(attachment['body_search']['results'])==1
    body_row=attachment['body_search']['results'][0]
    shared=attachment['shared_body_search']['results'][0]
    fetched=attachment['body_fetch']['results'][0]
    assert body_row['kind']==shared['kind']=='attachment'
    assert body_row['ref']==shared['ref']==fetched['ref']
    assert shared['snapshot']==fetched['snapshot']
    assert fetched['text']=='ATTACHMENT_BODY_ONLY_834 synthetic opaque payload'
    assert fetched['text_hash']==evaluate.digest(fetched['text'].encode())
    assert fetched['text_range']['projection']=='attachment.utf8/1'
    assert attachment['suppressed_fetch_rejected'] and attachment['suppressed_search_empty']
    boundary=next(r for r in details if r['query']=='boundary-exact')['stages']
    assert boundary['worker_rrf']['top5'][0]['label']=='boundary-exact'
    assert boundary['worker_rrf']['top5'][0]['semantic_rank'] is None
    assert boundary['host_anchor']['top5'][0]['protected'] is True
    assert boundary['shared_budget']['top5'][0]['label']=='boundary-exact'
    if chunk_bytes==256:
        ev=boundary['shared_budget']['top5'][0]['lexical_evidence']; assert ev['start_byte']<256<ev['end_byte']
    promotion=next(r for r in details if r['query']=='anchor-promotion')['stages']
    assert promotion['worker_rrf']['top5'][0]['label']!='anchor-promotion'
    assert promotion['host_anchor']['top5'][0]['label']=='anchor-promotion'
    assert promotion['shared_budget']['top5'][0]['label']=='anchor-promotion'
    return dict(chunks=len(chunks),sources=len({ref for ref,ordinal in chunks}),max_chunks_per_event=max(Counter(ref for ref,ordinal in chunks).values()),
        checks={'actual_rust_chunks_and_spans':True,'event_deduplication':True,'exact_anchor_tier':True,'exact_anchor_promoted_over_rrf_decoy':True,'rrf_formula':True,'source_hash_and_fetch':True,'roles_preserved_and_relevance_unassessed':True,'scope_suppression_revision_exclusion':True,'revoked_search_and_fetch':True,'attachment_lexical_body_and_bound_fetch':True,'attachment_suppression':True,'shared_response_8192_bytes':True},
        summary={name:evaluate.summarize(rows) for name,rows in quality.items()},details=details)

def run(repo,binary,out,modes,sizes):
    if out.exists(): raise ValueError('使用全新输出目录，避免覆盖任何资料')
    out.mkdir(parents=True); data=fixture(); fixture_path=out/'fixture.json'; fixture_path.write_bytes(evaluate.canonical(data)+b'\n'); attachment=out/'synthetic-attachment.zip'; attachment_archive(attachment)
    results={}
    with provider() as endpoint:
        for size in sizes:
            for mode in modes:
                label=f'{mode}-chunk{size}'; worker_root=out/(label+'-worker'); worker_copy(repo,worker_root,fixture_path,mode)
                environment=dict(os.environ); environment['RECALLCARD_STATE_DIR']=str(out/(label+'-state'))
                proc=subprocess.run([str(binary),str(fixture_path),str(out/label),str(worker_root),endpoint,str(size),str(attachment)],env=environment,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=300)
                (out/(label+'.log')).write_text(proc.stdout+proc.stderr)
                if proc.returncode: raise RuntimeError(f'{label}失败: {proc.stderr[-2000:]}')
                host=json.loads((out/label/'host.json').read_text())
                for path,sha in host['compiled_source_sha256'].items():
                    assert evaluate.digest((repo/path).read_bytes())==sha,'例程编译时的核心源码与本次工作区不一致'
                results[label]=analyze(data,host,worker_root,size)
                results[label]['compiled_source_sha256']=host['compiled_source_sha256']
                print(label,json.dumps(results[label]['summary'],ensure_ascii=False),flush=True)
    code_files=['crates/recallcard/src/context/attachments.rs','crates/recallcard/src/context.rs','crates/recallcard/src/context/paging.rs','crates/recallcard/src/context/search.rs','crates/recallcard/examples/synthetic_lexical_pipeline.rs','crates/recallcard/src/local_events/snapshot.rs','crates/recallcard/src/local_events/ranking.rs','crates/recallcard/src/local_events/lexical_context.rs','crates/recallcard/src/local_events.rs','crates/recallcard/src/context/recall.rs','python/recallcard_worker/event_index.py','python/recallcard_worker/event_lexical.py','scripts/lexical_ablation/evaluate_pipeline.py','scripts/lexical_ablation/pipeline_worker.py']
    report=dict(schema='recallcard.synthetic-lexical-pipeline-report/1',fixture_sha256=evaluate.digest(fixture_path.read_bytes()),source_sha256={p:evaluate.digest((repo/p).read_bytes()) for p in code_files},
        models_downloaded=0,real_model_calls=0,encoder='fixed adversarial numeric vectors: 206 distractors outrank ordinary sources; not model quality',documents=len(data['documents']),queries=len(data['queries']),
        note='实际生产分块、Event聚合、RRF、锚点优先和共享预算；仅临时worker副本替换同分词BM25候选顺序。合成来源，未更改生产默认。',results=results)
    (out/'pipeline-results.json').write_bytes(evaluate.canonical(report)+b'\n')
    return report

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--repo',type=Path,required=True);p.add_argument('--binary',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--modes',nargs='+',choices=MODES,default=list(MODES));p.add_argument('--chunk-sizes',nargs='+',type=int,choices=[256,8192],default=[256,8192]);a=p.parse_args();run(a.repo.resolve(),a.binary.resolve(),a.out.resolve(),a.modes,a.chunk_sizes)
