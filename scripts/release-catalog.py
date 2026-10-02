#!/usr/bin/env python3
"""Merge and validate the per-platform catalogs before publishing a release."""

import json
from pathlib import Path
import sys

import hub_download


def combine(directory):
    entries = []
    catalogs = sorted(directory.glob("catalog-*.json"))
    if len(catalogs) != 3:
        raise ValueError("Expected one catalog for each desktop platform")
    for path in catalogs:
        entries.extend(json.loads(path.read_bytes())["releases"])
    result = {"schema": 1, "releases": entries}
    hub_download.catalog_data(result, "")
    if len({entry["version"] for entry in entries}) != 1 or len(entries) != 3:
        raise ValueError("All platform packages must share one release version")
    for entry in entries:
        hub_download.verify_file(directory / entry["asset"], entry["sha256"], entry["size"])
    (directory / "blockloom-catalog.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    combine(Path(sys.argv[1]))
