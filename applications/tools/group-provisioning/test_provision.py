from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("provision", Path(__file__).with_name("provision.py"))
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

IDENTITY = "12" * 16


def fixture_bytes():
    return json.dumps({"configuration": {
        "format": 1, "group": [3] * 32, "local_member": 0,
        "members": [{"reticulum_identity": [18] * 16}, {"reticulum_identity": [19] * 16}],
        "signing_secret": "private-test-marker",
    }, "snapshot": {}}).encode()


class FixtureTests(unittest.TestCase):
    def read(self, contents, identity=IDENTITY):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.json"
            path.write_bytes(contents)
            path.chmod(0o600)
            return MODULE.read_fixture(path, identity)

    def test_public_binding_and_digest_are_exact(self):
        fixture = self.read(fixture_bytes())
        self.assertEqual((fixture.identity, fixture.group, fixture.digest),
                         (IDENTITY, "03" * 32, hashlib.sha256(fixture_bytes()).hexdigest()))
        self.assertNotIn("private-test-marker", repr(fixture))

    def test_wrong_member_identity_refuses_before_device_access(self):
        with self.assertRaisesRegex(MODULE.ProvisionError, "different public identity"):
            self.read(fixture_bytes(), "13" * 16)

    def test_malformed_and_oversized_inputs_do_not_echo_contents(self):
        for contents in (b'{"private-test-marker":', b'[' * 2000,
                         b'{"configuration": 0, "configuration": "private-test-marker"}',
                         b"x" * (MODULE.MAX_FIXTURE_BYTES + 1)):
            with self.subTest(size=len(contents)), self.assertRaises(MODULE.ProvisionError) as caught:
                self.read(contents)
            self.assertNotIn("private-test-marker", str(caught.exception))

    def test_non_integer_identity_bytes_rejected(self):
        contents = json.loads(fixture_bytes())
        contents["configuration"]["members"][0]["reticulum_identity"][0] = True
        with self.assertRaises(MODULE.ProvisionError):
            self.read(json.dumps(contents).encode())

    def test_permissions_and_symlinks_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.json"
            path.write_bytes(fixture_bytes())
            path.chmod(0o644)
            with self.assertRaises(MODULE.ProvisionError):
                MODULE.read_fixture(path, IDENTITY)
            path.chmod(0o600)
            link = Path(directory) / "link.json"
            link.symlink_to(path)
            with self.assertRaises(MODULE.ProvisionError):
                MODULE.read_fixture(link, IDENTITY)


class Backend:
    def __init__(self, digest=None, running=False, readback=None):
        self.digest = digest
        self.running = running
        self.writes = 0
        self.readback = readback

    def ensure_stopped(self):
        if self.running:
            raise MODULE.ProvisionError("running")

    def existing_digest(self):
        return self.digest

    def install(self, fixture):
        self.writes += 1
        self.digest = self.readback or fixture.digest


class ProvisionTests(unittest.TestCase):
    fixture = MODULE.Fixture(b"private-test-marker", IDENTITY, "03" * 32, "a" * 64)

    def test_check_is_read_only_and_same_artifact_is_idempotent(self):
        backend = Backend()
        self.assertEqual(MODULE.provision(backend, self.fixture, install=False), "ready-to-install")
        self.assertEqual(backend.writes, 0)
        self.assertEqual(MODULE.provision(backend, self.fixture, install=True), "installed")
        self.assertEqual(MODULE.provision(backend, self.fixture, install=True), "already-present")
        self.assertEqual(backend.writes, 1)

    def test_changed_artifact_and_running_app_never_write(self):
        for backend in (Backend(digest="b" * 64), Backend(running=True)):
            with self.subTest(backend=backend), self.assertRaises(MODULE.ProvisionError):
                MODULE.provision(backend, self.fixture, install=True)
            self.assertEqual(backend.writes, 0)

    def test_failed_verification_never_retries_or_replaces(self):
        backend = Backend(readback="b" * 64)
        with self.assertRaisesRegex(MODULE.ProvisionError, "Copy did not verify"):
            MODULE.provision(backend, self.fixture, install=True)
        self.assertEqual(backend.writes, 1)
        with self.assertRaisesRegex(MODULE.ProvisionError, "existing fixture differs"):
            MODULE.provision(backend, self.fixture, install=True)
        self.assertEqual(backend.writes, 1)

    def test_lock_rejects_concurrent_helper(self):
        with MODULE.device_lock("test", f"unit-{os.getpid()}"):
            with self.assertRaisesRegex(MODULE.ProvisionError, "Another helper"):
                with MODULE.device_lock("test", f"unit-{os.getpid()}"):
                    self.fail("lock admitted second owner")

    def test_command_error_omits_stdout_stderr_and_args(self):
        result = subprocess.CompletedProcess([], 1, b"private-test-marker", b"private-test-marker")
        with patch.object(MODULE.subprocess, "run", return_value=result):
            with self.assertRaises(MODULE.ProvisionError) as caught:
                MODULE.Commands().run(["command", "private-test-marker"])
        self.assertNotIn("private-test-marker", str(caught.exception))


class FakeCommands:
    def __init__(self, outputs=()):
        self.outputs = iter(outputs)
        self.calls = []

    def run(self, args, **kwargs):
        self.calls.append((args, kwargs))
        return subprocess.CompletedProcess(args, 0, next(self.outputs, b""), b"")


class AndroidTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("sha256sum"), "requires host SHA-256 utility")
    def test_actual_publication_cannot_replace_existing_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            class LocalCommands(MODULE.Commands):
                def run(self, args, **kwargs):
                    remote = shlex.split(args[5])
                    script = remote[4].replace(MODULE.ANDROID_ROOT, shlex.quote(directory))
                    script = script.replace("sync;", ":;").replace("; sync", "; :")
                    return super().run(["sh", "-c", script], **kwargs)

            backend = MODULE.Android("unused-device", LocalCommands())
            first = b"first private enrollment"
            fixture = MODULE.Fixture(first, IDENTITY, "03" * 32, hashlib.sha256(first).hexdigest())
            backend.install(fixture)
            path = Path(directory) / MODULE.FIXTURE_NAME
            inode = path.stat().st_ino
            replacement = b"replacement enrollment"
            with self.assertRaises(MODULE.ProvisionError):
                backend.install(MODULE.Fixture(replacement, IDENTITY, "03" * 32,
                                               hashlib.sha256(replacement).hexdigest()))
            self.assertEqual((path.read_bytes(), path.stat().st_ino, path.stat().st_mode & 0o777),
                             (first, inode, 0o600))
            self.assertEqual(sorted(item.name for item in Path(directory).iterdir()), [MODULE.FIXTURE_NAME])
            original = Path(directory) / "original"
            original.write_bytes(b"preserved target")
            path.unlink()
            path.symlink_to(original)
            with self.assertRaises(MODULE.ProvisionError):
                backend.install(fixture)
            self.assertTrue(path.is_symlink())
            self.assertEqual(original.read_bytes(), b"preserved target")
            self.assertFalse(list(Path(directory).glob(".group-fixture.*")))

    def test_private_staging_uses_stdin_and_exclusive_no_replace(self):
        commands = FakeCommands()
        fixture = MODULE.Fixture(b"private-test-marker", IDENTITY, "03" * 32, "a" * 64)
        MODULE.Android("explicit-serial", commands).install(fixture)
        args, kwargs = commands.calls[0]
        self.assertEqual(args[:5], ["adb", "-s", "explicit-serial", "shell", "-T"])
        self.assertEqual(kwargs["content"], fixture.content)
        self.assertNotIn("private-test-marker", " ".join(args))
        remote = shlex.split(args[5])
        self.assertEqual(remote[:4], ["run-as", MODULE.APP_ID, "sh", "-c"])
        script = remote[4]
        self.assertIn('(set -C; cat "$stage" > group-fixture.json)', script)
        self.assertNotIn("mv ", script)
        self.assertNotIn("sdcard", script)
        self.assertIn("umask 077", script)
        # Android 10 toybox sync accepts no file arguments.
        self.assertIn('sync; (set -C; cat "$stage" > group-fixture.json); sync', script)

    def test_running_main_or_service_process_is_rejected(self):
        for name in (MODULE.APP_ID, MODULE.APP_ID + ":service"):
            commands = FakeCommands([b"device\n", f"NAME\n{name}\n".encode()])
            with self.assertRaisesRegex(MODULE.ProvisionError, "Stop the Android app"):
                MODULE.Android("serial", commands).ensure_stopped()
            self.assertEqual(len(commands.calls), 2)

    def test_unknown_process_or_digest_output_fails_closed(self):
        with self.assertRaises(MODULE.ProvisionError):
            MODULE.Android("serial", FakeCommands([b"device\n", b""])).ensure_stopped()
        with self.assertRaises(MODULE.ProvisionError):
            MODULE.Android("serial", FakeCommands([b"tool failure"])).existing_digest()
        self.assertIsNone(MODULE.Android("serial", FakeCommands([b"absent\n"])).existing_digest())


class IOSTests(unittest.TestCase):
    def test_device_metadata_size_and_resource_flags_allow_exact_readback(self):
        content = fixture_bytes()
        fixture = MODULE.Fixture(content, IDENTITY, "03" * 32, hashlib.sha256(content).hexdigest())
        device = MODULE.IOS("udid", FakeCommands())
        entry = {
            "name": MODULE.FIXTURE_NAME,
            "relativePath": MODULE.FIXTURE_NAME,
            "metadata": {"extendedAttributes": {}, "ownerGid": 501, "ownerUid": 501,
                         "permissions": 384, "size": len(content)},
            "resources": {"isDirectory": False, "isHidden": False, "isReadable": True,
                          "isSymbolicLink": False, "isWritable": True},
        }

        def copy(direction, source, destination):
            self.assertEqual(direction, "from")
            self.assertEqual(source, MODULE.IOS_ROOT + "/" + MODULE.FIXTURE_NAME)
            destination.write_bytes(content)

        with patch.object(device, "ensure_stopped"), \
                patch.object(device, "call", return_value={"files": [entry]}), \
                patch.object(device, "copy", side_effect=copy), patch.object(device, "install") as install:
            self.assertEqual(MODULE.provision(device, fixture, install=True), "already-present")
        install.assert_not_called()

    def test_unknown_conflicting_or_oversized_nested_size_is_not_downloaded(self):
        entries = [{"metadata": {"size": size}} for size in
                   (None, -1, True, 1.5, "10", MODULE.MAX_FIXTURE_BYTES + 1)]
        entries += [{"metadata": None}, {"metadata": {}},
                    {"size": 10, "metadata": {"size": 11}},
                    {"fileSize": None, "metadata": {"size": 10}}]
        for entry in entries:
            device = MODULE.IOS("udid", FakeCommands())
            with self.subTest(entry=entry), \
                    patch.object(device, "files_at", return_value={MODULE.FIXTURE_NAME: entry}), \
                    patch.object(device, "copy") as copy, self.assertRaises(MODULE.ProvisionError):
                device.existing_digest()
            copy.assert_not_called()

    def test_nonregular_symlink_or_unrecognized_resource_flags_are_not_downloaded(self):
        resources = (None, {}, {"isDirectory": False},
                     {"isDirectory": True, "isSymbolicLink": False},
                     {"isDirectory": False, "isSymbolicLink": True},
                     {"isDirectory": 0, "isSymbolicLink": False},
                     {"isDirectory": False, "isSymbolicLink": "false"},
                     {"isDirectory": False, "isSymbolicLink": False, "isRegularFile": False},
                     {"isDirectory": False, "isSymbolicLink": False, "isReadable": False})
        for flags in resources:
            device = MODULE.IOS("udid", FakeCommands())
            entry = {"metadata": {"size": 10}, "resources": flags}
            with self.subTest(flags=flags), \
                    patch.object(device, "files_at", return_value={MODULE.FIXTURE_NAME: entry}), \
                    patch.object(device, "copy") as copy, self.assertRaises(MODULE.ProvisionError):
                device.existing_digest()
            copy.assert_not_called()

    def test_readback_size_change_refuses_success_without_replacing_file(self):
        device = MODULE.IOS("udid", FakeCommands())
        with patch.object(device, "files_at", return_value={MODULE.FIXTURE_NAME: {"metadata": {"size": 1}}}), \
                patch.object(device, "copy", side_effect=lambda direction, source, destination: destination.write_bytes(b"changed")), \
                self.assertRaisesRegex(MODULE.ProvisionError, "changed during readback"):
            device.existing_digest()

    def test_unknown_or_oversized_existing_fixture_is_not_downloaded(self):
        for size in (None, -1, True, MODULE.MAX_FIXTURE_BYTES + 1):
            device = MODULE.IOS("udid", FakeCommands())
            with patch.object(device, "files_at", return_value={MODULE.FIXTURE_NAME: {"size": size}}), \
                    patch.object(device, "copy") as copy, self.assertRaises(MODULE.ProvisionError):
                device.existing_digest()
            copy.assert_not_called()

    def test_running_exact_bundle_is_rejected(self):
        device = MODULE.IOS("explicit-udid", FakeCommands())
        with patch.object(device, "call", side_effect=[
            {"apps": [{"bundleIdentifier": MODULE.APP_ID, "url": "file:///private/App/prns.app/"}]},
            {"runningProcesses": [{"executable": "file:///private/App/prns.app/prns"}]},
        ]), self.assertRaisesRegex(MODULE.ProvisionError, "Stop the iOS app"):
            device.ensure_stopped()

    def test_unknown_inventory_fails_closed(self):
        for result in ({}, {"files": None}, {"files": ["bad"]}, {"files": [{"name": "../bad"}]}):
            device = MODULE.IOS("udid", FakeCommands())
            with patch.object(device, "call", return_value=result), self.assertRaises(MODULE.ProvisionError):
                device.names_at(MODULE.IOS_ROOT)

    def test_new_fixture_race_refuses_copy(self):
        device = MODULE.IOS("udid", FakeCommands())
        with patch.object(device, "ensure_stopped"), patch.object(device, "existing_digest", return_value="a" * 64), \
                patch.object(device, "copy") as copy, self.assertRaises(MODULE.ProvisionError):
            device.install(ProvisionTests.fixture)
        copy.assert_not_called()

    def test_staging_is_private_and_temporary(self):
        device = MODULE.IOS("udid", FakeCommands())
        paths = []

        def copy(direction, source, destination):
            paths.append(source)
            self.assertEqual(direction, "to")
            self.assertEqual(destination, MODULE.IOS_ROOT + "/group-fixture.json")
            self.assertEqual(source.read_bytes(), ProvisionTests.fixture.content)
            self.assertEqual(source.stat().st_mode & 0o777, 0o600)
            self.assertEqual(source.parent.stat().st_mode & 0o777, 0o700)

        with patch.object(device, "ensure_stopped"), patch.object(device, "existing_digest", return_value=None), \
                patch.object(device, "copy", side_effect=copy):
            device.install(ProvisionTests.fixture)
        self.assertTrue(paths)
        self.assertFalse(paths[0].exists())

    def test_copy_is_app_private_and_never_removes_existing_content(self):
        device = MODULE.IOS("udid", FakeCommands())
        with patch.object(device, "call", return_value={}) as call:
            device.copy("to", Path("/private/staged"), MODULE.IOS_ROOT + "/group-fixture.json")
        args = call.call_args.args[0]
        self.assertIn("appDataContainer", args)
        self.assertIn(MODULE.APP_ID, args)
        self.assertNotIn("--remove-existing-content", args)


if __name__ == "__main__":
    unittest.main()
