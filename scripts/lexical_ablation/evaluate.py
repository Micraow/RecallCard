#!/usr/bin/env python3
"""原创合成有界检索消融；无模型、网络、用户语料或生产写入。"""
from __future__ import annotations
import argparse
from collections import Counter, defaultdict, deque
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import sqlite3
import statistics

SEED = 'recallcard-synthetic-ngram-bm25-v1'
ASCII = re.compile(r'[A-Za-z0-9_./:+#-]+')
MODES = ('count_production', 'bm25_same', 'bm25_same_b0', 'bm25_han23', 'bm25_han12', 'bm25_han23_english_shingles')

def digest(raw): return hashlib.sha256(raw).hexdigest()
def canonical(obj): return json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
def han(c): return '\u3400' <= c <= '\u4dbf' or '\u4e00' <= c <= '\u9fff' or '\uf900' <= c <= '\ufaff' or '\U00020000' <= c <= '\U000323af'
def source_id(label): return 'event:' + digest((SEED + label).encode())[:24]

def fixture():
    docs, queries = [], []
    def doc(label, body, *, scope='personal', active=True):
        ident=source_id(label)
        docs.append(dict(id=ident,label=label,text=body,scope=scope,active=active,content_hash=digest(body.encode())))
        return ident
    def query(label, text, relevant=(), *, category, grades=None, expectation='source'):
        queries.append(dict(id=label,text=text,category=category,scope='personal',qrels={r:(grades or {}).get(r,1) for r in relevant},expectation=expectation))
    # 固定、不按结果调参的相邻领域背景。每条正文都是本文件原创。
    topics=('相机','灯架','背包','帐篷','雨衣','地图','电池','滤镜','三脚架','水杯')
    actions=('测试','检查','记录','采购','整理','维护','归还','装箱')
    for i in range(200):
        a=topics[i%len(topics)]; b=topics[(i*3+1)%len(topics)]; action=actions[(i//len(topics))%len(actions)]
        doc(f'background-{i}',f'项目记录 {a}与{b}的{action}工作已登记。今天讨论流程、时间和下一步安排。批次 BATCH-{i:04}。')
    for n,(subject,reason,query_text) in enumerate((
        ('相机冷靴','支架遮挡快门按钮','为什么换掉相机冷靴支架'),
        ('帐篷地钉','石质地面无法固定','帐篷地钉为何改成配重袋'),
        ('雨衣袖口','风吹时进水','雨衣袖口后来怎么调整'),
        ('地图图例','颜色太相近','地图图例为什么重画'),
        ('电池收纳','避免端子接触','电池收纳方案的原因'),
        ('滤镜包装','运输时边缘受压','滤镜包装后来为何加垫'),
    )):
        target=doc(f'zh-reason-{n}',f'{subject}方案已修改，原因是{reason}。调整后的版本保留在项目记录中。')
        doc(f'zh-distractor-{n}',f'{subject}方案评审正在准备，项目记录中仅登记了会议时间，没有修改原因。')
        query(f'zh-natural-{n}',query_text,[target],category='chinese_natural')
    # 稀有词与泛化长问句：idf与长度归一化各有可能改善或损害。
    for n,(rare,short,natural) in enumerate((
        ('铰链回弹','记录铰链回弹超过限值。','请找之前记录中铰链回弹的处理说明'),
        ('镜片镀膜','镜片镀膜有一道划痕。','之前项目里镜片镀膜出了什么问题'),
        ('防潮干燥剂','防潮干燥剂需要更换。','哪份记录说防潮干燥剂需要更换'),
        ('云台阻尼','云台阻尼在低温下增大。','项目测试记录里的云台阻尼有什么问题'),
    )):
        target=doc(f'rare-{n}',short)
        doc(f'rare-noise-{n}','请找之前记录项目处理说明测试问题。' * 7)
        query(f'rare-question-{n}',natural,[target],category='rare_term')
        query(f'rare-keyword-{n}',rare,[target],category='rare_term')
    # 同一关键词的短消息与长附件均为有效来源，长来源更完整；不人为调权。
    for n,key in enumerate(('TRIPOD-Q81','CASE-M42','FILTER-H63')):
        brief=doc(f'length-brief-{n}',f'{key} 检查完成。')
        detail=doc(f'length-detail-{n}',('整理路线和设备清单。' * 60)+f'{key} 检查结论是卡扣裂纹，需要更换锁止片。'+('其余设备检查正常。' * 60))
        query(f'length-key-{n}',key,[brief,detail],grades={brief:1,detail:2},category='length')
        query(f'length-detail-{n}',f'{key} 卡扣裂纹更换锁止片',[detail],category='length')
    # 英文word bigram只在现成ASCII token间组词，不冒充中文分词器。
    for n,(phrase,inverse) in enumerate((('camera mount','mount camera'),('battery case','case battery'),('tripod plate','plate tripod'),('lens hood','hood lens'))):
        target=doc(f'en-phrase-{n}',f'The {phrase} broke during inspection and was replaced.')
        doc(f'en-inverse-{n}',f'The {inverse} broke during inspection and was replaced.')
        query(f'en-phrase-{n}',phrase,[target],category='english_order')
        query(f'en-natural-{n}',f'why was the {phrase} replaced',[target],category='english_natural')
    # 编号、大小写、路径与C++边界：禁止字符gram跨入ASCII token。
    for n,(exact,near) in enumerate((('E_PIPE_71','E_PIPE_710'),('ZXQ-714','ZXQ-7140'),('v2.7.1','v2.7.10'),('/opt/maps/v2','/opt/maps/v20'),('C++','C'),('ABc_42','abc_42'))):
        target=doc(f'identifier-{n}',f'合成登记项 {exact} 已确认。')
        doc(f'identifier-near-{n}',f'合成登记项 {near} 已确认。')
        doc(f'identifier-prefix-{n}',f'合成登记项 pre{exact} 已确认。')
        query(f'identifier-{n}',exact,[target],category='exact_identifier')
    # 中文顺序、单字与错字：记录适用边界，不声称查到就是事实。
    target=doc('zh-order','蓝鲸观测小组完成记录。')
    doc('zh-order-reverse','鲸蓝观测小组完成记录。')
    query('zh-order','蓝鲸观测',[target],category='chinese_order')
    alone=doc('zh-single','独立标记 风 已核对。')
    embedded=doc('zh-single-embedded','风速测量已完成。')
    query('zh-single','风',[alone],category='single_han')
    typo=doc('typo-source','便携显示器支架的铰链需要润滑。')
    query('typo-query','便携显示器支驾铰链',[typo],category='typo')
    semantic=doc('semantic-gap','周末的露营因为暴雨取消了。')
    query('semantic-gap','户外过夜计划因恶劣天气作废',[semantic],category='paraphrase_gap')
    old=doc('decision-old','旧方案决定周六检查光圈控制器，记录已失效。')
    new=doc('decision-new','现在改为周日检查光圈控制器，周六的旧安排取消。')
    query('correction-current','现在光圈控制器哪天检查',[new],category='correction')
    query('negation','不要使用哪个光圈控制器方案',[old,new],category='negation_evidence')
    doc('no-answer-topic','轨迹仪只讨论过安装方式，尚未记录价格。')
    negatives=(('negative-unseen','火山岩质谱校准参数'),('negative-common','轨迹仪多少钱'),('negative-id','E_PIPE_7100'),('negative-case','aBc_42'),('negative-path','/opt/maps/v200'),('negative-semantic','有人同意支付定金了吗'),('negative-order','蓝观鲸测'),('negative-emoji','🦄'))
    for label,text in negatives: query(label,text,category='negative',expectation='no_answer')
    # 索引存在，但无可读来源。df/dl也必须先经scope及撤销过滤。
    hidden=doc('hidden','PRIVATE-SYNTHETIC-918 不可见材料',scope='project:hidden')
    revoked=doc('revoked','REVOKED-SYNTHETIC-729 已撤销材料',active=False)
    query('scope-hidden','PRIVATE-SYNTHETIC-918',category='scope_negative',expectation='no_answer')
    query('scope-revoked','REVOKED-SYNTHETIC-729',category='scope_negative',expectation='no_answer')
    # 不能截短长问题来避免预算；单独报告完整query受限比例。
    long=doc('long-question-target','海边露营设备名单在最后一次整理中记录，后续调整以新版清单为准。')
    query('query-budget','请帮我找海边露营设备名单在最后一次整理后具体改动以及相关原始记录',[long],category='query_budget')
    return dict(schema='recallcard.synthetic-ngram-bm25-fixture/1',seed=SEED,documents=docs,queries=queries,private_data=False)

def tokenize(text, mode, lexical):
    offsets=[0]
    for c in text: offsets.append(offsets[-1]+len(c.encode()))
    found=[]
    def add(kind,a,b,normalized=None):
        if len(text[a:b].encode())>128: return
        # same与生产完全相同的version/hash；新策略使用独立实验version。
        version=lexical.VERSION if kind in ('ascii','han1','han2') else 'synthetic-ngram-eval/1'
        key=digest(canonical([version,kind,normalized if normalized is not None else text[a:b]]))
        found.append((key,offsets[a],offsets[b],digest(text[a:b].encode())))
    ascii_tokens=[]
    for m in ASCII.finditer(text):
        if not any(c.isascii() and c.isalnum() for c in m[0]): continue
        add('ascii',*m.span()); ascii_tokens.append(m)
    if mode=='bm25_han23_english_shingles':
        for a,b in zip(ascii_tokens,ascii_tokens[1:]):
            if a[0].isalpha() and b[0].isalpha() and text[a.end():b.start()].isspace():
                add('ascii_words2',a.start(),b.end(),a[0]+' '+b[0])
    sizes=(2,3) if mode in ('bm25_han23','bm25_han23_english_shingles') else (1,2) if mode=='bm25_han12' else (2,)
    i=0
    while i<len(text):
        if not han(text[i]): i+=1; continue
        j=i+1
        while j<len(text) and han(text[j]): j+=1
        if j-i==1: add('han1',i,j)
        else:
            for size in sizes:
                for at in range(i,j-size+1): add('han'+str(size),at,at+size)
        i=j
    if mode in ('count_production','bm25_same','bm25_same_b0'):
        earliest={}
        for row in found:
            earliest[row[0]]=min(earliest.get(row[0],row[1:]),row[1:])
        expected=lexical.terms(text)
        assert sorted((key,*span) for key,span in earliest.items())==expected, 'tokenizer drift'
    return found

class Index:
    def __init__(self, docs, mode, lexical, scope='personal'):
        self.mode=mode; self.lexical=lexical
        self.docs={d['id']:d for d in docs if d['scope']==scope and d['active']}
        self.tokens={k:tokenize(d['text'],mode,lexical) for k,d in self.docs.items()}
        self.tf={k:Counter(t[0] for t in rows) for k,rows in self.tokens.items()}
        self.dl={k:sum(tf.values()) for k,tf in self.tf.items()}
        self.avgdl=statistics.mean(self.dl.values())
        self.postings=defaultdict(list); self.evidence={}
        for ident,rows in self.tokens.items():
            for key,start,end,sha in rows:
                old=self.evidence.get((key,ident))
                self.evidence[(key,ident)]=min(old,(start,end,sha)) if old else (start,end,sha)
            for key in self.tf[ident]: self.postings[key].append(ident)
        for rows in self.postings.values(): rows.sort()
        self.df={key:len(ids) for key,ids in self.postings.items()}
        self.db=None
        if mode=='count_production':
            self.db=sqlite3.connect(':memory:')
            self.db.execute('CREATE TABLE sources (generation TEXT, ref TEXT, scope TEXT)')
            lexical.initialize(self.db)
            self.db.execute('INSERT INTO lexical_generations(generation,version) VALUES (?,?)',('fixture',lexical.VERSION))
            for ident,d in self.docs.items():
                self.db.execute('INSERT INTO sources VALUES (?,?,?)',('fixture',ident,scope))
                lexical.insert_chunk(self.db,'fixture',ident,0,lexical.terms(d['text']))
            self.scope=scope
    def search(self, query, cap=True):
        if any(len(m[0].encode())>128 and any(c.isalnum() for c in m[0]) for m in ASCII.finditer(query)):
            return [],dict(status='limited_query',reason='lexical_query_term_length_limit',terms=0,postings=0,candidates=0,truncated=False)
        rows=tokenize(query,self.mode,self.lexical)
        bykey={}
        for key,*span in rows: bykey[key]=min(bykey.get(key,tuple(span)),tuple(span))
        keys=sorted(bykey)
        if cap and len(keys)>32: return [],dict(status='limited_query',terms=len(keys),postings=0,candidates=0,truncated=False)
        if self.mode=='count_production':
            original=32
            try:
                # cap=False is an explicitly labeled ideal-ranking diagnostic, not production.
                if not cap: self.lexical.MAX_QUERY_TERMS=max(len(keys),32)
                q=self.lexical.terms(query,query=True)
            finally: self.lexical.MAX_QUERY_TERMS=original
            result,cov=self.lexical.candidates(self.db,'fixture',self.scope,q)
            ranked=[(ident,float(count),evidence) for ident,ordinal,count,evidence in result]
            return self.verify(ranked),dict(status=cov['status'],terms=len(keys),postings=cov['postings_scanned'],candidates=cov['candidate_chunks'],truncated=cov['candidates_truncated'])
        # 同生产候选遍历预算；df与dl来自全部可见来源，而非截断窗口。
        active=deque((key,iter(self.postings.get(key,()))) for key in keys)
        candidates=set(); scanned=0; truncated=False; matches=defaultdict(set)
        while active and scanned<8192:
            key,cursor=active.popleft()
            ident=next(cursor,None)
            if ident is None: continue
            scanned+=1
            if len(candidates)<1024 or ident in candidates:
                candidates.add(ident); matches[ident].add(key)
            else: truncated=True
            active.append((key,cursor))
        truncated |= bool(active)
        N=len(self.docs); b=0 if self.mode=='bm25_same_b0' else .75; k1=1.2
        ranked=[]
        for ident in candidates:
            # 对已访问候选计算全部query term贡献，而非只计轮询偶然访问的词。
            shared=set(keys)&self.tf[ident].keys()
            score=0
            for key in sorted(shared):
                tf=self.tf[ident][key]
                idf=math.log(1+(N-self.df[key]+.5)/(self.df[key]+.5))
                score+=idf*tf*(k1+1)/(tf+k1*(1-b+b*self.dl[ident]/self.avgdl))
            evidence=min(self.evidence[(key,ident)] for key in shared)
            ranked.append((ident,score,evidence))
        ranked.sort(key=lambda r:(-r[1],r[0]))
        truncated |= len(ranked)>100
        return self.verify(ranked[:100]),dict(status='available' if keys else 'unsupported_query',terms=len(keys),postings=scanned,candidates=len(candidates),truncated=truncated)
    def verify(self,ranked):
        output=[]
        for ident,score,ev in ranked:
            d=self.docs[ident]; raw=d['text'].encode(); start,end,sha=ev
            assert 0<=start<end<=len(raw) and digest(raw[start:end])==sha
            raw[start:end].decode(); assert digest(raw)==d['content_hash']
            output.append(dict(id=ident,score=score,evidence=dict(start_byte=start,end_byte=end,text_sha256=sha),content_hash=d['content_hash'],scope=d['scope']))
        return output

def metrics(q,ranked):
    ids=[r['id'] for r in ranked]; rel=q['qrels']; positive=bool(rel)
    if not positive: return dict(any_candidate=bool(ids),candidate_count=len(ids))
    ranks=[i+1 for i,key in enumerate(ids) if key in rel]
    dcg=sum((2**rel.get(key,0)-1)/math.log2(i+2) for i,key in enumerate(ids[:5]))
    ideal=sum((2**grade-1)/math.log2(i+2) for i,grade in enumerate(sorted(rel.values(),reverse=True)[:5]))
    return dict(hit1=any(r==1 for r in ranks),hit5=any(r<=5 for r in ranks),recall5=sum(r<=5 for r in ranks)/len(rel),mrr10=1/min(ranks) if ranks and min(ranks)<=10 else 0,ndcg5=dcg/ideal)

def summarize(rows):
    positives=[r['metrics'] for r in rows if r['positive']]; negatives=[r['metrics'] for r in rows if not r['positive']]
    out=dict(queries=len(rows),positive=len(positives),negative=len(negatives),limited=sum(r['coverage']['status']=='limited_query' for r in rows),truncated=sum(r['coverage']['truncated'] for r in rows))
    if positives: out.update({key:round(statistics.mean(r[key] for r in positives),6) for key in ('hit1','hit5','recall5','mrr10','ndcg5')})
    if negatives: out.update(negative_any_candidate=sum(r['any_candidate'] for r in negatives),negative_total=len(negatives))
    anchored=[r['anchor_complete_top1'] for r in positives if 'anchor_complete_top1' in r]
    if anchored: out.update(anchored_positive_queries=len(anchored),anchor_complete_top1=sum(anchored))
    return out

def run(repo,out):
    source=repo/'python/recallcard_worker/event_lexical.py'
    spec=importlib.util.spec_from_file_location('production_event_lexical',source); lexical=importlib.util.module_from_spec(spec); spec.loader.exec_module(lexical)
    dataset=fixture(); (out/'fixture.json').write_bytes(canonical(dataset)+b'\n')
    results={}; safety=[]
    for mode in MODES:
        index=Index(dataset['documents'],mode,lexical); detail=[]
        for cap in (True,False):
            for q in dataset['queries']:
                ranked,cov=index.search(q['text'],cap)
                measured=metrics(q,ranked)
                anchors={m[0] for m in ASCII.finditer(q['text']) if any(c.isalnum() for c in m[0]) and not m[0].isalpha()}
                if q['qrels'] and anchors:
                    top_terms={m[0] for m in ASCII.finditer(index.docs[ranked[0]['id']]['text'])} if ranked else set()
                    measured['anchor_complete_top1']=anchors <= top_terms
                detail.append(dict(query=q['id'],category=q['category'],positive=bool(q['qrels']),budget='production_32' if cap else 'ideal_query_terms',coverage=cov,metrics=measured,top10=[dict(r,label=index.docs[r['id']]['label'],source_bytes=len(index.docs[r['id']]['text'].encode()),source_terms=index.dl[r['id']]) for r in ranked[:10]]))
        # 范围之外与撤销资料不得影响候选或可见语料idf；重复构建核对。
        changed=dataset['documents']+[dict(id=source_id('extra-hidden'+str(i)),label='hidden-stat',text='相机冷靴支架 '+str(i),scope='project:hidden',active=True,content_hash=digest(('相机冷靴支架 '+str(i)).encode())) for i in range(40)]
        extended=Index(changed,mode,lexical)
        for q in dataset['queries']:
            a=index.search(q['text'])[0]; b=extended.search(q['text'])[0]
            assert a==b, 'hidden corpus changed visible scores'
            if q['category'] in ('scope_negative',): assert not a
            if q['category']=='exact_identifier': assert a and {r['id'] for r in a}==set(q['qrels']), 'ASCII identifier boundary regression'
        safety.append(dict(mode=mode,scope_invariance=True,revocation_filter=True,exact_identifier_neighbors=True,source_range_hash=True))
        summary={}
        for budget in ('production_32','ideal_query_terms'):
            rows=[r for r in detail if r['budget']==budget]
            summary[budget]=dict(overall=summarize(rows),categories={cat:summarize([r for r in rows if r['category']==cat]) for cat in sorted({r['category'] for r in rows})})
        length_details=[dict(query=r['query'],metrics=r['metrics'],top3=[{key:hit[key] for key in ('label','source_bytes','source_terms','score')} for hit in r['top10'][:3]]) for r in detail if r['category']=='length' and r['budget']=='production_32']
        results[mode]=dict(summary=summary,length_details=length_details,stats=dict(visible_documents=len(index.docs),unique_postings=sum(index.df.values()),term_occurrences=sum(index.dl.values()),vocabulary=len(index.df)),details=detail)
    report=dict(schema='recallcard.synthetic-ngram-bm25-evaluation/1',fixture_sha256=digest((out/'fixture.json').read_bytes()),evaluator_sha256=digest(Path(__file__).read_bytes()),production_lexical_sha256=digest(source.read_bytes()),source_file='python/recallcard_worker/event_lexical.py',quality_claim='single_lexical_channel_known_original_synthetic_only_not_final_recall_or_real_blind_quality',models_downloaded=0,model_inference_calls=0,documents=len(dataset['documents']),queries=len(dataset['queries']),settings=dict(k1=1.2,b=.75,max_query_terms=32,max_postings=8192,max_candidates=1024,max_results=100,word_ngrams='English ASCII adjacent words only; no installed Chinese segmenter'),safety=safety,results=results)
    (out/'results.json').write_bytes(canonical(report)+b'\n')
    print(json.dumps({mode:{'bounded':r['summary']['production_32']['overall'],'ideal':r['summary']['ideal_query_terms']['overall'],'stats':r['stats']} for mode,r in results.items()},ensure_ascii=False,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--repo',type=Path,required=True); parser.add_argument('--out',type=Path,required=True); args=parser.parse_args(); args.out.mkdir(parents=True,exist_ok=True); run(args.repo,args.out)
