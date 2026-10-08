#!/usr/bin/env python3
"""打包已构建的 Linux 桌面预览版；只收集程序、合成示例、文档与许可证。"""
import argparse
import hashlib
import json
import re
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def resolved_packages(paths):
    """GUI与独立CLI来自不同锁文件，许可清单必须取两个已解析图的精确并集。"""
    packages = {}
    for path in paths:
        metadata = json.loads(path.read_text())
        ids = {node['id'] for node in metadata['resolve']['nodes']}
        found = {package['id']: package for package in metadata['packages'] if package['id'] in ids}
        if set(found) != ids:
            raise ValueError('依赖元数据缺少已解析包')
        for identifier, package in found.items():
            if identifier in packages and packages[identifier] != package:
                raise ValueError('同一依赖ID对应不同元数据')
            packages[identifier] = package
    return sorted(packages.values(), key=lambda item: (item['name'], item['version'], item['id']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--metadata', type=Path, action='append', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--packaging-commit')
    parser.add_argument('--acceptance-commit')
    parser.add_argument('--acceptance-run-id', type=int)
    parser.add_argument('--packaging-run-id', type=int)
    parser.add_argument('--gui-sha256')
    parser.add_argument('--cli-sha256')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--provenance', type=Path)
    parser.add_argument('--profile', choices=['dev','release'], default='dev')
    args = parser.parse_args()
    for commit in [args.commit, args.packaging_commit, args.acceptance_commit]:
        if commit is not None and not re.fullmatch(r'[a-f0-9]{40}', commit):
            parser.error('commit 必须是完整提交摘要')
    if bool(args.acceptance_commit) != bool(args.acceptance_run_id) or (args.acceptance_run_id is not None and args.acceptance_run_id <= 0):
        parser.error('验收提交和正整数运行ID必须同时提供')
    if bool(args.gui_sha256) != bool(args.cli_sha256):
        parser.error('GUI和CLI摘要必须同时提供')
    for path, expected in [(args.binary, args.gui_sha256), (args.cli, args.cli_sha256)]:
        if expected is not None and (not re.fullmatch(r'[a-f0-9]{64}', expected) or digest(path.read_bytes()) != expected):
            parser.error('程序摘要与已验收成品不符')
    provenance = json.loads(args.provenance.read_text()) if args.provenance else None
    if provenance is not None:
        if (provenance['application_commit'] != args.commit or provenance['profile'] != args.profile
                or provenance['binaries'] != {'recallcard-desktop': args.gui_sha256, 'recallcard': args.cli_sha256}
                or provenance['acceptance']['commit'] != args.acceptance_commit
                or provenance['acceptance']['run_id'] != args.acceptance_run_id):
            parser.error('成品来源与命令行身份不一致')
    version=json.loads((ROOT/'desktop/src-tauri/tauri.conf.json').read_text())['version']
    if provenance is not None:
        if provenance.get('application_version', version) != version:
            parser.error('包装版本与已验收程序来源不符')
        if version.startswith('0.6.') and (provenance.get('application_version') != version
                or provenance.get('acceptance', {}).get('suite') != 'v0.6-first-use-native'):
            parser.error('v0.6运行包必须绑定明确版本与新版原生验收合同')
    guide_name='desktop-quickstart-v0.6.md' if version.startswith('0.6.') else 'desktop-quickstart-v0.5.md'
    guide = (ROOT/'docs'/guide_name).read_text()
    guide = re.sub(r'\]\(([^)]+)\)', lambda match: match[0] if re.match(r'(?:[a-zA-Z][a-zA-Z0-9+.-]*:|/|#)', match[1]) else f'](docs/{match[1]})', guide)
    files = {
        'recallcard-desktop': (args.binary.read_bytes(), True),
        'recallcard': (args.cli.read_bytes(), True),
        '开始使用-中文.md': (guide.encode(), False),
        '运行桌面版.sh': (b'#!/bin/sh\nset -eu\nAPP_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\nexec "$APP_DIR/recallcard-desktop"\n', True),
    }
    for directory in ['docs', 'fixtures']:
        for path in (ROOT/directory).rglob('*'):
            if path.is_file() and path.suffix in {'.md', '.json', '.jsonl'}:
                files[path.relative_to(ROOT).as_posix()] = (path.read_bytes(), False)
    for path in (ROOT/'extension').iterdir():
        if path.is_file() and (path.suffix in {'.js','.html','.css'} or path.name == 'manifest.json'):
            files['extension/'+path.name] = (path.read_bytes(), False)
    # 可选后台整理的公开 Python 代码随读取组件分发，不包含任何配置或凭据。
    for module in ['recallcard_dream', 'recallcard_worker']:
        directory=ROOT/'python'/module
        for path in directory.rglob('*.py'):
            if path.is_symlink(): raise ValueError('Python 资源不能为符号链接')
            files[path.relative_to(ROOT).as_posix()] = (path.read_bytes(), False)
    # React 实际进入嵌入页面，必须与 Rust 依赖一起保留许可证正文。
    frontend_manifest=ROOT/'desktop/package.json'
    if frontend_manifest.exists():
        for name in ['react', 'react-dom', 'scheduler']:
            directory=ROOT/'desktop/node_modules'/name
            manifest=json.loads((directory/'package.json').read_text())
            license_file=directory/'LICENSE'
            files[f"third-party-licenses/npm-{name}-{manifest['version']}/LICENSE"]=(license_file.read_bytes(),False)
    notices = []
    packages = resolved_packages(args.metadata)
    for package in packages:
        if package.get('source') is None:
            continue
        directory = Path(package['manifest_path']).parent
        paths = {p for pattern in ('LICENSE*','LICENCE*','COPYING*','NOTICE*') for p in directory.rglob(pattern) if p.is_file()}
        if package.get('license_file'):
            paths.add(directory/package['license_file'])
        override = ROOT/'desktop/third-party-licenses'/f"{package['name']}-{package['version']}"
        if not paths and override.is_dir():
            paths = {p for pattern in ('LICENSE*','LICENCE*','COPYING*','NOTICE*') for p in override.rglob(pattern) if p.is_file()}
        if not paths:
            raise ValueError(f"依赖缺少许可证正文：{package['name']}")
        included = set()
        for path in sorted(paths):
            if not (path.resolve().is_relative_to(directory.resolve()) or path.resolve().is_relative_to(override.resolve())):
                raise ValueError('许可证路径超出依赖目录')
            base = directory if path.resolve().is_relative_to(directory.resolve()) else override
            relative = path.resolve().relative_to(base.resolve()).as_posix()
            included.add(relative)
            files[f"third-party-licenses/{package['name']}-{package['version']}/{relative}"] = (path.read_bytes(), False)
        # MPL 文件的对应源码原样随包提供，不把其文件许可扩展到其他项目文件。
        if package.get('license') == 'MPL-2.0':
            for source in directory.rglob('*'):
                if source.is_file():
                    files[f"third-party-source/{package['name']}-{package['version']}/{source.relative_to(directory).as_posix()}"] = (source.read_bytes(), False)
        notices.append({'name':package['name'],'version':package['version'],'license':package.get('license'),'repository':package.get('repository'),'source_archive':f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download",'included_files':sorted(included)})
    files['third-party-licenses/补充来源说明.md'] = ((ROOT/'desktop/third-party-licenses/README.md').read_bytes(), False)
    files['third-party-licenses/index.json'] = (json.dumps(notices,ensure_ascii=False,indent=2).encode(),False)
    version=json.loads((ROOT/'desktop/src-tauri/tauri.conf.json').read_text())['version']
    versions=set()
    for binary in (args.binary,args.cli):
        symbols=subprocess.check_output(['readelf','--version-info',str(binary)],text=True)
        versions.update(re.findall(r'GLIBC_([0-9.]+)',symbols))
    minimum=max(versions,key=lambda s:tuple(map(int,s.split('.'))))
    info = {'application':f'RecallCard Desktop {version}','application_version':version,'commit':args.commit,'packaging_commit':args.packaging_commit or args.commit,'target':'linux-x86_64','profile':args.profile,'gui_sha256':digest(files['recallcard-desktop'][0]),'cli_sha256':digest(files['recallcard'][0]),'requires':['Git',f'GLIBC >= {minimum}','GTK 3','WebKitGTK 4.1'],'optional_requires':{'background_model':'Python 3.10+'},'note':'系统运行库由系统包管理器提供；本包不包含任何用户资料、凭据、浏览器状态或可选API配置。'}
    if args.acceptance_commit:
        info['native_acceptance'] = {'commit':args.acceptance_commit,'run_id':args.acceptance_run_id,'url':f'https://github.com/Micraow/RecallCard/actions/runs/{args.acceptance_run_id}'}
    if provenance is not None:
        info['validated_source'] = provenance
    info['dependency_metadata'] = {'graphs':len(args.metadata),'resolved_packages':len(packages),'third_party_packages':len(notices),'target':'x86_64-unknown-linux-gnu'}
    if args.packaging_run_id:
        info['packaging_run'] = {'run_id':args.packaging_run_id,'url':f'https://github.com/Micraow/RecallCard/actions/runs/{args.packaging_run_id}','gate':'仅在本ZIP安全解压后对应版本的完整原生流程通过且二次文件校验成功时保存；具体结论以该运行最终状态为准'}
    files['build-info.json'] = (json.dumps(info,ensure_ascii=False,indent=2).encode(),False)
    files['SHA256SUMS'] = (''.join(f'{digest(data)}  {name}\n' for name,(data,_) in sorted(files.items())).encode(),False)
    args.out.mkdir(parents=True,exist_ok=True)
    archive=args.out/f'RecallCard-Desktop-{version}-linux-x86_64-{args.commit[:7]}.zip'
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
