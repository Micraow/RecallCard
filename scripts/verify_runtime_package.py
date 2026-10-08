#!/usr/bin/env python3
"""校验并安全解压运行包；可在原生验收后再次验证相同成品。"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import zipfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def safe_name(name):
    path = PurePosixPath(name)
    if (not name or '\\' in name or '\x00' in name or path.is_absolute()
            or any(part in ('', '.', '..') for part in name.split('/'))
            or ':' in name):
        raise ValueError('不安全的归档路径')
    return path


def archive_files(archive):
    files = {}
    total = 0
    with zipfile.ZipFile(archive) as opened:
        for entry in opened.infolist():
            safe_name(entry.filename)
            mode = entry.external_attr >> 16
            if entry.filename in files or not stat.S_ISREG(mode) or mode & 0o7777 not in (0o644, 0o755):
                raise ValueError('归档条目重复、不是普通文件或权限错误')
            total += entry.file_size
            if entry.file_size > 250 * 1024 * 1024 or total > 500 * 1024 * 1024 or len(files) > 10000:
                raise ValueError('归档超过解压上限')
            files[entry.filename] = (opened.read(entry), mode & 0o7777)
    return files


def check_files(files, source):
    manifest = files.get('SHA256SUMS', (b'', 0))[0].decode('utf-8')
    expected = {}
    for line in manifest.splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if not match or match[2] in expected:
            raise ValueError('校验清单无效或重复')
        safe_name(match[2])
        expected[match[2]] = match[1]
    if not expected or set(expected) != set(files) - {'SHA256SUMS'}:
        raise ValueError('校验清单与归档成员不一致')
    for name, digest in expected.items():
        if sha(files[name][0]) != digest:
            raise ValueError('文件摘要不符：' + name)
    for name, digest in source['binaries'].items():
        if name not in files or sha(files[name][0]) != digest or files[name][1] != 0o755:
            raise ValueError('程序与已验收字节或权限不符')
    if files.get('运行桌面版.sh', (None, None))[1] != 0o755:
        raise ValueError('启动脚本不可执行')
    info = json.loads(files['build-info.json'][0])
    if info.get('validated_source') != source or info['commit'] != source['application_commit']:
        raise ValueError('成品来源不符')
    return info


def unpack(archive, destination, source, verify_only=False):
    files = archive_files(archive)
    info = check_files(files, source)
    if verify_only:
        if not destination.is_dir() or destination.is_symlink():
            raise ValueError('验证目录无效')
    else:
        if destination.exists() or destination.is_symlink():
            raise ValueError('只允许解压到全新目录')
        destination.mkdir(parents=False)
        for name, (data, mode) in files.items():
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open('xb') as stream:
                stream.write(data)
            target.chmod(mode)
    actual = {}
    for path in destination.rglob('*'):
        status = path.lstat()
        if not (stat.S_ISREG(status.st_mode) or stat.S_ISDIR(status.st_mode)):
            raise ValueError('解压目录出现符号链接或特殊节点')
        if stat.S_ISREG(status.st_mode):
            actual[path.relative_to(destination).as_posix()] = (path.read_bytes(), stat.S_IMODE(status.st_mode))
    if actual != files:
        raise ValueError('解压目录内容或权限与归档不同')
    return {'archive_sha256':sha(archive.read_bytes()),'files':len(files),
            'destination':str(destination.resolve()),'application':str((destination/'运行桌面版.sh').resolve()),
            'gui_sha256':source['binaries']['recallcard-desktop'],'cli_sha256':source['binaries']['recallcard'],
            'packaging_commit':info['packaging_commit'],'verified':True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', required=True, type=Path)
    parser.add_argument('--destination', required=True, type=Path)
    parser.add_argument('--provenance', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--verify-only', action='store_true')
    parser.add_argument('--native-summary', type=Path)
    args = parser.parse_args()
    source = json.loads(args.provenance.read_text())
    result = unpack(args.archive, args.destination, source, args.verify_only)
    if args.native_summary:
        summary = json.loads(args.native_summary.read_text())
        steps = summary['passed_steps']
        if (summary.get('success') is not True or len(steps) != 37 or len(set(steps)) != 37
                or summary['application'] != result['application'] or not summary.get('synthetic_data_only')):
            raise ValueError('原生验收不是本解压目录完整37步成功')
        result['native_steps'] = len(steps)
        result['native_summary_sha256'] = sha(args.native_summary.read_bytes())
        result['post_native_verified'] = True
    args.report.write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
