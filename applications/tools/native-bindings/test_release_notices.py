import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid
import struct
import zipfile

import check_release_notices as notices
import ios_release_notices as ios


class ReleaseArchiveTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.bundle = self.directory / "bundle"
        self.bundle.write_bytes(b"release JavaScript")
        self.source_map = self.directory / "output.map"
        self.source_map.write_text('{"version":3,"sources":[],"mappings":""}')
        for name in ("npm-ios", "npm-android", "native-sources", "android-maven"):
            (self.directory / (name + ".txt")).write_text(name)
        self.patch = patch.object(notices, "NOTICES", self.directory)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def run_check(self, platform, changes=None):
        app = "Payload/PRNSChat.app/"
        ack = self.directory / "Pods-PRNSChat-acknowledgements.markdown"
        ack.write_bytes(b"pod acknowledgements")
        receipt = {"frameworks": ["prns_chat_app"], "device_framework_uuids": {}}
        if platform == "ios":
            archive = self.directory / "PRNS.ipa"
            entries = {app + "Info.plist": plistlib.dumps({"CFBundleIdentifier": "rs.reticulum.prns.chat"}),
                       app + "main.jsbundle": self.bundle.read_bytes(),
                       app + "prns-cocoapods-notices.txt": ack.read_bytes(),
                       app + "Frameworks/prns_chat_app.framework/prns_chat_app": b"native image",
                       app + "assets/path/legal-ios.txt": b"npm-ios",
                       app + "assets/path/legal-rust.txt": b"native-sources"}
        else:
            archive = self.directory / "PRNS.apk"
            entries = {"assets/index.android.bundle": self.bundle.read_bytes(),
                       "res/raw/generateda.txt": b"npm-android", "res/raw/generatedb.txt": b"native-sources",
                       "res/raw/generatedc.txt": b"android-maven"}
        entries.update(changes or {})
        with zipfile.ZipFile(archive, "w") as output:
            for member, content in entries.items():
                output.writestr(member, content)
        with patch.object(ios, "render", return_value=(ack.read_bytes(), receipt)):
            return notices.inspect_archive(archive, platform, self.bundle, ack, self.source_map)

    def test_exact_bundle_and_platform_notices(self):
        for platform in ("ios", "android"):
            result = self.run_check(platform)
            self.assertEqual(len(result["notice_members"]), 3)
            self.assertEqual(result["bundle_sha256"], notices.digest(self.bundle))

    def test_rejects_packaged_maps_including_renamed_by_packager(self):
        for platform, member in (("ios", "Payload/PRNSChat.app/main.jsbundle.map"),
                                  ("android", "res/raw/releaseasset.txt")):
            with self.subTest(platform=platform), self.assertRaisesRegex(ValueError, "source map"):
                self.run_check(platform, {member: self.source_map.read_bytes()})

    def test_frameworks_and_pod_notices_are_bound_to_archive(self):
        for name in ("prns-cocoapods-notices.txt", "Frameworks/Foreign.framework/Foreign"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "iOS"):
                self.run_check("ios", {"Payload/PRNSChat.app/" + name: b"different"})

    def test_debug_framework_does_not_match_release_uuid(self):
        value = uuid.UUID("4e9b3b7c-6961-3be7-8afd-8c71210a55a3")
        header = struct.pack("<IIIIIIII", 0xfeedfacf, 0x0100000c, 0, 6, 1, 24, 0, 0)
        binary = header + struct.pack("<II", 0x1b, 24) + value.bytes
        self.assertEqual(notices.arm64_framework_uuid(binary), str(value))
        with self.assertRaisesRegex(ValueError, "load command"):
            notices.arm64_framework_uuid(binary[:-1])

    def test_plugin_serializes_valid_xcode_paths_and_shell_phase(self):
        plutil = shutil.which("plutil")
        xcode = notices.ROOT / "applications/node_modules/xcode"
        if not plutil or not xcode.is_dir() or not Path("/usr/bin/xcodebuild").is_file():
            self.skipTest("requires Apple's plutil and installed app tooling")
        project = notices.ROOT / "personal-hopspot/mobile/ios/app/PersonalHopspot.xcodeproj/project.pbxproj"
        output = self.directory / "project.pbxproj"
        fixture = """
        const fs = require('node:fs');
        const xcode = require('./applications/node_modules/xcode');
        const plugin = require('./applications/prns/platform/app.plugin.cjs');
        const project = xcode.project(process.argv[1]);
        project.parseSync();
        (async () => {
          for (let n = 0; n < 2; n++) {
            const config = plugin({});
            await config.mods.ios.xcodeproj({modRequest:{}, modResults:project});
          }
          fs.writeFileSync(process.argv[2], project.writeSync());
        })().catch(error => { console.error(error); process.exitCode=1; });
        """
        subprocess.run(["node", "-e", fixture, project, output], check=True, cwd=notices.ROOT)
        objects = json.loads(subprocess.check_output([plutil, "-convert", "json", "-o", "-", output]))["objects"]
        phases = [row for row in objects.values() if row.get("name") == "Package CocoaPods acknowledgements"]
        self.assertEqual(len(phases), 1)
        self.assertTrue(phases[0]["inputPaths"][0].endswith("-acknowledgements.markdown"))
        self.assertIn('"$SCRIPT_INPUT_FILE_0"', phases[0]["shellScript"])


if __name__ == "__main__":
    unittest.main()
