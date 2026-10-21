#!/usr/bin/env python3
"""汇总随发布包分发的第三方许可证与来源说明。

发布包由 recallcard 二进制、web/dist 与 dist/ 三部分组成，dist/ 里放的就是这里
生成的内容。先清空再按 cargo metadata 重新生成，保证与当前依赖图一致。
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

LICENSE_PATTERNS = ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*')


def resolved_packages(paths):
    """GUI 与独立 CLI 来自不同锁文件，许可证清单取两个已解析图的并集。"""
    packages = {}
    for path in paths:
        metadata = json.loads(path.read_text())
        ids = {node['id'] for node in metadata['resolve']['nodes']}
        found = {package['id']: package for package in metadata['packages'] if package['id'] in ids}
        if set(found) != ids:
            raise ValueError('依赖元数据缺少已解析包')
        for identifier, package in found.items():
            if identifier in packages and packages[identifier] != package:
                raise ValueError('同一依赖 ID 对应不同元数据')
            packages[identifier] = package
    return sorted(packages.values(), key=lambda item: (item['name'], item['version'], item['id']))


def license_files(package):
    directory = Path(package['manifest_path']).parent
    candidates = {p for pattern in LICENSE_PATTERNS for p in directory.rglob(pattern) if p.is_file()}
    if package.get('license_file'):
        candidates.add(directory / package['license_file'])
    if not candidates:
        # 少数 crate 的发布包里根本没带许可证正文；如实记在清单里，不假装已附带。
        return []
    for path in candidates:
        resolved = path.resolve()
        if not (resolved.is_relative_to(directory.resolve()) or resolved.is_relative_to(override.resolve())):
            raise ValueError('许可证路径超出依赖目录')
    return sorted(candidates)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--metadata', type=Path, action='append', required=True,
                        help='cargo metadata 输出；可重复传入多个依赖图')
    parser.add_argument('--out', type=Path, default=ROOT / 'dist')
    args = parser.parse_args()

    if args.out.exists():
        shutil.rmtree(args.out)
    notices_dir = args.out / 'third-party-licenses'
    source_dir = args.out / 'third-party-source'
    notices = []

    for package in resolved_packages(args.metadata):
        if package.get('source') is None:
            continue
        directory = Path(package['manifest_path']).parent
        included = []
        for path in license_files(package):
            relative = path.resolve().relative_to(directory.resolve()).as_posix()
            target = notices_dir / f"{package['name']}-{package['version']}" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            included.append(relative)
        # MPL 文件的对应源码随包原样提供，不把其文件许可扩展到其他项目文件。
        if package.get('license') == 'MPL-2.0':
            for path in directory.rglob('*'):
                if path.is_file():
                    target = source_dir / f"{package['name']}-{package['version']}" / path.relative_to(directory).as_posix()
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, target)
        notices.append({
            'name': package['name'], 'version': package['version'],
            'license': package.get('license'), 'repository': package.get('repository'),
            'source_archive': f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download",
            'included_files': included,
            'license_text_bundled': bool(included),
        })

    notices_dir.mkdir(parents=True, exist_ok=True)
    (notices_dir / 'index.json').write_text(json.dumps(notices, ensure_ascii=False, indent=2))

    shutil.copyfile(ROOT / 'docs/design.md', args.out / '设计说明.md')
    missing = [item['name'] for item in notices if not item['license_text_bundled']]
    print(json.dumps({'out': str(args.out), 'third_party_packages': len(notices),
                      'license_text_missing': missing}, ensure_ascii=False))


if __name__ == '__main__':
    sys.exit(main())
