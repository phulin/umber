#!/usr/bin/env python3
"""Marginal user-space instructions per microbench iteration for the prototype.

Uses the interp-microbench method: (instructions at --long minus --short
iterations) / span, minus the base loop. Compare with
`scripts/interp-microbench.py --pdftex --umber ...` for the reference engines.
"""
import argparse
import subprocess

CASES = ("base relax mzero mbody mbody64 mone mone_brace mtwo mdelim let def group brace64 "
         "group_let advance count toks").split()


def instructions(binary: str, case: str, n: int, sudo: bool, cpu: str | None) -> int:
    command = ["perf", "stat", "-x,", "-e", "instructions:u", "--"]
    if cpu:
        command += ["taskset", "-c", cpu]
    if sudo:
        command = ["sudo", "-n", *command]
    result = subprocess.run(command + [binary, case, str(n)], capture_output=True, text=True, check=True)
    for line in result.stderr.splitlines():
        fields = line.split(",")
        if len(fields) > 2 and "instructions" in fields[2]:
            return int(fields[0])
    raise SystemExit(result.stderr)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("--short", type=int, default=50_000)
    parser.add_argument("--long", type=int, default=250_000)
    parser.add_argument("--sudo", action="store_true")
    parser.add_argument("--cpu")
    args = parser.parse_args()
    span = args.long - args.short
    per = {case: (instructions(args.binary, case, args.long, args.sudo, args.cpu)
                  - instructions(args.binary, case, args.short, args.sudo, args.cpu)) / span
           for case in CASES}
    for case in CASES:
        value = per[case] if case == "base" else per[case] - per["base"]
        print(f"{case:<12}{value:>8.0f}")


if __name__ == "__main__":
    main()
