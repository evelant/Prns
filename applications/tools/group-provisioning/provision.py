#!/usr/bin/env python3
"""Install one private development enrollment without replacing phone state."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import stat
import subprocess
import sys
import tempfile
from urllib.parse import unquote, urlparse

APP_ID = "rs.reticulum.prns.dev"
MAX_FIXTURE_BYTES = 4 * 1024 * 1024
ANDROID_ROOT = "no_backup/prns/development"
IOS_ROOT = "Library/Application Support/prns/development"
FIXTURE_NAME = "group-fixture.json"


class ProvisionError(Exception):
    """A safe diagnostic that must never include enrollment contents."""


@dataclass(frozen=True, repr=False)
class Fixture:
    content: bytes
    identity: str
    group: str
    digest: str


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ProvisionError("Fixture contains duplicate JSON fields.")
        result[key] = value
    return result


def byte_array(value, length):
    if not isinstance(value, list) or len(value) != length:
        raise ProvisionError("Invalid fixture identifier.")
    if any(type(item) is not int or not 0 <= item <= 255 for item in value):
        raise ProvisionError("Invalid fixture identifier.")
    return bytes(value)


def read_fixture(path: Path, expected_identity: str) -> Fixture:
    if not re.fullmatch(r"[0-9a-fA-F]{32}", expected_identity):
        raise ProvisionError("Expected identity must be the phone's 32-digit public identity hash.")
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor, "rb") as stream:
            metadata = os.fstat(stream.fileno())
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o077:
                raise ProvisionError("Fixture must be a private regular file (mode 0600 or 0400).")
            content = stream.read(MAX_FIXTURE_BYTES + 1)
    except OSError:
        raise ProvisionError("Cannot read private fixture file.") from None
    if len(content) > MAX_FIXTURE_BYTES:
        raise ProvisionError("Fixture exceeds the native 4 MiB limit.")
    try:
        document = json.loads(content, object_pairs_hook=unique_object)
        config = document["configuration"]
        if config["format"] != 1 or type(config["format"]) is not int:
            raise ProvisionError("Unsupported fixture format.")
        members = config["members"]
        local = config["local_member"]
        if not isinstance(members, list) or not 2 <= len(members) <= 8:
            raise ProvisionError("Unsupported fixture membership size.")
        if type(local) is not int or not 0 <= local < len(members):
            raise ProvisionError("Invalid fixture local member.")
        identity = byte_array(members[local]["reticulum_identity"], 16).hex()
        group = byte_array(config["group"], 32).hex()
        if not isinstance(document["snapshot"], dict):
            raise ProvisionError("Invalid fixture snapshot.")
    except (KeyError, TypeError, ValueError, UnicodeError, RecursionError):
        raise ProvisionError("Malformed enrollment fixture; contents omitted.") from None
    if identity != expected_identity.lower():
        raise ProvisionError("Fixture belongs to a different public identity; nothing copied.")
    return Fixture(content, identity, group, hashlib.sha256(content).hexdigest())


class Commands:
    def run(self, args, *, content=None, allowed=(0,)):
        try:
            result = subprocess.run(args, input=content, capture_output=True, timeout=45, check=False)
        except (OSError, subprocess.TimeoutExpired):
            raise ProvisionError("Device command unavailable or timed out; no automatic retry.") from None
        if result.returncode not in allowed:
            # Device tools may echo copied content or private container paths.
            raise ProvisionError("Device command failed; app must be installed, initialized, accessible, and stopped.")
        return result


class Android:
    def __init__(self, device: str, commands: Commands):
        self.prefix = ["adb", "-s", device]
        self.commands = commands

    def shell(self, script, *, content=None):
        return self.commands.run(
            self.prefix + ["shell", "-T", f"run-as {APP_ID} sh -c {shlex.quote(script)}"], content=content
        ).stdout

    def ensure_stopped(self):
        state = self.commands.run(self.prefix + ["get-state"]).stdout.strip()
        if state != b"device":
            raise ProvisionError("Selected Android device is not ready.")
        processes = self.commands.run(self.prefix + ["shell", "ps", "-A", "-o", "NAME"]).stdout
        names = processes.decode("utf-8", errors="strict").splitlines()
        if not names or names[0].strip() != "NAME":
            raise ProvisionError("Unrecognized Android process inventory; refusing to write.")
        if any(name.strip() == APP_ID or name.strip().startswith(APP_ID + ":") for name in names[1:]):
            raise ProvisionError("Stop the Android app before provisioning; this tool does not stop it.")
        self.shell(
            f"test -d {ANDROID_ROOT} && test ! -L {ANDROID_ROOT} && "
            f"test -f {ANDROID_ROOT}/identities/primary && test -f {ANDROID_ROOT}/application.redb"
        )

    def existing_digest(self):
        output = self.shell(
            f"cd {ANDROID_ROOT} || exit 1; "
            f"test ! -L {FIXTURE_NAME} || exit 1; "
            f"if test -e {FIXTURE_NAME}; then test -f {FIXTURE_NAME} || exit 1; "
            f"sha256sum {FIXTURE_NAME}; else echo absent; fi"
        ).strip()
        if output == b"absent":
            return None
        if not re.fullmatch(rb"[0-9a-f]{64}\s+group-fixture\.json", output):
            raise ProvisionError("Unrecognized Android fixture digest; refusing to write.")
        return output[:64].decode("ascii")

    def install(self, fixture: Fixture):
        # Android run-as SELinux policy can prohibit hard links. Noclobber creates
        # the final name exclusively; interruption can leave a partial fixture,
        # which later checks preserve and refuse to replace.
        self.shell(
            f"set -eu; umask 077; cd {ANDROID_ROOT}; "
            "stage=$(mktemp .group-fixture.XXXXXXXX); "
            "trap 'rm -f \"$stage\"' EXIT; cat > \"$stage\"; "
            f"test \"$(wc -c < \"$stage\")\" -eq {len(fixture.content)}; "
            f"test \"$(sha256sum \"$stage\" | cut -d ' ' -f 1)\" = {fixture.digest}; "
            f"sync; (set -C; cat \"$stage\" > {FIXTURE_NAME}); sync",
            content=fixture.content,
        )


def file_path(value):
    if not isinstance(value, str):
        raise ProvisionError("Unrecognized iOS path schema; refusing to write.")
    return unquote(urlparse(value).path) if value.startswith("file:") else value


class IOS:
    def __init__(self, device: str, commands: Commands):
        self.device = device
        self.commands = commands

    def call(self, arguments):
        # JSON goes to a private file so progress output cannot corrupt the response.
        with tempfile.TemporaryDirectory(prefix="prns-device-info-") as directory:
            destination = Path(directory) / "result.json"
            self.commands.run(["xcrun", "devicectl", "device", *arguments,
                               "--device", self.device, "--timeout", "30",
                               "--json-output", str(destination)])
            try:
                document = json.loads(destination.read_bytes())
                result = document["result"]
                if not isinstance(result, dict):
                    raise ValueError()
                return result
            except (OSError, ValueError, KeyError, TypeError):
                raise ProvisionError("Unrecognized devicectl result; refusing to write.") from None

    @staticmethod
    def rows(result, key):
        rows = result.get(key)
        if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
            raise ProvisionError("Unrecognized devicectl inventory; refusing to write.")
        return rows

    def ensure_stopped(self):
        apps = self.rows(self.call(["info", "apps", "--require-container-access"]), "apps")
        selected = [app for app in apps if app.get("bundleIdentifier") == APP_ID]
        if len(selected) != 1:
            raise ProvisionError("Development app container unavailable on selected iPhone.")
        bundle_path = file_path(selected[0].get("url")).rstrip("/") + "/"
        if not bundle_path.endswith(".app/"):
            raise ProvisionError("Unrecognized iOS app path; refusing to write.")
        processes = self.rows(self.call(["info", "processes"]), "runningProcesses")
        for process in processes:
            if file_path(process.get("executable")).startswith(bundle_path):
                raise ProvisionError("Stop the iOS app before provisioning; this tool does not stop it.")
        names = self.names_at(IOS_ROOT)
        if "application.redb" not in names or "identities" not in names:
            raise ProvisionError("Initialize the development app and identity before provisioning.")
        if "primary" not in self.names_at(IOS_ROOT + "/identities"):
            raise ProvisionError("Initialize the development app identity before provisioning.")

    def files_at(self, root):
        result = self.call(["info", "files", "--domain-type", "appDataContainer",
                            "--domain-identifier", APP_ID, "--subdirectory", root, "--no-recurse"])
        files = {}
        for entry in self.rows(result, "files"):
            name = entry.get("name")
            if not isinstance(name, str) or PurePosixPath(name).name != name:
                raise ProvisionError("Unrecognized iOS file inventory; refusing to write.")
            if name in files:
                raise ProvisionError("Duplicate iOS file entry; refusing to write.")
            files[name] = entry
        return files

    def names_at(self, root):
        return set(self.files_at(root))

    def copy(self, direction, source, destination):
        return self.call(["copy", direction, "--domain-type", "appDataContainer",
                          "--domain-identifier", APP_ID, "--source", str(source),
                          "--destination", str(destination)])

    def existing_digest(self):
        entry = self.files_at(IOS_ROOT).get(FIXTURE_NAME)
        if entry is None:
            return None
        if "resources" in entry:
            resources = entry["resources"]
            if (not isinstance(resources, dict)
                    or resources.get("isDirectory") is not False
                    or resources.get("isSymbolicLink") is not False
                    or ("isRegularFile" in resources and resources["isRegularFile"] is not True)
                    or ("isReadable" in resources and resources["isReadable"] is not True)):
                raise ProvisionError("Existing iOS fixture is not a readable regular file; refusing to copy it.")
        sizes = [entry[key] for key in ("fileSize", "size") if key in entry]
        if "metadata" in entry:
            metadata = entry["metadata"]
            if not isinstance(metadata, dict):
                raise ProvisionError("Unrecognized iOS fixture metadata; refusing to copy it.")
            if "size" in metadata:
                sizes.append(metadata["size"])
        if (not sizes or any(type(size) is not int or not 0 <= size <= MAX_FIXTURE_BYTES
                             for size in sizes) or any(size != sizes[0] for size in sizes)):
            raise ProvisionError("Existing iOS fixture size is unknown or oversized; refusing to copy it.")
        size = sizes[0]
        with tempfile.TemporaryDirectory(prefix="prns-private-readback-") as directory:
            path = Path(directory) / FIXTURE_NAME
            self.copy("from", IOS_ROOT + "/" + FIXTURE_NAME, path)
            if path.is_symlink() or not path.is_file():
                raise ProvisionError("Existing iOS fixture is not a regular file.")
            with path.open("rb") as stream:
                content = stream.read(MAX_FIXTURE_BYTES + 1)
            if len(content) > MAX_FIXTURE_BYTES:
                raise ProvisionError("Existing iOS fixture is oversized; refusing to replace it.")
            if len(content) != size:
                raise ProvisionError("Existing iOS fixture changed during readback; refusing to replace it.")
            return hashlib.sha256(content).hexdigest()

    def install(self, fixture: Fixture):
        # devicectl cannot atomically create-if-absent. The app must remain stopped;
        # the host lock excludes other invocations of this helper on this host.
        self.ensure_stopped()
        if self.existing_digest() is not None:
            raise ProvisionError("iOS fixture appeared during preflight; refusing to replace it.")
        with tempfile.TemporaryDirectory(prefix="prns-private-install-") as directory:
            path = Path(directory) / FIXTURE_NAME
            with path.open("xb") as stream:
                os.fchmod(stream.fileno(), 0o600)
                stream.write(fixture.content)
            self.copy("to", path, IOS_ROOT + "/" + FIXTURE_NAME)


def provision(backend, fixture, *, install):
    backend.ensure_stopped()
    existing = backend.existing_digest()
    if existing == fixture.digest:
        return "already-present"
    if existing is not None:
        raise ProvisionError("An existing fixture differs; refusing to overwrite enrollment or current MLS state.")
    if not install:
        return "ready-to-install"
    backend.install(fixture)
    if backend.existing_digest() != fixture.digest:
        raise ProvisionError("Copy did not verify. Preserve device state; do not replace the fixture or reset storage.")
    return "installed"


@contextmanager
def device_lock(platform, device):
    token = hashlib.sha256(f"{platform}:{device}".encode()).hexdigest()
    path = Path(tempfile.gettempdir()) / f"prns-provision-{os.getuid()}-{token}.lock"
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        if os.fstat(descriptor).st_uid != os.getuid():
            raise ProvisionError("Device provisioning lock belongs to another user.")
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ProvisionError("Another helper owns this device; refusing concurrent provisioning.") from None
        yield
    finally:
        os.close(descriptor)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "install"))
    parser.add_argument("--platform", required=True, choices=("android", "ios"))
    parser.add_argument("--device", required=True, help="Exact adb serial or iPhone UDID, never a display name")
    parser.add_argument("--identity", required=True, help="Public primary identity hash copied from this phone")
    parser.add_argument("--fixture", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if not re.fullmatch(r"[A-Za-z0-9._:-]+", args.device) or args.device.startswith("-"):
            raise ProvisionError("An explicit device serial/UDID is required.")
        if args.platform == "ios" and not re.fullmatch(r"(?:[0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}|[0-9A-Fa-f]{40})", args.device):
            raise ProvisionError("iOS requires a physical device UDID, not a name or alias.")
        fixture = read_fixture(args.fixture, args.identity)
        with device_lock(args.platform, args.device):
            backend = (Android if args.platform == "android" else IOS)(args.device, Commands())
            outcome = provision(backend, fixture, install=args.action == "install")
        print(json.dumps({"outcome": outcome, "platform": args.platform, "device": args.device,
                          "identity": fixture.identity, "group": fixture.group, "sha256": fixture.digest}))
        return 0
    except (ProvisionError, OSError, UnicodeError) as error:
        message = str(error) if isinstance(error, ProvisionError) else "Local file or device access failed."
        print(f"Provisioning refused: {message}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
