"""Attribute the production Android/iOS Metro exports to their exact redistribution inputs."""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
import os
import posixpath
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib

try:
    from . import app_npm
except ImportError:
    import app_npm

ROOT = Path(__file__).resolve().parents[2]
RECORD = Path("release/licenses/app-npm/mobile-bundles.json")
EMBEDDED = Path("release/licenses/app-npm/embedded-sources.json")
PLATFORMS = ("android", "ios")
APP = "applications/prns/app/"
DOCUMENTS = {
    os_: {
        APP + "assets/licenses/" + n + ".txt"
        for n in ("npm-" + os_, "native-sources", *(("android-maven",) if os_ == "android" else ()))
    }
    for os_ in PLATFORMS
}
# Only content generated from independently verified recipient notices is omitted
# from its own input digest. Its import, path and presence in the asset table are checked.
GENERATED_DOCUMENTS = set.union(*DOCUMENTS.values())


def read(path):
    return json.loads(path.read_text())


def sha(data):
    return hashlib.sha256(data).hexdigest()


def input_digest(root):
    # Include all app code/config/assets and the source-linked SDKs' build inputs,
    # including previously unreferenced imported files and Metro/Babel overrides.
    scopes = [
        "applications/prns/app",
        "applications/prns/platform",
        "applications/prns/bindings",
        "applications/package*.json",
        "applications/tools/metro-release-inventory.cjs",
        "applications/tools/native-bindings",
        "prns-js",
        "prns-react-native",
        "tools/uniffi/react-native.json",
        "validation/security/app_mobile_npm.py",
        str(EMBEDDED),
    ]
    output = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *scopes], cwd=root
    )
    digest = hashlib.sha256()
    for path in sorted(os.fsdecode(output).split("\0")):
        if not path or path in GENERATED_DOCUMENTS:
            continue
        file = root / path
        if file.is_symlink() or not file.is_file():
            raise ValueError(f"missing or linked qualification input: {path}")
        digest.update(path.encode() + b"\0" + hashlib.sha256(file.read_bytes()).digest())
    return digest.hexdigest()


def safe_path(value):
    if not isinstance(value, str) or not value or "\0" in value or "\\" in value:
        raise ValueError(f"invalid mobile source path: {value!r}")
    p = PurePosixPath(value)
    if p.is_absolute() or str(p) != value or ".." in p.parts:
        raise ValueError(f"mobile source escapes the project: {value!r}")
    return value


def npm_root(path):
    prefix, sep, leaf = path.rpartition("/node_modules/")
    if not sep:
        return None
    parts = leaf.split("/")
    count = 2 if parts[0].startswith("@") else 1
    if len(parts) <= count:
        raise ValueError(f"missing npm file path: {path}")
    return prefix + sep + "/".join(parts[:count])


def preserved_comments(source):
    # Published standalone notices sometimes omit notices carried inside the
    # actual implementation (including third-party licenses in copied code).
    comments = re.findall(r"/\*[^*]*\*+(?:[^/*][^*]*\*+)*/|(?:^[ \t]*//[^\n]*\n?)+", source, flags=re.M)
    return {
        comment.strip()
        for comment in comments
        if re.search(
            r"copyright|@license\b|\bSPDX-License-Identifier\b|\bpermission is hereby granted\b|\blicensed under\b",
            comment,
            re.I,
        )
    }


def reviewed_embedded(embedded, integrity, name):
    return [
        item
        for item in embedded
        if item["integrity"] == integrity
        and any(
            PurePosixPath(name).match(pattern) or pattern.endswith("/**") and name.startswith(pattern[:-2])
            for pattern in item["paths"]
        )
    ]


def outside_package(map_name, source):
    if not isinstance(source, str) or source.startswith(("/", "\\")) or ":" in source:
        return True
    path = posixpath.normpath(posixpath.join(posixpath.dirname(map_name), source))
    return path == ".." or path.startswith("../") or "node_modules" in PurePosixPath(path).parts


def staged_casework(root, path):
    if path not in ("prns-js/dist/casework.js", "prns-js/dist-cjs/casework.js"):
        return None
    # The SDK transpiles casework/briefcase.ts into its own package path.
    # Match the exact producer settings before attributing the renamed source.
    original = "prns-js/node_modules/casework/briefcase.ts"
    script = """
const fs = require('node:fs');
const ts = require('typescript');
const source = fs.readFileSync('node_modules/casework/briefcase.ts', 'utf8');
const result = ts.transpileModule(source, {fileName: 'casework.ts', compilerOptions: {
  target: ts.ScriptTarget.ES2022, removeComments: true, module: ts.ModuleKind[process.argv[1]]
}});
process.stdout.write(result.outputText);
"""
    module = "CommonJS" if "dist-cjs" in path else "ES2022"
    generated = subprocess.check_output(["node", "-e", script, module], cwd=root / "prns-js")
    if (root / path).read_bytes() != generated:
        raise ValueError(f"unverified generated casework source: {path}")
    return original


def first_party(root, path):
    if path.startswith(
        ("applications/prns/app/", "applications/prns/bindings/", "applications/prns/platform/")
    ):
        if (root / path).is_file() and subprocess.run(
            ["git", "ls-files", "--error-unmatch", "--", path],
            cwd=root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0:
            return
    if path.startswith("applications/target/react-native-sdk/src/"):
        suffix = path.split("react-native-sdk/src/", 1)[1]
        # The staged SDK uses the app-specific generated binding image.
        owning = root / "prns-react-native/src" / suffix
        if owning.is_file() and owning.read_bytes() == (root / path).read_bytes():
            return
        if suffix in {
            "generated/prns_host_uniffi.ts",
            "generated/prns_host_uniffi-ffi.ts",
            "generated/native-image.generated.ts",
        }:
            # The native-bindings stage checks these against the selected app image.
            # Their exact generated bytes are also recorded and checked at each export.
            return
    if path == "prns-js/dist/remote-control.generated.js":
        # This export contains just a fingerprint constant generated from own source.
        raw = (root / path).read_text()
        found = re.fullmatch(
            r'export const REMOTE_CONTROL_SEMANTIC_FINGERPRINT = "([0-9a-f]{64})";\n'
            r"//# sourceMappingURL=remote-control.generated.js.map\n?",
            raw,
        )
        if found and found[1] in (root / "prns-js/src/remote-control.generated.ts").read_text():
            return
    raise ValueError(f"unreviewed mobile source owner: {path}")


def mapped_path(source):
    if source == "\0polyfill:external-require":
        return APP + source
    if not source.startswith("/") or source.startswith("//"):
        raise ValueError(f"unknown mobile map source: {source!r}")
    return source[1:] if source.startswith("/prns-js/") else "applications" + source


def producer(path):
    if path in {APP + "\0polyfill:external-require", APP + "\0polyfill:environment-variables"}:
        return (
            "applications/node_modules/@expo/cli/build/src/start/server/metro/withMetroMultiPlatform.js",
            path.split(":")[-1],
        )
    if path == APP + "__prelude__":
        return "applications/node_modules/metro/src/lib/getPrependedScripts.js", "__prelude__"
    if re.fullmatch(re.escape(APP) + r"app\?ctx=[0-9a-f]+", path):
        return "applications/node_modules/metro/src/lib/contextModuleTemplates.js", "routes"
    return None


def collect_platform(root, platform, directory, locations):
    out = directory / "out"
    graph = read(directory / "graph" / f"{platform}.json")
    if graph["entry"] != "applications/node_modules/expo-router/entry.js" or graph["platform"] != platform:
        raise ValueError(f"unexpected {platform} entrypoint")
    meta = read(out / "metadata.json")["fileMetadata"][platform]
    bundle = safe_path(meta["bundle"])
    if not bundle.startswith("_expo/static/js/" + platform + "/") or not bundle.endswith(".hbc"):
        raise ValueError(f"expected a {platform} Hermes bundle")
    raw = (out / bundle).read_bytes()
    if raw[:8] != bytes.fromhex("c61fbc03c103191f"):
        raise ValueError(f"invalid {platform} Hermes bytecode")
    maps = read(out / (bundle + ".map"))
    sources = dict(zip(map(mapped_path, maps["sources"]), maps["sourcesContent"], strict=True))
    if not sources or len(sources) != len(maps["sources"]):
        raise ValueError(f"empty or duplicate {platform} source map")
    if not sources.keys() <= set(graph["sources"]):
        raise ValueError(f"{platform} source map contains sources outside the Metro graph")
    paths = set(graph["sources"])
    assetmap = read(out / "assetmap.json")
    assets = {}
    for info in assetmap.values():
        for key, filename in zip(info["fileHashes"], info["files"], strict=True):
            origin = Path(filename).resolve().relative_to(root).as_posix()
            if (
                key in assets
                and assets[key] != origin
                and (root / assets[key]).read_bytes() != (root / origin).read_bytes()
            ):
                raise ValueError(f"ambiguous Metro asset hash: {key}")
            assets[key] = origin
    exported = set()
    for entry in meta["assets"]:
        path = safe_path(entry["path"])
        if not path.startswith("assets/"):
            raise ValueError(f"unexpected asset location: {path}")
        origin = assets[Path(path).name]
        if not (root / origin).read_bytes() or (root / origin).read_bytes() != (out / path).read_bytes():
            raise ValueError(f"{platform} asset differs from Metro input: {origin}")
        paths.add(origin)
        exported.add(origin)
    if exported & GENERATED_DOCUMENTS != DOCUMENTS[platform]:
        raise ValueError(
            f"{platform} export is missing a required notice asset or includes the wrong OS notices"
        )
    registry = defaultdict(set)
    # MDN's separate grant in React Native applies explicitly to JSDoc prose.
    # Verify that these exact descriptions are stripped from the distributed code.
    for source in sources.values():
        if "The JSDoc comments in this file have been extracted from" in source:
            for comment in re.findall(r"/\*\*(.*?)\*/", source, re.S):
                for line in comment.splitlines():
                    prose = line.strip().removeprefix("*").strip()
                    if len(prose) >= 16 and (prose.encode() in raw or prose.encode("utf-16-le") in raw):
                        raise ValueError(f"{platform} bundle retains separately licensed JSDoc: {prose}")

    project = {}
    virtual = {}
    for path in sorted(paths):
        generated = producer(path)
        if generated:
            code, key = generated
            virtual[key] = code
            path = code
        safe_path(path)
        file = root / path
        if not file.resolve().is_relative_to(root) or not file.is_file():
            raise ValueError(f"missing mobile bundle source: {path}")
        if path in sources and file.read_bytes() != sources[path].encode():
            raise ValueError(f"{platform} map content disagrees with the producing file: {path}")
        upstream = staged_casework(root, path)
        if upstream:
            project[path] = sha(file.read_bytes())
            path = upstream
        owner = npm_root(path)
        if owner:
            if owner not in locations:
                raise ValueError(f"{platform} source outside locked production npm closure: {owner}")
            registry[owner].add(path[len(owner) + 1 :])
        elif path not in GENERATED_DOCUMENTS:
            if re.search(r"(^|/)(vendor|vendored|third[_-]party)(/|$)", path, re.I):
                raise ValueError(f"unreviewed embedded project source: {path}")
            first_party(root, path)
            project[path] = sha(file.read_bytes())
    return {
        "registry": {
            p: {f: sha((root / p / f).read_bytes()) for f in sorted(files)}
            for p, files in sorted(registry.items())
        },
        "project": project,
        "virtual": virtual,
        "assets": sorted(exported),
    }


def archive_evidence(root, cache, inventory, packages):
    by_location = {p: rec for rec in packages for p in rec["locations"]}
    used = defaultdict(set)
    for platform in PLATFORMS:
        for loc, paths in inventory[platform]["registry"].items():
            used[loc].update(paths)
    embedded = read(root / EMBEDDED)
    headers = defaultdict(set)
    for loc, wanted in sorted(used.items()):
        pkg = by_location[loc]
        archive = app_npm.package_archive(root, pkg, cache)
        key = pkg["integrity"]
        seen = set()
        with tarfile.open(archive, "r:gz") as source:
            for item in source:
                name = item.name.partition("/")[2]
                if item.isfile() and name.endswith(".map") and name[:-4] in wanted:
                    mapping = json.load(source.extractfile(item))
                    nested = [s for s in mapping.get("sources", []) if outside_package(name, s)]
                    if nested and not reviewed_embedded(embedded, key, name[:-4]):
                        raise ValueError(f"unreviewed prebundled source in {loc}/{name}: {nested}")
                if name not in wanted or not item.isfile():
                    continue
                content = source.extractfile(item).read()
                if content != (root / loc / name).read_bytes():
                    raise ValueError(f"mobile input diverges from the verified npm archive: {loc}/{name}")
                seen.add(name)
                if Path(name).suffix in (".js", ".cjs", ".mjs", ".ts", ".jsx", ".tsx"):
                    for text in preserved_comments(content.decode()):
                        headers[key].add(text)
                # Vendored paths require explicit attribution review, bound to
                # the host archive. This includes libraries compiled under its own name.
                if re.search(r"(^|/)(vendor|vendored|third[_-]party|fork)(/|$)", name, re.I):
                    if not reviewed_embedded(embedded, key, name):
                        raise ValueError(f"unreviewed embedded mobile source: {loc}/{name}")
        if seen != wanted:
            raise ValueError(
                f"mobile sources missing from verified {pkg['name']} archive: {sorted(wanted - seen)}"
            )
    return {k: sorted(v) for k, v in sorted(headers.items()) if v}


def export_inventory(root, cache):
    with tempfile.TemporaryDirectory(prefix="prns-mobile-npm-") as temp:
        directory = Path(temp).resolve()
        for platform in PLATFORMS:
            dest = directory / platform
            (dest / "graph").mkdir(parents=True)
            env = dict(
                os.environ,
                CI="1",
                NODE_ENV="production",
                EXPO_OFFLINE="1",
                EXPO_NO_TELEMETRY="1",
                PRNS_APP_VARIANT="production",
                PRNS_METRO_INVENTORY_DIR=str(dest / "graph"),
            )
            subprocess.run(
                [
                    root / "applications/node_modules/.bin/expo",
                    "export",
                    "--platform",
                    platform,
                    "--source-maps",
                    "external",
                    "--dump-assetmap",
                    "--max-workers",
                    "2",
                    "--output-dir",
                    dest / "out",
                ],
                cwd=root / "applications/prns/app",
                env=env,
                check=True,
            )
        return capture(root, cache, directory)


def capture(root, cache, directory):
    packages = app_npm.app_closure(root)["packages"]
    locations = {loc for p in packages for loc in p["locations"]}
    platforms = {p: collect_platform(root, p, directory / p, locations) for p in PLATFORMS}
    headers = archive_evidence(root, cache, platforms, packages)
    return {
        "schema": 1,
        "inputs_sha256": input_digest(root),
        "platforms": platforms,
        "identities": {
            loc: {k: p[k] for k in ("name", "version", "integrity", "resolved")}
            for p in packages
            for loc in p["locations"]
            if any(loc in platforms[os_]["registry"] for os_ in PLATFORMS)
        },
        "source_headers": headers,
    }


def shipped_report(root, cache, full_report, platform=None):
    if platform is not None and platform not in PLATFORMS:
        raise ValueError(f"unsupported app platform {platform!r}")
    inventory = read(root / RECORD)
    if inventory.get("schema") != 1 or inventory.get("inputs_sha256") != input_digest(root):
        raise ValueError(
            "Stale mobile bundle inventory. Rebuild the staged SDK and run app_mobile_npm.py --write."
        )
    selected_os = PLATFORMS if platform is None else (platform,)
    used = defaultdict(set)
    for os_ in selected_os:
        for loc, files in inventory["platforms"][os_]["registry"].items():
            used[loc].update(files)
    locations = {loc: p for p in full_report["packages"] for loc in p["locations"]}
    embedded = read(root / EMBEDDED)
    records = {}
    allowed = set(tomllib.loads((root / "deny.toml").read_text())["licenses"]["allow"])
    mpl_names = set()
    for loc, files in used.items():
        p = locations.get(loc)
        if p is None or inventory["identities"].get(loc) != {
            k: p[k] for k in ("name", "version", "integrity", "resolved")
        }:
            raise ValueError(f"shipped npm identity differs from locked closure: {loc}")
        key = p["integrity"]
        if key not in records:
            records[key] = dict(p, texts=list(p["texts"]), policy_issues=list(p["policy_issues"]))
        selected = records[key]
        if not app_npm.license_permitted(p["license"], allowed - {"MPL-2.0"}) and (
            p["license"] == "MPL-2.0" or app_npm.license_permitted(p["license"], allowed)
        ):
            if not app_npm.reviewed_mobile_runtime_license(
                p["name"], p["version"], p["license"], key, p["resolved"]
            ):
                selected["policy_issues"].append(
                    "Shipped MPL component requires verified corresponding source and modification notices"
                )
            else:
                mpl_names.add(p["name"])
        for text in inventory["source_headers"].get(key, []):
            item = {"source": p["resolved"] + " (published source notices)", "text": text}
            if item not in selected["texts"]:
                selected["texts"].append(item)
        for extra in embedded:
            if extra["integrity"] != key or not any(
                PurePosixPath(f).match(pat) or pat.endswith("/**") and f.startswith(pat[:-2])
                for f in files
                for pat in extra["paths"]
            ):
                continue
            if not app_npm.license_permitted(extra["license"], allowed - {"MPL-2.0"}):
                selected["policy_issues"].append(
                    f"Embedded {extra['component']} requires separate verified permission for {extra['license']!r}"
                )
            for item in extra["texts"]:
                content = (
                    (root / "release/licenses/app-npm/texts" / item["sha256"])
                    .with_suffix(".txt")
                    .read_bytes()
                )
                if sha(content) != item["sha256"]:
                    raise ValueError(f"bad embedded license digest for {extra['component']}")
                entry = {
                    "source": extra["component"] + " — " + item["source"],
                    "text": content.decode().strip(),
                }
                if entry not in selected["texts"]:
                    selected["texts"].append(entry)
    source_lines = []
    if mpl_names:
        sys.path.insert(0, str(root / "tools/uniffi"))
        import runtime_notices

        evidence = runtime_notices.evidence()
        runtime_notices.verify_policy(evidence)
        source_lines = runtime_notices.recipient_source_lines(evidence, mpl_names)
        if not source_lines:
            raise ValueError("Missing recipient source offer for the selected MPL runtime")
    return dict(
        full_report,
        packages=sorted(records.values(), key=lambda p: (p["name"], p["version"], p["integrity"])),
        corresponding_source_lines=source_lines,
    )


def render_notices(report):
    app_npm.audit_notices(report)
    lines = []
    for p in report["packages"]:
        lines.extend([f"{p['name']} {p['version']} ({p['license']})", p["resolved"]])
        for notice in p["texts"]:
            lines.extend(["", notice["source"], notice["text"]])
        lines.extend(["", ""])
    lines.extend(report.get("corresponding_source_lines", []))
    return "\n".join(lines).rstrip() + "\n"


def verify_bundle_map(root, platform, source_map, bundle=None):
    """Check the source map produced by the actual Gradle/Xcode release task.

    Metro can emit checkout-absolute names, workspace virtual names, or names
    relative to the app project. Match whole paths and optional mapped content.
    """
    if platform not in PLATFORMS:
        raise ValueError(f"unknown release platform: {platform}")
    record = read(root / RECORD)
    if record["inputs_sha256"] != input_digest(root):
        raise ValueError("stale production mobile inventory")
    evidence = record["platforms"][platform]
    known = dict(evidence["project"])
    for directory, members in evidence["registry"].items():
        known.update({directory + "/" + p: d for p, d in members.items()})
    expected_virtual = set(evidence["virtual"])
    known.update({p: None for p in evidence["assets"] if p in GENERATED_DOCUMENTS})
    data = read(Path(source_map))
    if data.get("version") != 3 or not isinstance(data.get("sources"), list) or not data["sources"]:
        raise ValueError(f"invalid {platform} Metro source map")
    seen = set()
    for source, content in zip(
        data["sources"], data.get("sourcesContent", [None] * len(data["sources"])), strict=True
    ):
        if not isinstance(source, str) or not source:
            raise ValueError("empty module in release source map")
        source_root = data.get("sourceRoot", "")
        raw = str(PurePosixPath(source_root) / source) if source_root else source
        candidates = []
        if raw in ("\0polyfill:external-require", "\0polyfill:environment-variables", "__prelude__"):
            candidates.append(APP + raw)
        elif raw.startswith("/"):
            if raw.startswith(str(root) + "/"):
                candidates.append(raw[len(str(root)) + 1 :])
            elif raw.startswith(("/node_modules/", "/prns/", "/target/", "/prns-js/")):
                candidates.append(mapped_path(raw))
        else:
            for directory in (root, root / "applications", root / APP):
                resolved = (directory / raw).resolve()
                if resolved.is_relative_to(root):
                    candidates.append(resolved.relative_to(root).as_posix())
        for path in candidates:
            virtual = producer(path)
            if virtual and virtual[1] in expected_virtual:
                seen.add(virtual[0])
                break
            if path not in known:
                continue
            if content is not None and known[path] is not None and sha(content.encode()) != known[path]:
                raise ValueError(f"shipped mapped source content changed: {path}")
            seen.add(path)
            break
        else:
            raise ValueError(f"unqualified source in final {platform} bundle: {source!r}")
    if "applications/node_modules/expo-router/entry.js" not in seen or not any(
        p.startswith(APP + "src/") for p in seen
    ):
        raise ValueError("release map does not identify the mobile application entry and implementation")
    binary = {}
    if bundle is not None:
        contents = Path(bundle).read_bytes()
        if len(contents) < 128 or contents[:8] != bytes.fromhex("c61fbc03c103191f"):
            raise ValueError("expected release Hermes bytecode")
        binary = {"bundle_sha256": sha(contents), "bundle_bytes": len(contents)}
    return {
        "platform": platform,
        "sources": len(seen),
        "map_sha256": sha(Path(source_map).read_bytes()),
        **binary,
    }


def main():
    args = argparse.ArgumentParser(description=__doc__)
    mode = args.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--verify-bundle-map", nargs=2, metavar=("PLATFORM", "MAP"))
    args.add_argument("--bundle", type=Path)
    options = args.parse_args()
    if options.verify_bundle_map:
        platform, source_map = options.verify_bundle_map
        print(json.dumps(verify_bundle_map(ROOT, platform, source_map, options.bundle), indent=2))
        return
    if options.bundle:
        args.error("--bundle requires --verify-bundle-map")
    cache = Path(os.environ.get("npm_config_cache", Path.home() / ".npm")) / "prns-notice-archives"
    inventory = export_inventory(ROOT, cache)
    if options.write:
        (ROOT / RECORD).write_text(json.dumps(inventory, indent=2, ensure_ascii=False) + "\n")
    elif read(ROOT / RECORD) != inventory:
        raise SystemExit(
            "Production mobile source or asset inventory drifted. Inspect and regenerate with --write."
        )
    print("Verified Android and iOS production npm sources and bundled assets.")


if __name__ == "__main__":
    main()
