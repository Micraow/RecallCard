"""Only synthetic sources/vectors. These test storage, not model quality."""
from dataclasses import replace
import math
import importlib.util
from pathlib import Path
import random
import sqlite3
import tempfile
import unittest
from recallcard_worker.bounded_index import BoundedIndex, EncodedChunk, IndexError, SourceState, digest, vector_bytes
import struct


def source(i, scope='personal', **kwargs):
    return SourceState(f'event:e{i}', 'event', scope, digest(f'synthetic text {i}'.encode()), **kwargs)


def chunk(s, i=0, vector=(1., 0., 0., 0.)):
    return EncodedChunk(s.ref, digest(f'input-{s.ref}-{i}'.encode()), i*10, i*10+9, tuple(vector))


class BoundedIndexTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name)/'vectors.sqlite'
        self.space = {'provider':'local-fixture','model':'synthetic','revision':'r1','dimensions':4,
                      'preprocessing':'utf8/chunk-v1','query_prefix':'q:','document_prefix':'d:','pooling':'mean','normalization':'l2'}
        self.index = BoundedIndex(self.path, self.space)

    def tearDown(self):
        self.tmp.cleanup()

    def ready(self, sources, chunks):
        generation = self.index.begin(sources, len(chunks))
        for start in range(0,len(chunks),64):
            self.index.append(generation, start, chunks[start:start+64])
        self.index.publish(generation)
        return generation

    def test_64_row_checkpoint_resume_idempotent_and_conflict(self):
        sources = [source(i) for i in range(65)]
        chunks = [chunk(s) for s in sources]
        g = self.index.begin(sources, 65)
        self.assertEqual(self.index.append(g,0,chunks[:64]),64)
        restarted = BoundedIndex(self.path,self.space)
        self.assertEqual(restarted.begin(sources,65),g)
        self.assertEqual(restarted.append(g,0,chunks[:64]),64)
        self.assertEqual(restarted.checkpoint(g)['completed'],64)
        with self.assertRaisesRegex(IndexError,'checkpoint_conflict'):
            restarted.append(g,0,[replace(chunks[0],vector=(0.,1.,0.,0.)),*chunks[1:64]])
        restarted.append(g,64,chunks[64:]);restarted.publish(g)
        self.assertEqual(restarted.query((1.,0.,0.,0.),sources,{'personal'})['coverage'],'complete')

    def test_failed_build_leaves_old_generation_readable(self):
        s=source(1);old=self.ready([s],[chunk(s)])
        newer=replace(s,content_hash=digest(b'new synthetic source'))
        g=self.index.begin([newer],1)
        with self.assertRaisesRegex(IndexError,'dimension_mismatch'):
            self.index.append(g,0,[chunk(newer,vector=(1.,0.))])
        self.assertEqual(self.index.query((1.,0.,0.,0.),[s],{'personal'})['generation'],old)
        self.assertEqual(self.index.checkpoint(g)['completed'],0)
        self.assertEqual(self.index.query((1.,0.,0.,0.),[newer],{'personal'})['results'],[])

    def test_publish_failure_rolls_back_pointer_and_status(self):
        s=source(1);old=self.ready([s],[chunk(s)])
        n=source(2);g=self.index.begin([n],1);self.index.append(g,0,[chunk(n)])
        with self.index.connection() as db:
            db.execute("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON published BEGIN SELECT RAISE(ABORT,'synthetic publish crash'); END")
        with self.assertRaises(sqlite3.IntegrityError):self.index.publish(g)
        self.assertEqual(self.index.query((1.,0.,0.,0.),[s],{'personal'})['generation'],old)
        self.assertEqual(self.index.checkpoint(g)['status'],'building')
        with self.index.connection() as db:db.execute('DROP TRIGGER synthetic_failure')
        self.index.publish(g)
        self.assertEqual(self.index.query((1.,0.,0.,0.),[n],{'personal'})['generation'],g)

    def test_build_does_not_hold_lock_while_external_inference_runs(self):
        s=source(1);old=self.ready([s],[chunk(s)])
        n=source(2);g=self.index.begin([n],1)
        # The encoder runs outside append; another connection can read or begin a transaction.
        other=BoundedIndex(self.path,self.space)
        with other.connection() as db:db.execute('BEGIN IMMEDIATE');db.rollback()
        self.assertEqual(other.query((1.,0.,0.,0.),[s],{'personal'})['generation'],old)
        self.index.append(g,0,[chunk(n)]);self.index.publish(g)

    def test_space_changes_get_separate_generation(self):
        s=source(1);g=self.ready([s],[chunk(s)])
        for changed in [{'revision':'r2'},{'dimensions':3},{'query_prefix':'other:'},{'preprocessing':'chunk-v2'},{'provider':'cloud-fixture'}]:
            other=BoundedIndex(self.path,{**self.space,**changed})
            self.assertNotEqual(other.signature,self.index.signature)
            self.assertEqual(other.query([1.]+[0.]*(other.dimensions-1),[s],{'personal'})['coverage'],'unavailable')
            with self.assertRaisesRegex(IndexError,'unknown_generation'):other.checkpoint(g)

    def test_paged_distinct_top_k_equals_exact_full_sort(self):
        rng=random.Random(731)
        sources=[source(i,session_ref=f'session:{i//3}') for i in range(400)]
        chunks=[chunk(s,j,tuple(rng.uniform(-1,1) for _ in range(4))) for s in sources for j in range(3)]
        self.ready(sources,chunks);q=(1.,0.,0.,0.)
        expected=[];used=set();groups={};ranked=[]
        for i,c in enumerate(chunks):
            values=struct.unpack('<ffff',vector_bytes(c.vector,4));ranked.append((values[0],-i,c.source_ref))
        by_ref={s.ref:s for s in sources}
        for score,neg,ref in sorted(ranked,reverse=True):
            group=by_ref[ref].session_ref
            if ref in used or groups.get(group,0)>=2:continue
            expected.append((ref,-neg));used.add(ref);groups[group]=groups.get(group,0)+1
            if len(expected)==25:break
        actual=self.index.query(q,sources,{'personal'},limit=25,max_chunks_per_source=1,per_session_limit=2)
        self.assertEqual([(r['ref'],r['ordinal']) for r in actual['results']],expected)
        self.assertEqual(actual['scanned_chunks'],1200)
        self.assertEqual(actual['page_rows'],256)

    def test_distant_facts_and_later_correction_are_not_silently_lost_by_quota(self):
        a=source(1,session_ref='topic')
        b=source(2,session_ref='topic',relations={'corrects':(a.ref,)},role='user')
        a=replace(a,relations={'next':(b.ref,)})
        chunks=[chunk(a,0,(1.,0.,0.,0.)),chunk(a,100,(.9,.1,0.,0.)),chunk(b,0,(.8,.2,0.,0.))]
        self.ready([a,b],chunks)
        full=self.index.query((1.,0.,0.,0.),[a,b],{'personal'},limit=3)
        self.assertEqual([r['range'] for r in full['results']],[[0,9],[1000,1009],[0,9]])
        # Diversity is opt-in and visibly lossy; preserve the bounded raw pool and links.
        diverse=self.index.query((1.,0.,0.,0.),[a,b],{'personal'},limit=3,max_chunks_per_source=1,
                                 per_session_limit=1,include_candidate_pool=True)
        self.assertEqual(len(diverse['results']),1)
        self.assertEqual(len(diverse['candidate_pool']),3)
        self.assertEqual(diverse['results'][0]['relations']['next'],[b.ref])
        self.assertEqual(diverse['candidate_pool'][-1]['relations']['corrects'],[a.ref])
        deeper=self.index.query((1.,0.,0.,0.),[a,b],{'personal'},limit=3,target_refs={a.ref})
        self.assertEqual([r['range'][0] for r in deeper['results']],[0,1000])
        hidden=self.index.query((1.,0.,0.,0.),[b],{'personal'},target_refs={a.ref})
        self.assertEqual(hidden['results'],[])

    def test_hidden_deleted_changed_and_cross_scope_sources_filter_immediately(self):
        a,b,c=source(1),source(2),source(3,'work')
        self.ready([a,b,c],[chunk(a),chunk(b),chunk(c)])
        self.assertEqual(len(self.index.query((1.,0.,0.,0.),[a,b,c],{'personal'})['results']),2)
        # Hidden/deleted refs are omitted by current canonical host snapshot.
        self.assertEqual([r['ref'] for r in self.index.query((1.,0.,0.,0.),[b,c],{'personal'})['results']],[b.ref])
        b2=replace(b,content_hash=digest(b'edited canonical text'))
        self.assertEqual(self.index.query((1.,0.,0.,0.),[b2,c],{'personal'})['results'],[])
        changed_scope=replace(a,scope='work')
        self.assertEqual(self.index.query((1.,0.,0.,0.),[changed_scope],{'personal','work'})['results'],[])

    def test_memory_requires_every_current_authorized_evidence_event(self):
        a,b=source(1),source(2,'work')
        m=SourceState('memory:m@1','memory','personal',digest(b'memory text'),{a.ref:a.content_hash,b.ref:b.content_hash})
        self.ready([a,b,m],[chunk(a),chunk(b),chunk(m)])
        self.assertEqual(self.index.query((1.,0.,0.,0.),[a,b,m],{'personal'},kind='memory')['results'],[])
        self.assertEqual(self.index.query((1.,0.,0.,0.),[a,b,m],{'personal','work'},kind='memory')['results'][0]['ref'],m.ref)
        self.assertEqual(self.index.query((1.,0.,0.,0.),[a,m],{'personal','work'},kind='memory')['results'],[])
        b2=replace(b,content_hash=digest(b'changed evidence'))
        self.assertEqual(self.index.query((1.,0.,0.,0.),[a,b2,m],{'personal','work'},kind='memory')['results'],[])

    def test_corrections_and_branch_links_come_from_current_canonical_state(self):
        a,b,w=source(1),source(2),source(3,'work')
        a=replace(a,relations={'next':(b.ref,w.ref),'branch_choices':(b.ref,)})
        b=replace(b,relations={'corrects':(a.ref,)},role='user',occurred_at='2026-01-02T00:00:00Z')
        self.ready([a,b,w],[chunk(a),chunk(b),chunk(w)])
        result=self.index.query((1.,0.,0.,0.),[a,b,w],{'personal'})
        self.assertEqual(result['results'][0]['relations']['next'],[b.ref])
        self.assertEqual(result['results'][1]['relations']['corrects'],[a.ref])
        self.assertEqual(result['results'][1]['role'],'user')
        self.assertEqual(self.index.query((1.,0.,0.,0.),[a,w],{'personal'})['results'][0]['relations']['next'],[])

    def test_resource_bounds_and_invalid_vectors_do_not_advance_cursor(self):
        s=source(1);g=self.index.begin([s],65)
        for chunks in [[chunk(s)]*65,[chunk(s,vector=(float('nan'),0.,0.,0.))],[chunk(s,vector=(0.,0.,0.,0.))]]:
            with self.assertRaises(IndexError):self.index.append(g,0,chunks)
        self.assertEqual(self.index.checkpoint(g)['completed'],0)
        with self.assertRaisesRegex(IndexError,'incomplete_generation'):self.index.publish(g)
        small=BoundedIndex(Path(self.tmp.name)/'small.sqlite',self.space,disk_cap=65536)
        with self.assertRaisesRegex(IndexError,'vector_disk_limit'):small.begin([s],10000)
        with self.assertRaisesRegex(IndexError,'duplicate_source'):self.index.begin([s,s],1)

    @unittest.skipUnless(importlib.util.find_spec('numpy'), 'optional NumPy backend is not installed')
    def test_numpy_page_ranking_matches_reference_and_filters_before_decode(self):
        rng=random.Random(94);sources=[source(i) for i in range(300)]
        chunks=[chunk(s,vector=tuple(rng.uniform(-1,1) for _ in range(4))) for s in sources]
        self.ready(sources,chunks);q=(.1,.4,.2,.9)
        a=self.index.query(q,sources,{'personal'},limit=31,backend='python')
        b=self.index.query(q,sources,{'personal'},limit=31,backend='numpy')
        self.assertEqual([r['ordinal'] for r in a['results']],[r['ordinal'] for r in b['results']])
        for x,y in zip(a['results'],b['results']):self.assertAlmostEqual(x['score'],y['score'],places=12)
        # A hidden corrupted vector must not reach either decoder or affect visible results.
        with self.index.connection() as db:db.execute('UPDATE chunks SET vector=? WHERE ordinal=0',(b'bad',))
        for backend in ['python','numpy']:
            self.assertTrue(self.index.query(q,sources[1:],{'personal'},backend=backend)['results'])
            with self.assertRaisesRegex(IndexError,'corrupt_vector_size'):
                self.index.query(q,sources,{'personal'},backend=backend)

    def test_disk_full_batch_rolls_back_and_preserves_published_index(self):
        bounded=BoundedIndex(Path(self.tmp.name)/'quota.sqlite',{**self.space,'dimensions':4096},disk_cap=131072)
        s=source(1);v=tuple([1.]+[0.]*4095);old=bounded.begin([s],1)
        bounded.append(old,0,[chunk(s,vector=v)]);bounded.publish(old)
        newer=source(2);g=bounded.begin([newer],6)
        with self.assertRaises(sqlite3.OperationalError):bounded.append(g,0,[chunk(newer,i,v) for i in range(6)])
        self.assertEqual(bounded.checkpoint(g)['completed'],0)
        self.assertEqual(bounded.query(v,[s],{'personal'})['generation'],old)

if __name__=='__main__':unittest.main()
