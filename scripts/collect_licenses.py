#!/usr/bin/env python3
"""把第三方许可证与说明写进桌面安装包的资源目录。

Tauri 打包时会把 desktop/src-tauri/distribution/ 整体装进安装包，因此这里只需
保证该目录内容与当前依赖图一致：先清空，再按 cargo metadata 重新生成。
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OVERRIDES = ROOT / 'desktop/third-party-licenses'
SYSTEM_PACKAGES = ('libwebkit2gtk-4.1-0', 'libjavascriptcoregtk-4.1-0',
                   'libgtk-3-0', 'libsoup-3.0-0')
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
    override = OVERRIDES / f"{package['name']}-{package['version']}"
    if not candidates and override.is_dir():
        candidates = {p for pattern in LICENSE_PATTERNS for p in override.rglob(pattern) if p.is_file()}
    if not candidates:
        raise ValueError(f"依赖缺少许可证正文：{package['name']}")
    for path in candidates:
        resolved = path.resolve()
        if not (resolved.is_relative_to(directory.resolve()) or resolved.is_relative_to(override.resolve())):
            raise ValueError('许可证路径超出依赖目录')
    return sorted(candidates)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--metadata', type=Path, action='append', required=True,
                        help='cargo metadata 输出；可重复传入多个依赖图')
    parser.add_argument('--out', type=Path, default=ROOT / 'desktop/src-tauri/distribution')
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
            base = directory.resolve() if path.resolve().is_relative_to(directory.resolve()) else OVERRIDES / f"{package['name']}-{package['version']}"
            relative = path.resolve().relative_to(base).as_posix()
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
        })

    notices_dir.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(OVERRIDES / 'README.md', notices_dir / '补充来源说明.md')
    (notices_dir / 'index.json').write_text(json.dumps(notices, ensure_ascii=False, indent=2))

    # 程序动态链接的系统库由发行版提供，随包附带其许可证文本。
    for package in SYSTEM_PACKAGES:
        source = Path('/usr/share/doc') / package / 'copyright'
        if source.is_file():
            target = args.out / 'system-licenses' / package / 'copyright'
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)

    shutil.copyfile(ROOT / 'docs/design.md', args.out / '设计说明.md')
    print(json.dumps({'out': str(args.out), 'third_party_packages': len(notices)}, ensure_ascii=False))


if __name__ == '__main__':
    sys.exit(main())
