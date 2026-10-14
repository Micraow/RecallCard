"""仅装入临时合成worker副本；不修改生产词项、协议或默认排序。"""
from collections import Counter, defaultdict, deque
import json
import math
from pathlib import Path


def install(worker, root):
    import evaluate
    lexical=worker.lexical
    config=json.loads((Path(root)/'pipeline_experiment.json').read_text())
    fixture=json.loads(Path(config['fixture']).read_text())
    assert fixture['schema']=='recallcard.synthetic-lexical-pipeline/1' and fixture['private_data'] is False
    originals={d['content_hash']:d['text'] for d in fixture['documents']}
    original_candidates=lexical.candidates
    mode=config['mode']
    assert mode in ('count_production','bm25_same','bm25_same_b0')
    cache={}

    def stats(db,generation,scope):
        identity=(generation,scope)
        if identity in cache: return cache[identity]
        chunks={}; df=Counter()
        rows=db.execute('SELECT c.source_ref,c.ordinal,c.start_byte,c.end_byte,s.content_hash FROM chunks c JOIN sources s ON s.generation=c.generation AND s.ref=c.source_ref WHERE c.generation=? AND s.scope=?',(generation,scope))
        for ref,ordinal,start,end,content_hash in rows:
            source=originals[content_hash]
            assert evaluate.digest(source.encode())==content_hash
            occurrences=[r for r in evaluate.tokenize(source,'bm25_same',lexical) if r[1]<end and r[2]>start]
            tf=Counter(r[0] for r in occurrences)
            actual={r[0] for r in db.execute('SELECT term_hash FROM lexical_terms WHERE generation=? AND source_ref=? AND ordinal=?',(generation,ref,ordinal))}
            assert set(tf)==actual,'实验统计必须与生产块的完整边界词项一致'
            chunks[(ref,ordinal)]=(tf,sum(tf.values()))
            df.update(tf.keys())
        average=sum(v[1] for v in chunks.values())/len(chunks) if chunks else 0
        cache[identity]=(chunks,df,average)
        return cache[identity]

    def candidates(db,generation,scope,query_terms):
        baseline,coverage=original_candidates(db,generation,scope,query_terms)
        if mode=='count_production': return baseline,coverage
        chunks,df,avgdl=stats(db,generation,scope)
        found={}; active=deque(); scanned=0
        for key,*_ in query_terms:
            active.append((key,iter(db.execute('SELECT t.source_ref,t.ordinal,t.start_byte,t.end_byte,t.text_sha256 FROM lexical_terms t JOIN sources s ON s.generation=t.generation AND s.ref=t.source_ref WHERE t.generation=? AND t.term_hash=? AND s.scope=? ORDER BY t.source_ref,t.ordinal LIMIT ?',(generation,key,scope,lexical.MAX_POSTINGS)))))
        while active and scanned<lexical.MAX_POSTINGS:
            key,cursor=active.popleft(); row=next(cursor,None)
            if row is None: continue
            scanned+=1; ref,ordinal,start,end,sha=row; ident=(ref,ordinal)
            if ident in found or len(found)<lexical.MAX_CANDIDATE_CHUNKS:
                matched,evidence=found.get(ident,(set(),(start,end,sha)))
                matched.add(key); found[ident]=(matched,min(evidence,(start,end,sha)))
            active.append((key,cursor))
        for _,cursor in active:
            cursor.close()
        assert scanned==coverage['postings_scanned'] and len(found)==coverage['candidate_chunks']
        k1=1.2; b=0 if mode=='bm25_same_b0' else .75; N=len(chunks)
        best={}; keys={q[0] for q in query_terms}
        for (ref,ordinal),(_visited,evidence) in found.items():
            tf,dl=chunks[(ref,ordinal)]; shared=keys&tf.keys()
            score=sum(math.log(1+(N-df[key]+.5)/(df[key]+.5))*tf[key]*(k1+1)/(tf[key]+k1*(1-b+b*dl/avgdl)) for key in sorted(shared))
            item=(score,-ordinal,len(shared),evidence)
            if ref not in best or item[:2]>best[ref][:2]: best[ref]=item
        ordered=sorted(best.items(),key=lambda r:(-r[1][0],r[0],-r[1][1]))[:lexical.CHANNEL_LIMIT]
        return [(ref,-item[1],item[2],item[3]) for ref,item in ordered],coverage

    lexical.candidates=candidates
    original_upsert=worker.EventIndex.upsert
    def upsert(self,request):
        result=original_upsert(self,request)
        with (Path(root)/'chunks.jsonl').open('a') as stream:
            stream.write(json.dumps({'source':request['source'],'chunks':request['chunks']},ensure_ascii=False)+'\n')
        return result
    worker.EventIndex.upsert=upsert
    original_query=worker.EventIndex.query
    def query(self,request):
        result=original_query(self,request)
        with (Path(root)/'queries.jsonl').open('a') as stream:
            stream.write(json.dumps({'query':request['query'],'result':result},ensure_ascii=False)+'\n')
        return result
    worker.EventIndex.query=query
