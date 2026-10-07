#!/usr/bin/env python3
"""打包已构建程序、扩展和中文说明；只收集明确白名单，不带任何 Vault/key。"""
import argparse
import hashlib
import json
import platform
import re
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def sha(data):
    return hashlib.sha256(data).hexdigest()

def write_zip(path, files):
    with zipfile.ZipFile(path, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, (data, executable) in sorted(files.items()):
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--metadata', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', args.target) or not re.fullmatch(r'[a-f0-9]{40}', args.commit):
        parser.error('target 或 commit 无效')
    binary = args.binary.resolve(strict=True)
    version = subprocess.check_output([str(binary), '--version'], text=True).strip()
    stem = f'RecallCard-{version.split()[-1]}-{args.target}-{args.commit[:7]}'
    args.out.mkdir(parents=True, exist_ok=True)
    files = {}
    executable_name = 'recallcard.exe' if args.target.startswith('windows') else 'recallcard'
    binary_bytes = binary.read_bytes()
    files[executable_name] = (binary_bytes, True)
    files['开始使用-中文.md'] = ((ROOT/'docs/quickstart-linux-v0.3.md').read_bytes(), False)
    for directory in ['docs', 'fixtures', 'integrations/claude-code']:
        for path in (ROOT/directory).rglob('*'):
            if path.is_file() and path.suffix in {'.md', '.json', '.jsonl'}:
                files[path.relative_to(ROOT).as_posix()] = (path.read_bytes(), False)
    extension = {}
    for path in (ROOT/'extension').iterdir():
        if path.is_file() and (path.suffix in {'.js', '.html', '.css'} or path.name == 'manifest.json'):
            extension[path.name] = (path.read_bytes(), False)
            files['extension/'+path.name] = extension[path.name]
    for package in ['recallcard_worker', 'recallcard_dream']:
        for path in (ROOT/'python'/package).glob('*.py'):
            files[path.relative_to(ROOT).as_posix()] = (path.read_bytes(), False)
    notices = []
    metadata = json.loads(args.metadata.read_text())
    resolved_ids = {node['id'] for node in metadata['resolve']['nodes']}
    for package in metadata['packages']:
        if package.get('source') is None or package['id'] not in resolved_ids:
            continue
        name = package['name']+'-'+package['version']
        directory = Path(package['manifest_path']).parent
        license_paths = {p for pattern in ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*') for p in directory.glob(pattern) if p.is_file()}
        if package.get('license_file'):
            license_paths.add(directory/package['license_file'])
        for path in sorted(license_paths):
            if not path.resolve().is_relative_to(directory.resolve()):
                raise ValueError('许可证路径超出依赖包目录')
            files['third-party-licenses/'+name+'/'+path.name] = (path.read_bytes(), False)
        notices.append({'name': package['name'], 'version': package['version'], 'license': package.get('license'), 'included_files': [p.name for p in sorted(license_paths)]})
    files['third-party-licenses/index.json'] = (json.dumps(notices, ensure_ascii=False, indent=2).encode(), False)
    info = {'application': version, 'commit': args.commit, 'target': args.target, 'machine': platform.machine(), 'libc_build_host': platform.libc_ver(), 'binary_sha256': sha(binary_bytes), 'source': 'https://github.com/Micraow/RecallCard', 'note': '开发版；扩展运行时没有Node依赖，Python仅用于可选worker。真实宿主和登录网页未实机验收。'}
    files['build-info.json'] = (json.dumps(info, ensure_ascii=False, indent=2).encode(), False)
    sums = ''.join(f'{sha(data)}  {name}\n' for name, (data, _) in sorted(files.items()))
    files['SHA256SUMS'] = (sums.encode(), False)
    archive = args.out/(stem+'.zip')
    extension_version = json.loads((ROOT/'extension/manifest.json').read_text())['version']
    extension_archive = args.out/(f'RecallCard-extension-{extension_version}-{args.commit[:7]}.zip')
    write_zip(archive, files)
    write_zip(extension_archive, extension)
    checksum_file = args.out/(stem+'-SHA256SUMS.txt')
    checksum_file.write_text(''.join(f'{sha(p.read_bytes())}  {p.name}\n' for p in [archive, extension_archive]))
    print(json.dumps({'package': str(archive), 'extension': str(extension_archive), 'checksums': str(checksum_file), 'binary_sha256': info['binary_sha256'], 'files': len(files), 'third_party_packages': len(notices)}, ensure_ascii=False))

if __name__ == '__main__':
    main()
