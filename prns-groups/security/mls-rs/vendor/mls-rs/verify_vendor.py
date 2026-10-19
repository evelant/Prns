#!/usr/bin/env python3
"""Reconstruct this narrow vendor from an existing official Git checkout + patch."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

vendor = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("upstream_checkout", type=Path)
args = parser.parse_args()
metadata = json.loads((vendor / "PROVENANCE.json").read_text())
revision = metadata["upstream_revision"]
patch = vendor / metadata["local_patch"]
assert hashlib.sha256(patch.read_bytes()).hexdigest() == metadata["patch_sha256"]


def git(*arguments):
    return subprocess.check_output(["git", "-C", str(args.upstream_checkout), *arguments])


assert git("rev-parse", revision + "^{commit}").decode().strip() == revision
files = git("ls-tree", "-rz", "--name-only", revision, "--", *metadata["copied_paths"]).decode().split("\0")
with tempfile.TemporaryDirectory(prefix="prns-mls-vendor-") as scratch:
    reconstructed = Path(scratch)
    for source in filter(None, files):
        relative = source.removeprefix("mls-rs/")
        destination = reconstructed / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(git("show", f"{revision}:{source}"))
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=reconstructed, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=reconstructed, check=True)
    expected = {p.relative_to(reconstructed) for p in reconstructed.rglob("*") if p.is_file()}
    actual = set()
    for copied in metadata["copied_paths"]:
        local = vendor / copied.removeprefix("mls-rs/")
        candidates = [local] if local.is_file() else local.rglob("*")
        actual.update(p.relative_to(vendor) for p in candidates if p.is_file())
    assert actual == expected, f"file inventory differs: {actual ^ expected}"
    for relative in sorted(expected):
        assert (reconstructed / relative).read_bytes() == (vendor / relative).read_bytes(), f"content differs: {relative}"
    print(f"Verified {len(expected)} vendored files against {revision} plus {patch.name}.")
