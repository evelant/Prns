"""Pack and inspect the JavaScript facade or a staged native platform archive."""

import argparse
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile

from js_sdk_notices import NOTICE, ROOT, TARGETS, render


WASM = ("prns_wasm.js", "prns_wasm.d.ts", "prns_wasm_bg.wasm", "prns_wasm_bg.wasm.d.ts")


def package_sources(directory):
    manifest = json.loads((directory / "package.json").read_text())
    common = {"package.json", "README.md", "LICENSE-APACHE", "LICENSE-MIT", NOTICE}
    sources = {name: directory / name for name in common}
    if manifest["name"] == "personal-rns":
        sources.update({"PACKAGE.md": directory / "PACKAGE.md", "native/addon.cjs": ROOT / "prns-napi/index.js"})
        sources.update({"wasm/" + name: ROOT / "prns-wasm/smoke/pkg" / name for name in WASM})
        for export in manifest["exports"].values():
            values = [export]
            while values:
                value = values.pop()
                if isinstance(value, dict):
                    values.extend(value.values())
                else:
                    path = value.removeprefix("./")
                    sources[path] = directory / path
        for subdir in ("dist", "dist-cjs"):
            for path in (directory / subdir).rglob("*"):
                if path.is_file():
                    if path.name != "package.json" and not path.name.endswith((".js", ".cjs", ".mjs", ".map", ".d.ts", ".d.cts")):
                        raise ValueError(f"unexpected compiled file: {path}")
                    sources[path.relative_to(directory).as_posix()] = path
        expected_platforms = {f"personal-rns-{platform}": manifest["version"] for platform in TARGETS}
        if manifest["optionalDependencies"] != expected_platforms:
            raise ValueError("facade and platform package inventory differ")
        if manifest["dependencies"].get("casework") is None or manifest.get("bundledDependencies") or manifest.get("bundleDependencies"):
            raise ValueError("facade declarations must resolve the external casework dependency")
        if not (ROOT / "prns-wasm/smoke/pkg/prns_wasm_bg.wasm").read_bytes().startswith(b"\x00asm\x01\x00\x00\x00"):
            raise ValueError("missing release WebAssembly module")
    else:
        platform = manifest["name"].removeprefix("personal-rns-")
        binding = f"personal-rns.{platform}.node"
        if platform not in TARGETS or manifest["main"] != binding:
            raise ValueError("unexpected native package identity or entry point")
        sources[binding] = directory / binding
    return sources


def verify(archive, directory):
    sources = package_sources(directory)
    with tarfile.open(archive, "r:gz") as tar:
        members = {}
        for entry in tar:
            path = Path(entry.name)
            if (not entry.isfile() or not path.parts or path.parts[0] != "package"
                    or ".." in path.parts or "node_modules" in path.parts):
                raise ValueError(f"unexpected packed path: {entry.name}")
            name = path.relative_to("package").as_posix()
            if name in members:
                raise ValueError(f"duplicate packed path: {name}")
            members[name] = entry
        if sources.keys() != members.keys():
            raise ValueError(f"packed file inventory differs: {sorted(sources.keys() ^ members.keys())}")
        for name, source in sources.items():
            if not source.is_file() or tar.extractfile(members[name]).read() != source.read_bytes():
                raise ValueError(f"packed bytes differ from the staged source: {name}")


def adapter_output(path, source):
    if path.suffix != ".map":
        return path.read_bytes()
    info = json.loads(path.read_text())
    if [(path.parent / relative).resolve() for relative in info.pop("sources")] != [source]:
        raise ValueError(f"unexpected Casework adapter source in {path}")
    return info


def prepare_facade(directory):
    for name, source in package_sources(directory).items():
        if name == NOTICE:
            continue
        staged = directory / name
        if not staged.is_file() or source != staged and source.read_bytes() != staged.read_bytes():
            raise ValueError(f"missing or stale SDK release content: {name}")
    # The public declarations use Casework's external types. The two runtime
    # modules must be compiled from the PRNS adapter, not staged vendor code.
    with tempfile.TemporaryDirectory(prefix="prns-js-compile-") as output:
        for project, subdir in (("tsconfig.json", "dist"), ("tsconfig.casework-cjs.json", "dist-cjs")):
            build = Path(output).resolve() / subdir
            subprocess.run(["node", "node_modules/typescript/bin/tsc", "-p", project, "--outDir", str(build)],
                           cwd=directory, check=True)
            outputs = ["casework.js"]
            if subdir == "dist":
                outputs.extend(["casework.js.map", "casework.d.ts", "casework.d.ts.map"])
            source = directory / "src/casework.ts"
            for name in outputs:
                staged = directory / subdir / name
                if adapter_output(build / name, source) != adapter_output(staged, source):
                    raise ValueError(f"staged Casework adapter differs from {project}: {staged}")
    (directory / NOTICE).write_text(render(), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--archive", type=Path)
    mode.add_argument("--prepare-facade", action="store_true")
    args = parser.parse_args()
    directory = args.directory.resolve()
    if args.prepare_facade:
        prepare_facade(directory)
        return
    if args.archive:
        verify(args.archive, directory)
        return
    with tempfile.TemporaryDirectory(prefix="prns-js-pack-") as temporary:
        packed = subprocess.check_output(["npm", "pack", "--json", "--pack-destination", temporary], cwd=directory)
        record, = json.loads(packed)
        verify(Path(temporary) / record["filename"], directory)
        print(f"Verified {record['filename']} ({record['entryCount']} files)")


if __name__ == "__main__":
    main()
