import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "release" / "stage-napi-platform-packages.py"
SPEC = importlib.util.spec_from_file_location("stage_napi_platform_packages", SCRIPT)
STAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGER)
PACK_SPEC = importlib.util.spec_from_file_location("verify_js_sdk_pack", ROOT / "tools/release/verify-js-sdk-pack.py")
PACK = importlib.util.module_from_spec(PACK_SPEC)
PACK_SPEC.loader.exec_module(PACK)



class StageNapiPlatformPackagesTests(unittest.TestCase):
    def setUp(self):
        fixture = patch.object(STAGER.js_sdk_notices, "render", side_effect=lambda platform: f"notices for {platform}")
        fixture.start()
        self.addCleanup(fixture.stop)

    def create_packages(self, root):
        version = (ROOT / "VERSION").read_text().strip()
        for platform in STAGER.PLATFORMS:
            directory = root / platform
            directory.mkdir()
            binding = f"personal-rns.{platform}.node"
            (directory / binding).write_bytes(b"binding")
            (directory / "package.json").write_text(
                json.dumps(
                    {
                        "name": f"personal-rns-{platform}",
                        "version": version,
                        "main": binding,
                        "files": [binding],
                    }
                )
            )

    def test_stage_includes_canonical_documents_in_every_allowlist(self):
        with tempfile.TemporaryDirectory() as temporary:
            packages = Path(temporary)
            self.create_packages(packages)
            STAGER.stage(packages)
            STAGER.stage(packages)
            for platform in STAGER.PLATFORMS:
                directory = packages / platform
                manifest = json.loads((directory / "package.json").read_text())
                self.assertEqual(
                    manifest["files"],
                    [manifest["main"], "README.md", "LICENSE-APACHE", "LICENSE-MIT", "THIRD_PARTY_NOTICES.md"],
                )
                self.assertEqual((directory / "THIRD_PARTY_NOTICES.md").read_text(), f"notices for {platform}")
                for name, source in STAGER.DOCUMENTS.items():
                    self.assertEqual((directory / name).read_bytes(), source.read_bytes())

    def test_platform_archive_requires_exact_payload_and_notices(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.create_packages(root)
            STAGER.stage(root)
            directory = root / "linux-x64-gnu"
            expected = {path.name: path.read_bytes() for path in directory.iterdir()}
            archive = root / "sample.tgz"
            for edit in (None, "missing", "changed", "wrong-platform", "bundled"):
                with self.subTest(edit=edit), tarfile.open(archive, "w:gz") as tar:
                    contents = expected.copy()
                    if edit == "missing":
                        del contents["THIRD_PARTY_NOTICES.md"]
                    elif edit == "changed":
                        contents["THIRD_PARTY_NOTICES.md"] = b"notices for windows"
                    elif edit == "wrong-platform":
                        contents["personal-rns.win32-x64-msvc.node"] = b"extra binary"
                    elif edit == "bundled":
                        contents["node_modules/dev-tool/index.js"] = b"unreviewed"
                    for name, data in contents.items():
                        member = tarfile.TarInfo("package/" + name)
                        member.size = len(data)
                        tar.addfile(member, io.BytesIO(data))
                if edit is None:
                    PACK.verify(archive, directory)
                else:
                    with self.assertRaises(ValueError):
                        PACK.verify(archive, directory)

    def test_facade_requires_the_release_wasm_loader_and_external_type_dependency(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "prns-js"
            directory.mkdir()
            metadata = {
                "name": "personal-rns", "version": "1.0.0", "dependencies": {"casework": "0.2.2"},
                "optionalDependencies": {f"personal-rns-{platform}": "1.0.0" for platform in STAGER.PLATFORMS},
                "exports": {".": {"node": {"import": "./dist/native/index.js"}}},
            }
            manifest = directory / "package.json"
            manifest.write_text(json.dumps(metadata))
            wasm = root / "prns-wasm/smoke/pkg/prns_wasm_bg.wasm"
            wasm.parent.mkdir(parents=True)
            wasm.write_bytes(b"\x00asm\x01\x00\x00\x00")
            with patch.object(PACK, "ROOT", root):
                sources = PACK.package_sources(directory)
                self.assertEqual(sources["native/addon.cjs"], root / "prns-napi/index.js")
                self.assertEqual(sources["wasm/prns_wasm_bg.wasm"], wasm)
                self.assertIn("dist/native/index.js", sources)
                with self.assertRaisesRegex(ValueError, "missing or stale"):
                    PACK.prepare_facade(directory)
                metadata["bundleDependencies"] = ["casework"]
                manifest.write_text(json.dumps(metadata))
                with self.assertRaisesRegex(ValueError, "external casework"):
                    PACK.package_sources(directory)
                del metadata["bundleDependencies"]
                manifest.write_text(json.dumps(metadata))
                wasm.write_bytes(b"placeholder")
                with self.assertRaisesRegex(ValueError, "WebAssembly"):
                    PACK.package_sources(directory)

    def test_stage_rejects_incomplete_platform_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            packages = Path(temporary)
            self.create_packages(packages)
            missing = packages / "darwin-arm64"
            for path in missing.iterdir():
                path.unlink()
            missing.rmdir()
            with self.assertRaisesRegex(ValueError, "expected platform packages"):
                STAGER.stage(packages)

    def test_stage_rejects_an_expanded_generated_allowlist(self):
        with tempfile.TemporaryDirectory() as temporary:
            packages = Path(temporary)
            self.create_packages(packages)
            manifest_path = packages / "linux-x64-gnu" / "package.json"
            manifest = json.loads(manifest_path.read_text())
            manifest["files"].append("unexpected.bin")
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "unexpected generated package files"):
                STAGER.stage(packages)


if __name__ == "__main__":
    unittest.main()
