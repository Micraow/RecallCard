#!/usr/bin/env python3
"""打包已构建的 Linux 桌面预览版；只收集程序、合成示例、文档与许可证。"""
import argparse
import hashlib
import json
import re
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--metadata', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-f0-9]{40}', args.commit):
        parser.error('commit 必须是完整提交摘要')
    files = {
        'recallcard-desktop': (args.binary.read_bytes(), True),
        'recallcard': (args.cli.read_bytes(), True),
        '开始使用-中文.md': ((ROOT/'docs/desktop-v0.1.md').read_bytes(), False),
        '运行桌面版.sh': (b'#!/bin/sh\nset -eu\nAPP_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\nexec "$APP_DIR/recallcard-desktop"\n', True),
    }
    for directory in ['docs', 'fixtures']:
        for path in (ROOT/directory).rglob('*'):
            if path.is_file() and path.suffix in {'.md', '.json', '.jsonl'}:
                files[path.relative_to(ROOT).as_posix()] = (path.read_bytes(), False)
    for path in (ROOT/'extension').iterdir():
        if path.is_file() and (path.suffix in {'.js','.html','.css'} or path.name == 'manifest.json'):
            files['extension/'+path.name] = (path.read_bytes(), False)
    metadata = json.loads(args.metadata.read_text())
    ids = {node['id'] for node in metadata['resolve']['nodes']}
    notices = []
    for package in metadata['packages']:
        if package.get('source') is None or package['id'] not in ids:
            continue
        directory = Path(package['manifest_path']).parent
        paths = {p for pattern in ('LICENSE*','LICENCE*','COPYING*','NOTICE*') for p in directory.glob(pattern) if p.is_file()}
        if package.get('license_file'):
            paths.add(directory/package['license_file'])
        override = ROOT/'desktop/third-party-licenses'/f"{package['name']}-{package['version']}"
        if not paths and override.is_dir():
            paths = {p for p in override.glob('LICENSE*') if p.is_file()}
        if not paths:
            raise ValueError(f"依赖缺少许可证正文：{package['name']}")
        for path in sorted(paths):
            if not (path.resolve().is_relative_to(directory.resolve()) or path.resolve().is_relative_to(override.resolve())):
                raise ValueError('许可证路径超出依赖目录')
            files[f"third-party-licenses/{package['name']}-{package['version']}/{path.name}"] = (path.read_bytes(), False)
        # MPL 文件的对应源码原样随包提供，不把其文件许可扩展到其他项目文件。
        if package.get('license') == 'MPL-2.0':
            for source in directory.rglob('*'):
                if source.is_file():
                    files[f"third-party-source/{package['name']}-{package['version']}/{source.relative_to(directory).as_posix()}"] = (source.read_bytes(), False)
        notices.append({'name':package['name'],'version':package['version'],'license':package.get('license'),'repository':package.get('repository'),'source_archive':f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download",'included_files':[p.name for p in sorted(paths)]})
    files['third-party-licenses/补充来源说明.md'] = ((ROOT/'desktop/third-party-licenses/README.md').read_bytes(), False)
    files['third-party-licenses/index.json'] = (json.dumps(notices,ensure_ascii=False,indent=2).encode(),False)
    info = {'application':'RecallCard Desktop 0.1.0','commit':args.commit,'target':'linux-x86_64','profile':'dev (debug=0, unoptimized)','gui_sha256':digest(files['recallcard-desktop'][0]),'cli_sha256':digest(files['recallcard'][0]),'requires':['Git','GLIBC >= 2.39','GTK 3','WebKitGTK 4.1'],'note':'系统运行库由系统包管理器提供；本包不包含任何用户资料、凭据、浏览器状态或可选API配置。'}
    files['build-info.json'] = (json.dumps(info,ensure_ascii=False,indent=2).encode(),False)
    files['SHA256SUMS'] = (''.join(f'{digest(data)}  {name}\n' for name,(data,_) in sorted(files.items())).encode(),False)
    args.out.mkdir(parents=True,exist_ok=True)
    archive=args.out/f'RecallCard-Desktop-0.1.0-linux-x86_64-{args.commit[:7]}.zip'
    with zipfile.ZipFile(archive,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=9) as z:
        for name,(data,executable) in sorted(files.items()):
            entry=zipfile.ZipInfo(name,(2026,1,1,0,0,0));entry.create_system=3
            entry.external_attr=(0o100755 if executable else 0o100644)<<16
            entry.compress_type=zipfile.ZIP_DEFLATED;z.writestr(entry,data)
    sums=args.out/(archive.stem+'-SHA256SUMS.txt')
    sums.write_text(f'{digest(archive.read_bytes())}  {archive.name}\n')
    print(json.dumps({'archive':str(archive),'checksums':str(sums),'bytes':archive.stat().st_size,'sha256':digest(archive.read_bytes()),'files':len(files),'dependency_notices':len(notices)},ensure_ascii=False))


if __name__ == '__main__':
    main()
