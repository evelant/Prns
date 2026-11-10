import io
import json
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from validation.security import app_mobile_npm as mobile


class MobileRedistributionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        subprocess.run(["git", "init", "-q", self.root], check=True)
        self.write("deny.toml", '[licenses]\nallow=["MIT","MPL-2.0"]\n')
        self.write(mobile.EMBEDDED, "[]")
        self.cache = self.root / "cache"

    def write(self, path, value):
        dest = self.root / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(value)
        return dest

    def pkg(self, name, license="MIT", issues=()):
        return {
            "name": name,
            "version": "1.0.0",
            "license": license,
            "integrity": "integrity-" + name,
            "resolved": f"https://registry.npmjs.org/{name}/-/{name}-1.0.0.tgz",
            "locations": ["applications/node_modules/" + name],
            "texts": [{"source": "published archive", "text": "Exact " + name + " copyright"}],
            "policy_issues": list(issues),
            "attribution_issues": [],
        }

    def qualify(self, platform_packages, **extra):
        platforms = {
            k: {
                "registry": {x["locations"][0]: {"index.js": mobile.sha(b"module")} for x in rows},
                "project": {mobile.APP + "src/main.ts": mobile.sha(b"source")},
                "virtual": {},
                "assets": [],
            }
            for k, rows in platform_packages.items()
        }
        inventory = {
            "schema": 1,
            "inputs_sha256": mobile.input_digest(self.root),
            "platforms": platforms,
            "source_headers": {},
            "identities": {
                p["locations"][0]: {k: p[k] for k in ("name", "version", "integrity", "resolved")}
                for packages in platform_packages.values()
                for p in packages
            },
            **extra,
        }
        self.write(mobile.RECORD, json.dumps(inventory))

    def test_platform_selection_keeps_declared_holds_for_actual_shipped_inputs(self):
        a, b, host = (
            self.pkg("android"),
            self.pkg("ios", issues=["existing restriction"]),
            self.pkg("tool", issues=["build-only restriction"]),
        )
        self.qualify({"android": [a], "ios": [b]})
        full = {"packages": [a, b, host], "unresolved_required_peers": []}
        self.assertIn(
            "Exact android copyright",
            mobile.render_notices(mobile.shipped_report(self.root, self.cache, full, "android")),
        )
        with self.assertRaisesRegex(ValueError, "existing restriction"):
            mobile.render_notices(mobile.shipped_report(self.root, self.cache, full, "ios"))
        self.assertEqual(len(full["packages"]), 3)
        full["unresolved_required_peers"] = [["applications", "app", "runtime", "1"]]
        with self.assertRaisesRegex(ValueError, "missing required peer"):
            mobile.render_notices(mobile.shipped_report(self.root, self.cache, full, "android"))

    def test_unknown_selected_mpl_requires_source_but_mit_alternative_is_available(self):
        ambiguous = self.pkg("both", "MIT OR MPL-2.0")
        required = self.pkg("copyleft", "MPL-2.0")
        self.qualify({"android": [ambiguous], "ios": [required]})
        full = {"packages": [ambiguous, required], "unresolved_required_peers": []}
        self.assertIn(
            "Exact both copyright",
            mobile.render_notices(mobile.shipped_report(self.root, self.cache, full, "android")),
        )
        with self.assertRaisesRegex(ValueError, "corresponding source"):
            mobile.render_notices(mobile.shipped_report(self.root, self.cache, full, "ios"))

    def test_changed_lock_identity_changed_source_and_generated_notice_cycle(self):
        p = self.pkg("selected")
        self.qualify({os_: [p] for os_ in mobile.PLATFORMS})
        full = {"packages": [p], "unresolved_required_peers": []}
        self.write(mobile.APP + "assets/licenses/npm-ios.txt", "Regenerated package notices")
        mobile.shipped_report(self.root, self.cache, full)
        with self.assertRaisesRegex(ValueError, "identity differs"):
            mobile.shipped_report(self.root, self.cache, dict(full, packages=[dict(p, integrity="other")]))
        self.write(mobile.APP + "src/route.ts", "require('previously unseen')")
        with self.assertRaisesRegex(ValueError, "Stale mobile"):
            mobile.shipped_report(self.root, self.cache, full)

    def test_final_artifact_map_rejects_new_modules_mismatched_contents_and_partial_receipts(self):
        p = self.pkg("expo-router")
        self.qualify({os_: [p] for os_ in mobile.PLATFORMS})
        file = self.root / mobile.RECORD
        selection = json.loads(file.read_text())
        for profile in selection["platforms"].values():
            profile["registry"][p["locations"][0]] = {"entry.js": mobile.sha(b"entry")}
        file.write_text(json.dumps(selection))
        sources = ["/node_modules/expo-router/entry.js", "/prns/app/src/main.ts"]
        m = self.write(
            "ship.map", json.dumps({"version": 3, "sources": sources, "sourcesContent": ["entry", "source"]})
        )
        result = mobile.verify_bundle_map(self.root, "android", m)
        self.assertEqual(result["sources"], 2)
        for extra, contents, issue in [
            (["/node_modules/tool/index.js"], ["tool"], "unqualified"),
            ([], ["wrong", "source"], "content changed"),
        ]:
            if extra:
                contents = ["entry", "source", *contents]
            m.write_text(json.dumps({"version": 3, "sources": sources + extra, "sourcesContent": contents}))
            with self.assertRaisesRegex(ValueError, issue):
                mobile.verify_bundle_map(self.root, "android", m)
        m.write_text(json.dumps({"version": 3, "sources": sources[:1], "sourcesContent": ["entry"]}))
        with self.assertRaisesRegex(ValueError, "implementation"):
            mobile.verify_bundle_map(self.root, "ios", m)

    def test_indexed_native_map_and_build_machine_route_prefixes(self):
        compressed = {
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["__prelude__"],
                        "sourcesContent": ["var __DEV__=false"],
                    },
                },
                {
                    "offset": {"line": 1, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["/../prns-js/dist/file.js"],
                        "sourcesContent": ["export const version = 1"],
                    },
                },
            ],
        }
        self.assertEqual(len(list(mobile.map_entries(compressed))), 2)
        self.assertEqual(mobile.mapped_path("/../prns-js/dist/file.js"), "prns-js/dist/file.js")
        with self.assertRaisesRegex(ValueError, "escapes"):
            mobile.mapped_path("/../prns-js/../../secret")
        compressed["sections"][1] = {"url": "external.map"}
        with self.assertRaisesRegex(ValueError, "self-contained"):
            list(mobile.map_entries(compressed))
        route = 'get() {return require("/tmp/build/applications/prns/app/app/route.tsx");}'
        self.assertEqual(
            mobile.virtual_digest("routes", route),
            mobile.virtual_digest("routes", route.replace("/tmp/build", "/var/lib/runner/repo")),
        )
        self.assertNotEqual(
            mobile.virtual_digest("routes", route),
            mobile.virtual_digest("routes", route.replace("route.tsx", "changed.tsx")),
        )

    def test_native_asset_stub_requires_matching_physical_bytes(self):
        pkg = self.pkg("expo-router")
        loc = pkg["locations"][0]
        self.qualify({os_: [pkg] for os_ in mobile.PLATFORMS})
        record = json.loads((self.root / mobile.RECORD).read_text())
        for profile in record["platforms"].values():
            profile["registry"][loc] = {
                "entry.js": mobile.sha(b"entry"),
                "assets/icon.png": mobile.sha(b"png"),
            }
            profile["assets"] = [loc + "/assets/icon@2x.android.png"]
        self.write(mobile.RECORD, json.dumps(record))
        stub = self.write(loc + "/assets/icon.png", "png")
        map_file = self.write(
            "native.map",
            json.dumps(
                {
                    "version": 3,
                    "sources": [
                        "/node_modules/expo-router/entry.js",
                        "/prns/app/src/main.ts",
                        "/node_modules/expo-router/assets/icon.png",
                    ],
                    "sourcesContent": ["entry", "source", ""],
                }
            ),
        )
        self.assertEqual(mobile.verify_bundle_map(self.root, "android", map_file)["sources"], 3)
        stub.write_text("tampered")
        with self.assertRaisesRegex(ValueError, "differs from qualified input"):
            mobile.verify_bundle_map(self.root, "android", map_file)

    def test_unreviewed_vendor_paths_and_nested_map_dependencies_fail_closed(self):
        p = self.pkg("publisher")
        loc = p["locations"][0]

        def probe(path, sourcemap=None):
            self.write(loc + "/" + path, "code")
            archive = self.root / "input.tgz"
            with tarfile.open(archive, "w:gz") as tf:
                for key, raw in [
                    (path, b"code"),
                    *(([(path + ".map", json.dumps(sourcemap).encode())]) if sourcemap else []),
                ]:
                    entry = tarfile.TarInfo("package/" + key)
                    entry.size = len(raw)
                    tf.addfile(entry, io.BytesIO(raw))
            inv = {s: {"registry": {loc: {path: mobile.sha(b"code")}}} for s in mobile.PLATFORMS}
            with patch.object(mobile.app_npm, "package_archive", return_value=archive):
                return mobile.archive_evidence(self.root, self.cache, inv, [p])

        with self.assertRaisesRegex(ValueError, "embedded"):
            probe("lib/vendor/hidden.js")
        with self.assertRaisesRegex(ValueError, "prebundled"):
            probe("lib/main.js", {"sources": ["../node_modules/hidden/source.js"]})
        self.assertEqual(probe("lib/main.js", {"sources": ["../src/main.ts"]}), {})

    def test_renamed_sdk_sources_and_modified_archive_input_cannot_gain_first_party_status(self):
        with self.assertRaisesRegex(ValueError, "unreviewed mobile source owner"):
            mobile.first_party(self.root, "prns-js/dist/shadow-library.js")
        self.assertEqual(
            mobile.npm_root("applications/node_modules/css-tree/node_modules/source-map/lib/index.js"),
            "applications/node_modules/css-tree/node_modules/source-map",
        )
        with self.assertRaisesRegex(ValueError, "escapes"):
            mobile.safe_path("applications/../../outside/foreign.js")


if __name__ == "__main__":
    unittest.main()
