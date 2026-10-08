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


FIRST_USE_STEPS = [
    '首次只需导入不要求格式范围或模型',
    '官方形状ZIP经真实选择器导入后即刻阅读原话',
    '从背景一键回到确切原话及原始时间',
    '取消不写入且近况一次保存原话保持不变',
    '补充近况立即可搜索',
    '真实stdioMCP从同一正本读到刚保存近况',
    'ChatGPT真实宿主未授权保持待连接',
    '820窗口加载后的背景',
    '820窗口已加载来源详情',
    '820详情返回来源列表',
    '重启恢复已有背景不要求重新导入',
]
CONNECTED_STEPS = [
    '全新导入后显示真实连接状态且未自动授予权限',
    '导入的原始来源可读可追溯',
    '原生选择项目后只预览不写文件也不提前授权',
    '一次批准写入真实项目配置并本机试读不伪造宿主回执',
    '补充后入口自动刷新且受管理MCP从同一正本读到更新',
    '重启保留配置并区分本机试读和实际客户端请求',
    '820窗口连接状态与操作完整可读',
    '撤权清除文件入口并拒绝旧连接继续读取且正本保留',
]
MODERN_SUITES = {'0.6.': ('v0.6-first-use-native', FIRST_USE_STEPS),
                 '0.7.': ('v0.7-connected-context-native', CONNECTED_STEPS)}


def check_native_summary(summary, result, source):
    if (summary.get('success') is not True or summary.get('synthetic_data_only') is not True
            or summary.get('application') != result['application']):
        raise ValueError('原生验收未成功，或不是本解压目录的启动程序')
    steps = summary.get('passed_steps')
    version = result.get('application_version', '')
    suite = source.get('acceptance', {}).get('suite', 'legacy-native-37')
    modern = next((value for prefix, value in MODERN_SUITES.items() if version.startswith(prefix)), None)
    if modern:
        expected_cli = str((Path(result['destination'])/'recallcard').resolve())
        if (suite != modern[0] or summary.get('suite') != suite or steps != modern[1]
                or summary.get('cli') != expected_cli or summary.get('invoke_mocked') is not False):
            raise ValueError('新版原生验收的流程、CLI位置或真实调用证据不符')
        if version.startswith('0.7.') and summary.get('external_agents_executed') is not False:
            raise ValueError('v0.7 验收不能冒充外部Agent或账号已验证')
        build = summary.get('build') or {}
        if source.get('application_version') != version:
            raise ValueError('新版包版本与已验收程序来源不符')
        for name in ['gui', 'cli']:
            identity = build.get(name) or {}
            if (identity.get('commit') != source['application_commit']
                    or identity.get('version') != version or identity.get('dirty') is not False):
                raise ValueError('新版GUI与CLI不是同一已验收构建身份')
    elif version.startswith('0.5.') and suite == 'legacy-native-37':
        if (source.get('application_version', version) != version
                or summary.get('suite') is not None or not isinstance(steps, list)
                or len(steps) != 37 or len(set(steps)) != 37):
            raise ValueError('历史原生验收不是完整37步，不能代替v0.6流程')
    else:
        raise ValueError('未识别的原生验收合同')
    return {'native_suite': suite, 'native_steps': len(steps), 'post_native_verified': True}


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
    # 版本来自已校验的包元数据，不能因外部旧格式来源缺字段而退回旧流程。
    label = re.fullmatch(r'RecallCard Desktop (\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)', info.get('application', ''))
    if label is None:
        raise ValueError('包应用版本无效')
    version = info.get('application_version', label[1])
    if version != label[1] or source.get('application_version', version) != version:
        raise ValueError('包应用版本与成品来源不符')
    if version.startswith('0.6.') and (source.get('application_version') != version
            or source.get('acceptance', {}).get('suite') != 'v0.6-first-use-native'):
        raise ValueError('v0.6包不能使用缺少版本或新版验收合同的历史来源')
    if version.startswith('0.7.') and (source.get('application_version') != version
            or source.get('acceptance', {}).get('suite') != 'v0.7-connected-context-native'):
        raise ValueError('v0.7包必须绑定本版真实项目接入验收，不能借用旧版流程')
    info['application_version'] = version
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
            'application_version':info['application_version'],
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
        result.update(check_native_summary(summary, result, source))
        result['native_summary_sha256'] = sha(args.native_summary.read_bytes())
    args.report.write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
