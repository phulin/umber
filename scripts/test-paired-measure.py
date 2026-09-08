#!/usr/bin/env python3
"""Hermetic tests for the paired prebuilt-executable measurement runner."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "paired-measure.py"
MODULE_PATH = ROOT / "scripts" / "paired_measure.py"
spec = importlib.util.spec_from_file_location("paired_measure", MODULE_PATH)
assert spec is not None and spec.loader is not None
paired_measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paired_measure)


class PairedMeasureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="paired-measure-test-")
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.run_git("init", "-q")
        self.run_git("config", "user.email", "paired-measure@example.invalid")
        self.run_git("config", "user.name", "Paired Measure Test")
        (self.repo / "README").write_text("fixture\n", encoding="utf-8")
        self.run_git("add", "README")
        self.run_git("commit", "-q", "-m", "fixture")
        self.revision = self.run_git("rev-parse", "HEAD").stdout.strip()
        self.fixture = self.root / "input.tex"
        self.fixture.write_text("\\relax\n", encoding="utf-8")
        self.fixture_hash = paired_measure.digest_file(self.fixture)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_git(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(self.repo), *arguments],
            check=True,
            capture_output=True,
            text=True,
        )

    def mock(self, name: str, payload: dict[str, object], *, exit_code: int = 0, sleep: float = 0.0) -> Path:
        payload = {"elapsed_ns": 123, **payload}
        path = self.root / name
        source = (
            "#!/usr/bin/env python3\n"
            "import json\n"
            "import time\n"
            "from pathlib import Path\n"
            "Path('result.bin').write_bytes(b'result')\n"
            f"time.sleep({sleep!r})\n"
            f"print(json.dumps({json.dumps(payload, sort_keys=True)}))\n"
            f"raise SystemExit({exit_code})\n"
        )
        path.write_text(source, encoding="utf-8")
        path.chmod(0o755)
        return path

    def mock_jsonl(self, name: str, rows: list[dict[str, object]]) -> Path:
        path = self.root / name
        source = (
            "#!/usr/bin/env python3\n"
            "import json\n"
            "from pathlib import Path\n"
            "Path('result.bin').write_bytes(b'result')\n"
            f"rows = {json.dumps(rows, sort_keys=True)}\n"
            "for row in rows:\n"
            "    print(json.dumps(row, sort_keys=True))\n"
        )
        path.write_text(source, encoding="utf-8")
        path.chmod(0o755)
        return path

    def manifest(self, path: Path, *, semantic_value: str = "same", expected_exit: int = 0) -> None:
        path.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "lane": "release-unobserved",
                    "profile": "release",
                    "features": [],
                    "instrumented": False,
                    "build": {"target": "test"},
                    "toolchain": {"rustc": "test-rustc"},
                    "environment": {},
                    "workloads": [
                        {
                            "name": "mock-command-core",
                            "args": ["--fixture", str(self.fixture)],
                            "files": [{"path": str(self.fixture), "sha256": self.fixture_hash}],
                            "semantic": {
                                "format": "json",
                                "stream": "stdout",
                                "ignore_keys": ["elapsed_ns"],
                                "expected": {
                                    "schema": 1,
                                    "semantic": {"value": semantic_value},
                                },
                            },
                            "inner_metrics": [
                                {"name": "elapsed_ns", "path": ["elapsed_ns"], "unit": "ns"}
                            ],
                            "outputs": [{"path": "result.bin"}],
                            "expected_exit": expected_exit,
                        }
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )

    def command(self, manifest: Path, baseline: Path, candidate: Path, output: Path, **options: str) -> list[str]:
        command = [
            sys.executable,
            str(SCRIPT),
            "--manifest",
            str(manifest),
            "--lane",
            "release-unobserved",
            "--baseline-worktree",
            str(self.repo),
            "--baseline-revision",
            self.revision,
            "--baseline-binary",
            str(baseline),
            "--candidate-worktree",
            str(self.repo),
            "--candidate-revision",
            self.revision,
            "--candidate-binary",
            str(candidate),
            "--output",
            str(output),
            "--run-root",
            str(self.root / "runs"),
        ]
        for key, value in options.items():
            command.extend([f"--{key.replace('_', '-')}", value])
        return command

    def receipt(self, output: Path) -> list[dict[str, object]]:
        return [json.loads(line) for line in output.read_text(encoding="utf-8").splitlines()]

    def test_success_records_identity_warmups_and_balanced_pairs(self) -> None:
        payload = {"schema": 1, "semantic": {"value": "same"}, "elapsed_ns": 123}
        baseline = self.mock("baseline", payload)
        candidate = self.mock("candidate", {**payload, "elapsed_ns": 456})
        manifest = self.root / "manifest.json"
        self.manifest(manifest)
        output = self.root / "receipt.jsonl"
        result = subprocess.run(
            self.command(manifest, baseline, candidate, output, pairs="2", warmups="1"),
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        records = self.receipt(output)
        metadata = records[0]
        self.assertEqual(metadata["lane"], "release-unobserved")
        self.assertEqual(metadata["baseline"]["revision"], self.revision)
        self.assertEqual(metadata["candidate"]["revision"], self.revision)
        self.assertEqual(metadata["build"]["target"], "test")
        self.assertEqual(metadata["toolchain"]["rustc"], "test-rustc")
        self.assertEqual(len(metadata["workload_manifest_sha256"]), 64)
        samples = [record for record in records if record["record"] == "sample"]
        measured = [record for record in samples if record["role"] == "measure"]
        self.assertEqual(len(samples), 8)
        self.assertTrue(all(record["schema"] == 1 and record["lane"] == "release-unobserved" for record in measured))
        self.assertEqual([record["order"] for record in measured], ["AB", "AB", "BA", "BA"])
        self.assertTrue(all(isinstance(record["wall_ns"], int) for record in measured))
        self.assertEqual(
            [record["inner_metrics"]["elapsed_ns"]["values"]["0"] for record in measured],
            [123, 456, 456, 123],
        )
        pairs = [record for record in records if record["record"] == "pair"]
        self.assertEqual([record["order"] for record in pairs], ["AB", "BA"])
        self.assertTrue(all(record["schema"] == 1 and record["lane"] == "release-unobserved" for record in pairs))
        self.assertEqual(
            [record["inner_metrics"]["elapsed_ns"]["values"]["0"]["delta"] for record in pairs],
            [333, 333],
        )
        summary = records[-1]
        self.assertEqual(summary["status"], "PASS")
        self.assertEqual(summary["workloads"][0]["pair_count"], 2)
        self.assertEqual(summary["workloads"][0]["paired"]["wall_ns"]["delta"]["count"], 2)
        self.assertEqual(summary["workloads"][0]["inner"]["elapsed_ns"]["records"]["0"]["paired"]["delta"]["mean"], 333)

    def test_revision_mismatch_fails_before_executing_binary(self) -> None:
        marker = self.root / "executed"
        binary = self.mock("binary", {"schema": 1, "semantic": {"value": "same"}})
        binary.write_text(binary.read_text(encoding="utf-8") + f"\nPath({str(marker)!r}).touch()\n", encoding="utf-8")
        manifest = self.root / "manifest.json"
        self.manifest(manifest)
        output = self.root / "receipt.jsonl"
        command = self.command(manifest, binary, binary, output)
        command[command.index("--candidate-revision") + 1] = "0" * 40
        result = subprocess.run(command, check=False, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requested", result.stderr)
        self.assertFalse(marker.exists())
        self.assertFalse(output.exists())

    def test_jsonl_inner_metrics_are_keyed_separately_from_semantics(self) -> None:
        rows = [
            {"schema": "command-core-matrix-v1", "profile": "release", "iterations": 10},
            {"case": "plain", "storage": "raw", "delivery": "direct", "observer": "disabled", "elapsed_ns": 10, "checksum": "same"},
            {"case": "stored", "storage": "stored", "delivery": "direct", "observer": "disabled", "elapsed_ns": 20, "checksum": "same"},
        ]
        baseline = self.mock_jsonl("jsonl-baseline", rows)
        candidate = self.mock_jsonl(
            "jsonl-candidate",
            [
                {**row, "elapsed_ns": row["elapsed_ns"] * 2}
                if "elapsed_ns" in row
                else row
                for row in rows
            ],
        )
        manifest = self.root / "jsonl-manifest.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "lane": "release-unobserved",
                    "profile": "release",
                    "features": [],
                    "instrumented": False,
                    "workloads": [
                        {
                            "name": "jsonl-matrix",
                            "args": [],
                            "files": [],
                            "semantic": {
                                "stream": "stdout",
                                "format": "jsonl",
                                "ignore_keys": ["elapsed_ns"],
                            },
                            "inner_metrics": [
                                {
                                    "name": "elapsed_ns",
                                    "path": ["elapsed_ns"],
                                    "key": ["case", "storage", "delivery", "observer"],
                                    "unit": "ns",
                                }
                            ],
                            "outputs": [{"path": "result.bin"}],
                        }
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )
        output = self.root / "jsonl-receipt.jsonl"
        result = subprocess.run(self.command(manifest, baseline, candidate, output, pairs="1", warmups="0"), check=False, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        records = self.receipt(output)
        pair = next(record for record in records if record["record"] == "pair")
        key = '{"case":"plain","delivery":"direct","observer":"disabled","storage":"raw"}'
        self.assertEqual(pair["inner_metrics"]["elapsed_ns"]["values"][key]["delta"], 10)
        summary = records[-1]["workloads"][0]["inner"]["elapsed_ns"]["records"][key]
        self.assertEqual(summary["paired"]["delta"]["mean"], 10)

    def test_semantic_mismatch_writes_failed_receipt(self) -> None:
        baseline = self.mock("baseline", {"schema": 1, "semantic": {"value": "same"}, "elapsed_ns": 1})
        candidate = self.mock("candidate", {"schema": 1, "semantic": {"value": "different"}, "elapsed_ns": 2})
        manifest = self.root / "manifest.json"
        self.manifest(manifest)
        output = self.root / "receipt.jsonl"
        result = subprocess.run(self.command(manifest, baseline, candidate, output, pairs="1", warmups="0"), check=False, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        records = self.receipt(output)
        self.assertEqual(records[-1]["status"], "FAIL")
        self.assertIn("semantic", records[-1]["error"])

    def test_release_lane_rejects_instrumented_manifest(self) -> None:
        binary = self.mock("binary", {"schema": 1, "semantic": {"value": "same"}})
        manifest = self.root / "manifest.json"
        self.manifest(manifest)
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["instrumented"] = True
        value["features"] = ["profiling"]
        manifest.write_text(json.dumps(value), encoding="utf-8")
        output = self.root / "receipt.jsonl"
        result = subprocess.run(self.command(manifest, binary, binary, output), check=False, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("instrumented", result.stderr)
        self.assertFalse(output.exists())

    def test_process_failure_and_timeout_are_failures(self) -> None:
        baseline = self.mock("baseline", {"schema": 1, "semantic": {"value": "same"}}, exit_code=7)
        candidate = self.mock("candidate", {"schema": 1, "semantic": {"value": "same"}})
        manifest = self.root / "manifest.json"
        self.manifest(manifest)
        output = self.root / "exit-receipt.jsonl"
        result = subprocess.run(self.command(manifest, baseline, candidate, output, pairs="1", warmups="0"), check=False, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exit status", result.stderr)

        slow = self.mock("slow", {"schema": 1, "semantic": {"value": "same"}}, sleep=1.0)
        timeout_output = self.root / "timeout-receipt.jsonl"
        result = subprocess.run(self.command(manifest, slow, candidate, timeout_output, pairs="1", warmups="0", timeout_seconds="0.05"), check=False, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("timeout", result.stderr)

    def test_summary_uses_integer_median_and_paired_deltas(self) -> None:
        samples = [
            {"role": "measure", "side": "baseline", "wall_ns": 10, "user_cpu_ns": 5, "system_cpu_ns": 1, "max_rss_kib": 2},
            {"role": "measure", "side": "baseline", "wall_ns": 30, "user_cpu_ns": 15, "system_cpu_ns": 3, "max_rss_kib": 4},
            {"role": "measure", "side": "candidate", "wall_ns": 8, "user_cpu_ns": 4, "system_cpu_ns": 1, "max_rss_kib": 2},
            {"role": "measure", "side": "candidate", "wall_ns": 20, "user_cpu_ns": 10, "system_cpu_ns": 2, "max_rss_kib": 3},
        ]
        pairs = [
            {"baseline": {"wall_ns": 10, "user_cpu_ns": 5, "system_cpu_ns": 1, "max_rss_kib": 2}, "candidate": {"wall_ns": 8, "user_cpu_ns": 4, "system_cpu_ns": 1, "max_rss_kib": 2}},
            {"baseline": {"wall_ns": 30, "user_cpu_ns": 15, "system_cpu_ns": 3, "max_rss_kib": 4}, "candidate": {"wall_ns": 20, "user_cpu_ns": 10, "system_cpu_ns": 2, "max_rss_kib": 3}},
        ]
        summary = paired_measure.summarize_workload("mock", samples, pairs)
        self.assertEqual(summary["sides"]["baseline"]["wall_ns"]["median"], 20)
        self.assertEqual(summary["paired"]["wall_ns"]["delta"]["median"], -6)
        self.assertEqual(summary["paired"]["wall_ns"]["delta"]["mean"], -6)


if __name__ == "__main__":
    unittest.main()
