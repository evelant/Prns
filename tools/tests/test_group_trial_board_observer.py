"""Board capture privacy, bounds, and no-reset/open-order regressions."""
from __future__ import annotations

import argparse
import importlib.util
import itertools
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "device/group-trial/board_observer.py"
SPEC = importlib.util.spec_from_file_location("group_trial_board_observer", SCRIPT)
assert SPEC and SPEC.loader
BOARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BOARD)


def line(seq=1, phase="request_ingress", extra=None, observed=100):
    if extra is None:
        extra = "disposition=RecursiveForward mode=Some(Full) recursive=true cached_interface=0000000000000000"
    return (f"INFO - prns.path.discovery seq={seq} observed_ms={observed} core_ms=Some(99) "
            f"phase={phase} destination=0123456789abcdef interface=abcdef0123456789 "
            f"request=1234567812345678 {extra}")


class Recorder:
    def __init__(self):
        self.rows = []

    def append(self, kind, **fields):
        self.rows.append({"kind": kind, **fields})


class FakePort:
    def __init__(self, chunks=(), fail=False):
        self.calls = []
        self.chunks = iter(chunks)
        self.fail = fail
        self.is_open = False

    def __setattr__(self, key, value):
        if key in {"dtr", "rts", "port"}:
            self.calls.append((key, value))
        object.__setattr__(self, key, value)

    def open(self):
        self.calls.append(("open", self.dtr, self.rts))
        self.is_open = True

    def close(self):
        self.calls.append(("close",))
        self.is_open = False

    def read(self, maximum):
        self.calls.append(("read", maximum))
        if self.fail:
            raise OSError("PRIVATE driver detail must never be persisted")
        return next(self.chunks, b"")


class BoardObserverTests(unittest.TestCase):
    def test_all_emitted_phase_shapes_parse_without_raw_text(self):
        examples = [
            line(), line(phase="request_ingress", extra="disposition=Ignored(Superseded) mode=None recursive=false cached_interface=0000000000000000"),
            line(phase="request_egress", extra="disposition=Dispatched"),
            line(phase="response_ingress", extra="disposition=RetainedRouteMatched"),
            line(phase="local_request", extra="command=3"),
            line(phase="local_timeout", extra="command=3"),
            line(phase="response_dispatched", extra=""),
            line(phase="recursive_timeout", extra=""),
        ]
        for text in examples:
            with self.subTest(text=text):
                result = BOARD.parse_safe_line(text)
                self.assertEqual(result["kind"], "route_event")
                self.assertNotIn("raw", result)
        self.assertIsNone(BOARD.parse_safe_line(line().replace("Some(99)", "Some(18446744073709551616)")))
        self.assertIsNone(BOARD.parse_safe_line(line(seq=2049)))

    def test_unknown_fields_values_free_text_and_duplicates_are_rejected(self):
        for value in [line() + " secret=PRIVATE", line() + " mode=Some(Full)",
                      line().replace("RecursiveForward", "PRIVATE"),
                      line().replace("destination=0123456789abcdef", "destination=PRIVATE"),
                      line().replace("Some(Full)", "Some(PRIVATE)"),
                      line().replace("request_ingress", "new_phase"),
                      "PRIVATE " + line(), "other log PRIVATE"]:
            self.assertIsNone(BOARD.parse_safe_line(value))

    def test_obvious_cross_core_log_interleaving_is_rejected(self):
        route = line()
        interleaved = route[:75] + "INFO - boot-stage t_ms=100 stage=network.ready" + route[75:]
        self.assertIsNone(BOARD.parse_safe_line(interleaved))
        self.assertIsNone(BOARD.parse_safe_line(route + line(seq=2)))
        for fragment in (route[:75], route[75:]):
            self.assertIsNone(BOARD.parse_safe_line(fragment))

    def test_cap_and_boot_markers_only_keep_closed_metadata(self):
        self.assertEqual(BOARD.parse_safe_line("INFO - prns.path.discovery limit_reached=2048"),
                         {"kind": "board_diagnostics_limit", "limit": 2048})
        self.assertIsNone(BOARD.parse_safe_line("INFO - prns.path.discovery limit_reached=2048 PRIVATE"))
        for value, source in [("INFO - HOPSPOT_HELTEC_E290 boot version=PRIVATE", "firmware_banner"),
                              ("ESP-ROM:esp32s3-PRIVATE", "rom_banner")]:
            self.assertEqual(BOARD.parse_safe_line(value), {"kind": "boot_boundary", "source": source})

    def test_none_core_time_and_ansi_logger_prefix(self):
        value = "\x1b[32m" + line().replace("Some(99)", "None") + "\x1b[0m\r\n"
        self.assertIsNone(BOARD.parse_safe_line(value)["core_ms"])

    def test_partial_oversize_line_does_not_admit_a_valid_suffix(self):
        framing = BOARD.Lines()
        self.assertEqual(framing.feed(b"x" * (BOARD.MAX_LINE + 1)), [])
        self.assertEqual(framing.feed(line().encode() + b"\n"), [])
        self.assertEqual(framing.oversized_lines, 1)
        data = line().encode()
        self.assertEqual(framing.feed(data[:10]), [])
        self.assertEqual(framing.feed(data[10:] + b"\n"), [data])

    def test_sequence_loss_restart_and_clock_regression_are_explicit(self):
        journal = Recorder()
        observer = BOARD.Observation(journal)
        for seq, clock in [(3, 100), (5, 200), (1, 10)]:
            observer.accept(BOARD.parse_safe_line(line(seq=seq, observed=clock)))
        reasons = {row.get("reason") for row in journal.rows}
        self.assertTrue({"events_before_first_observation", "board_event_gap",
                         "board_sequence_regressed", "board_clock_regressed"} <= reasons)
        self.assertTrue(any(row.get("source") == "inferred_sequence_restart" for row in journal.rows))

    def test_explicit_boot_clears_prior_sequence_and_time(self):
        journal = Recorder()
        observer = BOARD.Observation(journal)
        observer.accept(BOARD.parse_safe_line(line(seq=10)))
        observer.accept({"kind": "boot_boundary", "source": "firmware_banner"})
        observer.accept(BOARD.parse_safe_line(line(seq=1, observed=1)))
        self.assertEqual(observer.problems, {"events_before_first_observation"})
        observer.accept({"kind": "board_diagnostics_limit", "limit": 2048})
        self.assertIn("board_diagnostic_budget_exhausted", observer.problems)

    def test_control_lines_disabled_before_port_is_assigned_or_opened(self):
        port = FakePort()
        constructor = []
        def factory(**kwargs):
            constructor.append(kwargs)
            return port
        args = argparse.Namespace(port="/dev/test")
        BOARD.open_read_only(factory, args)
        self.assertIsNone(constructor[0]["port"])
        self.assertEqual(port.calls, [("dtr", False), ("rts", False),
                                     ("port", "/dev/test"), ("open", False, False)])
        self.assertFalse(constructor[0]["rtscts"])
        self.assertFalse(constructor[0]["dsrdtr"])

    def run_capture(self, *, chunks=(), fail=False, matching=True, max_events=100, seconds=2, setup_reset=False):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "capture.jsonl"
            args = argparse.Namespace(port="/dev/test", usb_serial="EXPECTED", vid=0x303A,
                                      pid=0x1001, output=output, seconds=seconds,
                                      label="board-test", max_events=max_events, setup_reset=setup_reset)
            port = FakePort(chunks=chunks, fail=fail)
            usb = types.SimpleNamespace(device=args.port, serial_number="EXPECTED" if matching else "WRONG",
                                        vid=args.vid, pid=args.pid)
            serial = types.SimpleNamespace(Serial=lambda **kwargs: port)
            clock = itertools.count(0, 0.3)
            with patch.object(BOARD.time, "monotonic", side_effect=lambda: next(clock)):
                result = BOARD.capture(args, serial_module=serial, ports_provider=lambda: [usb])
            text = output.read_text()
            return result, [json.loads(row) for row in text.splitlines()], text, port

    def test_identity_mismatch_never_opens_a_port(self):
        result, rows, _, port = self.run_capture(matching=False)
        self.assertEqual(result, 2)
        self.assertFalse(port.calls)
        self.assertEqual(rows[-1]["reason"], "usb_identity_mismatch")

    def test_identity_match_requires_all_usb_identity_fields(self):
        args = argparse.Namespace(port="/dev/test", usb_serial="EXPECTED", vid=0x303A, pid=0x1001)
        for key in ("device", "serial_number", "vid", "pid"):
            values = {"device": args.port, "serial_number": args.usb_serial,
                      "vid": args.vid, "pid": args.pid}
            values[key] = "WRONG" if isinstance(values[key], str) else 0
            self.assertFalse(BOARD.identity_matches([types.SimpleNamespace(**values)], args))

    def test_termination_enters_interrupted_capture_cleanup(self):
        with self.assertRaises(KeyboardInterrupt):
            BOARD.interrupt_capture(None, None)

    def test_capture_drops_private_console_and_records_no_silence_claim(self):
        result, rows, text, port = self.run_capture(seconds=4, chunks=[
            b"private config PRIVATE\n", (line() + "\n").encode(),
            (line() + " secret=PRIVATE\n").encode(), b"\xff\n", b"unfinished PRIVATE"])
        self.assertEqual(result, 0)
        self.assertNotIn("PRIVATE", text)
        self.assertEqual(sum(row["kind"] == "route_event" for row in rows), 1)
        self.assertFalse(rows[-1]["silence_proves_no_event"])
        self.assertFalse(rows[-1]["board_heartbeat_available"])
        self.assertIn("rejected_route_lines", rows[-1]["problems"])
        self.assertIn("incomplete_final_line", rows[-1]["problems"])
        self.assertEqual(port.calls[-1], ("close",))

    def test_serial_failure_is_closed_category_without_exception_text(self):
        result, rows, text, _ = self.run_capture(fail=True)
        self.assertEqual(result, 2)
        self.assertEqual(rows[-1]["reason"], "serial_read_or_journal_failed")
        self.assertNotIn("PRIVATE", text)

    def test_event_cap_is_hard_including_health_plus_one_terminal_row(self):
        stream = b"".join((line(seq=n) + "\n").encode() for n in range(1, 10))
        result, rows, _, _ = self.run_capture(chunks=[stream], max_events=4)
        self.assertEqual(result, 2)
        self.assertEqual(len(rows), 5)
        self.assertEqual(rows[-1]["reason"], "event_limit")

    def test_host_health_is_read_loop_only(self):
        _, rows, _, _ = self.run_capture(seconds=8)
        health = [row for row in rows if row["kind"] == "capture_health"]
        self.assertTrue(health)
        self.assertTrue(health[0]["read_loop_alive"])
        self.assertEqual(health[0]["bytes_read"], 0)

    def test_existing_output_is_never_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "capture.jsonl"
            output.write_text("original")
            with self.assertRaises(FileExistsError):
                BOARD.Journal(output, 4)
            self.assertEqual(output.read_text(), "original")


if __name__ == "__main__":
    unittest.main()
