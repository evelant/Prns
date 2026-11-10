"""Assemble the iOS app's target acknowledgements and embedded framework notices."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import plistlib
import re


ROOT = Path(__file__).resolve().parents[3]
SOURCES = ROOT / "release/licenses/app-ios/runtime-notices.json"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checked_text(path, checksum):
    data = path.read_bytes()
    if sha(data) != checksum:
        raise ValueError(f"iOS notice input changed: {path.name}")
    return data.decode("utf-8")


def target_frameworks(script):
    release = re.search(r'^if \[\[ "\$CONFIGURATION" == "Release" \]\]; then\n(.*?)^fi\s*$',
                        script.read_text(), re.MULTILINE | re.DOTALL)
    if not release:
        raise ValueError("Missing CocoaPods Release framework phase")
    return sorted(set(re.findall(r'^\s*install_framework\s+"[^"]*/([^/"]+)\.framework"\s*$',
                                 release[1], re.MULTILINE)))


def render(acknowledgements, modules):
    spec = json.loads(SOURCES.read_bytes())
    if spec["schema"] != 1:
        raise ValueError("Unsupported iOS framework notice schema")
    support = acknowledgements.parent
    pods = support.parents[1]
    lock = (pods.parent / "Podfile.lock").read_bytes()
    if lock != (pods / "Manifest.lock").read_bytes():
        raise ValueError("Resolved CocoaPods manifest differs from Podfile.lock")
    package_lock = json.loads((modules.parent / "package-lock.json").read_bytes())["packages"]
    target = support.name
    framework_names = target_frameworks(support / f"{target}-frameworks.sh")
    unknown = set(framework_names) - set(spec["frameworks"])
    if unknown or "prns_chat_app" not in framework_names:
        raise ValueError(f"Unreviewed iOS target framework inventory: {sorted(unknown)}")
    for path, checksum in spec["inputs"].items():
        if sha((modules / path).read_bytes()) != checksum:
            raise ValueError(f"iOS native source selection changed: {path}")

    pod_text = acknowledgements.read_text()
    pod_entries = plistlib.loads(acknowledgements.with_suffix(".plist").read_bytes())["PreferenceSpecifiers"]
    body = {row["Title"]: row["FooterText"].strip() for row in pod_entries
            if row.get("Title") and row["Title"] != "Acknowledgements" and row.get("FooterText")}
    if not body or any(f"\n## {title}\n" not in pod_text or text not in pod_text for title, text in body.items()):
        raise ValueError("CocoaPods acknowledgement texts differ between its plist and markdown")

    texts = defaultdict(set)
    for framework in framework_names:
        owner = spec["frameworks"][framework]
        if owner == "PRNS chat native image":
            texts["See the Rust and UniFFI notices in the app’s Native libraries document."].add(framework)
            continue
        package = spec["packages"][owner]
        record = package_lock["node_modules/" + owner]
        installed = json.loads((modules / owner / "package.json").read_bytes())
        if (record["version"] != package["version"] or record["integrity"] != package["integrity"]
                or installed["name"] != owner or installed["version"] != package["version"]):
            raise ValueError(f"iOS framework package changed: {owner}")
        if package.get("notice") == "native-sources.txt":
            text = "See the MPL-2.0 license, patches and corresponding source in the app’s Native libraries document."
        else:
            text = checked_text(modules / owner / package["license_file"], package["license_sha256"])
        texts[text.strip()].add(f"{framework} ({owner} {package['version']})")
    if sha(json.dumps(spec["supplemental_notices"], sort_keys=True, separators=(",", ":")).encode()) != spec["supplemental_notices_sha256"]:
        raise ValueError("iOS supplemental notice inventory changed")
    for item in spec["supplemental_notices"]:
        if item["framework"] in framework_names:
            path = (ROOT / item["file"]).resolve()
            if not path.is_relative_to(ROOT / "release/licenses"):
                raise ValueError("iOS license source is outside the evidence directory")
            text = checked_text(path, item["sha256"])
            texts[text.strip()].add(f"{item['framework']} / {item['name']} / {item['url']}")

    lines = [pod_text.strip(), "", "# Embedded iOS frameworks", ""]
    for text, sources in sorted(texts.items(), key=lambda entry: sorted(entry[1])):
        lines.extend([*sorted(sources), "", text, ""])
    result = ("\n".join(lines).rstrip() + "\n").encode()
    receipt = {"pod_lock_sha256": sha(lock), "pod_acknowledgements_sha256": sha(acknowledgements.read_bytes()),
               "acknowledged_pods": sorted(body), "frameworks": framework_names,
               "device_framework_uuids": spec["device_framework_uuids"],
               "framework_review_sha256": sha(SOURCES.read_bytes()), "notice_sha256": sha(result)}
    return result, receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--acknowledgements", required=True, type=Path)
    parser.add_argument("--node-modules", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    text, _ = render(args.acknowledgements.resolve(), args.node_modules.resolve())
    args.output.write_bytes(text)


if __name__ == "__main__":
    main()
