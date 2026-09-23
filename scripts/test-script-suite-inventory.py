#!/usr/bin/env python3
"""Keep script-suite discovery and aggregate ownership in exact agreement."""

from pathlib import Path


root = Path(__file__).resolve().parent.parent
scripts = root / "scripts"
inventory = scripts / "script-suite-inventory.tsv"
rows = {}
for raw in inventory.read_text().splitlines():
    if not raw or raw.startswith("#"):
        continue
    fields = raw.split("\t")
    assert len(fields) == 5, f"invalid inventory row: {raw}"
    path, lane, case_class, owner, prerequisites = fields
    assert path not in rows, f"duplicate inventory row: {path}"
    assert lane in {"routine", "subsystem", "manual"}, (path, lane)
    assert case_class and prerequisites, path
    test = root / path
    assert test.is_file(), f"inventory names absent test: {path}"
    if lane == "manual":
        assert owner == "-", f"manual suite has aggregate owner: {path}"
    else:
        assert owner != path, f"suite owns itself: {path}"
        owner_path = root / owner
        assert owner_path.is_file(), f"aggregate owner missing: {owner}"
        assert test.name in owner_path.read_text(), (
            f"{path} is inventoried under {owner}, but the owner does not select it"
        )
    rows[path] = (lane, case_class, owner, prerequisites)

discovered = {
    str(path.relative_to(root))
    for path in scripts.iterdir()
    if path.name.startswith("test-") and path.suffix in {".sh", ".py"}
}
assert discovered == rows.keys(), (
    f"script test discovery drift: unowned={sorted(discovered - rows.keys())}, "
    f"absent={sorted(rows.keys() - discovered)}"
)
print(f"script suite inventory: PASS ({len(rows)} discovered suites)")
