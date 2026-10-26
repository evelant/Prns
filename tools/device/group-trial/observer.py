#!/usr/bin/env python3
"""Durable, metadata-only phone diagnostic capture and fail-closed trial gates.

All timestamps are host observations, not device event times. This tool never
reads app containers, clears logs/data, sends messages, changes radio settings,
or treats plain carrier acceptance as an authenticated group receipt.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import time
import uuid

SCHEMA = 1
SAFE_TAGS = ("prns.groups", "prns.bluetooth", "prns.trial", "prns.routes", "prns.schedule", "prns.links")
# Producers are responsible for metadata-only values. Reject free-form output;
# do not preserve rejected/raw console lines, paths, exceptions, or UI contents.
FIELD = re.compile(r"([a-z][a-z0-9_]*)=([A-Za-z0-9_.:+-]{1,160})\Z")
BRACKET = re.compile(r"\[(prns\.(?:groups|bluetooth|trial|routes|schedule|links))\]\s+(.*)\Z")
ENUM_FIELD = re.compile(r"(kind|mode|disposition|detail)=([A-Za-z0-9_()]{1,160})\Z")
LOGCAT = re.compile(r"^\s*(\d+\.\d+)\s+(\d+)\s+(\d+)\s+([VDIWEF])\s+([^:]+):\s*(.*)$")
MAX_LINE = 8192


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds")


def epoch(value: str) -> float:
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("UTC timestamps must include an offset")
    return parsed.timestamp()


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8") as output:
        json.dump(value, output, indent=2, sort_keys=True)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def parse_safe_line(line: str, expected_pid: int | None = None) -> dict | None:
    """Discard unrelated logs entirely, including non-matching Android PIDs."""
    if len(line) > MAX_LINE:
        return None
    process_id = None
    device_log_time = None
    android = LOGCAT.fullmatch(line.strip())
    if android:
        device_log_time, pid, _, level, tag, payload = android.groups()
        process_id = int(pid)
        if expected_pid is not None and process_id != expected_pid:
            return None
        if tag.strip() not in SAFE_TAGS or level not in ("I", "W", "E"):
            return None
        bracket = BRACKET.search(payload)
        if bracket:
            if bracket.group(1) != tag.strip():
                return None
            payload = bracket.group(2)
        tag = tag.strip()
    else:
        bracket = BRACKET.search(line.rstrip("\r\n"))
        if not bracket:
            return None
        tag, payload = bracket.groups()
        # A PID-pinned Android capture must never admit an unframed console line.
        if expected_pid is not None:
            return None
    fields = {}
    for token in payload.split():
        match = FIELD.fullmatch(token) or (ENUM_FIELD.fullmatch(token) if tag in ("prns.routes", "prns.links") else None)
        if match is None or match.group(1) in fields:
            return None
        fields[match.group(1)] = match.group(2)
    if "stage" not in fields:
        return None
    result = {"tag": tag, "fields": fields}
    if process_id is not None:
        result.update(device_pid=process_id, device_log_epoch=device_log_time)
    return result


class Journal:
    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.output = path.open("x", encoding="utf-8")
        self.capture_id = uuid.uuid4().hex
        self.sequence = 0

    def append(self, kind: str, **values: object) -> dict:
        self.sequence += 1
        row = {"schema": SCHEMA, "capture_id": self.capture_id,
               "sequence": self.sequence, "kind": kind,
               "host_observed_utc": utc_now(), "host_monotonic_ns": time.monotonic_ns(),
               **values}
        self.output.write(json.dumps(row, sort_keys=True) + "\n")
        self.output.flush()
        os.fsync(self.output.fileno())
        return row

    def close(self) -> None:
        self.output.close()


def android_replay_lines(value: str) -> int:
    try:
        count = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("Android replay lines must be an integer from 1 to 10000") from error
    if not 1 <= count <= 10000:
        raise argparse.ArgumentTypeError("Android replay lines must be from 1 to 10000")
    return count


def android_capture_command(args: argparse.Namespace) -> list[str]:
    if args.pid is None:
        raise ValueError("Android capture requires the freshly verified app --pid")
    replay = android_replay_lines(str(getattr(args, "android_replay_lines", 1)))
    return [args.adb, "-s", args.android, "logcat", "-v", "epoch", "-T", str(replay),
            "--pid", str(args.pid), *(tag + ":I" for tag in SAFE_TAGS), "*:S"]


def capture(args: argparse.Namespace) -> int:
    environment = os.environ.copy()
    if args.android:
        command = android_capture_command(args)
        detach_safe = True
    elif args.ios:
        if args.pid is not None:
            raise ValueError("iOS console PID comes from its launch evidence, not --pid")
        launch_output = args.output.with_suffix(".launch.json")
        if launch_output.exists():
            raise ValueError("refusing to overwrite existing iOS launch evidence")
        command = ["/usr/bin/xcrun", "devicectl", "device", "process", "launch",
                   "--device", args.ios, "--console", "--json-output", str(launch_output)]
        if args.terminate_existing:
            command.append("--terminate-existing")
        command.append(args.app)
        environment["DEVICECTL_CHILD_PRNS_GROUP_DIAGNOSTICS"] = "1"
        environment["DEVICECTL_CHILD_PRNS_BLUETOOTH_DIAGNOSTICS"] = "1"
        environment["DEVICECTL_CHILD_PRNS_TRIAL_DIAGNOSTICS"] = "1"
        environment["DEVICECTL_CHILD_PRNS_ROUTE_DIAGNOSTICS"] = "1"
        if getattr(args, "link_diagnostics", False):
            environment["DEVICECTL_CHILD_PRNS_LINK_DIAGNOSTICS"] = "1"
        detach_safe = False
    else:
        command = json.loads(args.command_json.read_text())
        if not isinstance(command, list) or not command or not all(isinstance(x, str) for x in command):
            raise ValueError("--command-json must contain a nonempty argv array, never shell text")
        detach_safe = args.detach_safe
    journal = Journal(args.output)
    process = None
    selector = selectors.DefaultSelector()
    status = "complete"
    rejected = 0
    events = 0
    buffer = b""
    discard_line = False
    started = time.monotonic()
    last_host_heartbeat = 0.0
    capped = False
    try:
        journal.append("capture_started", command_executable=Path(command[0]).name,
                       command_sha256=hashlib.sha256(json.dumps(command).encode()).hexdigest(),
                       expected_pid=args.pid,
                       heartbeat_seconds=args.heartbeat_seconds,
                       max_seconds=args.max_seconds, max_events=args.max_events,
                       **({"android_replay_lines": int(command[7])} if args.android else {}),
                       timestamp_semantics="Host complete-line receipt; buffering can delay observation")
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   env=environment, bufsize=0, start_new_session=True)
        assert process.stdout is not None
        os.set_blocking(process.stdout.fileno(), False)
        selector.register(process.stdout, selectors.EVENT_READ)
        eof = False
        while not eof:
            now = time.monotonic()
            if now - last_host_heartbeat >= args.heartbeat_seconds and not capped:
                journal.append("observer_heartbeat", child_alive=process.poll() is None,
                               accepted_events=events, discarded_lines=rejected,
                               capped=capped)
                last_host_heartbeat = now
            if now - started >= args.max_seconds and not capped:
                journal.append("observer_fault", reason="capture_duration_limit")
                capped = True
                status = "capped"
                if detach_safe:
                    process.terminate()
                # Never signal devicectl: its signal may be forwarded to the app.
                # Continue draining without keeping events until a coordinated
                # ordinary app restart closes the stream. The bounded limit
                # invalidates further evidence and is durable immediately.
            ready = selector.select(timeout=min(0.25, args.heartbeat_seconds))
            if not ready and process.poll() is not None:
                # Still drain any bytes already in the pipe before observing EOF.
                ready = [(None, None)]
            for _ in ready:
                data = os.read(process.stdout.fileno(), 65536)
                if not data:
                    eof = True
                    break
                for piece in data.splitlines(keepends=True):
                    buffer += piece
                    if len(buffer) > MAX_LINE:
                        discard_line = True
                        buffer = buffer[-1:]
                    if not piece.endswith(b"\n"):
                        continue
                    if discard_line:
                        rejected += 1
                    else:
                        parsed = parse_safe_line(buffer.decode("utf-8", errors="replace").rstrip("\r\n"), args.pid)
                        if parsed is not None and not capped:
                            if events >= args.max_events:
                                journal.append("observer_fault", reason="capture_event_limit")
                                capped = True
                                status = "capped"
                                if detach_safe:
                                    process.terminate()
                            else:
                                journal.append("event", **parsed)
                                events += 1
                        else:
                            rejected += 1
                    buffer = b""
                    discard_line = False
        if buffer:
            journal.append("observer_fault", reason="partial_console_line_at_eof")
            status = "incomplete"
        code = process.wait(timeout=5)
        journal.append("capture_ended", returncode=code, accepted_events=events,
                       discarded_lines=rejected, status=status)
        return 0 if code == 0 and status == "complete" else 2
    except BaseException:
        journal.append("observer_fault", reason="capture_interrupted")
        if process is not None and process.poll() is None and detach_safe:
            process.terminate()
        raise
    finally:
        selector.close()
        journal.close()


def read_journal(path: Path) -> tuple[list[dict], list[str]]:
    if not path.exists():
        return [], ["capture_missing"]
    contents = path.read_bytes()
    if not contents.endswith(b"\n"):
        # Concurrent final append may be in flight: never accept its partial row.
        contents = contents.rsplit(b"\n", 1)[0] + b"\n" if b"\n" in contents else b""
    rows, errors = [], []
    for line in contents.splitlines():
        try:
            row = json.loads(line)
            if (row.get("schema") != SCHEMA or row.get("sequence") != len(rows) + 1
                    or not isinstance(row.get("host_monotonic_ns"), int)):
                raise ValueError("schema or sequence")
            epoch(row["host_observed_utc"])
            if rows:
                if row["capture_id"] != rows[0]["capture_id"]:
                    raise ValueError("capture_id")
                elapsed = (row["host_monotonic_ns"] - rows[-1]["host_monotonic_ns"]) / 1e9
                wall = epoch(row["host_observed_utc"]) - epoch(rows[-1]["host_observed_utc"])
                if elapsed < 0 or abs(elapsed - wall) > 1:
                    raise ValueError("host clock discontinuity")
            rows.append(row)
        except (ValueError, TypeError, KeyError):
            errors.append("invalid_journal_row")
            break
    if not rows or rows[0].get("kind") != "capture_started":
        errors.append("capture_start_missing")
    return rows, errors


def matching(row: dict, where: dict) -> bool:
    return row.get("kind") == "event" and all(row.get("fields", {}).get(k) == str(v) for k, v in where.items())


def continuous(samples: list[dict], start: float, end: float, gap: float) -> bool:
    before = [row for row in samples if epoch(row["host_observed_utc"]) <= start]
    after = [row for row in samples if epoch(row["host_observed_utc"]) >= end]
    if not before or not after:
        return False
    first, last = before[-1], after[0]
    between = [row for row in samples if first["sequence"] <= row["sequence"] <= last["sequence"]]
    return all((b["host_monotonic_ns"] - a["host_monotonic_ns"]) / 1e9 <= gap
               for a, b in zip(between, between[1:]))


def message_bindings(spec: dict, root: Path, streams: dict) -> dict:
    bindings = {}
    for reference in spec["correlation_evidence"]:
        stream_name = reference["stream"]
        source = streams[stream_name]
        try:
            contents = (root / reference["path"]).read_bytes()
            if hashlib.sha256(contents).hexdigest() != reference["sha256"]:
                raise ValueError("mapping hash mismatch")
            binding = json.loads(contents)
            if (binding["case_id"] != spec["case_id"] or not source["rows"]
                    or binding["capture_id"] != source["rows"][0]["capture_id"]
                    or binding["session"] != source["config"].get("session")):
                raise ValueError("mapping case/capture/session mismatch")
            ui_path = Path(binding["ui_evidence"])
            if not ui_path.is_absolute():
                ui_path = (root / reference["path"]).parent / ui_path
            if hashlib.sha256(ui_path.read_bytes()).hexdigest() != binding["ui_evidence_sha256"]:
                raise ValueError("UI evidence changed")
            if not any(row["sequence"] == binding["first_capture_sequence"]
                       and matching(row, {"group": binding["group"], "record": binding["record"]})
                       for row in source["rows"]):
                raise ValueError("mapped record missing")
            bindings.setdefault(stream_name, set()).add((binding["group"], binding["record"]))
        except (KeyError, ValueError, OSError, TypeError):
            source["errors"].append("message_mapping_invalid")
    return bindings


def evaluate(spec: dict, root: Path) -> dict:
    if spec.get("gate_kind") == "schedule":
        return evaluate_schedule(spec, root)
    if spec.get("gate_kind", "delivery") != "delivery":
        raise ValueError("unknown gate_kind")
    start, deadline = epoch(spec["start_utc"]), epoch(spec["deadline_utc"])
    if deadline <= start:
        raise ValueError("deadline must follow the start")
    gap = float(spec.get("max_observer_gap_seconds", 15))
    if not 1 <= gap <= 30:
        raise ValueError("observer gap must be between 1 and 30 seconds")
    checks = spec["checks"]
    if not checks or not spec.get("correlation_evidence"):
        raise ValueError("exact message mapping evidence and at least one check are required")
    streams = {}
    results = []
    all_observed = True
    finish = start
    for name, stream in spec["streams"].items():
        rows, errors = read_journal(root / stream["capture"])
        streams[name] = {"rows": rows, "errors": errors, "config": stream}
    bindings = message_bindings(spec, root, streams)
    for check in checks:
        if not check.get("where"):
            raise ValueError("each check needs exact diagnostic fields")
        source = streams[check["stream"]]
        where = check["where"]
        if ("record" in where or check.get("kind") == "authenticated_receipt") and (
                where.get("group"), where.get("record")) not in bindings.get(check["stream"], set()):
            source["errors"].append("message_mapping_missing")
        candidates = [row for row in source["rows"]
                      if start <= epoch(row["host_observed_utc"]) <= deadline
                      and matching(row, check["where"])]
        first = candidates[0] if candidates else None
        # Receipt checks must always bind both opaque record and group within
        # this process. A peer's generic accepted response is not a receipt.
        if check.get("kind") == "authenticated_receipt":
            where = check["where"]
            if (where.get("stage") != "receipt_transition"
                    or where.get("category") != "authenticated_receipt_applied"
                    or not where.get("group") or not where.get("record")):
                raise ValueError("authenticated receipt requires exact group/record and durable receipt category")
        if first:
            finish = max(finish, epoch(first["host_observed_utc"]))
        else:
            all_observed = False
        results.append({"name": check["name"], "stream": check["stream"],
                        "observed_by_deadline": first is not None,
                        "first_observed_utc": first["host_observed_utc"] if first else None,
                        "capture_sequence": first["sequence"] if first else None,
                        "matching_observations": len(candidates)})
    # Complete success needs coverage through the last required observation;
    # timeout needs coverage all the way through the actual deadline.
    coverage_end = finish if all_observed else deadline
    stream_results = {}
    for name, source in streams.items():
        rows, errors = source["rows"], list(source["errors"])
        diagnostic_limits = []
        require_attempts = source["config"].get("require_sync_attempt_correlation", False)
        if not isinstance(require_attempts, bool):
            raise ValueError("require_sync_attempt_correlation must be a boolean")
        delivery_categories = {
            ("local_send_result", "durable_observed"),
            ("receipt_transition", "durable_record_received"),
            ("receipt_transition", "authenticated_receipt_applied"),
        }
        # Only explicitly independent attempt exhaustion can be ignored for a
        # delivery gate. Unknown/legacy limits and send-result exhaustion remain
        # fatal. A check of any other diagnostic family requires full attempt
        # correlation, even when the explicit option was omitted.
        require_attempts |= any(
            (check["where"].get("stage"), check["where"].get("category"))
            not in delivery_categories for check in checks if check["stream"] == name
        )
        interval = [row for row in rows if start <= epoch(row["host_observed_utc"]) <= coverage_end]
        host = [row for row in rows if row.get("kind") == "observer_heartbeat" and row.get("child_alive")]
        native = [row for row in rows if row.get("tag") == "prns.trial" and matching(row, {"stage": "heartbeat"})]
        if not continuous(host, start, coverage_end, gap):
            errors.append("host_observer_coverage_gap")
        if not continuous(native, start, coverage_end, gap):
            errors.append("native_stream_coverage_gap")
        relevant_native = [row for row in native if start - gap <= epoch(row["host_observed_utc"]) <= coverage_end + gap]
        expected_session = source["config"].get("session")
        required_ready = source["config"].get("required_readiness", ["groups_ready"])
        if not isinstance(required_ready, list) or any(value not in ("groups_ready", "bluetooth_ready", "routes_ready") for value in required_ready):
            raise ValueError("unknown native readiness requirement")
        if any(any(row["fields"].get(key) != "true" for key in required_ready) for row in relevant_native):
            errors.append("native_diagnostics_not_ready")
        if not expected_session:
            errors.append("native_session_not_pinned")
        elif any(row["fields"].get("session") != expected_session for row in relevant_native):
            errors.append("native_session_changed")
        for a, b in zip(relevant_native, relevant_native[1:]):
            try:
                if int(b["fields"]["sequence"]) != int(a["fields"]["sequence"]) + 1:
                    errors.append("native_heartbeat_sequence_gap")
            except (KeyError, ValueError):
                errors.append("native_heartbeat_sequence_missing")
        if any(row.get("kind") in ("observer_fault", "capture_ended") for row in interval):
            errors.append("observer_interrupted")
        required_tags = source["config"].get("required_tags", ["prns.groups", "prns.trial"])
        if any(row.get("kind") == "event" and row.get("tag") in required_tags
               and row["fields"].get("category") in ("output_truncated", "entropy_unavailable", "heartbeat_start_failed")
               for row in rows if epoch(row["host_observed_utc"]) <= coverage_end):
            errors.append("native_output_invalid")
        if any(row.get("kind") == "event" and row.get("tag") in required_tags
               and row.get("tag") in ("prns.bluetooth", "prns.routes")
               and row["fields"].get("category") in ("line_limit", "family_limit")
               for row in rows if epoch(row["host_observed_utc"]) <= coverage_end):
            errors.append("required_diagnostic_budget_exhausted")
        if any(row.get("kind") == "event" and (row["fields"].get("stage") == "delivery_limit"
               or row["fields"].get("category") in ("delivery_line_limit", "heartbeat_limit"))
               for row in rows if epoch(row["host_observed_utc"]) <= coverage_end):
            errors.append("diagnostic_budget_exhausted")
        for row in rows:
            if (row.get("kind") != "event" or row["fields"].get("category") != "correlation_limit"
                    or epoch(row["host_observed_utc"]) > coverage_end):
                continue
            if (row.get("tag") == "prns.groups" and row["fields"].get("family") == "sync_attempt"
                    and not require_attempts):
                diagnostic_limits.append("sync_attempt_correlation_exhausted")
            else:
                errors.append("diagnostic_budget_exhausted")
        stream_results[name] = {"capture": source["config"]["capture"],
                                "capture_id": rows[0]["capture_id"] if rows else None,
                                "qualified_coverage": not errors,
                                "problems": sorted(set(errors)),
                                "diagnostic_limits": sorted(set(diagnostic_limits))}
    covered = all(item["qualified_coverage"] for item in stream_results.values())
    status = "passed" if covered and all_observed else "failed_deadline" if covered else "unqualified"
    return {"schema": SCHEMA, "case_id": spec["case_id"], "status": status,
            "generated_utc": utc_now(), "start_utc": spec["start_utc"],
            "deadline_utc": spec["deadline_utc"], "max_observer_gap_seconds": gap,
            "coverage_through_utc": dt.datetime.fromtimestamp(coverage_end, dt.timezone.utc).isoformat(),
            "checks": results, "streams": stream_results,
            "correlation_evidence": spec["correlation_evidence"],
            "semantics": "Host observed deadline; buffering can delay observation. Duplicate receipt observations are not unique commits. Missing coverage is not a delivery failure."}



SCHEDULE_CATEGORIES = {"completed", "fresh_work", "pending_work", "peer_recovered", "admitted"}
SCHEDULE_FIELDS = {"stage", "category", "sequence", "group", "peer", "activity", "due_in_ms",
                   "floor_in_ms", "failures", "pending", "fresh", "repair", "accelerated"}
RECOVERY_CATEGORIES = {"service_framing_rejected", "service_requester_unknown", "service_scope_stale",
                       "service_scope_unavailable", "offered", "coalesced", "queue_full", "queue_poisoned", "scope_unavailable",
                       "scope_stale", "candidate_missing", "peer_active", "not_failed", "credited",
                       "consumed_no_pending", "consumed_probe_wait", "consumed_fresh_work",
                       "consumed_accelerated", "consumed_no_shortening"}
RECOVERY_FIELDS = {"stage", "category", "sequence", "group", "peer"}


def valid_recovery(fields: dict) -> bool:
    return (set(fields) == RECOVERY_FIELDS and fields.get("stage") == "recovery"
            and fields.get("category") in RECOVERY_CATEGORIES
            and all(re.fullmatch(r"[0-9a-f]{16}", fields.get(key, "")) for key in ("group", "peer"))
            and re.fullmatch(r"[0-9]{1,4}", fields.get("sequence", "")) is not None
            and 1 <= int(fields["sequence"]) <= 1024)


def valid_schedule(fields: dict) -> bool:
    if (set(fields) != SCHEDULE_FIELDS or fields.get("stage") != "schedule"
            or fields.get("category") not in SCHEDULE_CATEGORIES
            or fields.get("activity") not in ("progress", "idle", "failed", "none")
            or fields.get("pending") not in ("true", "false", "unknown")
            or any(fields.get(key) not in ("true", "false") for key in ("fresh", "repair", "accelerated"))
            or any(not re.fullmatch(r"[0-9a-f]{16}", fields.get(key, "")) for key in ("group", "peer"))):
        return False
    for key in ("sequence", "due_in_ms", "floor_in_ms", "failures"):
        if not re.fullmatch(r"[0-9]{1,20}", fields.get(key, "")) or int(fields[key]) > 2**64 - 1:
            return False
    return 1 <= int(fields["sequence"]) <= 1024


def evaluate_schedule(spec: dict, root: Path) -> dict:
    """Qualify group/peer schedule decisions, independently of delivery gates.

    A heartbeat watermark closes the otherwise invisible missing-tail case.
    It is a lower bound: concurrent emission can precede the output of an older
    heartbeat snapshot, so extra consecutive records are safe to retain.
    """
    start, deadline = epoch(spec["start_utc"]), epoch(spec["deadline_utc"])
    gap = float(spec.get("max_observer_gap_seconds", 15))
    if deadline <= start or not 1 <= gap <= 30:
        raise ValueError("invalid schedule deadline or observer gap")
    if not spec.get("checks") or not spec.get("correlation_evidence") or not spec.get("streams"):
        raise ValueError("schedule checks require streams and immutable message mapping evidence")
    streams = {}
    for name, config in spec["streams"].items():
        rows, errors = read_journal(root / config["capture"])
        streams[name] = {"rows": rows, "errors": errors, "config": config}
    bindings = message_bindings(spec, root, streams)
    checks, all_observed, finish = [], True, start
    for check in spec["checks"]:
        where = check["where"]
        allowed_fields, categories = ((RECOVERY_FIELDS, RECOVERY_CATEGORIES)
                                      if where.get("stage") == "recovery"
                                      else (SCHEDULE_FIELDS, SCHEDULE_CATEGORIES))
        if (set(where) - allowed_fields or where.get("stage") not in ("schedule", "recovery")
                or where.get("category") not in categories
                or any(not re.fullmatch(r"[0-9a-f]{16}", where.get(key, "")) for key in ("group", "peer"))):
            raise ValueError("schedule checks require exact category/group/peer and closed stage fields")
        source = streams[check["stream"]]
        if where["group"] not in {group for group, _ in bindings.get(check["stream"], set())}:
            source["errors"].append("schedule_group_mapping_missing")
        bound = float(check.get("max_after_start_seconds", deadline - start))
        if not 0 < bound <= deadline - start:
            raise ValueError("schedule target must be positive and within the fixed deadline")
        candidates = [row for row in source["rows"]
                      if start <= epoch(row["host_observed_utc"]) <= start + bound
                      and row.get("tag") == "prns.schedule" and matching(row, where)]
        first = candidates[0] if candidates else None
        if first:
            finish = max(finish, epoch(first["host_observed_utc"]))
        else:
            all_observed = False
        checks.append({"name": check["name"], "stream": check["stream"],
                       "observed_by_deadline": first is not None,
                       "max_after_start_seconds": bound,
                       "first_observed_utc": first["host_observed_utc"] if first else None,
                       "host_seconds_after_start": epoch(first["host_observed_utc"]) - start if first else None,
                       "capture_sequence": first["sequence"] if first else None,
                       "fields": first["fields"] if first else None,
                       "matching_observations": len(candidates)})
    coverage_end = finish if all_observed else deadline
    results = {}
    for name, source in streams.items():
        rows, errors, config = source["rows"], list(source["errors"]), source["config"]
        host = [row for row in rows if row.get("kind") == "observer_heartbeat" and row.get("child_alive")]
        native = [row for row in rows if row.get("tag") == "prns.trial" and matching(row, {"stage": "heartbeat"})]
        if not continuous(host, start, coverage_end, gap):
            errors.append("host_observer_coverage_gap")
        if not continuous(native, start, coverage_end, gap):
            errors.append("native_stream_coverage_gap")
        after = [row for row in native if epoch(row["host_observed_utc"]) >= coverage_end]
        anchor = after[0] if after else None
        nearby = [row for row in native if start - gap <= epoch(row["host_observed_utc"])
                  and (anchor is None or row["sequence"] <= anchor["sequence"])]
        if not config.get("session"):
            errors.append("native_session_not_pinned")
        elif any(row["fields"].get("session") != config["session"] for row in nearby):
            errors.append("native_session_changed")
        if any(row["fields"].get("schedule_ready") != "true" for row in nearby):
            errors.append("schedule_diagnostics_not_ready")
        for a, b in zip(nearby, nearby[1:]):
            try:
                if int(b["fields"]["sequence"]) != int(a["fields"]["sequence"]) + 1:
                    errors.append("native_heartbeat_sequence_gap")
            except (ValueError, KeyError):
                errors.append("native_heartbeat_sequence_missing")
        prefix = [row for row in rows if anchor and row["sequence"] <= anchor["sequence"]]
        if not any(row.get("tag") == "prns.schedule" and matching(row, {"stage": "logger", "category": "schedule_probe"})
                   and epoch(row["host_observed_utc"]) <= start for row in prefix):
            errors.append("schedule_probe_missing")
        events, watermark, previous_watermark = [], None, 0
        for row in prefix:
            fields = row.get("fields", {})
            if row.get("tag") == "prns.trial":
                if fields.get("stage") == "heartbeat":
                    if fields.get("session") != config.get("session"):
                        errors.append("native_session_changed")
                    value = fields.get("schedule_sequence", "")
                    if not re.fullmatch(r"[0-9]{1,4}", value) or int(value) > 1024:
                        errors.append("schedule_watermark_invalid")
                    else:
                        watermark = int(value)
                        if watermark < previous_watermark:
                            errors.append("schedule_watermark_regressed")
                        previous_watermark = watermark
                elif fields.get("category") in ("heartbeat_limit", "heartbeat_start_failed", "output_truncated"):
                    errors.append("native_output_invalid")
            if row.get("tag") != "prns.schedule":
                continue
            if fields.get("stage") == "schedule":
                if not valid_schedule(fields):
                    errors.append("schedule_record_invalid")
                else:
                    events.append(row)
            elif fields.get("stage") == "recovery":
                if not valid_recovery(fields):
                    errors.append("recovery_record_invalid")
                else:
                    events.append(row)
            elif fields == {"stage": "logger", "category": "schedule_probe"}:
                continue
            else:
                # Includes producer/sink caps, unavailable correlation, truncated
                # output and unknown future markers. Never infer a safe family.
                errors.append("schedule_output_invalid")
        sequences = [int(row["fields"]["sequence"]) for row in events]
        if sequences != list(range(1, len(sequences) + 1)):
            errors.append("schedule_sequence_gap")
        if watermark is None or len(sequences) < watermark:
            errors.append("schedule_tail_missing")
        if any(row.get("kind") in ("observer_fault", "capture_ended")
               and start <= epoch(row["host_observed_utc"]) for row in prefix):
            errors.append("observer_interrupted")
        independent_limits = sorted({row["fields"].get("category") for row in prefix
                                     if row.get("kind") == "event" and row.get("tag") not in ("prns.schedule", "prns.trial")
                                     and "limit" in row.get("fields", {}).get("category", "")})
        results[name] = {"capture": config["capture"], "capture_id": rows[0]["capture_id"] if rows else None,
                         "qualified_coverage": not errors, "problems": sorted(set(errors)),
                         "schedule_events_through_anchor": len(events), "schedule_watermark": watermark,
                         "watermark_observed_utc": anchor["host_observed_utc"] if anchor else None,
                         "independent_limits": independent_limits}
    covered = all(value["qualified_coverage"] for value in results.values())
    return {"schema": SCHEMA, "gate_kind": "schedule", "case_id": spec["case_id"],
            "status": "passed" if covered and all_observed else "failed_deadline" if covered else "unqualified",
            "generated_utc": utc_now(), "start_utc": spec["start_utc"], "deadline_utc": spec["deadline_utc"],
            "max_observer_gap_seconds": gap,
            "coverage_through_utc": dt.datetime.fromtimestamp(coverage_end, dt.timezone.utc).isoformat(),
            "checks": checks, "streams": results, "correlation_evidence": spec["correlation_evidence"],
            "semantics": "Group/peer scheduler decisions in one pinned process, with complete numbered output through a native heartbeat watermark. Message mapping binds the group, not a decision to a particular record. Host latency includes console buffering; it is not RF timing or a delivery guarantee."}


def bind_message(args: argparse.Namespace) -> int:
    """Bind one observed action to exactly one process-local opaque record."""
    start, end = epoch(args.start_utc), epoch(args.end_utc)
    if end < start:
        raise ValueError("mapping window ends before it starts")
    if not re.fullmatch(r"[0-9a-f]{64}", args.marker_sha256):
        raise ValueError("marker SHA-256 must be exactly 64 lowercase hex digits")
    rows, errors = read_journal(args.capture)
    if errors:
        raise ValueError("cannot bind an invalid capture: " + ", ".join(errors))
    category = "durable_observed" if args.kind == "local-send" else "durable_record_received"
    stage = "local_send_result" if args.kind == "local-send" else "receipt_transition"
    candidates = [row for row in rows if start <= epoch(row["host_observed_utc"]) <= end
                  and matching(row, {"stage": stage, "category": category})
                  and (args.group is None or row["fields"].get("group") == args.group)]
    unique = {(row["fields"].get("group"), row["fields"].get("record")) for row in candidates}
    if len(unique) != 1 or None in next(iter(unique), (None, None)):
        raise ValueError("message mapping requires exactly one distinct group/record in the observed window")
    sessions = {row["fields"].get("session") for row in rows
                if matching(row, {"stage": "heartbeat"})
                and start - 15 <= epoch(row["host_observed_utc"]) <= end + 15}
    if len(sessions) != 1 or None in sessions:
        raise ValueError("mapping requires one observed native process session")
    ui_bytes = args.ui_evidence.read_bytes()
    group, record = unique.pop()
    value = {"schema": SCHEMA, "case_id": args.case_id, "kind": args.kind,
             "marker_sha256": args.marker_sha256, "start_utc": args.start_utc, "end_utc": args.end_utc,
             "capture": str(args.capture), "capture_id": rows[0]["capture_id"],
             "session": sessions.pop(), "group": group, "record": record,
             "first_capture_sequence": candidates[0]["sequence"],
             "matching_observations": len(candidates),
             "ui_evidence": str(args.ui_evidence.resolve()), "ui_evidence_sha256": hashlib.sha256(ui_bytes).hexdigest(),
             "scope": "Operator-selected action/UI window; uniqueness is checked within it. Different phones' tags are not equated."}
    if args.output.exists():
        raise ValueError("refusing to replace an existing message mapping")
    atomic_json(args.output, value)
    print(json.dumps(value, sort_keys=True))
    return 0


def run_evaluation(args: argparse.Namespace) -> int:
    raw_spec = args.spec.read_bytes()
    spec = json.loads(raw_spec)
    spec_hash = hashlib.sha256(raw_spec).hexdigest()
    if args.output.exists():
        previous = json.loads(args.output.read_text())
        if previous.get("finalized"):
            raise ValueError("refusing to overwrite a finalized gate; use a separate late-recovery result")
        if previous.get("spec_sha256") != spec_hash:
            raise ValueError("existing gate belongs to a different specification")
    deadline = epoch(spec["deadline_utc"])
    stop = deadline + float(spec.get("max_observer_gap_seconds", 15)) + args.grace_seconds
    while True:
        if args.spec.read_bytes() != raw_spec:
            atomic_json(args.output, {"schema": SCHEMA, "case_id": spec["case_id"],
                                      "status": "unqualified", "finalized": True,
                                      "generated_utc": utc_now(), "spec_sha256": spec_hash,
                                      "reason": "case_specification_changed"})
            return 3
        result = evaluate(spec, args.spec.parent)
        result["spec_sha256"] = spec_hash
        final = not args.watch or result["status"] == "passed" or time.time() >= stop
        result["finalized"] = final
        if not final:
            result["provisional_status"] = result["status"]
            result["status"] = "observing"
        atomic_json(args.output, result)
        if final:
            print(json.dumps({"case_id": result["case_id"], "status": result["status"], "output": str(args.output)}))
            return {"passed": 0, "failed_deadline": 1, "unqualified": 2}[result["status"]]
        time.sleep(min(1, max(0.05, stop - time.time())))


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="action", required=True)
    capture_parser = commands.add_parser("capture")
    capture_parser.add_argument("--link-diagnostics", action="store_true", help="Opt into bounded Link diagnostics on an iOS launch")
    source = capture_parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--android", metavar="DEVICE")
    source.add_argument("--ios", metavar="DEVICE")
    source.add_argument("--command-json", type=Path, help="JSON argv array for a controlled source or host fixture")
    capture_parser.add_argument("--output", type=Path, required=True)
    capture_parser.add_argument("--pid", type=int)
    capture_parser.add_argument("--adb", default="adb")
    capture_parser.add_argument("--android-replay-lines", type=android_replay_lines, default=1,
                                help="Bounded Android logcat replay (1..10000 lines, default 1); use 5000 for startup scheduling evidence")
    capture_parser.add_argument("--app", default="rs.reticulum.prns.dev")
    capture_parser.add_argument("--terminate-existing", action="store_true", help="Explicit iOS ordinary process replacement; preserves app storage")
    capture_parser.add_argument("--detach-safe", action="store_true", help="Custom command may be terminated safely at capture cap")
    capture_parser.add_argument("--max-seconds", type=float, default=7200)
    capture_parser.add_argument("--max-events", type=int, default=20000)
    capture_parser.add_argument("--heartbeat-seconds", type=float, default=5)
    mapping = commands.add_parser("bind")
    mapping.add_argument("--capture", type=Path, required=True)
    mapping.add_argument("--output", type=Path, required=True)
    mapping.add_argument("--case-id", required=True)
    mapping.add_argument("--kind", choices=("local-send", "incoming"), required=True)
    mapping.add_argument("--start-utc", required=True)
    mapping.add_argument("--end-utc", required=True)
    mapping.add_argument("--marker-sha256", required=True)
    mapping.add_argument("--ui-evidence", type=Path, required=True)
    mapping.add_argument("--group", help="Optional known process-local group tag")
    evaluation = commands.add_parser("evaluate")
    evaluation.add_argument("--spec", type=Path, required=True)
    evaluation.add_argument("--output", type=Path, required=True)
    evaluation.add_argument("--watch", action="store_true", help="Persist status continuously until pass or the deadline grace expires")
    evaluation.add_argument("--grace-seconds", type=float, default=5, help="Only for observing heartbeat coverage; never extends the event deadline")
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.action == "capture":
            if not 0 < args.heartbeat_seconds <= 10 or not 0 < args.max_seconds <= 86400 or not 0 < args.max_events <= 100000:
                raise ValueError("invalid capture bounds")
            return capture(args)
        if args.action == "bind":
            return bind_message(args)
        if not 0 <= args.grace_seconds <= 30:
            raise ValueError("coverage grace must be between 0 and 30 seconds")
        return run_evaluation(args)
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(f"Observer error: {error}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
