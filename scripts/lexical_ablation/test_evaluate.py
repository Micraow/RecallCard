"""合成评测器方法回归；不调用模型或更改生产配置。"""
import importlib.util
import math
import os
from pathlib import Path
import unittest
import evaluate as evaluation

REPO=Path(os.environ.get('RECALLCARD_EVAL_REPO',Path(__file__).resolve().parents[2]))
SPEC=importlib.util.spec_from_file_location('production_lexical',REPO/'python/recallcard_worker/event_lexical.py')
LEXICAL=importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(LEXICAL)

def doc(ident,text,scope='personal',active=True):
    return dict(id='event:'+ident,label=ident,text=text,scope=scope,active=active,content_hash=evaluation.digest(text.encode()))

class MethodTests(unittest.TestCase):
    def test_production_terms_retain_all_occurrences_for_bm25(self):
        text='编号 E_PIPE_71 编号 E_PIPE_71 正确'
        terms=evaluation.tokenize(text,'bm25_same',LEXICAL)
        counts=evaluation.Counter(t[0] for t in terms)
        self.assertEqual(max(counts.values()),2)
        self.assertEqual(set(counts),{t[0] for t in LEXICAL.terms(text)})

    def test_bm25_matches_hand_computation_with_corpus_df_and_repeated_tf(self):
        corpus=[doc('a','rare rare common'),doc('b','common'),doc('c','other other')]
        index=evaluation.Index(corpus,'bm25_same',LEXICAL)
        row=index.search('rare')[0][0]
        # N=3,df=1,tf=2,dl=3,avgdl=2,k1=1.2,b=.75。
        expected=math.log(1+(3-1+.5)/(1+.5))*2*2.2/(2+1.2*(1-.75+.75*3/2))
        self.assertAlmostEqual(row['score'],expected,places=12)
        doubled=index.search('rare rare')[0][0]
        self.assertEqual(row,doubled,'query repetitions must not multiply score')

    def test_b0_removes_length_normalization_without_changing_tf_or_idf(self):
        corpus=[doc('a','rare '+('common '*30)),doc('b','rare')]
        normal=evaluation.Index(corpus,'bm25_same',LEXICAL)
        flat=evaluation.Index(corpus,'bm25_same_b0',LEXICAL)
        scores={r['id']:r['score'] for r in flat.search('rare')[0]}
        self.assertEqual(scores['event:a'],scores['event:b'])
        self.assertEqual(normal.search('rare')[0][0]['id'],'event:b')

    def test_ngram_expansion_obeys_same_query_cap_without_truncating_query(self):
        text='甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未'
        corpus=[doc('a',text)]
        for mode,expected in [('count_production',17),('bm25_han23',33),('bm25_han12',35)]:
            index=evaluation.Index(corpus,mode,LEXICAL)
            result,coverage=index.search(text)
            self.assertEqual(coverage['terms'],expected)
            self.assertEqual(bool(result),expected<=32)
            self.assertTrue(index.search(text,cap=False)[0])

    def test_long_identifier_is_rejected_not_cut_into_short_query_tokens(self):
        for mode in evaluation.MODES:
            index=evaluation.Index([doc('a','short')],mode,LEXICAL)
            rows,cov=index.search('a'*129)
            self.assertEqual(rows,[])
            self.assertEqual(cov['reason'],'lexical_query_term_length_limit')

    def test_ascii_identifier_prefix_case_and_paths_stay_exact(self):
        cases=['E_PIPE_71','E_PIPE_710','e_pipe_71','/opt/a/v2','/opt/a/v20','C++','C']
        for mode in evaluation.MODES:
            index=evaluation.Index([doc(str(i),t) for i,t in enumerate(cases)],mode,LEXICAL)
            for i,text in enumerate(cases):
                self.assertEqual([r['id'] for r in index.search(text)[0]],['event:'+str(i)])

    def test_english_word_bigram_does_not_cross_punctuation_or_identifiers(self):
        def words(text):
            base={t[0] for t in evaluation.tokenize(text,'bm25_han23',LEXICAL)}
            full={t[0] for t in evaluation.tokenize(text,'bm25_han23_english_shingles',LEXICAL)}
            return full-base
        self.assertEqual(len(words('camera mount')),1)
        self.assertEqual(len(words('camera, mount')),0)
        self.assertEqual(len(words('E_PIPE_71 mount')),0)
        self.assertEqual(words('camera mount'),words('camera  mount'))
        self.assertEqual(words('camera mount'),words('camera\nmount'))
        index=evaluation.Index([doc('a','camera\nmount')],'bm25_han23_english_shingles',LEXICAL)
        self.assertTrue(index.search('camera mount')[0])

    def test_unauthorized_and_revoked_docs_never_change_visible_idf(self):
        visible=[doc('a','rare marker'),doc('b','other')]
        hidden=visible+[doc('secret','rare marker','project:hidden'),doc('revoked','rare marker',active=False)]
        for mode in evaluation.MODES:
            self.assertEqual(evaluation.Index(visible,mode,LEXICAL).search('rare'),evaluation.Index(hidden,mode,LEXICAL).search('rare'))

    def test_production_count_ties_are_stable_without_gold_dependent_doc_ids(self):
        corpus=[doc('b','marker'),doc('a','marker marker')]
        index=evaluation.Index(corpus,'count_production',LEXICAL)
        rows,cov=index.search('marker')
        self.assertEqual([r['id'] for r in rows],['event:a','event:b'])
        self.assertEqual([r['score'] for r in rows],[1.,1.])

    def test_bounded_candidates_keep_round_robin_rare_term_and_report_truncation(self):
        corpus=[doc(f'{i:04}','common marker') for i in range(1100)]+[doc('zzzz','rare-token')]
        for mode in ('count_production','bm25_same'):
            rows,cov=evaluation.Index(corpus,mode,LEXICAL).search('common rare-token')
            self.assertTrue(cov['truncated'])
            self.assertLessEqual(cov['postings'],8192)
            self.assertLessEqual(cov['candidates'],1024)
            # BM25带idf把稀有词排序提前；count可因最终top100并列窗口丢失它。
            if mode=='bm25_same': self.assertIn('event:zzzz',[r['id'] for r in rows])

    def test_candidate_evidence_rejects_changed_source_hash(self):
        corpus=[doc('a','中文 marker')]
        index=evaluation.Index(corpus,'bm25_han23',LEXICAL)
        index.docs['event:a']['content_hash']='0'*64
        with self.assertRaises(AssertionError): index.search('marker')

    def test_answerability_is_not_retrieval_precision(self):
        result=evaluation.metrics({'qrels':{}},[{'id':'event:topic-with-no-answer'}])
        self.assertEqual(result,{'any_candidate':True,'candidate_count':1})
        self.assertNotIn('incorrect_answer',result)

if __name__=='__main__': unittest.main()
