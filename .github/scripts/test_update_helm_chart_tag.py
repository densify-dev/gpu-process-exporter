# SPDX-License-Identifier: Apache-2.0

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("update-helm-chart-tag.sh").resolve()


class HelmChartReleaseTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.remote = self.root / "remote"
        self.chart = self.remote / "charts/gpu-process-exporter"
        self.chart.mkdir(parents=True)
        (self.chart / "Chart.yaml").write_text("name: gpu-process-exporter\nversion: 1.1.9\n")
        (self.chart / "values.yaml").write_text(
            'image:\n  repository: densify/gpu-process-exporter\n  tag: "1.1.2"\n'
            'other:\n  tag: "unchanged"\n'
        )
        self.readme = self.chart / "README.md"
        self.readme.write_text(
            "| Parameter | Mandatory | Description | Default |\n"
            "| `image.tag` | required | Exporter image tag. |  |\n"
            "| `image.pullPolicy` | | Pull policy. | `Always` |\n"
        )
        self.git("init", "--quiet", "-b", "master")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.com")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Initial chart")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text('''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys

args = sys.argv[1:]
if args[0] == "api":
    if "/git/ref/heads/" in args[1]:
        branch = args[1].split("/git/ref/heads/", 1)[1]
        sys.exit(subprocess.run(["git", "-C", os.environ["TEST_REMOTE"],
            "show-ref", "--verify", "--hash", "refs/heads/" + branch]).returncode)
    if args[1].startswith("/users/"):
        print("1234")
    else:
        fields = dict(arg.split("=", 1) for arg in args if "=" in arg)
        ref = fields.get("ref") or "refs/heads/" + args[3].split("/git/refs/heads/", 1)[1]
        subprocess.run(["git", "-C", os.environ["TEST_REMOTE"],
            "update-ref", ref, fields["sha"]], check=True)
elif args[:2] == ["pr", "list"]:
    print(os.environ.get("TEST_EXISTING_PR", ""))
else:
    body = Path(args[args.index("--body-file") + 1]).read_text()
    Path(os.environ["TEST_PR_LOG"]).write_text(json.dumps({"args": args, "body": body}))
''')
        gh.chmod(0o755)
        self.env = os.environ | {
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "TAG": "1.2.3",
            "GH_TOKEN": "test",
            "APP_SLUG": "test-app",
            "CHARTS_REPO": "test/helm-charts",
            "CHART_DIR": "charts/gpu-process-exporter",
            "GITHUB_REPOSITORY": "test/gpu-process-exporter",
            "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_RUN_ID": "123",
            "TEST_REMOTE": str(self.remote),
            "TEST_PR_LOG": str(self.root / "pr.json"),
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": f"url.{self.remote.as_uri()}.insteadOf",
            "GIT_CONFIG_VALUE_0": "https://x-access-token:test@github.com/test/helm-charts.git",
        }

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.remote), *args], text=True)

    def run_update(self, *, succeeds=True):
        runner = Path(tempfile.mkdtemp(dir=self.root))
        result = subprocess.run(
            ["bash", str(SCRIPT)], env=self.env | {"RUNNER_TEMP": str(runner)},
            text=True, capture_output=True,
        )
        self.assertEqual(result.returncode == 0, succeeds, result.stdout + result.stderr)
        return runner / "helm-charts/charts/gpu-process-exporter"

    def test_empty_readme_default_and_new_pr(self):
        chart = self.run_update()
        self.assertIn("version: 1.1.10\n", (chart / "Chart.yaml").read_text())
        self.assertIn('  tag: "1.2.3"\n', (chart / "values.yaml").read_text())
        self.assertIn('  tag: "unchanged"\n', (chart / "values.yaml").read_text())
        self.assertIn(
            "| `image.tag` | required | Exporter image tag. | `1.2.3` |",
            (chart / "README.md").read_text(),
        )
        self.assertIn("| `image.pullPolicy` | | Pull policy. | `Always` |", (chart / "README.md").read_text())
        pr = json.loads((self.root / "pr.json").read_text())
        self.assertEqual(pr["args"][:2], ["pr", "create"])
        self.assertIn("--base", pr["args"])
        self.assertIn("master", pr["args"])
        self.assertIn("1.1.10", pr["body"])
        self.assertNotIn("floatingTag", pr["body"])

    def test_populated_default_and_retry_updates_existing_pr(self):
        self.readme.write_text(self.readme.read_text().replace("tag. |  |", "tag. | `1.1.2` |"))
        self.git("commit", "--quiet", "-am", "Populate default")
        self.env["TEST_EXISTING_PR"] = "42"
        self.run_update()
        chart = self.run_update()
        self.assertIn("version: 1.1.10\n", (chart / "Chart.yaml").read_text())
        self.assertIn("tag. | `1.2.3` |", (chart / "README.md").read_text())
        pr = json.loads((self.root / "pr.json").read_text())
        self.assertEqual(pr["args"][:3], ["pr", "edit", "42"])

    def test_rejects_unstable_tag(self):
        self.env["TAG"] = "1.2.3-beta2"
        self.run_update(succeeds=False)
        self.assertFalse((self.root / "pr.json").exists())

    def test_missing_readme_row_stops_before_push(self):
        self.readme.write_text("No defaults table\n")
        self.git("commit", "--quiet", "-am", "Remove table")
        self.run_update(succeeds=False)
        self.assertEqual(
            self.git("rev-parse", "master"),
            self.git("rev-parse", "update/gpu-process-exporter-1.2.3"),
        )
        self.assertFalse((self.root / "pr.json").exists())


if __name__ == "__main__":
    unittest.main()
