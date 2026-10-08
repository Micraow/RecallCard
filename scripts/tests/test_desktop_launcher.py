"""在临时目录执行真实 shell；不启动 GUI、不安装包、不修改系统路径。"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DesktopLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='recallcard launcher ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root/'bin'; self.bin.mkdir()
        self.helpers = self.root/'helpers'; self.helpers.mkdir()
        self.marker = self.root/'debian_version'; self.marker.touch()
        source = (ROOT/'scripts/desktop_launcher.sh').read_text()
        # 仅合成测试替换受控路径；生产脚本不提供绕过检查的环境开关。
        source = source.replace('/etc/debian_version', str(self.marker))
        source = source.replace('/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1', str(self.helpers))
        source = source.replace('[ -f '+str(self.marker)+' ]', '[ -f "'+str(self.marker)+'" ]')
        self.launcher = self.root/'运行桌面版.sh'
        self.launcher.write_text(source)
        self.exe(self.root/'recallcard-desktop', 'printf "APP:%s\\n" "$@"')
        self.exe(self.bin/'ldd', 'printf "%s\\n" "${TEST_LDD_OUTPUT:-libgtk.so => /synthetic/libgtk.so}"; exit "${TEST_LDD_EXIT:-0}"')
        self.exe(self.bin/'uname', 'printf "x86_64\\n"')
        self.exe(self.bin/'zenity', 'printf "%s\\n" "$@" > "$TEST_DIALOG_FILE"')
        for helper in ['WebKitNetworkProcess', 'WebKitWebProcess']:
            self.exe(self.helpers/helper, 'exit 99')
        self.env = {'PATH': str(self.bin)+':/usr/bin:/bin', 'DISPLAY': ':synthetic',
                    'TEST_DIALOG_FILE': str(self.root/'dialog.txt')}

    def exe(self, path, body):
        path.write_text('#!/bin/sh\n'+body+'\n'); path.chmod(0o755)

    def run_launcher(self, *args, **changes):
        return subprocess.run(['/bin/sh', str(self.launcher), *args], env=self.env|changes,
                              capture_output=True, text=True, timeout=3)

    def test_check_only_does_not_open_window_or_run_app(self):
        result = self.run_launcher('--check-runtime')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('运行库检查通过', result.stdout)
        self.assertNotIn('APP:', result.stdout)
        self.assertFalse((self.root/'dialog.txt').exists())

    def test_missing_fixed_helper_is_actionable_before_app_launch(self):
        (self.helpers/'WebKitNetworkProcess').unlink()
        result = self.run_launcher()
        self.assertEqual(result.returncode, 78)
        self.assertIn('WebKitNetworkProcess', result.stderr)
        self.assertIn('--reinstall libwebkit2gtk-4.1-0', result.stderr)
        self.assertNotIn('APP:', result.stdout)
        self.assertIn('WebKitNetworkProcess', (self.root/'dialog.txt').read_text())

    def test_missing_library_and_glibc_error_do_not_start_app(self):
        for output, status in [('libwebkit2gtk-4.1.so.0 => not found', '0'),
                               ('GLIBC_2.35 not found', '1')]:
            with self.subTest(output=output):
                result = self.run_launcher('--check-runtime', TEST_LDD_OUTPUT=output, TEST_LDD_EXIT=status)
                self.assertEqual(result.returncode, 78)
                self.assertIn(output, result.stderr)
                self.assertNotIn('APP:', result.stdout)

    def test_success_preserves_arguments_and_headless_has_cli_guidance(self):
        result = self.run_launcher('a b', '中文')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'APP:a b\nAPP:中文\n')
        result = self.run_launcher(DISPLAY='')
        self.assertEqual(result.returncode, 78)
        self.assertIn('没有检测到桌面显示会话', result.stderr)
        self.assertIn('命令行', result.stderr)

    def test_non_debian_does_not_assume_debian_helper_directory(self):
        self.marker.unlink()
        (self.helpers/'WebKitNetworkProcess').unlink()
        self.assertEqual(self.run_launcher('--check-runtime').returncode, 0)


if __name__ == '__main__':
    unittest.main()
