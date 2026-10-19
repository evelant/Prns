#!/usr/bin/env python3
"""Bounded E290 Link/route/queue capture; default has no reset or serial data writes.

Only closed Link/route/queue schemas and capture/boot-boundary metadata are persisted.
Host health proves the reader loop is running, never that a board is alive or
that an absent route event/transmission did not happen.
An explicit --setup-reset resets once after open, before trial acceptance.
Opening/closing a serial device can affect hardware through its driver despite
the requested DTR/RTS settings. Inspect boot hints before qualifying a trial.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import signal
import time
import uuid

_link_spec = importlib.util.spec_from_file_location("board_link_schema", Path(__file__).with_name("board_link_schema.py"))
assert _link_spec and _link_spec.loader
LINK = importlib.util.module_from_spec(_link_spec)
_link_spec.loader.exec_module(LINK)

MAX_FIRMWARE_FAULTS = 32
MAX_LINE = 1024
MAX_BYTES = 32 * 1024 * 1024
U64_MAX = (1 << 64) - 1
ANSI = re.compile(r"\x1b\[[0-9;]*m")
BASE = re.compile(
    r"seq=([0-9]{1,4}) observed_ms=([0-9]{1,20}) "
    r"core_ms=(None|Some\([0-9]{1,20}\)) phase=([a-z_]+) "
    r"destination=([0-9a-f]{16}) interface=([0-9a-f]{16}) "
    r"request=([0-9a-f]{16})(?: (.*))?\Z"
)
QUEUE = re.compile(
    r"seq=([0-9]{1,3}) observed_ms=([0-9]{1,20}) phase=drop "
    r"source=(runtime_lane|driver_queue|active) "
    r"reason=(expired|disabled|pre_resume|missing_time|clock_rollback) "
    r"age_ms=(None|Some\([0-9]{1,20}\))\Z"
)
IGNORED = {
    "Consumed", "Malformed", "UnhandledContext", "Duplicate", "Superseded",
    "NotForUs", "NoRoute", "HopLimitReached", "LoopPrevented", "RouteUnresponsive",
    "OtherInstance", "UnknownLink", "LinkPhaseMismatch", "DecryptFailed",
    "ProofInvalid", "UnknownIdentity", "LinkRequestsRefused", "PermissionDenied",
    "RateLimited", "CapacityExhausted", "RequestTooLarge", "StrategyDeclined",
    "UnmatchedResponse", "IfacRefused",
}
REQUEST_DISPOSITIONS = {
    "LocalResponse", "CachedResponse", "ResponseQueueFull", "RecursiveForward",
    "BoundaryForward", "LocalClientForward", "OfferedToLocalClients",
} | {f"Ignored({reason})" for reason in IGNORED}
MODES = {"None"} | {f"Some({mode})" for mode in (
    "Full", "PointToPoint", "AccessPoint", "Roaming", "Boundary", "Gateway", "Internal"
)}


def firmware_fault(line: str) -> dict | None:
    # Exact prefixes from the selected esp-backtrace/esp-rtos/esp-hal/xtensa
    # sources. Never retain panic messages, paths, task pointers or registers.
    exact = {"====================== PANIC ======================": "panic_banner", "Backtrace:": "backtrace_header"}
    prefixes = (("panicked at ", "panic_location"),
                ("Stack overflow detected in ", "stack_overflow"),
                ("assertion `left == right` failed: Stack overflow detected in ", "stack_overflow"),
                ("Detected a write to the stack guard value on ", "stack_guard"),
                ("Exception occurred on ", "exception"),
                ("Exception: ", "exception"), ("Double Exception: ", "double_exception"),
                ("Breakpoint on ", "breakpoint"))
    category = exact.get(line)
    if category is None:
        category = next((value for prefix,value in prefixes if line.startswith(prefix)), None)
    return None if category is None else {"kind":"firmware_fault", "category":category}


def parse_safe_line(line: str) -> dict | None:
    """No raw text survives this parser, including rejected diagnostic lines."""
    if len(line) > MAX_LINE:
        return None
    line = ANSI.sub("", line).strip()
    line = re.sub(r"^INFO(?: \([0-9]+\))? - ", "", line)
    if line.startswith(LINK.PREFIX):
        return LINK.parse(line)
    fault = firmware_fault(line)
    if fault is not None:
        return fault
    if line.startswith("HOPSPOT_HELTEC_E290 boot version="):
        return {"kind": "boot_boundary", "source": "firmware_banner"}
    if line.startswith("ESP-ROM:esp32s3-"):
        return {"kind": "boot_boundary", "source": "rom_banner"}
    if line.startswith("prns.lora.queue "):
        payload = line.removeprefix("prns.lora.queue ")
        if payload == "limit_reached=512":
            return {"kind": "queue_diagnostics_limit", "limit": 512}
        match = QUEUE.fullmatch(payload)
        if match is None:
            return None
        seq, observed, source, reason, age = match.groups()
        sequence, observed_ms = int(seq), int(observed)
        age_ms = None if age == "None" else int(age[5:-1])
        if not 1 <= sequence <= 512 or max(observed_ms, age_ms or 0) > U64_MAX:
            return None
        return {"kind": "queue_event", "board_sequence": sequence,
                "board_observed_ms": observed_ms, "phase": "drop", "source": source,
                "reason": reason, "age_ms": age_ms}
    prefix = "prns.path.discovery "
    if not line.startswith(prefix):
        return None
    payload = line[len(prefix):].rstrip()
    if payload == "limit_reached=2048":
        return {"kind": "board_diagnostics_limit", "limit": 2048}
    match = BASE.fullmatch(payload)
    if match is None:
        return None
    seq, observed, core, phase, destination, interface, request, extra = match.groups()
    sequence, observed_ms = int(seq), int(observed)
    core_ms = None if core == "None" else int(core[5:-1])
    if not 1 <= sequence <= 2048 or observed_ms > U64_MAX or (core_ms or 0) > U64_MAX:
        return None
    fields = {}
    for token in (extra or "").split():
        if "=" not in token:
            return None
        key, value = token.split("=", 1)
        if key in fields:
            return None
        fields[key] = value
    if phase == "request_ingress":
        if (set(fields) != {"disposition", "mode", "recursive", "cached_interface"}
                or fields["disposition"] not in REQUEST_DISPOSITIONS
                or fields["mode"] not in MODES
                or fields["recursive"] not in {"true", "false"}
                or not re.fullmatch(r"[0-9a-f]{16}", fields["cached_interface"])):
            return None
        fields["recursive"] = fields["recursive"] == "true"
    elif phase in {"request_egress", "response_ingress"}:
        allowed = ({"Dispatched", "Offline", "RateLimited"} if phase == "request_egress"
                   else {"VerificationPending", "Accepted", "RetainedRouteMatched", "Ignored", "Blackholed"})
        if set(fields) != {"disposition"} or fields["disposition"] not in allowed:
            return None
    elif phase in {"local_request", "local_timeout"}:
        if (set(fields) != {"command"} or not re.fullmatch(r"[0-9]{1,20}", fields["command"])
                or int(fields["command"]) > U64_MAX):
            return None
        fields["command"] = int(fields["command"])
    elif phase not in {"response_dispatched", "recursive_timeout"} or fields:
        return None
    return {"kind": "route_event", "board_sequence": sequence,
            "board_observed_ms": observed_ms, "core_ms": core_ms, "phase": phase,
            "destination": destination, "interface": interface, "request": request, **fields}


class Lines:
    """Bounded streaming framing; discard an oversized line through its newline."""
    def __init__(self):
        self.pending = bytearray()
        self.discarding = False
        self.oversized_lines = 0

    def feed(self, data: bytes) -> list[bytes]:
        complete = []
        for byte in data:
            if byte == 10:
                if not self.discarding:
                    complete.append(bytes(self.pending))
                self.pending.clear()
                self.discarding = False
            elif not self.discarding:
                if len(self.pending) == MAX_LINE:
                    self.pending.clear()
                    self.discarding = True
                    self.oversized_lines += 1
                else:
                    self.pending.append(byte)
        return complete


class LimitReached(Exception):
    pass


class Journal:
    def __init__(self, path: Path, maximum: int):
        path.parent.mkdir(parents=True, exist_ok=True)
        # x prevents an accidental overwrite of earlier evidence.
        self.file = path.open("x", encoding="utf-8")
        self.capture_id = uuid.uuid4().hex
        self.sequence = 0
        self.maximum = maximum
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)

    def append(self, kind: str, *, terminal=False, **fields):
        if self.sequence >= self.maximum and not terminal:
            raise LimitReached
        self.sequence += 1
        row = {"schema": 1, "capture_id": self.capture_id, "sequence": self.sequence,
               "host_observed_utc": dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds"),
               "host_monotonic_ns": time.monotonic_ns(), "kind": kind, **fields}
        self.file.write(json.dumps(row, sort_keys=True) + "\n")
        self.file.flush()
        os.fsync(self.file.fileno())

    def close(self):
        self.file.close()


class Observation:
    def __init__(self, journal):
        self.journal = journal
        self.last_sequence = None
        self.last_board_ms = None
        self.last_queue_sequence = None
        self.last_queue_ms = None
        self.segment = 0
        self.last_link_sequence = self.last_link_ms = self.link_boot = None
        self.link_health_seen = False
        self.firmware_faults = 0
        self.problems = set()

    def begin_segment(self, source=None):
        self.segment += 1
        self.last_link_sequence = self.last_link_ms = self.link_boot = None
        self.last_sequence = self.last_board_ms = None
        self.last_queue_sequence = self.last_queue_ms = None
        if source is not None:
            self.journal.append("boot_boundary", source=source, board_segment=self.segment)

    def accept(self, event: dict):
        kind = event["kind"]
        if kind == "firmware_fault":
            self.firmware_faults += 1
            self.problems.add("firmware_fault_observed")
            if self.firmware_faults > MAX_FIRMWARE_FAULTS:
                if self.firmware_faults == MAX_FIRMWARE_FAULTS + 1:
                    self.problems.add("firmware_fault_budget_exhausted")
                    self.journal.append("firmware_fault_limit", limit=MAX_FIRMWARE_FAULTS, board_segment=self.segment)
                return
        if kind in {"link_probe", "link_health", "link_event", "link_limit"}:
            self.accept_link(event)
        elif kind == "boot_boundary":
            self.begin_segment()
        elif kind == "queue_diagnostics_limit":
            self.problems.add("queue_diagnostic_budget_exhausted")
        elif kind == "queue_event":
            self.accept_queue(event)
        elif kind == "board_diagnostics_limit":
            self.problems.add("board_diagnostic_budget_exhausted")
        elif kind == "route_event":
            seq, now = event["board_sequence"], event["board_observed_ms"]
            restarted = False
            if self.last_sequence is None and seq != 1:
                self.problems.add("events_before_first_observation")
                self.journal.append("observation_gap", reason="events_before_first_observation",
                                    missed=seq - 1, board_segment=self.segment)
            elif self.last_sequence is not None and seq != self.last_sequence + 1:
                reason = "board_sequence_regressed" if seq <= self.last_sequence else "board_event_gap"
                self.problems.add(reason)
                self.journal.append("observation_gap", reason=reason, previous=self.last_sequence,
                                    current=seq, board_segment=self.segment)
                if seq <= self.last_sequence:
                    restarted = True
            if self.last_board_ms is not None and now < self.last_board_ms:
                self.problems.add("board_clock_regressed")
                self.journal.append("observation_gap", reason="board_clock_regressed",
                                    previous=self.last_board_ms, current=now, board_segment=self.segment)
            if restarted:
                self.begin_segment("inferred_sequence_restart")
            self.last_sequence, self.last_board_ms = seq, now
        self.journal.append(**event, board_segment=self.segment)

    def accept_link(self, event: dict):
        seq, now, boot = event["board_sequence"], event["board_observed_ms"], event["boot_token"]
        if self.link_boot is not None and boot != self.link_boot:
            self.problems.add("link_boot_changed")
            self.begin_segment("link_boot_token_changed")
        if self.last_link_sequence is None and seq != 1:
            self.problems.add("link_events_before_first_observation")
            self.journal.append("observation_gap", reason="link_events_before_first_observation", missed=seq-1, board_segment=self.segment)
        elif self.last_link_sequence is not None and seq != self.last_link_sequence + 1:
            reason = "link_sequence_regressed" if seq <= self.last_link_sequence else "link_event_gap"
            self.problems.add(reason)
            self.journal.append("observation_gap", reason=reason, previous=self.last_link_sequence, current=seq, board_segment=self.segment)
        if self.last_link_ms is not None and now < self.last_link_ms:
            self.problems.add("link_clock_regressed")
            self.journal.append("observation_gap", reason="link_clock_regressed", previous=self.last_link_ms, current=now, board_segment=self.segment)
        self.link_boot, self.last_link_sequence, self.last_link_ms = boot, seq, now
        fields = event["fields"]
        if event["kind"] == "link_health":
            self.link_health_seen = True
            if fields["healthy"] != "true" or fields["core_capped"] != "false" or fields["lora_capped"] != "false" or int(fields["sink_dropped"]):
                self.problems.add("link_diagnostics_unhealthy")
        elif event["kind"] == "link_limit":
            self.problems.add("link_health_budget_exhausted")
        elif event["kind"] == "link_event" and fields["phase"] in {"producer_limit", "LimitReached"}:
            self.problems.add("link_diagnostic_budget_exhausted")

    def accept_queue(self, event: dict):
        # Independent family: route traffic cannot consume or advance this sequence.
        seq, now = event["board_sequence"], event["board_observed_ms"]
        restarted = False
        if self.last_queue_sequence is None and seq != 1:
            self.problems.add("queue_events_before_first_observation")
            self.journal.append("observation_gap", reason="queue_events_before_first_observation",
                                missed=seq - 1, board_segment=self.segment)
        elif self.last_queue_sequence is not None and seq != self.last_queue_sequence + 1:
            reason = "queue_sequence_regressed" if seq <= self.last_queue_sequence else "queue_event_gap"
            self.problems.add(reason)
            self.journal.append("observation_gap", reason=reason, previous=self.last_queue_sequence,
                                current=seq, board_segment=self.segment)
            if seq <= self.last_queue_sequence:
                restarted = True
        if self.last_queue_ms is not None and now < self.last_queue_ms:
            self.problems.add("queue_clock_regressed")
            self.journal.append("observation_gap", reason="queue_clock_regressed",
                                previous=self.last_queue_ms, current=now, board_segment=self.segment)
        if restarted:
            self.begin_segment("inferred_queue_sequence_restart")
        self.last_queue_sequence, self.last_queue_ms = seq, now


def identity_matches(ports, args) -> bool:
    matches = [port for port in ports if port.device == args.port]
    return (len(matches) == 1 and matches[0].serial_number == args.usb_serial
            and matches[0].vid == args.vid and matches[0].pid == args.pid)


def open_read_only(factory, args):
    # Do not pass a port to the constructor: pySerial otherwise opens it before
    # DTR/RTS can be disabled. No explicit reset, break, or data writes follow.
    # OS open occurs before pySerial applies these settings; a driver can still
    # change control lines or affect hardware during open/close.
    # pySerial itself purges pending host input during open(); metadata records
    # that unavailable pre-open coverage. Do not override its private internals.
    port = factory(port=None, baudrate=115200, timeout=0.25, exclusive=True,
                   rtscts=False, dsrdtr=False, xonxoff=False)
    port.dtr = False
    port.rts = False
    port.port = args.port
    try:
        port.open()
    except Exception:
        port.close()
        raise
    return port


def setup_reset(port, *, vid, pid):
    """Opt-in ESP USB Serial/JTAG reset, before acceptance; never a serial data write.

    Matches espflash 4.5.0 connection/reset.rs reset_after_flash for PID 0x1001.
    It is deliberately not a generic serial reset facility.
    """
    if (vid, pid) != (0x303A, 0x1001):
        raise ValueError("unsupported setup reset identity")
    time.sleep(0.1)
    port.dtr = False
    time.sleep(0.1)
    port.rts = True
    port.dtr = False
    port.rts = True
    time.sleep(0.1)
    port.rts = False


def capture(args, *, serial_module=None, ports_provider=None) -> int:
    if serial_module is None:
        import serial as serial_module
    if ports_provider is None:
        from serial.tools import list_ports
        ports_provider = list_ports.comports
    journal = Journal(args.output, args.max_events)
    observation = Observation(journal)
    framing = Lines()
    counters = {"bytes_read": 0, "lines_read": 0, "unrelated_lines": 0,
                "rejected_route_lines": 0, "rejected_queue_lines": 0, "rejected_link_lines": 0, "undecodable_lines": 0}
    started = time.monotonic()
    last_health = started
    port = None
    reason = "duration_complete"
    code = 0
    stage = "identity_check"
    try:
        if not identity_matches(ports_provider(), args):
            reason, code = "usb_identity_mismatch", 2
        else:
            journal.append("capture_started", label=args.label, seconds=args.seconds,
                           max_events=args.max_events, max_bytes=MAX_BYTES,
                           usb_serial_sha256=hashlib.sha256(args.usb_serial.encode()).hexdigest(),
                           vid=args.vid, pid=args.pid, board_heartbeat_available=False,
                           link_heartbeat_expected=getattr(args, "link_trace", False),
                           setup_reset_requested=getattr(args, "setup_reset", False),
                           pre_open_input_unobserved=True,
                           pyserial_open_purges_pending_input=True,
                           buffered_prior_output_possible=True)
            stage = "open"
            port = open_read_only(serial_module.Serial, args)
            journal.append("serial_opened", dtr=False, rts=False, writes_enabled=False)
            if getattr(args, "setup_reset", False):
                stage = "setup_reset"
                journal.append("setup_reset_started", method="esp_usb_jtag_after_flash", outside_acceptance=True)
                setup_reset(port, vid=args.vid, pid=args.pid)
                journal.append("setup_reset_completed", method="esp_usb_jtag_after_flash", outside_acceptance=True)
            stage = "read"
            while time.monotonic() - started < args.seconds:
                if not port.is_open:
                    reason, code = "serial_closed", 2
                    break
                data = port.read(4096)
                counters["bytes_read"] += len(data)
                if counters["bytes_read"] > MAX_BYTES:
                    reason, code = "byte_limit", 2
                    break
                for raw in framing.feed(data):
                    counters["lines_read"] += 1
                    try:
                        line = raw.decode("utf-8", errors="strict")
                    except UnicodeDecodeError:
                        counters["undecodable_lines"] += 1
                        continue
                    event = parse_safe_line(line)
                    if event is not None:
                        observation.accept(event)
                    elif "prns.link.trace" in line:
                        counters["rejected_link_lines"] += 1
                    elif "prns.path.discovery" in line:
                        counters["rejected_route_lines"] += 1
                    elif "prns.lora.queue" in line:
                        counters["rejected_queue_lines"] += 1
                    else:
                        counters["unrelated_lines"] += 1
                now = time.monotonic()
                if now - last_health >= 5:
                    stage = "identity_check"
                    if not identity_matches(ports_provider(), args):
                        reason, code = "usb_identity_changed", 2
                        break
                    stage = "read"
                    journal.append("capture_health", read_loop_alive=True, port_open=port.is_open,
                                   elapsed_ms=int((now - started) * 1000),
                                   oversized_lines=framing.oversized_lines, firmware_faults_observed=observation.firmware_faults, **counters)
                    last_health = now
    except LimitReached:
        reason, code = "event_limit", 2
    except KeyboardInterrupt:
        reason, code = "capture_interrupted", 2
    except Exception:
        # Persist a closed category, never driver exceptions or arbitrary console.
        reason, code = {"identity_check": "identity_check_failed", "open": "serial_open_failed",
                        "read": "serial_read_or_journal_failed", "setup_reset": "setup_reset_failed"}[stage], 2
    finally:
        if port is not None:
            try:
                port.close()
            except Exception:
                observation.problems.add("serial_close_failed")
                code = 2
        if counters["rejected_link_lines"]:
            observation.problems.add("rejected_link_lines")
        if counters["rejected_route_lines"]:
            observation.problems.add("rejected_route_lines")
        if counters["rejected_queue_lines"]:
            observation.problems.add("rejected_queue_lines")
        if counters["undecodable_lines"] or framing.oversized_lines:
            observation.problems.add("discarded_or_undecodable_lines")
        incomplete = bool(framing.pending) or framing.discarding
        if incomplete:
            observation.problems.add("incomplete_final_line")
        try:
            journal.append("capture_ended", terminal=True, reason=reason,
                           elapsed_ms=int((time.monotonic() - started) * 1000),
                           incomplete_final_line=incomplete, oversized_lines=framing.oversized_lines,
                           problems=sorted(observation.problems), board_heartbeat_available=observation.link_health_seen,
                           silence_proves_no_event=False, firmware_faults_observed=observation.firmware_faults, **counters)
        finally:
            journal.close()
    return code


def interrupt_capture(_signal, _frame):
    raise KeyboardInterrupt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", required=True)
    parser.add_argument("--usb-serial", required=True, help="Exact USB serial from prior inventory")
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x303A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x1001)
    parser.add_argument("--label", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--seconds", type=float, default=900)
    parser.add_argument("--setup-reset", action="store_true", help="Reset ESP USB Serial/JTAG once after opening; setup only, before acceptance")
    parser.add_argument("--link-trace", action="store_true", help="Expect the qualified CRC-framed board Link health contract")
    parser.add_argument("--max-events", type=int, default=4096,
                        help="All persisted rows, plus one terminal row; maximum 16384")
    args = parser.parse_args()
    if (not math.isfinite(args.seconds) or not 1 <= args.seconds <= 3600
            or not 4 <= args.max_events <= 16384
            or not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", args.label)
            or (args.setup_reset and (args.vid, args.pid) != (0x303A, 0x1001))):
        parser.error("invalid bounded capture limits or label")
    signal.signal(signal.SIGTERM, interrupt_capture)
    try:
        return capture(args)
    except Exception:
        # Includes existing-output refusal and a final journal I/O failure.
        print('{"kind":"capture_error","reason":"setup_or_journal_failed"}')
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
