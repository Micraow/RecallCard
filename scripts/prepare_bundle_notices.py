#!/usr/bin/env python3
"""从本次已验证运行包提取说明和依赖声明，交给标准 Tauri 打包器。"""
import pathlib
import shutil
import sys
import zipfile

archives = list(pathlib.Path(sys.argv[1]).glob('RecallCard-Desktop-*.zip'))
if len(archives) != 1:
    raise SystemExit('需要本次唯一的桌面运行包')
destination = pathlib.Path(sys.argv[2])
destination.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(archives[0]) as archive:
    for entry in archive.infolist():
        path = pathlib.PurePosixPath(entry.filename)
        if path.is_absolute() or '..' in path.parts:
            raise ValueError('打包路径无效')
        if entry.is_dir() or not (entry.filename.startswith(('third-party-licenses/', 'third-party-source/')) or entry.filename in ('开始使用-中文.md', 'build-info.json', 'recallcard')):
            continue
        output = destination / pathlib.Path(*path.parts)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(archive.read(entry))
        if entry.filename == "recallcard":
            output.chmod(0o755)
# 保留本次系统构建依赖的发行版许可证；程序只动态链接这些库。
for package in ('libwebkit2gtk-4.1-0', 'libjavascriptcoregtk-4.1-0', 'libgtk-3-0', 'libsoup-3.0-0'):
    copyright_file = pathlib.Path('/usr/share/doc') / package / 'copyright'
    if copyright_file.is_file():
        output = destination / 'system-licenses' / package / 'copyright'
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(copyright_file, output)
