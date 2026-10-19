from __future__ import annotations

import importlib.util
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest

if os.name == "posix":
    import pty

SCRIPT = Path(__file__).resolve().parents[1] / "device/hopspot-mls-e290.py"
SPEC = importlib.util.spec_from_file_location("hopspot_mls_e290", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


@unittest.skipUnless(os.name == "posix", "E290 qualification task supports macOS and Linux")
class QualificationSafetyTests(unittest.TestCase):
    def test_qualification_flags_override_workspace_policy_consistently(self) -> None:
        env = {"UNRELATED": "preserved"}
        recorded = MODULE.qualification_flags(env)
        self.assertEqual(recorded["rustflags"], ["-C", "link-arg=-Tlinkall.x", "-C", "force-frame-pointers=no"])
        self.assertEqual(env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"), recorded["rustflags"])
        self.assertEqual(env["RUSTFLAGS"], env[MODULE.TARGET_RUSTFLAGS_VARIABLE])
        self.assertEqual(env["UNRELATED"], "preserved")
        self.assertTrue(recorded["applied_to_build"])
        # Reapplying the approved environment is supported, including for baseline builds.
        self.assertEqual(set(MODULE.qualification_flags(env)["inherited_variables"].values()), {"compatible"})

    def test_inherited_flags_cannot_silently_override_qualification_flags(self) -> None:
        for variable in MODULE.RUSTFLAGS_VARIABLES:
            with self.subTest(variable=variable), self.assertRaisesRegex(ValueError, variable):
                MODULE.qualification_flags({variable: "-C force-frame-pointers=yes"})

    def test_cargo_encoded_flags_replace_config_without_duplicate_linker_script(self) -> None:
        """Check actual Cargo precedence, not only the wrapper's constructed strings."""
        banner = subprocess.check_output(["rustc", "-vV"], text=True)
        host = next(line.split(": ", 1)[1] for line in banner.splitlines() if line.startswith("host: "))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".cargo").mkdir()
            (root / "src").mkdir()
            (root / "Cargo.toml").write_text('[package]\nname="qualification-flags-probe"\nversion="0.0.0"\nedition="2021"\n[workspace]\n')
            (root / "src/lib.rs").write_text('#![no_std]\npub fn probe() {}\n')
            (root / ".cargo/config.toml").write_text(
                f'[target.{host}]\nrustflags=["-C","link-arg=-Tlinkall.x","-C","force-frame-pointers=yes"]\n')
            env = dict(os.environ)
            for variable in MODULE.RUSTFLAGS_VARIABLES:
                env.pop(variable, None)
            MODULE.qualification_flags(env)
            result = subprocess.run(["cargo", "check", "--offline", "--target", host, "-vv"],
                                    cwd=root, env=env, check=True, text=True, capture_output=True)
            invocation = next(line for line in result.stderr.splitlines()
                              if "Running " in line and "--crate-name qualification_flags_probe" in line)
            self.assertEqual(invocation.count("link-arg=-Tlinkall.x"), 1, invocation)
            self.assertEqual(invocation.count("force-frame-pointers=no"), 1, invocation)
            self.assertNotIn("force-frame-pointers=yes", invocation)

    def test_existing_elf_flag_profile_is_an_explicit_operator_declaration(self) -> None:
        args = MODULE.parse_args(["--output", "/tmp/fixture", "--build-only", "--elf", "/tmp/fixture.elf",
                                  "--elf-flags-profile", "no-frame-pointers"])
        self.assertEqual(args.elf_flags_profile, "no-frame-pointers")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            MODULE.parse_args(["--output", "/tmp/fixture", "--build-only", "--elf-flags-profile", "no-frame-pointers"])

    def test_device_operations_require_exact_port_and_erase_is_explicit_mls_only(self) -> None:
        for flags in ([], ["--erase-mls", "--build-only"],
                      ["--variant", "baseline", "--port", "/dev/fixture", "--erase-mls"],
                      ["--port", "/dev/fixture", "--monitor-only"]):
            with self.subTest(flags=flags), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                MODULE.parse_args(["--output", "/tmp/fixture", *flags])
        args = MODULE.parse_args(["--output", "/tmp/fixture", "--build-only", "--variant", "baseline"])
        self.assertNotIn("--features", MODULE.build_command(args, Path("/tmp/target")))
        args.variant = "mls"
        self.assertIn(MODULE.FEATURE, MODULE.build_command(args, Path("/tmp/target")))

    def test_secure_variant_selects_only_secure_feature_and_shared_partitions(self) -> None:
        args = MODULE.parse_args(["--output", "/tmp/fixture", "--variant", "secure",
                                  "--port", "/dev/fixture", "--erase-mls", "--offline"])
        command = MODULE.build_command(args, Path("/tmp/target"))
        self.assertEqual(command[command.index("--features") + 1], MODULE.SECURE_FEATURE)
        self.assertNotIn(MODULE.FEATURE, command)
        self.assertIn("--locked", command)
        self.assertIn("--offline", command)
        self.assertEqual(MODULE.partition_file("secure"), MODULE.partition_file("mls"))
        MODULE.validate_partition(MODULE.partition_file("secure"), "secure")

    def test_erase_stays_within_qualification_stores(self) -> None:
        command = MODULE.erase_mls_command("/dev/fixture")
        self.assertEqual(command[:2], ["espflash", "erase-region"])
        self.assertEqual(command[command.index("--port") + 1], "/dev/fixture")
        self.assertEqual(command[-2:], ["0x600000", "0x800000"])
        self.assertEqual(int(command[-2], 0) + int(command[-1], 0), 0xE00000)
        self.assertNotIn("erase-flash", command)

    def test_memory_contract_rejects_persistent_offset_change(self) -> None:
        original = MODULE.partition_file("baseline").read_text()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "partitions.csv"
            path.write_text(original)
            MODULE.validate_partition(path, "baseline")
            path.write_text(original.replace("0xe000,0x1000", "0xe004,0xffc"))
            with self.assertRaises(ValueError):
                MODULE.validate_partition(path, "baseline")

    def test_experimental_memory_contract_requires_both_mls_partitions(self) -> None:
        original = MODULE.partition_file("baseline").read_text()
        table = original.replace("0xe6d000", "0x5f0000") + (
            "mls_alice,0x47,0x00,0x600000,0x400000,\n"
            "mls_bob,0x47,0x01,0xa00000,0x400000,\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "partitions.csv"
            path.write_text(table)
            MODULE.validate_partition(path, "mls")
            MODULE.validate_partition(path, "secure")
            path.write_text(table.replace("0xa00000,0x400000", "0xa00000,0x300000"))
            with self.assertRaises(ValueError):
                MODULE.validate_partition(path, "mls")
            with self.assertRaises(ValueError):
                MODULE.validate_partition(path, "secure")

    def test_preflight_rejects_wrong_chip_or_flash(self) -> None:
        MODULE.validate_board_info("Chip type: esp32s3 (revision v0.2)\nFlash size: 16MB\n")
        for text in ("Chip type: esp32s3\nFlash size: 8MB", "Chip type: esp32\nFlash size: 16MB", ""):
            with self.subTest(text=text), self.assertRaises(ValueError):
                MODULE.validate_board_info(text)

    @staticmethod
    def secure_log() -> str:
        samples = [
            "MLS secure fixture-key=plaintext-header confidentiality=unqualified",
            "MLS memory phase=operation t_ms=14000 current=100000 boot_peak=180000 sampled_peak=175000 "
            "internal_free=30000 external_free=8000000 min_internal=29000 min_external=7900000 core1_heartbeat=440",
            "MLS radio lora_rx=0 lora_tx=100 usb_rx=0 usb_tx=12 espnow_rx=0 espnow_tx=40 status=1",
            "MLS secure phase=compact duration_us=95000 reads=10 read_bytes=4000 read_us=1100 read_max_us=200 "
            "writes=4 write_bytes=2000 write_us=15000 write_max_us=4100 "
            "erases=2 erase_bytes=8192 erase_us=78000 erase_max_us=41000",
        ]
        for cut in range(1, 5):
            samples.extend([
                f"MLS secure cut={cut} boundary=boundary-{cut} software-reset",
                f"MLS secure recovery cut={cut} outcome=verified exact-outbox=verified",
            ])
            if cut in (3, 4):
                samples.append(f"MLS secure recovery cut={cut} immutable-ciphertext=verified")
        samples.append("MLS secure qualification complete alice_revision=10 bob_revision=7 retired_through=8 campaigns=4")
        return "\n".join(f"\x1b[32m[INFO] {sample}\x1b[0m" for sample in samples)

    def test_secure_evidence_retains_measurements_and_explicit_software_reset_scope(self) -> None:
        evidence = MODULE.secure_evidence(self.secure_log())
        self.assertEqual(evidence["qualification_status"], "complete-markers-observed")
        self.assertEqual(evidence["scope"], "instrumented-software-reset-campaign")
        self.assertEqual(evidence["verified_recovery_cuts"], [1, 2, 3, 4])
        self.assertEqual(evidence["verified_storage_cuts"], [3, 4])
        self.assertEqual(evidence["flash_maxima_us"]["erase_max_us"], 41000)
        self.assertEqual(evidence["flash_samples"][0]["read_bytes"], 4000)
        self.assertEqual(evidence["memory_extrema_bytes"]["min_internal"], 29000)
        self.assertEqual(evidence["radio_samples"][0]["lora_rx"], 0)
        self.assertEqual(evidence["key_profiles"][0]["confidentiality"], "unqualified")

    def test_secure_evidence_accepts_mixed_live_recovery_schema_and_optional_sample_gap(self) -> None:
        log = self.secure_log().replace("core1_heartbeat=440", "core1_heartbeat=440 sample_gap_max_us=37691")
        evidence = MODULE.secure_evidence(log)
        self.assertEqual(evidence["qualification_status"], "complete-markers-observed")
        self.assertFalse(evidence["malformed"])
        self.assertEqual(len(evidence["recoveries"]), 4)
        self.assertEqual(len(evidence["storage_checks"]), 2)
        self.assertEqual(evidence["memory_samples"][0]["sample_gap_max_us"], 37691)

    def test_secure_evidence_requires_distinct_verified_recoveries_and_measurements(self) -> None:
        original = self.secure_log()
        for text in (
            "[monitor 90.000s connected]\n",
            original.replace("recovery cut=4", "recovery cut=3"),
            original.replace("outcome=verified", "outcome=failed"),
            original.replace("immutable-ciphertext=verified", "immutable-ciphertext=failed"),
            original.replace("MLS secure qualification complete", "MLS secure qualification already-complete"),
            original.replace("MLS memory phase=", "other memory phase="),
        ):
            with self.subTest(text=text):
                self.assertEqual(MODULE.secure_evidence(text)["qualification_status"], "incomplete-evidence")

    def test_secure_evidence_rejects_malformed_and_failure_claims(self) -> None:
        original = self.secure_log()
        for text in (
            original.replace("read_us=1100", "read_us=-1"),
            original.replace("read_us=1100", "read_us=bad"),
            original.replace("read_us=1100", "read_us=1100 read_us=2100"),
            original.replace("read_us=1100 ", ""),
            original + "\nMLS qualification failed: flash failure\n",
            original + "\nthread 'worker' panicked at invalid state\n",
        ):
            with self.subTest(text=text):
                self.assertEqual(MODULE.secure_evidence(text)["qualification_status"], "failure-or-malformed-evidence")

    def test_monitor_deadline_records_timeout_without_calling_it_a_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "monitor.log"
            result = MODULE.run_logged([sys.executable, "-c", "import time; print('ready', flush=True); time.sleep(3)"],
                                       path, dict(os.environ), seconds=1)
            self.assertTrue(result["monitor_deadline_reached"])
            self.assertIn(b"ready", path.read_bytes())
            self.assertNotEqual(result["return_code"], 0)

    def test_monitor_reconnects_only_the_named_port(self) -> None:
        first_master, first_slave = pty.openpty()
        second_master, second_slave = pty.openpty()
        try:
            with tempfile.TemporaryDirectory() as directory:
                port = Path(directory) / "serial"
                port.symlink_to(os.ttyname(first_slave))
                def write_reboot() -> None:
                    time.sleep(0.1)
                    os.write(first_master, b"before reboot\n")
                    time.sleep(0.15)
                    port.unlink()
                    time.sleep(0.15)
                    port.symlink_to(os.ttyname(second_slave))
                    time.sleep(0.15)
                    os.write(second_master, b"after reboot\n")
                worker = threading.Thread(target=write_reboot)
                worker.start()
                log = Path(directory) / "monitor.log"
                result = MODULE.monitor_port(str(port), log, 1)
                worker.join()
                self.assertEqual(result["connections"], 2)
                self.assertIn(b"before reboot", log.read_bytes())
                self.assertIn(b"after reboot", log.read_bytes())
        finally:
            for descriptor in (first_master, first_slave, second_master, second_slave):
                os.close(descriptor)


if __name__ == "__main__":
    unittest.main()
