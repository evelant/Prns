import base64
import copy
import json
from pathlib import Path
import unittest

from validation.security.npm_lockfile import reviewed_mobile_runtime_license, verify_development_packages


class MobileRuntimeLicenseTests(unittest.TestCase):
    def test_exceptions_require_the_complete_reviewed_registry_identity(self):
        lock = Path(__file__).resolve().parents[2] / "tools/uniffi/react-native.json"
        packages = json.loads(lock.read_text())["packages"]
        self.assertEqual({item["name"] for item in packages}, {"@ubjs/core", "@ubjs/react-native"})
        for item in packages:
            identity = dict(name=item["name"], version=item["version"],
                            license_expression=item["license"], integrity=item["integrity"],
                            archive_url=item["archive"])
            self.assertTrue(reviewed_mobile_runtime_license(**identity))
            for field, changed in (
                ("name", "@ubjs/unreviewed"), ("version", "0.31.0-7"),
                ("license_expression", "MPL-2.0 OR MIT"),
                ("integrity", next(p["integrity"] for p in packages if p["name"] != item["name"])),
                ("archive_url", item["archive"].replace("registry.npmjs.org", "example.com")),
            ):
                with self.subTest(package=item["name"], field=field):
                    self.assertFalse(reviewed_mobile_runtime_license(**{**identity, field: changed}))


class NpmBundleProvenanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.parent = "node_modules/@vendor/archive"
        self.child = f"{self.parent}/node_modules/@vendor/child"
        self.packages = {
            self.parent: {
                "version": "1.0.0",
                "dev": True,
                "license": "MIT",
                "resolved": "https://registry.npmjs.org/@vendor/archive/-/archive-1.0.0.tgz",
                "integrity": "sha512-" + base64.b64encode(b"x" * 64).decode(),
                "bundleDependencies": ["@vendor/child"],
            },
            self.child: {
                "version": "1.0.0",
                "dev": True,
                "license": "0BSD",
                "inBundle": True,
            },
        }

    def test_declared_bundle_inherits_archive_integrity_but_keeps_its_license(self) -> None:
        verify_development_packages(self.packages, {"MIT", "0BSD"})
        with self.assertRaisesRegex(ValueError, "license"):
            verify_development_packages(self.packages, {"MIT"})

    def test_bundle_cannot_hide_missing_or_untrusted_parent_provenance(self) -> None:
        changes = (
            {"resolved": "https://untrusted.example/archive.tgz"},
            {"integrity": ""},
            {"integrity": "sha512-invalid"},
            {"bundleDependencies": []},
            {"bundleDependencies": "@vendor/child"},
            {"dev": False},
            {"inBundle": True},
        )
        for change in changes:
            with self.subTest(change=change):
                packages = copy.deepcopy(self.packages)
                packages[self.parent].update(change)
                with self.assertRaises(ValueError):
                    verify_development_packages(packages, {"MIT", "0BSD"})
        del self.packages[self.parent]
        with self.assertRaises(ValueError):
            verify_development_packages(self.packages, {"MIT", "0BSD"})

    def test_unbundled_package_must_have_its_own_integrity(self) -> None:
        del self.packages[self.child]["inBundle"]
        with self.assertRaisesRegex(ValueError, "integrity"):
            verify_development_packages(self.packages, {"MIT", "0BSD"})


if __name__ == "__main__":
    unittest.main()
