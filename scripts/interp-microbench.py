#!/usr/bin/env python3
"""Count user-space instructions per primitive in a tight initex loop.

Each case runs ``\\iter`` (``\\ifnum``, ``\\advance``, ``\\expandafter`` and a
self-call) around one snippet in an initex job with no format. The per-iteration
cost is the instruction difference between a short and a long run, so startup
cancels. ``base`` is the loop alone; every other row is the snippet's cost
above ``base``. Instruction counts are near-deterministic, which makes them a
better regression signal than wall time on a shared host.

Engines are Umber executables given with ``--umber LABEL=PATH`` and, with
``--pdftex``, the pinned clean pdfTeX oracle in e-TeX initex mode. The report
compares the first Umber engine with the per-operation targets in
docs/minimal_hot_loop.md; ``--json`` writes a receipt with the engine identities.
Counter access usually needs ``--sudo``, so this is a measured review gate and
not part of the routine test suite.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import hashlib
import json
import os
import platform
import subprocess
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# The pinned oracle lives in the primary checkout's target, which linked
# worktrees share through the common git directory.
PRIMARY = Path(subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                              cwd=REPO, capture_output=True, text=True, check=True).stdout.strip()).parent
DEFAULT_PDFTEX = PRIMARY / "target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean"

# Case name -> (snippet, per-operation target from docs/minimal_hot_loop.md).
CASES: dict[str, tuple[str, int | None]] = {
    "base": ("", 4500),
    "relax": (r"\relax", 100),
    "mzero": (r"\mzero", 150),
    "mbody": (r"\mbody", 350),
    "mone": (r"\mone x", 350),
    "mone_brace": (r"\mone{xy}", None),
    "mtwo": (r"\mtwo\relax\relax", 900),
    "mdelim": (r"\mdelim a.b\stop", 700),
    "let": (r"\let\x\relax", 350),
    "def": (r"\def\x{abc}", 900),
    "edef": (r"\edef\x{\mzero abc}", 1200),
    "ifx": (r"\ifx\x\relax\fi", None),
    "iffalse": (r"\iffalse\relax\mzero\def\x{ab}\let\y\z\else\fi", None),
    "expandafter": (r"\expandafter\relax\mzero", None),
    "csname": (r"\csname relax\endcsname", None),
    "group": (r"\begingroup\endgroup", 300),
    "group_let": (r"\begingroup\let\y\relax\endgroup", None),
    "advance": (r"\advance\count2 by 1 ", 1200),
    "count": (r"\count2=5 ", 900),
    "dimen": (r"\dimen2=1pt \advance\dimen2 by 2pt ", None),
    "toks": (r"\toks2={abc}", 1500),
    "setbox": (r"\setbox2\hbox{}", None),
}

# Enough of kpathsea's configuration for the oracle to run initex in a
# scratch directory without the distribution tree.
TEXMF_CNF = "TEXINPUTS = .\nTEXFORMATS = .\nTFMFONTS = .\nTEXMFCNF = .\n"


def document(snippet: str, iterations: int) -> str:
    return "\n".join((
        r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \count255=0",
        r"\def\mzero{}\def\mone#1{}\def\mtwo#1#2{#2#1}\def\mdelim#1.#2\stop{}",
        r"\def\mbody{\relax\relax}\let\x\relax",
        r"\def\iter{\ifnum\count255<" + str(iterations)
        + r" \advance\count255 by 1 " + snippet + r"\expandafter\iter\fi}\iter",
        r"\end",
        "",
    ))


def file_identity(path: Path) -> dict[str, str]:
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"path": str(path), "sha256": digest}


def perf_instructions(command: list[str], cwd: Path, env: dict[str, str], cpu: str | None, sudo: bool) -> int:
    runner = ["taskset", "-c", cpu] if cpu else []
    if sudo:
        prefix = ["sudo", "-n", "-E", "perf", "stat", "-x,", "-e", "instructions:u", "--",
                  "sudo", "-n", "-E", "-u", os.environ["USER"], *runner]
    else:
        prefix = ["perf", "stat", "-x,", "-e", "instructions:u", "--", *runner]
    result = subprocess.run(prefix + command, cwd=cwd, env=env, capture_output=True, text=True)
    # pdfTeX in batchmode reports only to its log; Umber reports to stdout.
    log = cwd / "t.log"
    transcript = log.read_text(errors="replace") if log.exists() else result.stdout
    if result.returncode != 0 or "\n! " in transcript:
        raise RuntimeError(f"{command} reported an error:\n{transcript[-2000:]}")
    for line in result.stderr.splitlines():
        if "instructions" in line and line.split(",")[0].isdigit():
            return int(line.split(",")[0])
    raise RuntimeError(f"no instruction count from {command[0]} (exit {result.returncode}):\n{result.stderr[-2000:]}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--umber", action="append", default=[], metavar="LABEL=PATH",
                        help="Umber executable to measure; repeat to compare builds")
    parser.add_argument("--pdftex", action="store_true", help="also measure the pinned pdfTeX oracle")
    parser.add_argument("--pdftex-path", type=Path, default=DEFAULT_PDFTEX)
    parser.add_argument("--cases", help="comma-separated subset of: " + ",".join(CASES))
    parser.add_argument("--short", type=int, default=50_000, help="iterations in the short run")
    parser.add_argument("--long", type=int, default=250_000, help="iterations in the long run")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--cpu", help="taskset CPU list for measured processes")
    parser.add_argument("--sudo", action="store_true",
                        help="run perf through sudo -n when perf_event_paranoid forbids user counting")
    parser.add_argument("--json", type=Path, help="write the receipt here")
    arguments = parser.parse_args()
    if arguments.long <= arguments.short:
        parser.error("--long must exceed --short")

    engines = [(label, Path(path).resolve())
               for label, path in (spec.split("=", 1) for spec in arguments.umber)]
    if arguments.pdftex:
        engines.append(("pdftex", arguments.pdftex_path.resolve()))
    if not engines:
        parser.error("give at least one --umber LABEL=PATH or --pdftex")
    cases = arguments.cases.split(",") if arguments.cases else list(CASES)
    unknown = [case for case in cases if case not in CASES]
    if unknown:
        parser.error("unknown cases: " + ",".join(unknown))
    if "base" not in cases:
        cases.insert(0, "base")

    work = Path(tempfile.mkdtemp(prefix="interp-microbench-"))
    (work / "texmf.cnf").write_text(TEXMF_CNF)

    def measure(engine: tuple[str, Path], case: str, iterations: int) -> int:
        label, path = engine
        run = work / label / f"{case}_{iterations}"
        run.mkdir(parents=True, exist_ok=True)
        (run / "t.tex").write_text(document(CASES[case][0], iterations))
        env = dict(os.environ)
        if label == "pdftex":
            env["TEXMFCNF"] = str(work)
            command = [str(path), "-ini", "-etex", "-interaction=batchmode", "t.tex"]
        else:
            # sudo may reset HOME, which locates Umber's user cache.
            command = ["env", "HOME=" + os.environ["HOME"], str(path), "run", "t.tex"]
        return perf_instructions(command, run, env, arguments.cpu, arguments.sudo)

    sizes = (arguments.short, arguments.long)
    jobs = [(engine, case, n) for engine in engines for case in cases for n in sizes]
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        counts = dict(zip(jobs, pool.map(lambda job: measure(*job), jobs)))
    span = arguments.long - arguments.short
    raw = {
        engine[0]: {case: (counts[(engine, case, arguments.long)] - counts[(engine, case, arguments.short)]) / span
                    for case in cases}
        for engine in engines
    }
    marginal = {
        label: {case: value if case == "base" else value - values["base"] for case, value in values.items()}
        for label, values in raw.items()
    }

    labels = [engine[0] for engine in engines]
    gated = labels[0] if labels[0] != "pdftex" else None
    header = f"{'case':<12}" + "".join(f"{label:>12}" for label in labels)
    if len(labels) > 1:
        header += f"{'ratio':>9}"
    if gated:
        header += f"{'target':>9}  gate"
    print(header)
    verdicts = {}
    for case in cases:
        values = [marginal[label][case] for label in labels]
        line = f"{case:<12}" + "".join(f"{value:>12.0f}" for value in values)
        if len(values) > 1:
            line += f"{values[0] / values[-1]:>8.2f}x" if values[-1] > 0 else " " * 9
        target = CASES[case][1]
        if gated and target is not None:
            verdicts[case] = values[0] <= target
            line += f"{target:>9}  {'pass' if verdicts[case] else 'FAIL'}"
        print(line)

    if arguments.json:
        receipt = {
            "schema": "umber.interp-microbench.v2",
            "created": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "revision": subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO,
                                       capture_output=True, text=True).stdout.strip(),
            "host": {"machine": platform.machine(), "node": platform.node(), "release": platform.release()},
            "iterations": {"short": arguments.short, "long": arguments.long},
            "engines": {label: file_identity(path) for label, path in engines},
            "cases": {case: {"snippet": CASES[case][0], "target": CASES[case][1]} for case in cases},
            "per_iteration": raw,
            "marginal": marginal,
            "gated_engine": gated,
            "gates": verdicts,
        }
        arguments.json.parent.mkdir(parents=True, exist_ok=True)
        arguments.json.write_text(json.dumps(receipt, indent=2) + "\n")
    return 0 if all(verdicts.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
