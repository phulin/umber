"""Declared numeric metric extraction for paired JSON/JSONL workloads."""

from __future__ import annotations

import json
import math
import statistics
from typing import Any, Iterable, Mapping


def _record_key(record: Mapping[str, Any], index: int, fields: list[str]) -> str:
    if not fields:
        return str(index)
    return json.dumps(
        {field: record[field] for field in fields},
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    )


def _path_value(record: Mapping[str, Any], path: list[str]) -> Any:
    value: Any = record
    for field in path:
        if not isinstance(value, dict) or field not in value:
            raise KeyError(field)
        value = value[field]
    return value


def _is_number(value: Any) -> bool:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    return not isinstance(value, float) or math.isfinite(value)


def extract_inner_metrics(
    records: Iterable[Any], specs: Iterable[Mapping[str, Any]]
) -> tuple[dict[str, Any], list[str]]:
    rows = list(records)
    extracted: dict[str, Any] = {}
    errors: list[str] = []
    for spec in specs:
        name = str(spec["name"])
        path = [str(field) for field in spec["path"]]
        fields = [str(field) for field in spec.get("key", [])]
        values: dict[str, int | float] = {}
        for index, record in enumerate(rows):
            if not isinstance(record, dict):
                errors.append(f"inner metric {name} record {index} is not an object")
                continue
            if fields and any(field not in record for field in fields):
                continue
            try:
                key = _record_key(record, index, fields)
                value = _path_value(record, path)
            except (KeyError, TypeError) as error:
                errors.append(f"inner metric {name} record {index} is missing {error}")
                continue
            if not _is_number(value):
                errors.append(f"inner metric {name} record {index} is not a finite number")
                continue
            if key in values:
                errors.append(f"inner metric {name} has duplicate record key {key}")
                continue
            values[key] = value
        extracted[name] = {"unit": spec.get("unit"), "values": values}
    return extracted, errors


def compare_inner_shapes(
    sample: Mapping[str, Any], reference: Mapping[str, Any]
) -> list[str]:
    current = sample.get("inner_metrics", {})
    expected = reference.get("inner_metrics", {})
    errors: list[str] = []
    if set(current) != set(expected):
        errors.append("inner metric names differ from baseline")
        return errors
    for name in expected:
        current_metric = current[name]
        expected_metric = expected[name]
        if current_metric.get("unit") != expected_metric.get("unit"):
            errors.append(f"inner metric unit differs from baseline: {name}")
        if set(current_metric.get("values", {})) != set(expected_metric.get("values", {})):
            errors.append(f"inner metric record keys differ from baseline: {name}")
    return errors


def _ratio(candidate: int | float, baseline: int | float) -> float | None:
    return candidate / baseline if baseline else None


def pair_inner_metrics(
    baseline: Mapping[str, Any], candidate: Mapping[str, Any]
) -> dict[str, Any]:
    paired: dict[str, Any] = {}
    for name in sorted(set(baseline) | set(candidate)):
        baseline_metric = baseline.get(name, {"unit": None, "values": {}})
        candidate_metric = candidate.get(name, {"unit": None, "values": {}})
        values: dict[str, Any] = {}
        for key in sorted(set(baseline_metric.get("values", {})) | set(candidate_metric.get("values", {}))):
            before = baseline_metric.get("values", {}).get(key)
            after = candidate_metric.get("values", {}).get(key)
            values[key] = {
                "baseline": before,
                "candidate": after,
                "delta": after - before if before is not None and after is not None else None,
                "ratio": _ratio(after, before) if before is not None and after is not None else None,
            }
        paired[name] = {"unit": baseline_metric.get("unit"), "values": values}
    return paired


def _number(value: int | float) -> int | float:
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return value


def number_summary(values: Iterable[int | float]) -> dict[str, Any]:
    ordered = sorted(values)
    if not ordered:
        return {"count": 0}
    return {
        "count": len(ordered),
        "min": ordered[0],
        "median": _number(statistics.median(ordered)),
        "mean": _number(statistics.mean(ordered)),
        "max": ordered[-1],
    }


def summarize_inner_metrics(
    samples: Iterable[Mapping[str, Any]], pairs: Iterable[Mapping[str, Any]]
) -> dict[str, Any]:
    measured = [sample for sample in samples if sample.get("role") == "measure"]
    pair_rows = list(pairs)
    names = sorted(
        {
            name
            for sample in measured
            for name in sample.get("inner_metrics", {})
        }
    )
    summary: dict[str, Any] = {}
    for name in names:
        first = next(sample["inner_metrics"][name] for sample in measured if name in sample.get("inner_metrics", {}))
        keys = sorted(
            {
                key
                for sample in measured
                for key in sample.get("inner_metrics", {}).get(name, {}).get("values", {})
            }
        )
        records: dict[str, Any] = {}
        for key in keys:
            side_values = {
                side: [
                    sample["inner_metrics"][name]["values"][key]
                    for sample in measured
                    if sample["side"] == side
                    and key in sample.get("inner_metrics", {}).get(name, {}).get("values", {})
                ]
                for side in ("baseline", "candidate")
            }
            deltas = [
                row["inner_metrics"][name]["values"][key]["delta"]
                for row in pair_rows
                if row.get("inner_metrics", {}).get(name, {}).get("values", {}).get(key, {}).get("delta")
                is not None
            ]
            ratios = [
                row["inner_metrics"][name]["values"][key]["ratio"]
                for row in pair_rows
                if row.get("inner_metrics", {}).get(name, {}).get("values", {}).get(key, {}).get("ratio")
                is not None
            ]
            records[key] = {
                "baseline": number_summary(side_values["baseline"]),
                "candidate": number_summary(side_values["candidate"]),
                "paired": {
                    "delta": number_summary(deltas),
                    "ratio": number_summary(ratios),
                },
            }
        summary[name] = {"unit": first.get("unit"), "records": records}
    return summary
