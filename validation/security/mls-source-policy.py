#!/usr/bin/env python3
"""Bind MLS source declarations and resolved locks to the reviewed vendor pin."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = "https://github.com/awslabs/mls-rs"
REVISION = "d4c5b19f4bbf8a931face27fc9e30f2db33ce83e"
VERSION = "0.56.0"
VENDOR = Path("prns-groups/security/mls-rs/vendor/mls-rs")
LOCK_SOURCE = f"git+{UPSTREAM}?rev={REVISION}#{REVISION}"
SKIP_DIRECTORIES = {".git", "target", "node_modules", ".cache", ".expo", ".gradle"}
DEPENDENCY_TABLES = {"dependencies", "dev-dependencies", "build-dependencies"}


def cargo_files(root: Path):
    # Also works in a source export without a .git directory. Do not follow links
    # into build caches or scan dependency-manager copies of repository source.
    for directory, names, files in os.walk(root, followlinks=False):
        names[:] = sorted(name for name in names if name not in SKIP_DIRECTORIES)
        for name in sorted(set(files) & {"Cargo.toml", "Cargo.lock"}):
            path = Path(directory) / name
            yield path


def dependency_tables(document: dict):
    for key, value in document.items():
        if not isinstance(value, dict):
            continue
        if key in DEPENDENCY_TABLES:
            yield value
        elif key in {"patch", "replace"}:
            if key == "replace":
                yield value
            else:
                yield from value.values()
        else:
            yield from dependency_tables(value)


def is_mls(name: str) -> bool:
    return name == "mls-rs" or name.startswith("mls-rs-")


def check_vendor_files(vendor: Path, provenance: dict) -> list[str]:
    """Bind every shipped vendor file to the reviewed reconstruction inventory."""
    failures: list[str] = []
    recorded = provenance.get("vendored_files_sha256")
    if not isinstance(recorded, dict) or not recorded:
        return [f"{VENDOR}/PROVENANCE.json: complete vendor file hashes are required"]
    for relative, digest in recorded.items():
        path = Path(relative)
        if (path.is_absolute() or path.as_posix() != relative or
                ".." in path.parts or relative in {".", "PROVENANCE.json"} or
                not isinstance(digest, str) or len(digest) != 64 or
                any(char not in "0123456789abcdef" for char in digest)):
            failures.append(f"{VENDOR}/PROVENANCE.json: invalid vendor file hash: {relative}")
    if failures:
        return failures
    if recorded.get("PRNS.patch") != provenance.get("patch_sha256"):
        failures.append(f"{VENDOR}/PRNS.patch: inventory must bind the reviewed patch hash")

    actual: set[str] = set()
    for directory, names, files in os.walk(vendor, followlinks=False):
        for name in names + files:
            path = Path(directory) / name
            relative = path.relative_to(vendor).as_posix()
            if path.is_symlink():
                failures.append(f"{VENDOR}/{relative}: linked vendor paths are not allowed")
            elif path.is_file():
                if relative != "PROVENANCE.json":
                    actual.add(relative)
            elif not path.is_dir():
                failures.append(f"{VENDOR}/{relative}: unsupported vendor file type")
        # The vendor's dedicated validation workspace writes only this ignored
        # build-output directory. Its source and manifest remain in the inventory.
        names[:] = [name for name in names
                    if (Path(directory) / name).relative_to(vendor).as_posix() != "validation/target"]
    missing = set(recorded) - actual
    extra = actual - set(recorded)
    if missing:
        failures.append(f"{VENDOR}: missing reviewed files: {', '.join(sorted(missing))}")
    if extra:
        failures.append(f"{VENDOR}: unreviewed files: {', '.join(sorted(extra))}")
    for relative in sorted(actual & set(recorded)):
        if hashlib.sha256((vendor / relative).read_bytes()).hexdigest() != recorded[relative]:
            failures.append(f"{VENDOR}/{relative}: source differs from reviewed reconstruction")
    return failures


def check_policy(root: Path) -> list[str]:
    failures: list[str] = []
    policy = tomllib.loads((root / "deny.toml").read_text())["sources"]
    if policy.get("unknown-git") != "deny" or UPSTREAM not in policy.get("allow-git", []):
        failures.append("deny.toml must deny unknown git sources and allow the exact MLS repository")
    vendor = root / VENDOR
    if any((root / Path(*VENDOR.parts[:index])).is_symlink()
           for index in range(1, len(VENDOR.parts) + 1)):
        return [f"{VENDOR}: linked vendor directories are not allowed"]
    if (vendor / "PROVENANCE.json").is_symlink():
        return [f"{VENDOR}/PROVENANCE.json: linked provenance is not allowed"]
    provenance = json.loads((vendor / "PROVENANCE.json").read_text())
    expected = {"upstream_repository": UPSTREAM, "upstream_revision": REVISION,
                "upstream_crate": "mls-rs", "upstream_version": VERSION}
    for key, value in expected.items():
        if provenance.get(key) != value:
            failures.append(f"{VENDOR}/PROVENANCE.json: unreviewed {key}")
    failures.extend(check_vendor_files(vendor, provenance))
    patch = vendor / "PRNS.patch"
    if (patch.is_symlink() or provenance.get("local_patch") != patch.name or
            provenance.get("patch_sha256") != hashlib.sha256(patch.read_bytes()).hexdigest()):
        failures.append(f"{VENDOR}/PRNS.patch: provenance hash mismatch")

    for path in cargo_files(root):
        relative = path.relative_to(root)
        if path.is_symlink():
            failures.append(f"{relative}: linked Cargo files cannot establish the MLS source policy")
            continue
        document = tomllib.loads(path.read_text())
        if path.name == "Cargo.lock":
            for package in document.get("package", []):
                name = package["name"]
                source = package.get("source", "")
                if not is_mls(name) and "github.com/awslabs/mls-rs" not in source:
                    continue
                if name == "mls-rs" and not source and package.get("version") == VERSION:
                    continue  # The local, reproducibly patched top-level crate.
                if source != LOCK_SOURCE:
                    failures.append(f"{relative}: {name} resolves outside the reviewed MLS revision")
            continue

        for dependencies in dependency_tables(document):
            for alias, value in dependencies.items():
                spec = value if isinstance(value, dict) else {}
                name = spec.get("package", alias.split(":", 1)[0])
                git = spec.get("git", "")
                if not is_mls(name) and "github.com/awslabs/mls-rs" not in git:
                    continue
                if spec.get("workspace") is True:
                    continue  # The workspace declaration is scanned separately.
                if name == "mls-rs" and "path" in spec:
                    if (path.parent / spec["path"]).resolve() == vendor.resolve() and not git:
                        continue
                if (git != UPSTREAM or spec.get("rev") != REVISION or
                        "branch" in spec or "tag" in spec or "path" in spec):
                    failures.append(f"{relative}: {alias} must use the reviewed MLS source/revision")
    return failures


def main() -> int:
    try:
        failures = check_policy(ROOT)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"MLS source policy could not be checked: {error}", file=sys.stderr)
        return 1
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"MLS_SOURCE_POLICY_OK {REVISION}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
