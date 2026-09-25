"""Parse pdfTeX's bounded PDF completion report, including line wraps."""

from __future__ import annotations

import re


def pdf_completion(log: str, jobname: str) -> tuple[int, int] | None:
    """Read TeX's bounded completion line, including max_print_line wraps."""
    prefix = "Output written on "
    start = log.rfind(prefix)
    if start < 0:
        return None
    completion = log[start:start + 4096].split("\nTranscript written on ", 1)[0]
    completion = completion.replace("\r\n", "").replace("\n", "")
    match = re.match(
        rf"Output written on {re.escape(jobname)}\.pdf \((\d+) pages?, (\d+) bytes\)\.",
        completion,
    )
    return (int(match.group(1)), int(match.group(2))) if match else None


