#!/usr/bin/env python3
"""Check that every release license decision has a matching notice policy and graph."""

from __future__ import annotations

from pathlib import Path
import runpy
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]


def accepted_licenses(path: Path, key: str) -> set[str]:
    with path.open("rb") as source:
        document = tomllib.load(source)
    value = document
    for segment in key.split("."):
        value = value[segment]
    return set(value)


def missing_notice_graphs(audited: list[str], graph_definitions: dict) -> list[str]:
    # A target or feature change can select a different MPL crate. Require the
    # corresponding notice and source checks for the very same Cargo graph.
    covered = {(manifest, target, frozenset(graph_definitions["GRAPH_FEATURES"].get(name, ())))
               for name, manifest, target in graph_definitions["GRAPHS"]}
    missing = []
    for row in audited:
        _, manifest, target, *features = row.split("|")
        if (manifest, target, frozenset(" ".join(features).split())) not in covered:
            missing.append(row)
    return missing


def main() -> int:
    deny = accepted_licenses(ROOT / "deny.toml", "licenses.allow")
    notices = accepted_licenses(ROOT / "about.toml", "accepted")
    if sys.argv[1:]:
        graphs = runpy.run_path(str(ROOT / "tools/repo/generate-third-party-notices.py"))
        missing = missing_notice_graphs(sys.argv[1:], graphs)
        if missing:
            print("Release Cargo graphs missing corresponding notice/source coverage:", *missing,
                  sep="\n", file=sys.stderr)
            return 1
    if deny == notices:
        print("LICENSE_POLICY_PARITY_OK")
        return 0

    for license_id in sorted(deny - notices):
        print(f"cargo-about does not accept cargo-deny license: {license_id}", file=sys.stderr)
    for license_id in sorted(notices - deny):
        print(f"cargo-deny does not accept cargo-about license: {license_id}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
