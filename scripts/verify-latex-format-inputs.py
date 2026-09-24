#!/usr/bin/env python3
"""Check consumed format resources against a SHA-verified construction closure."""

import argparse
from pathlib import Path
import sys

import latex_input_admissions as admissions


def verify(receipt: Path, authorized: dict[str, admissions.Identity], main_key: str) -> None:
    main, files = admissions.read_receipt(receipt)
    if main_key not in authorized or main != authorized[main_key]:
        raise ValueError(f"{receipt}: main input differs from locked {main_key}")
    for status, key, observed in files:
        if status == "used" and authorized.get(key) != observed:
            raise ValueError(f"{receipt}: consumed {key} is outside the locked source closure")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--authorized", type=Path, required=True)
    parser.add_argument("--main-key", required=True)
    args = parser.parse_args()
    try:
        verify(args.receipt, admissions.read_authorized(args.authorized), args.main_key)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"verify-latex-format-inputs.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
