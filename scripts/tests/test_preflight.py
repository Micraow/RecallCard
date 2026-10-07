"""在临时副本中制造真实回归；不改应用、不启动浏览器或原生驱动。"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("preflight", ROOT / "scripts/preflight.py")
preflight = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(preflight)


class PreflightTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="recallcard-preflight-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        ci = patch.dict(os.environ, {"GITHUB_ACTIONS": "false"})
        ci.start()
        self.addCleanup(ci.stop)

    def copy(self, relative):
        source = ROOT / relative
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, target, dirs_exist_ok=True)
        else:
            shutil.copyfile(source, target)
        return target

    def write(self, relative, content):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
        return target

    def workflow(self, run, extra=""):
        return self.write(".github/workflows/check.yml", "name: 合成门禁\non: workflow_dispatch\njobs:\n"
                          "  check:\n    runs-on: ubuntu-latest\n    steps:\n"
                          + extra + "      - run: |\n" + "".join("          " + line + "\n" for line in run.splitlines()))

    def node_fixture(self):
        self.copy(".github/workflows")
        for directory in ("extension", "desktop/tests/local-dom"):
            self.copy(directory + "/package.json")
            self.copy(directory + "/package-lock.json")
            (self.root / directory / "node_modules").symlink_to(ROOT / directory / "node_modules", target_is_directory=True)

    def asset_fixture(self):
        self.copy("desktop/ui")
        self.copy("desktop/tests/workspace-browser-helpers.mjs")
        self.copy("scripts/check_browser_assets.mjs")
        self.copy("desktop/package.json")

    def test_actual_node_version_must_match_release_ci(self):
        self.copy(".github/workflows")
        workflow = self.root / ".github/workflows/package-desktop.yml"
        workflow.write_text(workflow.read_text().replace("node-version: '24.19.0'", "node-version: '22.0.0'"))
        with self.assertRaisesRegex(preflight.PreflightError, "与安装包 CI .* 不一致"):
            preflight.check_node(self.root)

    def test_package_and_lock_engines_must_agree(self):
        self.node_fixture()
        path = self.root / "extension/package.json"
        data = json.loads(path.read_text())
        data["engines"]["node"] = ">=999.0.0"
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(preflight.PreflightError, "锁文件 engines 不一致"):
            preflight.check_node(self.root)

    def test_actual_semver_rejects_unsupported_locked_engine(self):
        self.node_fixture()
        path = self.root / "desktop/tests/local-dom/package-lock.json"
        data = json.loads(path.read_text())
        data["packages"]["node_modules/jsdom"]["engines"]["node"] = ">=999.0.0"
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(preflight.PreflightError, "999.0.0"):
            preflight.check_node(self.root)

    def test_ci_local_dom_cannot_silently_use_node_22(self):
        self.node_fixture()
        self.workflow("npm --prefix desktop/tests/local-dom test", extra="      - uses: actions/setup-node@v4\n        with:\n          node-version: '22'\n")
        with self.assertRaisesRegex(preflight.PreflightError, "运行 local-dom 前必须固定 Node"):
            preflight.check_node(self.root)

    def test_missing_browser_asset_is_rejected_by_real_helper(self):
        self.asset_fixture()
        (self.root / "desktop/ui/background.js").unlink()
        with self.assertRaisesRegex(preflight.PreflightError, "ENOENT"):
            preflight.check_assets(self.root)

    def test_omitted_browser_asset_is_rejected(self):
        self.asset_fixture()
        path = self.root / "desktop/tests/workspace-browser-helpers.mjs"
        path.write_text(path.read_text().replace("'background.js', ", ""))
        with self.assertRaisesRegex(preflight.PreflightError, "浏览器资源白名单必须覆盖"):
            preflight.check_assets(self.root)

    def test_module_dependency_is_parsed_by_v8_without_evaluation(self):
        self.asset_fixture()
        path = self.root / "desktop/ui/app.js"
        path.write_text("import './missing.mjs';\n" + path.read_text())
        with self.assertRaisesRegex(preflight.PreflightError, "缺少模块"):
            preflight.check_assets(self.root)

    def test_html_reference_must_exist_in_assets(self):
        self.asset_fixture()
        path = self.root / "desktop/ui/index.html"
        path.write_text(path.read_text().replace("styles.css", "missing.css"))
        with self.assertRaisesRegex(preflight.PreflightError, "index.html 引用的资源"):
            preflight.check_assets(self.root)

    def test_workflow_missing_script_path_is_rejected(self):
        self.workflow("python3 scripts/missing.py")
        with self.assertRaisesRegex(preflight.PreflightError, "源码路径不存在"):
            preflight.check_workflow_sources(self.root)

    def test_workflow_python_script_syntax_is_checked(self):
        self.write("scripts/broken.py", "if True\n    pass\n")
        self.workflow("python3 scripts/broken.py")
        with self.assertRaises(SyntaxError):
            preflight.check_workflow_sources(self.root)

    def test_workflow_cannot_reference_source_outside_repo(self):
        self.workflow("python3 scripts/../../outside.py")
        with self.assertRaisesRegex(preflight.PreflightError, "超出仓库"):
            preflight.check_workflow_sources(self.root)
        self.root.joinpath("scripts").symlink_to(ROOT / "scripts", target_is_directory=True)
        self.workflow("python3 scripts/preflight.py")
        with self.assertRaisesRegex(preflight.PreflightError, "超出仓库"):
            preflight.check_workflow_sources(self.root)

    def test_workflow_missing_working_directory_is_rejected(self):
        self.workflow("true", extra="      - run: true\n        working-directory: missing\n")
        with self.assertRaisesRegex(preflight.PreflightError, "working-directory 不存在"):
            preflight.check_workflow_sources(self.root)

    def test_duplicate_native_workflow_invocation_is_rejected(self):
        self.copy("desktop/tests/native_smoke.py")
        self.workflow("python3 desktop/tests/native_smoke.py\npython3 desktop/tests/native_smoke.py")
        with self.assertRaisesRegex(preflight.PreflightError, "重复调用原生完整验收 2 次"):
            preflight.check_workflow_sources(self.root)

    def test_native_density_cannot_be_called_twice(self):
        path = self.copy("desktop/tests/native_smoke.py")
        text = path.read_text()
        marker = "        workspace_evidence = self.exercise_workspace_usability()"
        self.assertEqual(text.count(marker), 1)
        path.write_text(text.replace(marker, marker + "\n" + marker))
        with self.assertRaisesRegex(preflight.PreflightError, "重复调用流程"):
            preflight.check_native_order(self.root)

    def test_density_must_follow_other_native_flows(self):
        path = self.copy("desktop/tests/native_smoke.py")
        text = path.read_text()
        marker = "        workspace_evidence = self.exercise_workspace_usability()\n"
        text = text.replace(marker, "")
        text = text.replace("        background_evidence = self.exercise_background_selection", marker + "        background_evidence = self.exercise_background_selection")
        path.write_text(text)
        with self.assertRaisesRegex(preflight.PreflightError, "最后一个"):
            preflight.check_native_order(self.root)

    def test_native_flow_cannot_hide_in_conditional(self):
        path = self.copy("desktop/tests/native_smoke.py")
        marker = "        workspace_evidence = self.exercise_workspace_usability()"
        path.write_text(path.read_text().replace(marker, "        if False:\n    " + marker))
        with self.assertRaisesRegex(preflight.PreflightError, "直接、无条件调用"):
            preflight.check_native_order(self.root)

    def test_invalid_shell_syntax_fails_without_running(self):
        self.workflow("if true; then\n  echo synthetic")
        with self.assertRaisesRegex(preflight.PreflightError, "退出码"):
            preflight.check_workflow_sources(self.root)

    def test_inline_python_syntax_error_is_rejected(self):
        self.workflow("python3 - <<'PY'\nif True\n    pass\nPY")
        with self.assertRaises(SyntaxError):
            preflight.check_workflow_sources(self.root)

    def test_inline_python_is_compiled_but_never_executed(self):
        marker = self.root / "must-not-exist"
        self.workflow("python3 - <<'PY'\nfrom pathlib import Path\nPath(" + repr(str(marker)) + ").write_text('wrong')\nPY")
        result = preflight.check_workflow_sources(self.root)
        self.assertEqual(result["python_syntax_only"], 1)
        self.assertFalse(marker.exists())

    def test_duplicate_yaml_key_is_rejected(self):
        self.write(".github/workflows/check.yml", "name: one\nname: two\njobs: {}\n")
        with self.assertRaisesRegex(preflight.PreflightError, "重复键"):
            preflight.workflows(self.root)

    def test_github_script_syntax_is_checked_without_running(self):
        self.workflow("true", extra="      - uses: actions/github-script@v7\n        with:\n          script: const = broken;\n")
        with self.assertRaisesRegex(preflight.PreflightError, "退出码"):
            preflight.check_workflow_sources(self.root)

    def test_expression_placeholder_cannot_execute_commands(self):
        source = "echo '${{ github.sha }}'\nx=${{ runner.temp }}"
        self.assertEqual(preflight.syntax_placeholder(source), "echo '0'\nx=0")
        with self.assertRaises(preflight.PreflightError):
            preflight.syntax_placeholder("${{ github.sha\n}}")
        with self.assertRaises(preflight.PreflightError):
            preflight.syntax_placeholder("${{ $(touch /tmp/unsafe) }}")

    def test_ci_environment_requires_real_temp_without_logging_values(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "RUNNER_TEMP": "", "GITHUB_WORKSPACE": str(self.root)}):
            with self.assertRaisesRegex(preflight.PreflightError, "缺少 RUNNER_TEMP"):
                preflight.check_environment(self.root)
        with patch.dict(os.environ, {"SYNTHETIC_API_KEY": "synthetic-hidden-value"}):
            self.assertNotIn("synthetic-hidden-value", preflight.sanitized("error synthetic-hidden-value"))
        with patch.dict(os.environ, {"SYNTHETIC_SECRET": "false"}):
            result = preflight.sanitized_data({"flag": False, "description": "false"})
            self.assertIs(result["flag"], False)
            self.assertEqual(result["description"], "[已隐藏]")

    def test_missing_actionlint_cannot_be_green(self):
        with self.assertRaisesRegex(preflight.PreflightError, "缺少 actionlint"):
            preflight.check_actionlint(self.root, str(self.root / "missing-actionlint"))

    def test_official_actionlint_rejects_runner_context_at_top_level(self):
        actionlint = os.environ.get("RECALLCARD_PREFLIGHT_ACTIONLINT") or shutil.which("actionlint")
        if not actionlint:
            self.skipTest("未安装 actionlint；完整预检仍会因缺工具而失败")
        path = self.workflow("true")
        path.write_text("env:\n  STATE: ${{ runner.temp }}/synthetic\n" + path.read_text())
        with self.assertRaisesRegex(preflight.PreflightError, "runner"):
            preflight.check_actionlint(self.root, actionlint)

    def test_cli_keeps_independent_results_and_nonzero_exit(self):
        self.copy("desktop/tests/native_smoke.py")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = preflight.main(["--root", str(self.root), "--checks", "actionlint,native_order", "--actionlint", str(self.root / "missing")])
        report = json.loads(out.getvalue())
        self.assertEqual(status, 1)
        self.assertEqual([item["status"] for item in report["checks"]], ["failed", "passed"])
        self.assertFalse(report["local_gate_passed"])
        self.assertFalse(report["full_acceptance_passed"])
        self.assertEqual([item["name"] for item in report["not_run"]], ["chromium", "native", "rust_and_packaging"])

    def test_zero_tests_or_missing_summary_cannot_be_green(self):
        for runner, output in [("node", "ℹ tests 0\nℹ fail 0\n"), ("python", "Ran 0 tests in 0.001s\nOK"), ("node", "exit 0"), ("python", "OK")]:
            with self.subTest(runner=runner, output=output), self.assertRaisesRegex(preflight.PreflightError, "未证明有用例执行"):
                preflight.tested_output(output, runner)

    def test_actual_runner_summaries_require_a_positive_count(self):
        for runner, output in [("node", "\x1b[32mℹ tests 59\x1b[0m\n"), ("python", "Ran 21 tests in 0.42s\nOK"), ("python", "Ran 1 test in 0.01s\nOK")]:
            self.assertEqual(preflight.tested_output(output, runner), output)

    def test_partial_run_never_claims_full_gate(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = preflight.main(["--root", str(self.root), "--checks", "environment"])
        report = json.loads(out.getvalue())
        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "partial")
        self.assertFalse(report["local_gate_passed"])


if __name__ == "__main__":
    unittest.main()
