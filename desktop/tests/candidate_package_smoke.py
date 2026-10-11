#!/usr/bin/env python3
"""Minimal extracted-package startup check, not the full connected-context journey."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
from urllib.request import urlopen

from first_use_native import FirstUseJourney, JourneyDriver
from native_smoke import wait_for, run
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from verify_runtime_package import archive_files


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--archive', type=Path, required=True)
    p.add_argument('--artifacts', type=Path, required=True)
    p.add_argument('--commit', required=True)
    args = p.parse_args()
    args.artifacts.mkdir(parents=True, exist_ok=True)
    files = archive_files(args.archive)
    sums = dict((line.split('  ', 1)[1], line.split('  ', 1)[0]) for line in files['SHA256SUMS'][0].decode().splitlines())
    assert set(sums) == set(files) - {'SHA256SUMS'}
    assert all(hashlib.sha256(files[name][0]).hexdigest() == digest for name, digest in sums.items())
    info = json.loads(files['build-info.json'][0])
    assert info['commit'] == args.commit
    receipt = {'suite': 'extracted-candidate-startup-smoke', 'success': False, 'commit': args.commit,
               'archive_sha256': hashlib.sha256(args.archive.read_bytes()).hexdigest(),
               'full_native_journey_rerun': False, 'synthetic_only': True}
    with tempfile.TemporaryDirectory(prefix='recallcard-package-') as t:
        root = Path(t)
        package = root / 'package'
        for name, (data, mode) in files.items():
            dest = package / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
            dest.chmod(mode)
        for name, suffix in [('HOME', 'home'), ('XDG_DATA_HOME', 'data'), ('XDG_CONFIG_HOME', 'config'), ('RECALLCARD_STATE_DIR', 'state')]:
            dest = root / suffix
            dest.mkdir()
            os.environ[name] = str(dest)
        args.application = package / '运行桌面版.sh'
        args.cli = package / 'recallcard'
        receipt['cli_version'] = run(str(args.cli), '--version').strip()
        assert info['application_version'] in receipt['cli_version']
        receipt['runtime_check'] = run(str(args.application), '--check-runtime').strip()
        smoke = FirstUseJourney(args, root)
        try:
            smoke.start(['openbox'], 'window-manager.log')
            server = smoke.start_native_driver()
            def ready():
                assert server.poll() is None
                with urlopen('http://127.0.0.1:4444/status', timeout=2) as response:
                    return json.load(response).get('value', {}).get('ready') is True
            wait_for(ready, 'package WebKit driver')
            smoke.driver = JourneyDriver(args.application)
            wait_for(lambda: smoke.content('导入聊天记录'), 'extracted package first-use UI')
            smoke.assert_clean()
            smoke.capture('package-first-use')
            # Recheck all extracted payload bytes after normal startup.
            assert all(hashlib.sha256((package / name).read_bytes()).hexdigest() == digest for name, digest in sums.items())
            receipt.update(success=True, extracted_checksums_verified=True, first_use_ui_visible=True)
        finally:
            smoke.close()
            (args.artifacts / 'summary.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
