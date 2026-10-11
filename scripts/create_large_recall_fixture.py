#!/usr/bin/env python3
"""Deterministic synthetic corpus, through the real CLI; never uses personal inputs.
Ground-truth file is evaluator-only and must not enter the operator/model context.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess

SEED = 20261011
SCOPE = 'project:large-recall-ux'
NAMES = ['Atlas', 'AtlasLab', 'Cedar', 'CedarNotes', 'Boreal', 'Borealis', 'Delta', 'DeltaDocs', 'Ember', 'EmberUI', 'Harbor', 'Harbour', 'Lumen', 'LumenLab', 'Maple', 'MapleRead', 'Nimbus', 'NimbusKit', 'Orion', 'OrionNotes', 'Astronomy', 'Geometry', 'History', 'Languages', 'Music', 'Botany', 'Literature', 'Reading', 'Garden', 'TravelPlans']
TOPICS = ['overview', 'goals', 'architecture', 'delivery', 'validation', 'schedule', 'constraints', 'references', 'progress', 'next']


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--source-commit', required=True)
    args = p.parse_args()
    root = args.output.resolve()
    root.mkdir(exist_ok=False)
    binary = args.binary.resolve(strict=True)
    vault = root / 'vault'
    env = dict(os.environ, RECALLCARD_STATE_DIR=str(root / 'state'))
    def cli(name, *arguments):
        process = subprocess.run([str(binary), '--vault', str(vault), '--json', *arguments], env=env, capture_output=True)
        (root / (name + '.stdout')).write_bytes(process.stdout)
        (root / (name + '.stderr')).write_bytes(process.stderr)
        if process.returncode:
            raise RuntimeError(name + ': ' + process.stderr.decode()[:400])
        result = json.loads(process.stdout)
        return result.get('result', result)
    cli('setup', 'setup')
    rng = random.Random(SEED)
    events = []
    for session in range(1000):
        project = session % len(NAMES)
        for turn in range(12):
            identifier = f's{session:04}-t{turn:02}'
            role = 'assistant' if turn == 10 else 'user'
            content = f'合成 {NAMES[project]} 第{session//30+1}次讨论：{TOPICS[turn%10]}主题。记录编号{rng.randrange(10000,99999)}。当前只在本地验证，先核对输入样本，再记录结果；未完成的实验不说成已完成。'
            if role == 'assistant':
                content = f'合成助手对 {NAMES[project]} 的备选建议：可以用云端服务；只是建议，用户没有采纳。'
            events.append({'scope':SCOPE if session<990 else 'project:isolated-large','role':role,'origin':'native','occurred_at':f'2026-{8+session//400:02}-{1+session%27:02}T10:{turn:02}:00Z','content':content,'source':{'platform':'synthetic-large','conversation_id':f'session-{session:04}','message_id':identifier}})
    by_name = {e['source']['message_id']:e for e in events}
    by_name['s0001-t02']['content'] = 'AtlasLab 旧决定：默认导出格式用 CSV，暂不做 HTML。注意 Atlas 是另一个项目。'
    by_name['s0001-t02']['occurred_at'] = '2026-09-01T09:00:00Z'
    padding = ''.join(f'评审附件段落{i:03}：检查不同纸张的分页、图注及中文标点；这一段仅记录样式检查过程，不确定任何新的产品决策。\n' for i in range(100))
    by_name['s0031-t02']['content'] = '以下是合成项目评审的长原文。\n' + padding + '\n最终用户改口：AtlasLab 现在默认导出 HTML，CSV 只作为兼容选项保留；不接云端转换服务。九月一日默认 CSV 的决定已过时。'
    by_name['s0031-t02']['occurred_at'] = '2026-10-10T16:00:00Z'
    by_name['s0000-t02']['content'] = 'Atlas 当前默认导出 PDF，不是 AtlasLab；两个项目的导出规则不要合并。'
    by_name['s0001-t10']['content'] = '助手对 AtlasLab 的建议：默认用云端 PDF 转换，用户没有批准，也没有实施。'
    by_name['s0979-t11']['content'] = '合成未整理安排，代号珊瑚校样：下周四下午两点半和周岚核对 AtlasLab 的 HTML 打印样式；不讨论 Atlas 的 PDF 导出。地点尚未确定。'
    by_name['s0979-t11']['occurred_at'] = '2026-10-11T08:00:00Z'
    for e in events:
        if e['scope'] != SCOPE: e['content'] = 'ISOLATED-LARGE-MARKER-4819 ' + e['content']
    for scope,label in [(SCOPE,'corpus'),('project:isolated-large','isolated')]:
        file=root/(label+'.jsonl');file.write_text(''.join(json.dumps(e,ensure_ascii=False)+'\n' for e in events if e['scope']==scope))
        lines=file.read_text().splitlines(True)
        for offset in range(0,len(lines),4000):
            chunk=root/f'{label}-part-{offset//4000:02}.jsonl';chunk.write_text(''.join(lines[offset:offset+4000]))
            cli(f'import-{label}-{offset//4000:02}','import','--format','manual-jsonl','--file',str(chunk),'--scope',scope)
    duplicate_added=0
    for chunk in sorted(root.glob('corpus-part-*.jsonl')):
        duplicate_added+=cli('duplicate-'+chunk.stem,'import','--format','manual-jsonl','--file',str(chunk),'--scope',SCOPE)['events_added']
    assert duplicate_added==0
    ids={}
    for file in (vault/'events').rglob('*.jsonl'):
        for line in file.read_text().splitlines():
            e=json.loads(line);ids[e['source']['message_id']]=e['id']
    inputs=root/'reviewed-memory-inputs';inputs.mkdir()
    for project,name in enumerate(NAMES):
        domain='work' if project<20 else 'learning' if project<27 else 'life'
        for topic in range(10):
            source=f's{project:04}-t{topic:02}'
            refs=[ids[source]]; content=by_name[source]['content']
            if project==1 and topic==2:
                refs.append(ids['s0031-t02']);content='AtlasLab 当前默认 HTML 导出，CSV 仅兼容保留；不使用云端转换。旧默认CSV已由用户明确替代，不能套用Atlas的PDF规则。'
            data={'scope':SCOPE,'content':content,'source_refs':refs,'evidence':'user_explicit','navigation':[{'path':f'{domain}/{name.lower()}/{TOPICS[topic]}','title':name+' '+TOPICS[topic],'description':content,'keywords':[name,TOPICS[topic]],'aliases':[],'related_paths':[]}]}
            file=inputs/f'{project:02}-{topic:02}.json';file.write_text(json.dumps(data,ensure_ascii=False));cli(f'memory-{project:02}-{topic:02}','memory','add','--file',str(file))
        if project%5==0: print(json.dumps({'memories_created':(project+1)*10}),flush=True)
    old={'scope':SCOPE,'content':'AtlasLab 默认导出CSV，暂不做HTML。','source_refs':[ids['s0001-t02']],'evidence':'user_explicit','navigation':[{'path':'work/atlaslab/architecture','title':'AtlasLab 旧导出决定','description':'旧CSV方案','keywords':[],'aliases':[],'related_paths':[]}]}
    file=inputs/'old-memory.json';file.write_text(json.dumps(old,ensure_ascii=False));m=cli('old-memory','memory','add','--file',str(file));cli('old-state','memory','state',m['id'],'superseded','--revision','1')
    grant=cli('grant','connect','codex','--host-identity','codex','--platform','codex','--recall-scope',SCOPE,'--provider-disclosure','--auto-recall')
    runtime={'binary':str(binary),'vault':str(vault),'environment':{'RECALLCARD_STATE_DIR':str(root/'state')},'source_commit':args.source_commit}
    (root/'runtime.json').write_text(json.dumps(runtime))
    questions=['AtlasLab 现在的默认导出格式决定是什么？不要混成 Atlas，旧方案和没采纳的建议要分清。','请把 AtlasLab 默认导出格式的旧决定和最新改口定位到原话，给时间和完整关键句；如果内容长，就按需定位或续读。','我有个还没整理的新安排，叫“珊瑚校样”；讨论什么、跟谁、什么时候？如果只是相对日期请保留不确定项。']
    (root/'followups.json').write_text(json.dumps(questions,ensure_ascii=False))
    summary={'seed':SEED,'synthetic_only':True,'sessions':1000,'events':len(events),'visible_events':sum(e['scope']==SCOPE for e in events),'active_memories':300,'superseded_memories':1,'projects':30,'scope':SCOPE,'connection_id':grant['id'],'duplicate_added':duplicate_added,'corpus_sha256':hashlib.sha256((root/'corpus.jsonl').read_bytes()).hexdigest(),'corpus_bytes':(root/'corpus.jsonl').stat().st_size,'long_event_bytes':len(by_name['s0031-t02']['content'].encode()),'fixture_kind':'reviewed synthetic memories via production CLI; no model generation claimed'}
    (root/'corpus-summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2));print(json.dumps(summary),flush=True)
    (root/'EVALUATOR-ONLY.json').write_text(json.dumps({'never_send_to_model':True,'old_ref':'event:'+ids['s0001-t02'],'new_ref':'event:'+ids['s0031-t02'],'raw_arrangement_ref':'event:'+ids['s0979-t11'],'old_memory_id':m['id'],'expected_current':'HTML default, CSV compatibility, no cloud conversion; Atlas PDF is unrelated','expected_arrangement':'Zhou Lan, AtlasLab HTML print style, next Thursday 14:30, location unspecified, no timezone asserted'},ensure_ascii=False,indent=2))


if __name__=='__main__':main()
