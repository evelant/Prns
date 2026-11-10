"""Collect the target-specific notices distributed in the JavaScript SDKs."""

import importlib.util
import json
import os
from pathlib import Path
import tarfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("release_notices", ROOT / "tools/repo/generate-third-party-notices.py")
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)

TARGETS = {
    "darwin-arm64": "aarch64-apple-darwin",
    "darwin-x64": "x86_64-apple-darwin",
    "linux-arm64-gnu": "aarch64-unknown-linux-gnu",
    "linux-arm64-musl": "aarch64-unknown-linux-musl",
    "linux-x64-gnu": "x86_64-unknown-linux-gnu",
    "linux-x64-musl": "x86_64-unknown-linux-musl",
    "win32-arm64-msvc": "aarch64-pc-windows-msvc",
    "win32-x64-msvc": "x86_64-pc-windows-msvc",
}
NOTICE = "THIRD_PARTY_NOTICES.md"


def javascript_notices():
    # The generated native loader and the CommonJS export helper contain code
    # from these exact generators. Neither generator package is bundled whole.
    cache = Path(os.environ.get("npm_config_cache", Path.home() / ".npm")) / "prns-notice-archives"
    lines = []
    inputs = (
        ("prns-napi", "@napi-rs/cli", "native/addon.cjs", "MIT", "LICENSE"),
        ("prns-js", "typescript", "dist-cjs/contract.js", "Apache-2.0", "LICENSE.txt"),
    )
    for directory, name, shipped, identifier, license_file in inputs:
        lock = json.loads((ROOT / directory / "package-lock.json").read_text())["packages"]
        entry = lock["node_modules/" + name]
        archive = notices.app_npm.package_archive(ROOT, entry, cache)
        with tarfile.open(archive, "r:gz") as tar:
            manifest = json.load(tar.extractfile("package/package.json"))
            if (manifest["name"], manifest["version"], manifest["license"]) != (name, entry["version"], identifier):
                raise ValueError(f"unexpected generator identity or license for {name}")
            license_text = tar.extractfile("package/" + license_file).read().decode()
            if name == "typescript":
                # The compiler's Apache license file and the copyright header on
                # the source of its emitted helpers are separate in the archive.
                header = tar.extractfile("package/lib/_tsc.js").read(4096).decode().partition("*/")[0]
                if not header.startswith("/*!") or "Copyright (c) Microsoft Corporation" not in header:
                    raise ValueError("TypeScript emit helper is missing its expected copyright header")
                license_text = header + "*/\n\n" + license_text
        lines.extend([f"## {name} {entry['version']}", "",
                      f"Generated code in `{shipped}`. [Upstream package]({entry['resolved']}).", "",
                      "```text", notices.normalized_notice_text(license_text), "```", ""])
    return lines


def render(platform=None):
    if platform is None:
        package, manifest, target = "personal-rns", "prns-wasm/Cargo.toml", "wasm32-unknown-unknown"
    else:
        package, manifest, target = "personal-rns-" + platform, "prns-napi/Cargo.toml", TARGETS[platform]
    _, collected = notices.collect_rust_notices(((package, manifest, target),), {})
    if platform and platform.startswith("linux-"):
        for name, identifier, path, graphs in notices.VENDORED:
            if "Node addon Linux" in graphs:
                text = notices.normalized_notice_text((ROOT / path).read_text())
                entry = collected.setdefault((identifier, text), {"name": identifier, "packages": set(), "graphs": set()})
                entry["packages"].add(name)
                entry["graphs"].add(package)
    lines = [f"# Third-party notices for {package}", "",
             f"Rust release graph: `{manifest}` for `{target}` (locked default features).", ""]
    if platform is None:
        lines.extend(javascript_notices())
    return "\n".join([*lines, *notices.license_text_lines(collected)]).rstrip() + "\n"
