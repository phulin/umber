#!/usr/bin/env python3
"""Verify the corpus's used TeX Live resources without physical-path pins."""

import argparse
from pathlib import Path
import sys

import latex_input_admissions as admissions


GENERATED_KEYS = frozenset(
    f"tex:document.{extension}" for extension in ("aux", "toc", "lof", "lot", "out")
)


def verify(
    receipt: Path,
    authorized: dict[str, admissions.Identity],
    main: admissions.Identity,
    workdir: Path,
) -> list[tuple[str, admissions.Identity]]:
    observed_main, files = admissions.read_receipt(receipt)
    if observed_main != main:
        raise ValueError(f"{receipt}: main input differs from committed document source")
    runtime: list[tuple[str, admissions.Identity]] = []
    for status, key, observed in files:
        if status != "used":
            continue
        if key == "tex:document.tex":
            if observed != main:
                raise ValueError(f"{receipt}: consumed main input has a different identity")
        elif key in authorized:
            if observed != authorized[key]:
                raise ValueError(f"{receipt}: consumed {key} differs from locked runtime identity")
            runtime.append((key, observed))
        elif key in GENERATED_KEYS and (workdir / key.removeprefix("tex:")).is_file():
            continue
        else:
            raise ValueError(f"{receipt}: consumed {key} is outside the locked runtime closure")
    return runtime


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--authorized", type=Path, required=True)
    parser.add_argument("--main-bytes", required=True)
    parser.add_argument("--main-ahash64", required=True)
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--used-out", type=Path, required=True)
    args = parser.parse_args()
    try:
        main_identity = admissions.identity(args.main_bytes, args.main_ahash64)
        used = verify(
            args.receipt,
            admissions.read_authorized(args.authorized),
            main_identity,
            args.workdir,
        )
        args.used_out.write_text(
            "".join(f"{key}\t{value[1]}\t{value[0]}\n" for key, value in used)
        )
    except (OSError, UnicodeError, ValueError) as error:
        print(f"verify-latex-corpus-inputs.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
