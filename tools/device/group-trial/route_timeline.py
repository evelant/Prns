#!/usr/bin/env python3
"""Read-only positive route-phase analysis of existing phone/board journals.

Shared nonce tokens correlate request observations. Endpoint/interface tokens
remain capture/boot-local. Response events have no nonce and are only contextual.
This does not assess delivery, prove missing transmissions, or bind a request to
a group message. Input snapshots may be taken while captures are still running.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re

MAX_INPUT = 16 * 1024 * 1024
MAX_ROWS = 50_000
TOKEN = re.compile(r"[0-9a-f]{16}\Z")
ZERO = "0" * 16
PHASES = {"local_request", "request_ingress", "request_egress", "response_ingress",
          "response_dispatched", "local_timeout", "recursive_timeout"}
KINDS = {"Loopback", "TcpClient", "TcpServer", "Udp", "Serial", "UsbAutoHost",
         "UsbAutoDevice", "AutoWifi", "WifiPeer", "LocalServer", "LocalClient",
         "TcpServerPeer", "BluetoothAuto", "BluetoothPeer", "LoRa", "Kiss",
         "Ax25Kiss", "Pipe", "Rnode", "BackboneServer", "BackboneServerPeer",
         "BackboneClient", "EspNow", "WebSocketClient", "WebSocketServer",
         "WebSocketServerPeer", "WifiDirect", "WifiDirectPeer", "WifiAware",
         "WifiAwarePeer", "I2p", "I2pPeer", "Weave", "WeavePeer", "WifiHaLow",
         "WifiHaLowPeer", "WifiHaLowBroadcast"}


def load_sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PHONE = load_sibling("observer")
BOARD = load_sibling("board_observer")


def snapshot(path: Path):
    """One bounded read, one hash, one validated prefix; no concurrent reread."""
    try:
        with path.open("rb") as source:
            data = source.read(MAX_INPUT + 1)
    except OSError:
        return [], {"errors": ["input_unavailable"]}
    info = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "errors": []}
    if len(data) > MAX_INPUT:
        info["errors"].append("input_byte_limit")
        return [], info
    partial = bool(data) and not data.endswith(b"\n")
    info["partial_final_row_unobserved"] = partial
    if partial:
        data = data.rsplit(b"\n", 1)[0] + b"\n" if b"\n" in data else b""
    rows = []
    for line in data.splitlines():
        if len(rows) >= MAX_ROWS:
            info["errors"].append("input_row_limit")
            break
        try:
            row = json.loads(line)
            if (row.get("schema") != 1 or row.get("sequence") != len(rows) + 1
                    or not isinstance(row.get("host_monotonic_ns"), int)
                    or not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", row.get("capture_id", ""))):
                raise ValueError
            wall = PHONE.epoch(row["host_observed_utc"])
            if rows:
                prior = rows[-1]
                elapsed = (row["host_monotonic_ns"] - prior["host_monotonic_ns"]) / 1e9
                wall_delta = wall - PHONE.epoch(prior["host_observed_utc"])
                if (row["capture_id"] != rows[0]["capture_id"] or elapsed < 0
                        or abs(elapsed - wall_delta) > 1):
                    raise ValueError
            rows.append(row)
        except (ValueError, TypeError, KeyError, AttributeError):
            info["errors"].append("invalid_journal_row")
            break
    if not rows or rows[0].get("kind") != "capture_started":
        info["errors"].append("capture_start_missing")
    info["rows"] = len(rows)
    info["terminal_observed"] = any(row.get("kind") == "capture_ended" for row in rows)
    if rows:
        info.update(capture_id=rows[0]["capture_id"],
                    first_observed_utc=rows[0]["host_observed_utc"],
                    last_observed_utc=rows[-1]["host_observed_utc"])
    return rows, info


def normalize_route(row: dict, source: str) -> dict | None:
    board = row.get("kind") == "route_event"
    if board:
        fields = row
    elif row.get("kind") == "event" and row.get("tag") == "prns.routes":
        fields = row.get("fields", {})
        if fields.get("stage") != "path":
            return None
    else:
        return None
    phase = fields.get("phase")
    if phase not in PHASES:
        return None
    event = {"event_id": f"{source}:{row['sequence']}", "source": source,
             "capture_id": row["capture_id"], "capture_sequence": row["sequence"],
             "host_observed_utc": row["host_observed_utc"],
             "host_monotonic_ns": row["host_monotonic_ns"], "phase": phase,
             "board_segment": 0}
    for field in ("destination", "interface", "request"):
        value = fields.get(field)
        present = board or fields.get("has_" + field) == "true"
        event[field] = value if present and isinstance(value, str) and TOKEN.fullmatch(value) and value != ZERO else None
    disposition = fields.get("disposition")
    if phase == "request_ingress":
        if disposition not in BOARD.REQUEST_DISPOSITIONS:
            return None
    elif phase == "request_egress":
        if disposition not in {"Dispatched", "Offline", "RateLimited"}:
            return None
    elif phase == "response_ingress":
        if disposition not in {"VerificationPending", "Accepted", "RetainedRouteMatched", "Ignored", "Blackholed"}:
            return None
    elif disposition is not None:
        return None
    if disposition is not None:
        event["disposition"] = disposition
    if fields.get("mode") in BOARD.MODES:
        event["mode"] = fields["mode"]
    if fields.get("recursive") in (True, False, "true", "false"):
        event["recursive"] = fields["recursive"] in (True, "true")
    if isinstance(fields.get("cached_interface"), str) and TOKEN.fullmatch(fields["cached_interface"]):
        event["cached_interface"] = fields["cached_interface"]
    kind = fields.get("kind")
    if kind in {"None"} | {f"Some({name})" for name in KINDS}:
        event["interface_kind"] = kind
    if board:
        for key in ("board_sequence", "board_observed_ms", "core_ms", "board_segment", "command"):
            value = fields.get(key)
            if isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= BOARD.U64_MAX:
                event[key] = value
    return event


def marker(row: dict, source: str):
    kind = row.get("kind")
    fields = row.get("fields", {})
    result = {"source": source, "capture_sequence": row["sequence"],
              "host_observed_utc": row["host_observed_utc"], "kind": kind}
    allowed = {"capture_started", "capture_ended", "capture_health", "observer_heartbeat",
               "observer_fault", "observation_gap", "boot_boundary", "board_diagnostics_limit"}
    if kind in allowed:
        for key in ("reason", "source"):
            value = row.get(key)
            if isinstance(value, str) and re.fullmatch(r"[a-z_]{1,64}", value):
                result["marker_" + key] = value
        if isinstance(row.get("problems"), list):
            result["problems"] = [p for p in row["problems"] if isinstance(p, str) and re.fullmatch(r"[a-z_]{1,64}", p)]
        for key in ("board_segment", "missed", "previous", "current", "rejected_route_lines",
                    "undecodable_lines", "oversized_lines", "discarded_lines"):
            value = row.get(key)
            if isinstance(value, int) and 0 <= value <= BOARD.U64_MAX:
                result[key] = value
        return result
    if kind == "event" and row.get("tag") == "prns.routes" and fields.get("stage") == "diagnostics":
        result["kind"] = "route_diagnostics_status"
        result["category"] = fields.get("category") if fields.get("category") in {"line_limit", "entropy_unavailable"} else "unknown"
        return result
    return None


def analyze(inputs: dict[str, Path], *, start=None, end=None, same_host=False):
    start_epoch = PHONE.epoch(start) if start else float("-inf")
    end_epoch = PHONE.epoch(end) if end else float("inf")
    if end_epoch < start_epoch:
        raise ValueError("invalid time window")
    timelines, markers, sources = [], [], {}
    for source, path in inputs.items():
        rows, info = snapshot(path)
        sources[source] = info
        info["rejected_route_rows"] = 0
        for row in rows:
            at = PHONE.epoch(row["host_observed_utc"])
            relation = "before" if at < start_epoch else "after" if at > end_epoch else "within"
            note = marker(row, source)
            if note and (relation == "within" or note["kind"] in {
                    "capture_ended", "observer_fault", "observation_gap", "boot_boundary",
                    "board_diagnostics_limit", "route_diagnostics_status"}):
                # Caps can disable a producer before the requested case window.
                # Retain their context without treating later faults as case faults.
                note["window_relation"] = relation
                markers.append(note)
            if relation != "within":
                continue
            event = normalize_route(row, source)
            if event:
                timelines.append(event)
            elif row.get("kind") == "route_event" or (row.get("tag") == "prns.routes" and row.get("fields", {}).get("stage") == "path"):
                info["rejected_route_rows"] += 1
    timelines.sort(key=lambda event: (event["host_monotonic_ns"] if same_host else PHONE.epoch(event["host_observed_utc"]), event["source"], event["capture_sequence"]))
    threads, contexts = defaultdict(list), defaultdict(list)
    for event in timelines:
        if event["request"]:
            threads[event["request"]].append(event)
        if event["destination"]:
            key = f"{event['source']}:{event['capture_id']}:{event['board_segment']}:{event['destination']}"
            event["local_destination_context"] = key
            contexts[key].append(event)
    requests = []
    for token, events in threads.items():
        phases = Counter((e["source"], e["phase"], e.get("disposition")) for e in events)
        request = {"request_token": token, "sources_observed": sorted({e["source"] for e in events}),
                   "event_ids": [e["event_id"] for e in events],
                   "positive_phase_counts": [{"source": s, "phase": p, "disposition": d, "count": n}
                                             for (s, p, d), n in phases.items()],
                   "local_destination_contexts": sorted({e["local_destination_context"] for e in events if "local_destination_context" in e})}
        if same_host:
            request["adjacent_host_observation_gaps_ms"] = [
                {"from": a["event_id"], "to": b["event_id"],
                 "milliseconds": round((b["host_monotonic_ns"] - a["host_monotonic_ns"]) / 1e6, 3)}
                for a, b in zip(events, events[1:])]
        requests.append(request)
    return {"schema": 1, "analysis_kind": "positive_route_observations_only",
            "window": {"start_utc": start, "end_utc": end}, "same_host_clock_asserted": same_host,
            "limits": ["No delivery or deadline qualification is made.",
                       "Absent phases are unobserved, not proof of absent transmissions.",
                       "Dispatched means runtime handoff, not RF transmission.",
                       "Host observation gaps include buffering, scheduling, logging and capture delay.",
                       "Response events lack request nonces; destination context is not request attribution.",
                       "Phone local-timeout records lack command/destination correlation and remain unassigned.",
                       "Caps, faults and boot boundaries include whole-capture context with before/within/after window labels.",
                       "Destination/interface tags are scoped to one capture and boot segment.",
                       "Route diagnostics do not bind a route request to a group message record."],
            "inputs": sources, "route_event_count": len(timelines), "timeline": timelines,
            "request_threads": requests,
            "local_destination_contexts": [{"context": key, "event_ids": [e["event_id"] for e in events],
                                            "request_tokens": sorted({e["request"] for e in events if e["request"]}),
                                            "response_event_ids": [e["event_id"] for e in events if e["phase"].startswith("response_")]}
                                           for key, events in contexts.items()],
            "capture_markers": markers}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", action="append", required=True, metavar="LABEL=PATH")
    parser.add_argument("--start-utc")
    parser.add_argument("--end-utc")
    parser.add_argument("--same-host", action="store_true", help="Assert all captures use this host's monotonic clock")
    parser.add_argument("--output", required=True, type=Path, help="New derived JSON file; input files are never modified")
    args = parser.parse_args()
    inputs = {}
    for value in args.input:
        label, separator, path = value.partition("=")
        if not separator or not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", label) or label in inputs:
            parser.error("inputs require unique bounded labels and paths")
        inputs[label] = Path(path)
    if len(inputs) > 12:
        parser.error("at most twelve input captures")
    result = analyze(inputs, start=args.start_utc, end=args.end_utc, same_host=args.same_host)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2, sort_keys=True)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
    print(json.dumps({"route_events": result["route_event_count"], "request_threads": len(result["request_threads"])}))


if __name__ == "__main__":
    main()
