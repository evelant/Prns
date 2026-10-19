"""Explicit E290 MLS qualification lane; never used by the shipping builder."""
from __future__ import annotations

import argparse
import csv
from datetime import datetime, timezone
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

if os.name == "posix":
    import termios
    import tty

ROOT = Path(__file__).resolve().parents[2]
WORKSPACE = ROOT / "personal-hopspot/embedded/esp32"
TARGET = "xtensa-esp32s3-none-elf"
BINARY = "hopspot-heltec-e290"
FEATURE = "group-mls-qualification"
SECURE_FEATURE = "group-mls-secure-qualification"
MLS_VARIANTS = ("mls", "secure")
QUALIFICATION_RUSTFLAGS = ["-C", "link-arg=-Tlinkall.x", "-C", "force-frame-pointers=no"]
TARGET_RUSTFLAGS_VARIABLE = "CARGO_TARGET_XTENSA_ESP32S3_NONE_ELF_RUSTFLAGS"
RUSTFLAGS_VARIABLES = ("CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", TARGET_RUSTFLAGS_VARIABLE)
MLS_START = 0x600000
MLS_END = 0xE00000
LOG_LIMIT = 64 * 1024 * 1024


def parse_args(arguments: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", choices=("baseline", *MLS_VARIANTS), default="mls")
    parser.add_argument("--output", type=Path, required=True, help="New evidence directory.")
    parser.add_argument("--target-dir", type=Path, help="Cargo output; defaults to a new temporary directory.")
    parser.add_argument("--export-esp", type=Path, default=Path.home() / "export-esp.sh")
    parser.add_argument("--elf", type=Path, help="Package/flash this existing ELF without building.")
    parser.add_argument("--elf-flags-profile", choices=("unknown", "workspace-default", "no-frame-pointers"),
                        default="unknown", help="Operator-declared compiler flags of an existing --elf; not independently verified.")
    parser.add_argument("--build-only", action="store_true", help="Build/package without opening a device.")
    parser.add_argument("--port", help="Exact serial port; device auto-selection is never used.")
    parser.add_argument("--erase-mls", action="store_true", help="Erase only 0x600000..0xE00000 before flashing an MLS variant.")
    parser.add_argument("--monitor-only", action="store_true", help="Monitor an existing --elf without flashing or resetting.")
    parser.add_argument("--monitor-seconds", type=int, default=90, help="0 disables monitoring; maximum 3600 seconds.")
    parser.add_argument("--offline", action="store_true", help="Require locally cached Cargo dependencies.")
    args = parser.parse_args(arguments)
    if not 0 <= args.monitor_seconds <= 3600:
        parser.error("--monitor-seconds must be between 0 and 3600")
    if not args.build_only and not args.port:
        parser.error("device operations require an exact --port")
    if args.erase_mls and (args.build_only or args.monitor_only or args.variant not in MLS_VARIANTS):
        parser.error("--erase-mls requires an MLS flash operation")
    if args.monitor_only and (args.build_only or args.elf is None or args.monitor_seconds == 0):
        parser.error("--monitor-only requires --elf, a positive duration, and no --build-only")
    if args.elf is None and args.elf_flags_profile != "unknown":
        parser.error("--elf-flags-profile requires --elf")
    return args


def esp_environment(export: Path) -> dict[str, str]:
    env = dict(os.environ)
    if export.is_file():
        # The user-owned espup export is executed as a positional path, never shell-interpolated.
        result = subprocess.run(
            ["bash", "-c", 'source "$1" >/dev/null && env -0', "esp-environment", str(export)],
            check=True, capture_output=True, env=env,
        )
        env = dict(item.decode().split("=", 1) for item in result.stdout.split(b"\0") if item)
    env["ESPFLASH_SKIP_UPDATE_CHECK"] = "true"
    env["CARGO_TERM_COLOR"] = "never"
    return env


def qualification_flags(env: dict[str, str]) -> dict:
    """Fence this experimental lane from inherited flags and the shipping frame-pointer policy."""
    inherited = {}
    for name in RUSTFLAGS_VARIABLES:
        value = env.get(name, "")
        if not value:
            inherited[name] = "absent"
            continue
        flags = value.split("\x1f") if name == "CARGO_ENCODED_RUSTFLAGS" else shlex.split(value)
        if flags != QUALIFICATION_RUSTFLAGS:
            raise ValueError(f"incompatible inherited {name}; unset it or use the documented qualification flag list")
        inherited[name] = "compatible"
    encoded = "\x1f".join(QUALIFICATION_RUSTFLAGS)
    textual = " ".join(QUALIFICATION_RUSTFLAGS)
    # Cargo's encoded variable has highest priority. Keep the textual alternatives
    # consistent as well; none can re-enable the affected Xtensa frame-pointer ABI.
    env["CARGO_ENCODED_RUSTFLAGS"] = encoded
    env["RUSTFLAGS"] = textual
    env[TARGET_RUSTFLAGS_VARIABLE] = textual
    return {"profile": "qualification-no-frame-pointers", "input_mode": "tool-built",
            "rustflags": QUALIFICATION_RUSTFLAGS, "encoded_rustflags": encoded,
            "inherited_variables": inherited, "applied_to_build": True}


def partition_file(variant: str) -> Path:
    suffix = "-mls" if variant in MLS_VARIANTS else ""
    return WORKSPACE / f"partitions-hopspot-16mb{suffix}.csv"


def validate_partition(path: Path, variant: str) -> None:
    rows = []
    with path.open(newline="") as handle:
        for row in csv.reader(line for line in handle if line.strip() and not line.lstrip().startswith("#")):
            name, _, _, start, size, *_ = (field.strip() for field in row)
            rows.append((name, int(start, 0), int(start, 0) + int(size, 0)))
    intervals = sorted(rows, key=lambda row: row[1])
    if any(a[2] > b[1] for a, b in zip(intervals, intervals[1:])):
        raise ValueError("partition table overlaps")
    entries = {name: (start, end) for name, start, end in rows}
    if len(entries) != len(rows):
        raise ValueError("partition table has duplicate names")
    required = {
        "nvs": (0x9000, 0xC000),
        "ble_id": (0xC000, 0xD000),
        "hopcfg": (0xD000, 0xE000),
        "node_id": (0xE000, 0xF000),
        "phy_init": (0xF000, 0x10000),
        "factory": (0x10000, MLS_START if variant in MLS_VARIANTS else 0xE7D000),
        "remote_ctl_id": (0xE7D000, 0xE7E000),
        "radio_cfg": (0xE7E000, 0xE80000),
        "prns_state": (0xE80000, 0xFFE000),
        "wifi_cfg": (0xFFE000, 0x1000000),
    }
    if any(entries.get(name) != bounds for name, bounds in required.items()):
        raise ValueError("partition table does not preserve the E290 qualification contract")
    if variant in MLS_VARIANTS and (
        entries.get("mls_alice") != (MLS_START, 0xA00000)
        or entries.get("mls_bob") != (0xA00000, MLS_END)
    ):
        raise ValueError("MLS partitions must own 0x600000..0xA00000 and 0xA00000..0xE00000")


def build_command(args: argparse.Namespace, target_dir: Path) -> list[str]:
    command = ["cargo", "+esp", "build", "--release", "--locked", "--package", BINARY,
               "--target", TARGET, "--target-dir", str(target_dir), "-Zbuild-std=core,alloc"]
    if args.variant in MLS_VARIANTS:
        command.extend(["--features", SECURE_FEATURE if args.variant == "secure" else FEATURE])
    if args.offline:
        command.append("--offline")
    return command


def image_options(partition: Path) -> list[str]:
    return ["--chip", "esp32s3", "--flash-mode", "dio", "--flash-freq", "40mhz",
            "--flash-size", "16mb", "--partition-table", str(partition),
            "--partition-table-offset", "0x8000", "--target-app-partition", "factory"]


def device_options(port: str, *, before: str = "usb-reset", after: str = "no-reset") -> list[str]:
    return ["--port", port, "--chip", "esp32s3", "--before", before,
            "--after", after, "--non-interactive", "--skip-update-check"]


def erase_mls_command(port: str) -> list[str]:
    """Erase only the two qualification stores, preserving all radio and identity regions."""
    return ["espflash", "erase-region", *device_options(port, before="no-reset"),
            hex(MLS_START), hex(MLS_END - MLS_START)]


def sha256(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def run_logged(command: list[str], path: Path, env: dict[str, str], *,
               cwd: Path = ROOT, seconds: int | None = None) -> dict:
    """Stream bounded logs; a monitor deadline is recorded, never called a test pass."""
    started = time.monotonic()
    timed_out = False
    output_bytes = 0
    process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert process.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        with path.open("wb") as log:
            while selector.get_map():
                if seconds is not None and time.monotonic() - started >= seconds and process.poll() is None:
                    timed_out = True
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                for key, _ in selector.select(timeout=0.2):
                    data = os.read(key.fileobj.fileno(), 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                        continue
                    output_bytes += len(data)
                    if output_bytes > LOG_LIMIT:
                        raise RuntimeError("command exceeded the 64 MiB evidence log bound")
                    log.write(data)
                    log.flush()
                    sys.stdout.buffer.write(data)
                    sys.stdout.buffer.flush()
        code = process.wait()
    finally:
        selector.close()
        if process.poll() is None:
            process.kill()
            process.wait()
        process.stdout.close()
    result = {"command": command, "return_code": code, "duration_seconds": time.monotonic() - started,
              "monitor_deadline_reached": timed_out, "log": path.name, "sha256": sha256(path)}
    if code and not timed_out:
        raise RuntimeError(f"command failed ({code}); see {path}")
    return result


def validate_board_info(text: str) -> None:
    plain = re.sub(r"\x1b\[[0-9;]*m", "", text)
    if not re.search(r"Chip type:\s*esp32s3\b", plain, re.IGNORECASE):
        raise ValueError("preflight did not identify ESP32-S3")
    if not re.search(r"Flash size:\s*16\s*MB\b", plain, re.IGNORECASE):
        raise ValueError("preflight did not identify 16 MiB flash")


def monitor_port(port: str, path: Path, seconds: int) -> dict:
    """Read one exact POSIX port through USB re-enumeration; never toggle reset lines."""
    if os.name != "posix":
        raise RuntimeError("the E290 qualification monitor requires macOS or Linux")
    started = time.monotonic()
    descriptor = None
    connections = 0
    received = 0
    logged = 0
    events = []
    selector = selectors.DefaultSelector()
    try:
        with path.open("wb") as log:
            def record(data: bytes) -> None:
                nonlocal logged
                logged += len(data)
                if logged > LOG_LIMIT:
                    raise RuntimeError("monitor exceeded the 64 MiB evidence log bound")
                log.write(data)
                log.flush()
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()

            def disconnect(reason: str) -> None:
                nonlocal descriptor
                if descriptor is not None:
                    selector.unregister(descriptor)
                    os.close(descriptor)
                    descriptor = None
                elapsed = time.monotonic() - started
                events.append({"at_seconds": elapsed, "event": "disconnected", "reason": reason})
                record(f"\n[monitor {elapsed:.3f}s disconnected: {reason}]\n".encode())

            while time.monotonic() - started < seconds:
                if descriptor is None:
                    try:
                        opened = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
                        try:
                            tty.setraw(opened, termios.TCSANOW)
                            attributes = termios.tcgetattr(opened)
                            attributes[2] |= termios.CLOCAL | termios.CREAD
                            attributes[4] = termios.B115200
                            attributes[5] = termios.B115200
                            termios.tcsetattr(opened, termios.TCSANOW, attributes)
                            selector.register(opened, selectors.EVENT_READ)
                        except BaseException:
                            os.close(opened)
                            raise
                        descriptor = opened
                    except OSError as error:
                        if error.errno not in (errno.ENOENT, errno.ENXIO, errno.EBUSY, errno.EIO):
                            raise
                        time.sleep(0.1)
                        continue
                    connections += 1
                    elapsed = time.monotonic() - started
                    events.append({"at_seconds": elapsed, "event": "connected"})
                    record(f"\n[monitor {elapsed:.3f}s connected]\n".encode())
                for _, _ in selector.select(timeout=0.1):
                    try:
                        data = os.read(descriptor, 65536)
                    except BlockingIOError:
                        continue
                    except OSError as error:
                        disconnect(str(error))
                        break
                    if data:
                        received += len(data)
                        record(data)
                    else:
                        disconnect("end of stream")
                        break
                if descriptor is not None and not Path(port).exists():
                    disconnect("port disappeared")
    finally:
        if descriptor is not None:
            selector.unregister(descriptor)
            os.close(descriptor)
        selector.close()
    if not connections:
        raise RuntimeError(f"exact serial port never became available during {seconds} second monitor window")
    return {"kind": "posix-serial-monitor", "port": port, "duration_seconds": time.monotonic() - started,
            "connections": connections, "received_bytes": received, "events": events,
            "log": path.name, "sha256": sha256(path), "monitor_deadline_reached": True,
            "payload_evidence": "captured-unassessed" if received else "insufficient-no-payload"}


def secure_evidence(log: str) -> dict:
    """Extract firmware claims and measurements; never infer success from a monitor deadline.

    Completion is restricted to the four instrumented software-reset boundaries. It
    does not qualify physical power loss, flash confidentiality, or over-air traffic.
    Preserve all samples so reboot-local counters are never added together blindly.
    """
    evidence = {
        "schema": 1,
        "scope": "instrumented-software-reset-campaign",
        "qualification_status": "incomplete-evidence",
        "flash_samples": [], "memory_samples": [], "radio_samples": [],
        "cuts": [], "recoveries": [], "storage_checks": [], "completions": [], "key_profiles": [],
        "failures": [], "malformed": [],
    }
    flash_numbers = ("duration_us", "reads", "read_bytes", "read_us", "read_max_us",
                     "writes", "write_bytes", "write_us", "write_max_us",
                     "erases", "erase_bytes", "erase_us", "erase_max_us")
    memory_numbers = ("t_ms", "current", "boot_peak", "sampled_peak", "internal_free",
                      "external_free", "min_internal", "min_external", "core1_heartbeat")
    radio_numbers = ("lora_rx", "lora_tx", "usb_rx", "usb_tx", "espnow_rx", "espnow_tx", "status")
    patterns = (
        ("MLS secure phase=", "flash_samples", ("phase",), flash_numbers),
        ("MLS memory phase=", "memory_samples", ("phase",), memory_numbers),
        ("MLS radio lora_rx=", "radio_samples", (), radio_numbers),
        ("MLS secure cut=", "cuts", ("boundary",), ("cut",)),
        ("MLS secure recovery cut=", "recoveries", ("outcome", "exact-outbox"), ("cut",)),
        ("MLS secure qualification complete ", "completions", (),
         ("alice_revision", "bob_revision", "retired_through", "campaigns")),
        ("MLS secure fixture-key=", "key_profiles", ("fixture-key", "confidentiality"), ()),
    )
    for line_number, raw_line in enumerate(log.splitlines(), 1):
        line = re.sub(r"\x1b\[[0-9;]*m", "", raw_line)
        if re.search(r"MLS (?:secure )?qualification failed|MLS worker refused|\bpanicked\b|\bPANIC\b|\bGuru Meditation\b", line):
            evidence["failures"].append({"line": line_number, "text": line})
        for marker, collection, text_fields, number_fields in patterns:
            position = line.find(marker)
            if position < 0:
                continue
            pairs = re.findall(r"([A-Za-z_][A-Za-z0-9_-]*)=([^\s]+)", line[position:])
            fields = dict(pairs)
            if collection == "recoveries" and "immutable-ciphertext" in fields and "outcome" not in fields:
                collection, text_fields = "storage_checks", ("immutable-ciphertext",)
            try:
                if len(fields) != len(pairs):
                    raise ValueError("duplicate field")
                missing = set((*text_fields, *number_fields)) - fields.keys()
                if missing:
                    raise ValueError(f"missing fields: {', '.join(sorted(missing))}")
                optional_numbers = ("sample_gap_max_us",) if collection == "memory_samples" else ()
                for field in (*number_fields, *(name for name in optional_numbers if name in fields)):
                    if not re.fullmatch(r"[0-9]+", fields[field]):
                        raise ValueError(f"invalid nonnegative decimal field: {field}")
                    fields[field] = int(fields[field])
                if collection == "cuts" and not line[position:].endswith("software-reset"):
                    raise ValueError("missing software-reset boundary marker")
                evidence[collection].append({"line": line_number, **fields})
            except ValueError as error:
                evidence["malformed"].append({"line": line_number, "text": line, "error": str(error)})
            break
    expected_cuts = set(range(1, 5))
    verified_cuts = {sample["cut"] for sample in evidence["recoveries"]
                     if sample["outcome"] == "verified" and sample["exact-outbox"] == "verified"}
    complete = any(sample["campaigns"] == len(expected_cuts) for sample in evidence["completions"])
    evidence["verified_recovery_cuts"] = sorted(verified_cuts)
    evidence["missing_recovery_cuts"] = sorted(expected_cuts - verified_cuts)
    verified_storage = {sample["cut"] for sample in evidence["storage_checks"]
                        if sample["immutable-ciphertext"] == "verified"}
    evidence["verified_storage_cuts"] = sorted(verified_storage)
    evidence["missing_storage_cuts"] = sorted({3, 4} - verified_storage)
    evidence["flash_maxima_us"] = {
        field: max((sample[field] for sample in evidence["flash_samples"]), default=None)
        for field in ("duration_us", "read_max_us", "write_max_us", "erase_max_us")
    }
    evidence["memory_extrema_bytes"] = {
        "boot_peak": max((sample["boot_peak"] for sample in evidence["memory_samples"]), default=None),
        "sampled_peak": max((sample["sampled_peak"] for sample in evidence["memory_samples"]), default=None),
        "min_internal": min((sample["min_internal"] for sample in evidence["memory_samples"]), default=None),
        "min_external": min((sample["min_external"] for sample in evidence["memory_samples"]), default=None),
    }
    if evidence["failures"] or evidence["malformed"]:
        evidence["qualification_status"] = "failure-or-malformed-evidence"
    elif complete and verified_cuts == expected_cuts and {3, 4} <= verified_storage and all(evidence[name] for name in (
            "flash_samples", "memory_samples", "radio_samples", "key_profiles")):
        evidence["qualification_status"] = "complete-markers-observed"
    return evidence


def main(arguments: list[str] | None = None) -> int:
    args = parse_args(arguments)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    metadata = {"schema": 1, "scope": "experimental-e290-mls", "variant": args.variant,
                "source": "existing-elf" if args.elf else "working-tree-build",
                "started_utc": datetime.now(timezone.utc).isoformat(),
                "tool_sha256": sha256(Path(__file__)),
                "role_evidence": "operator-selected-unverified-existing-elf" if args.elf else "recorded-cargo-feature-selection",
                "port": args.port, "commands": [], "status": "running"}
    def save() -> None:
        path = output / "metadata.json"
        path.write_text(json.dumps(metadata, indent=2) + "\n")
        (output / "metadata.sha256").write_text(f"{sha256(path)}  metadata.json\n")
    save()
    try:
        env = esp_environment(args.export_esp)
        if args.elf:
            metadata["compiler_flags"] = {
                "profile": args.elf_flags_profile,
                "input_mode": "operator-declared-existing-elf",
                "applied_to_build": False,
                "verified_from_elf": False,
            }
        else:
            metadata["compiler_flags"] = qualification_flags(env)
        save()
        partition = partition_file(args.variant)
        validate_partition(partition, args.variant)
        copied_partition = output / partition.name
        shutil.copyfile(partition, copied_partition)
        metadata["partition_sha256"] = sha256(copied_partition)
        profile_files = (
            WORKSPACE / "boards/heltec-e290/Cargo.toml",
            WORKSPACE / "Cargo.toml",
            WORKSPACE / "Cargo.lock",
            ROOT / "personal-hopspot/memory/src/profiles/espressif/mod.rs",
        )
        metadata["profile_snapshot"] = {str(path.relative_to(ROOT)): sha256(path) for path in profile_files}
        metadata["profile_snapshot_scope"] = "current checkout at capture; not provenance of an externally supplied ELF"
        metadata["git_head"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        for name, command in (("esp-rustc", ["rustc", "+esp", "-vV"]), ("espflash", ["espflash", "--version"])):
            metadata["commands"].append(run_logged(command, output / f"{name}.log", env))
        if args.elf:
            source_elf = args.elf.resolve(strict=True)
        else:
            target_dir = args.target_dir.resolve() if args.target_dir else Path(tempfile.mkdtemp(prefix="prns-e290-mls-target-"))
            metadata["target_dir"] = str(target_dir)
            metadata["commands"].append(run_logged(build_command(args, target_dir), output / "build.log", env, cwd=WORKSPACE))
            source_elf = target_dir / TARGET / "release" / BINARY
        elf = output / f"{BINARY}.elf"
        shutil.copyfile(source_elf, elf)
        metadata["elf"] = {"file": elf.name, "sha256": sha256(elf), "bytes": elf.stat().st_size}
        metadata["artifact_pair_sha256"] = hashlib.sha256(
            f"e290-qualification-v1\n{sha256(elf)}\n{sha256(copied_partition)}\n".encode()
        ).hexdigest()
        metadata["commands"].append(run_logged(
            ["espflash", "save-image", *image_options(copied_partition), "--skip-update-check", str(elf), str(output / "image.bin")],
            output / "package.log", env))
        if not args.build_only and not args.monitor_only:
            metadata["commands"].append(run_logged(["espflash", "board-info", *device_options(args.port)], output / "board-info.log", env))
            validate_board_info((output / "board-info.log").read_text(errors="replace"))
            if args.erase_mls:
                metadata["commands"].append(run_logged(
                    erase_mls_command(args.port),
                    output / "erase-mls.log", env))
            options = image_options(copied_partition)
            metadata["commands"].append(run_logged(
                ["espflash", "flash", *device_options(args.port, before="no-reset", after="watchdog-reset"), *options[2:], str(elf)], output / "flash.log", env))
        if not args.build_only and args.monitor_seconds:
            metadata["monitor"] = monitor_port(args.port, output / "monitor.log", args.monitor_seconds)
            if args.variant == "secure":
                parsed = secure_evidence((output / "monitor.log").read_text(errors="replace"))
                evidence_path = output / "secure-evidence.json"
                evidence_path.write_text(json.dumps(parsed, indent=2) + "\n")
                metadata["secure_evidence"] = {
                    "file": evidence_path.name, "sha256": sha256(evidence_path),
                    "qualification_status": parsed["qualification_status"],
                    "scope": parsed["scope"],
                }
        metadata["artifacts"] = [{"file": path.name, "sha256": sha256(path), "bytes": path.stat().st_size}
                                 for path in sorted(output.glob("*.bin"))]
        metadata["status"] = "captured"
        save()
        print(f"Evidence saved to {output}; inspect logs to judge workload and recovery outcomes.")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        metadata["status"] = "failed"
        metadata["error"] = str(error)
        save()
        print(f"E290 MLS qualification failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
