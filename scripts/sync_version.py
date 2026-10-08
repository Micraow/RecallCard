#!/usr/bin/env python3
"""根 VERSION 是唯一版本来源；同步清单/锁文件和前端常量，--check 只读核对。"""
import argparse,json,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]

def expected():
    version=(ROOT/'VERSION').read_text().strip()
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?',version):raise ValueError('VERSION 不是有效开发/发行版本')
    outputs={}
    for relative,section in [('Cargo.toml','workspace.package'),('desktop/src-tauri/Cargo.toml','package')]:
        p=ROOT/relative;s=p.read_text();pattern=r'(\['+re.escape(section)+r'\][\s\S]*?\nversion\s*=\s*)"[^"]+"'
        new,n=re.subn(pattern,lambda m:m[1]+'"'+version+'"',s,count=1)
        if n!=1:raise ValueError('没有找到版本字段：'+relative)
        outputs[p]=new
    for relative in ['Cargo.lock','desktop/src-tauri/Cargo.lock']:
        p=ROOT/relative;s=p.read_text()
        s=re.sub(r'(\[\[package\]\]\nname = "(?:recallcard|recallcard-desktop)"\nversion = ")[^"]+("\n)',lambda m:m[1]+version+m[2],s)
        outputs[p]=s
    for relative in ['desktop/src-tauri/tauri.conf.json','desktop/package.json','desktop/package-lock.json']:
        p=ROOT/relative;v=json.loads(p.read_text());v['version']=version
        if relative.endswith('package-lock.json'):v['packages']['']['version']=version
        outputs[p]=json.dumps(v,ensure_ascii=False,indent=2)+'\n'
    for relative in ['extension/package.json','extension/package-lock.json','extension/manifest.json']:
        p=ROOT/relative
        if not p.exists():continue
        v=json.loads(p.read_text());v['version']=version.split('-',1)[0] if relative.endswith('manifest.json') else version
        if relative.endswith('manifest.json'):v['version_name']=version
        if relative.endswith('package-lock.json'):v['packages']['']['version']=version
        outputs[p]=json.dumps(v,ensure_ascii=False,indent=2)+'\n'
    outputs[ROOT/'desktop/frontend/src/generated/version.ts']='// 由 scripts/sync_version.py 从根 VERSION 生成；不要手工修改。\nexport const VERSION = '+json.dumps(version)+' as const;\n'
    return outputs

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args()
    changed=[]
    for path,content in expected().items():
        if not path.exists() or path.read_text()!=content:
            changed.append(str(path.relative_to(ROOT)))
            if not args.check:path.parent.mkdir(parents=True,exist_ok=True);path.write_text(content)
    if changed and args.check:raise SystemExit('版本来源不一致：'+', '.join(changed)+'；请运行 python3 scripts/sync_version.py')
    print('版本一致' if not changed else '已同步：'+', '.join(changed))
if __name__=='__main__':main()
