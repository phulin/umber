#!/usr/bin/env python3
"""Count user-space instructions per primitive in a tight LaTeX loop.

Each case runs ``\\iter`` (``\\ifnum``, ``\\advance``, ``\\expandafter`` and a
self-call) around one snippet for ``--iterations`` rounds in the LaTeX
preamble. The per-iteration cost is the instruction difference between that
run and a zero-iteration run, so startup and format loading cancel. Instruction
counts are near-deterministic, which makes them a better regression signal
than wall time on a shared host.

Engines are Umber executables given with ``--umber LABEL=PATH`` and, with
``--pdftex``, the pinned pdfTeX oracle from the authority record. Umber runs
use the authority record's distribution and format.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# Corpus rows and the pinned oracle live in the primary checkout's target,
# which linked worktrees share through the common git directory.
PRIMARY = Path(subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                              cwd=REPO, capture_output=True, text=True, check=True).stdout.strip()).parent
DEFAULT_AUTHORITY = PRIMARY / "target/arxiv-pdf-wave3-final/rows/2606.24937/result.json"
DEFAULT_PDFTEX = PRIMARY / "target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean"

CASES = {
    "base": "",
    "macro0": r"\mzero",
    "macro_delim": r"\mdelim a.b\stop",
    "let": r"\let\x\relax",
    "def": r"\def\x{abc}",
    "edef": r"\edef\x{\mzero abc}",
    "futurelet": r"\futurelet\x\mzero\relax",
    "expandafter": r"\expandafter\relax\mzero",
    "ifx": r"\ifx\x\relax\fi",
    "ifnum_false": r"\ifnum1>2 \fi",
    "skip": r"\iffalse\relax\mzero\def\x{ab}\let\y\z\else\fi",
    "csname": r"\csname relax\endcsname",
    "string": r"\edef\x{\string\relax}",
    "the": r"\edef\x{\the\count255}",
    "advance": r"\advance\count2 by 1 ",
    "dimen": r"\dimen2=1pt \advance\dimen2 by 2pt ",
    "toks": r"\toks2={abc}",
    "group": r"\begingroup\endgroup",
    "group_let": r"\begingroup\let\y\relax\endgroup",
    "setbox": r"\setbox2\hbox{}",
    "box_wd": r"\setbox2\hbox{}\wd2=1pt ",
}


def document(snippet: str, iterations: int) -> str:
    return "\n".join((
        r"\documentclass{article}",
        r"\def\mzero{}",
        r"\def\mdelim#1.#2\stop{}",
        r"\count255=0 \def\iter{\ifnum\count255<" + str(iterations)
        + r" \advance\count255 by 1 " + snippet + r"\expandafter\iter\fi}\iter",
        r"\begin{document}x\end{document}",
        "",
    ))


def load_reference_environment():
    sys.path.insert(0, str(REPO / "scripts"))
    spec = importlib.util.spec_from_file_location("run_arxiv_texlive", REPO / "scripts/run-arxiv-texlive.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.reference_environment


def perf_instructions(command: list[str], cwd: Path, env: dict[str, str], arguments) -> int:
    runner = ["taskset", "-c", arguments.cpu] if arguments.cpu else []
    if arguments.sudo:
        prefix = ["sudo", "-n", "-E", "perf", "stat", "-x,", "-e", "instructions:u", "--",
                  "sudo", "-n", "-E", "-u", os.environ["USER"], *runner]
    else:
        prefix = ["perf", "stat", "-x,", "-e", "instructions:u", "--", *runner]
    result = subprocess.run(prefix + command, cwd=cwd, env=env, capture_output=True, text=True)
    if not (cwd / "t.log").exists() and not list(cwd.glob("*.pdf")):
        raise RuntimeError(f"{command} produced no output (exit {result.returncode}):\n{result.stderr[-2000:]}")
    for line in result.stderr.splitlines():
        if "instructions" in line and line.split(",")[0].isdigit():
            return int(line.split(",")[0])
    raise RuntimeError(f"no instruction count from {command[0]} (exit {result.returncode}):\n{result.stderr[-2000:]}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--umber", action="append", default=[], metavar="LABEL=PATH",
                        help="Umber executable to measure; repeat to compare builds")
    parser.add_argument("--pdftex", action="store_true", help="also measure the pinned pdfTeX oracle")
    parser.add_argument("--authority", type=Path, default=DEFAULT_AUTHORITY,
                        help="arXiv row result.json naming the distribution and formats")
    parser.add_argument("--cases", help="comma-separated subset of: " + ",".join(CASES))
    parser.add_argument("--iterations", type=int, default=100_000)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--cpu", help="taskset CPU list for measured processes")
    parser.add_argument("--sudo", action="store_true",
                        help="run perf through sudo -n when perf_event_paranoid forbids user counting")
    parser.add_argument("--json", type=Path, help="write raw per-iteration results here")
    arguments = parser.parse_args()

    authority = json.loads(arguments.authority.read_text())["authority"]
    engines = [(label, str(Path(path).resolve()))
               for label, path in (spec.split("=", 1) for spec in arguments.umber)]
    if arguments.pdftex:
        engines.append(("pdftex", str(DEFAULT_PDFTEX)))
    if not engines:
        parser.error("give at least one --umber LABEL=PATH or --pdftex")
    cases = arguments.cases.split(",") if arguments.cases else list(CASES)
    if "base" not in cases:
        cases.insert(0, "base")

    work = Path(tempfile.mkdtemp(prefix="interp-microbench-"))
    reference_env = None
    if arguments.pdftex:
        reference_environment = load_reference_environment()
        reference_env = reference_environment(
            Path(authority["runtime_root"]), work, Path(authority["reference_format"]["path"]),
            authority["source_date_epoch"], Path(authority["reference_view"]),
            Path(authority["generated_fontmaps"]))
    umber_env = dict(os.environ)

    def measure(engine: tuple[str, str], case: str, iterations: int) -> int:
        label, path = engine
        run = work / label / f"{case}_{iterations}"
        run.mkdir(parents=True, exist_ok=True)
        (run / "t.tex").write_text(document(CASES[case], iterations))
        if label == "pdftex":
            env = dict(reference_env)
            env["TEXINPUTS"] = str(run) + ":" + env["TEXINPUTS"]
            command = [path, "-progname=pdflatex", "--fmt=" + authority["reference_format"]["path"],
                       "--output-format=pdf", "--interaction=nonstopmode", "t.tex"]
        else:
            env = umber_env
            # sudo may reset HOME, which locates Umber's user cache.
            command = ["env", "HOME=" + os.environ["HOME"], path, "run", "--pdflatex", "--distribution", authority["umber_distribution"],
                       "--distribution-ahash64", authority["distribution_ahash64"],
                       "--format", authority["umber_format"]["path"], "--offline",
                       "--expansion-fuel", "4000000000", "--execution-steps", "100000000",
                       "--pdf", str(run / "t.pdf"), "t.tex"]
        return perf_instructions(command, run, env, arguments)

    jobs = [(engine, case, n) for engine in engines for case in cases for n in (0, arguments.iterations)]
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        counts = dict(zip(jobs, pool.map(lambda job: measure(*job), jobs)))
    per_iteration = {
        engine[0]: {case: (counts[(engine, case, arguments.iterations)] - counts[(engine, case, 0)])
                    / arguments.iterations for case in cases}
        for engine in engines
    }

    labels = [engine[0] for engine in engines]
    print(f"{'case':<12}" + "".join(f"{label:>14}" for label in labels)
          + ("   ratio(first/last)" if len(labels) > 1 else ""))
    for case in cases:
        values = []
        for label in labels:
            value = per_iteration[label][case]
            values.append(value if case == "base" else value - per_iteration[label]["base"])
        ratio = f"{values[0] / values[-1]:>10.2f}x" if len(values) > 1 and values[-1] > 0 else ""
        name = case if case == "base" else f"+{case}"
        print(f"{name:<12}" + "".join(f"{value:>14.0f}" for value in values) + ratio)
    if arguments.json:
        arguments.json.write_text(json.dumps(per_iteration, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
