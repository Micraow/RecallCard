#!/usr/bin/env python3
"""仅收集真实、非空的安装包，跳过 AppDir 中间文件。"""
import hashlib
import pathlib
import re
import shutil
import sys

source, output = map(pathlib.Path, sys.argv[1:3])
commit = sys.argv[3]
if not re.fullmatch(r'[0-9a-f]{40}', commit):
    raise SystemExit('提交摘要无效')
output.mkdir(parents=True, exist_ok=True)
found = set()
for path in source.rglob('*'):
    if not path.is_file() or not path.stat().st_size or any(part.endswith('.AppDir') for part in path.parts):
        continue
    if path.suffix not in ('.AppImage', '.deb'):
        continue
    found.add(path.suffix)
    shutil.copyfile(path, output / f'RecallCard-{commit[:7]}-{path.name}')
if found != {'.AppImage', '.deb'}:
    raise SystemExit(f'安装包不完整：{sorted(found)}')
rows = [f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in sorted(output.iterdir()) if p.is_file() and 'SHA256SUMS' not in p.name]
(output / 'SHA256SUMS.txt').write_text(''.join(rows))
