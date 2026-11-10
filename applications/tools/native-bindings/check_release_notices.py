#!/usr/bin/env python3
"""Verify release notices, JavaScript and its build-time source map for a chat APK or IPA."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import struct
import subprocess
import uuid
import sys
import zipfile

import ios_release_notices


ROOT = Path(__file__).resolve().parents[3]
NOTICES = ROOT / "applications/prns/app/assets/licenses"
AUDIT = ROOT / "validation/security"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def arm64_framework_uuid(binary):
    if len(binary) < 32 or struct.unpack_from("<II", binary) != (0xfeedfacf, 0x0100000c):
        raise ValueError("Expected a device arm64 Mach-O framework")
    count, size = struct.unpack_from("<II", binary, 16)
    end = 32 + size
    if end > len(binary):
        raise ValueError("Invalid Mach-O load command table")
    offset = 32
    for _ in range(count):
        if offset + 8 > end:
            break
        command, length = struct.unpack_from("<II", binary, offset)
        if length < 8 or length > end - offset:
            break
        if command == 0x1b and length == 24:
            return str(uuid.UUID(bytes=binary[offset + 8:offset + 24]))
        offset += length
    raise ValueError("Missing framework build UUID")


def inspect_archive(package, platform, bundle, acknowledgements, source_map, modules=None):
    sources = [NOTICES / f"npm-{platform}.txt", NOTICES / "native-sources.txt"]
    if platform == "android":
        sources.append(NOTICES / "android-maven.txt")
    expected = {digest(path): path for path in sources}
    sizes = {path.stat().st_size for path in sources}
    found = {}
    pod_receipt = {}
    map_size, map_sha = source_map.stat().st_size, digest(source_map)
    with zipfile.ZipFile(package) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("Duplicate file entries in mobile release archive")
        if platform == "ios":
            plists = [name for name in archive.namelist() if name.startswith("Payload/") and
                      name.endswith(".app/Info.plist") and name.count("/") == 2]
            if len(plists) != 1 or plistlib.loads(archive.read(plists[0]))["CFBundleIdentifier"] != "rs.reticulum.prns.chat":
                raise ValueError("Expected the PRNS Chat production iOS app")
            app = plists[0].removesuffix("Info.plist")
            native_notice = app + "prns-cocoapods-notices.txt"
            pod_bytes, pod_receipt = ios_release_notices.render(
                acknowledgements.resolve(), modules or ROOT / "applications/node_modules")
            if archive.read(native_notice) != pod_bytes:
                raise ValueError("Packaged iOS acknowledgements or native framework notices differ")
            framework_root = app + "Frameworks/"
            framework_names = {name[len(framework_root):].split("/", 1)[0].removesuffix(".framework")
                               for name in names if name.startswith(framework_root)
                               and name[len(framework_root):].split("/", 1)[0].endswith(".framework")}
            if framework_names != set(pod_receipt["frameworks"]):
                raise ValueError("Packaged iOS frameworks differ from the reviewed CocoaPods release target")
            framework_hashes = {}
            for framework in sorted(framework_names):
                binary = archive.read(f"{framework_root}{framework}.framework/{framework}")
                expected_uuid = pod_receipt["device_framework_uuids"].get(framework)
                if expected_uuid and arm64_framework_uuid(binary) != expected_uuid:
                    raise ValueError(f"Packaged {framework} differs from its reviewed native build")
                framework_hashes[framework] = hashlib.sha256(binary).hexdigest()
            pod_receipt["packaged_framework_sha256"] = framework_hashes
            found[native_notice] = hashlib.sha256(pod_bytes).hexdigest()
            bundle_name = app + "main.jsbundle"
            resource_roots = (app + "assets/",)
        else:
            bundle_name = "assets/index.android.bundle"
            resource_roots = ("assets/",)
        bytecode = hashlib.sha256(archive.read(bundle_name)).hexdigest()
        if bytecode != digest(bundle):
            raise ValueError("Packaged JavaScript differs from this release build's bundle")
        for member in archive.infolist():
            if member.is_dir():
                continue
            if member.filename.lower().endswith((".map", ".map.gz", ".map.br")) or (
                    member.file_size == map_size and hashlib.sha256(archive.read(member)).hexdigest() == map_sha):
                raise ValueError(f"Build-time source map was packaged: {member.filename}")
            parent = member.filename.rsplit("/", 1)[0]
            android_raw = platform == "android" and (parent == "res/raw" or parent.startswith("res/raw-"))
            if (android_raw or member.filename.startswith(resource_roots)) and member.file_size in sizes:
                checksum = hashlib.sha256(archive.read(member)).hexdigest()
                if checksum in expected:
                    found[member.filename] = checksum
    missing = [p.name for checksum, p in expected.items() if checksum not in found.values()]
    if missing:
        raise ValueError(f"Missing or changed packaged notices: {', '.join(missing)}")
    report = {"package_sha256": digest(package), "bundle_sha256": bytecode, "notice_members": found}
    if pod_receipt:
        report["cocoapods"] = pod_receipt
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path, help="APK or IPA from the release build")
    parser.add_argument("--bundle", type=Path, required=True, help="Hermes output from the same bundling step")
    parser.add_argument("--source-map", type=Path, required=True, help="release Metro or composed Hermes source map")
    parser.add_argument("--pods-acknowledgements", type=Path, help="the iOS target's resolved acknowledgements.markdown")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    platform = {".apk": "android", ".ipa": "ios"}.get(args.package.suffix)
    if not platform or platform == "ios" and not args.pods_acknowledgements:
        parser.error("Supply an APK or an IPA with its CocoaPods acknowledgements")
    report = inspect_archive(args.package, platform, args.bundle, args.pods_acknowledgements, args.source_map)
    subprocess.run([sys.executable, AUDIT / "app_mobile_npm.py", "--verify-bundle-map",
                    platform, args.source_map, "--bundle", args.bundle], check=True)
    report["source_map_sha256"] = digest(args.source_map)
    if platform == "android":
        subprocess.run([sys.executable, AUDIT / "app_maven.py", "--verify-apk", args.package], check=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Verified {platform} release notices for {report['package_sha256']}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, zipfile.BadZipFile, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Mobile release notice verification: {error}")
