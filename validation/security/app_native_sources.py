#!/usr/bin/env python3
"""Generate the chat app's native license texts and corresponding-source instructions."""
import argparse
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "prns-react-native/tools"))
import package_notices


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    provider = package_notices.generate.image_provider(ROOT / "applications/tools/native-bindings/image.json")
    runtime = package_notices.notice_generator.runtime_notices.evidence()
    package_notices.notice_generator.runtime_notices.verify_policy(runtime)
    generator = package_notices.notice_generator
    manifest = provider["manifest"].relative_to(ROOT).as_posix()
    graphs = [(f"PRNS Chat {target}", manifest, target) for target in runtime["source"]["targets"]]
    features = {graph: tuple(provider["features"]) for graph, _, _ in graphs}
    _, notices = generator.collect_rust_notices(graphs, features)
    lines = ["PRNS Chat native libraries and corresponding sources", "",
             "These notices cover the selected chat image for the supported Apple and Android targets.",
             *generator.runtime_notices.declaration_lines(runtime), "",
             *generator.license_text_lines(notices)]
    expected = ("\n".join(lines).rstrip() + "\n").encode("utf-8")
    path = ROOT / "applications/prns/app/assets/licenses/native-sources.txt"
    if args.write:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(expected)
    elif not path.is_file() or path.read_bytes() != expected:
        raise ValueError("Chat app native notices differ from the selected image; regenerate with --write")
    print("Chat app native licenses and corresponding sources verified")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(str(error))
