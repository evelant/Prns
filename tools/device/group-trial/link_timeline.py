#!/usr/bin/env python3
"""Read-only Link phase observations with independent, fail-closed coverage.

Input is one native capture. Cross-capture public ephemeral link tokens may be
joined, but never establish group-message identity, RF success or delivery.
"""
from __future__ import annotations
import argparse
import importlib.util
import json
from pathlib import Path
import re

_spec = importlib.util.spec_from_file_location("route_timeline", Path(__file__).with_name("route_timeline.py"))
assert _spec and _spec.loader
ROUTES = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ROUTES)
PHONE = ROUTES.PHONE

_schema_spec = importlib.util.spec_from_file_location("link_schema", Path(__file__).with_name("link_schema.py"))
assert _schema_spec and _schema_spec.loader
SCHEMA = importlib.util.module_from_spec(_schema_spec)
_schema_spec.loader.exec_module(SCHEMA)
CORE_BASE, CORE_PHASES = SCHEMA.CORE_BASE, SCHEMA.CORE_PHASES
EGRESS_FIELDS, EGRESS_PHASES = SCHEMA.EGRESS_FIELDS, SCHEMA.EGRESS_PHASES
IGNORE, DETAILS, valid = SCHEMA.IGNORE, SCHEMA.DETAILS, SCHEMA.valid


def analyze(rows, start, end, gap=12.0):
    """Qualify one fixed interval using all prior sequence and watermark evidence."""
    errors, events = [], []
    native = [r for r in rows if r.get("tag") == "prns.trial" and r.get("fields", {}).get("stage") == "heartbeat"]
    anchors = [r for r in native if PHONE.epoch(r["host_observed_utc"]) >= end]
    if not anchors: return {"qualified_coverage": False, "problems": ["closing_heartbeat_missing"], "events": []}
    anchor = anchors[0]
    prefix = [r for r in rows if r["sequence"] <= anchor["sequence"]]
    relevant = [r for r in native if start - gap <= PHONE.epoch(r["host_observed_utc"]) and r["sequence"] <= anchor["sequence"]]
    # Host and native heartbeats have independent phases. The native watermark
    # closes Link events; a later host heartbeat may close the same fixed window.
    host_anchor = next((r for r in rows if r.get("kind") == "observer_heartbeat"
                        and PHONE.epoch(r["host_observed_utc"]) >= end), None)
    horizon = max(anchor["sequence"], host_anchor["sequence"] if host_anchor else 0)
    host = [r for r in rows if r.get("kind") == "observer_heartbeat" and r.get("child_alive")
            and host_anchor is not None and r["sequence"] <= host_anchor["sequence"]]
    if host_anchor is None: errors.append("closing_host_heartbeat_missing")
    if not PHONE.continuous(host, start, end, gap): errors.append("host_coverage_gap")
    if any(r.get("kind") in {"observer_fault", "capture_ended"} for r in rows if r["sequence"] <= horizon):
        errors.append("observer_interrupted")
    if not PHONE.continuous(relevant, start, end, gap): errors.append("native_coverage_gap")
    if not relevant or any(r["fields"].get("links_ready") != "true" for r in relevant): errors.append("link_diagnostics_not_ready")
    trusted_native = [r for r in native if r["sequence"] <= anchor["sequence"]]
    if len({r["fields"].get("session") for r in trusted_native}) != 1 or any(not r["fields"].get("session") for r in trusted_native): errors.append("native_session_changed")
    for a, b in zip(trusted_native, trusted_native[1:]):
        try:
            if int(b["fields"]["sequence"]) != int(a["fields"]["sequence"]) + 1: errors.append("native_sequence_gap")
        except (KeyError, ValueError): errors.append("native_sequence_invalid")
    probe, sequence, sources, watermark = 0, 0, {"core": 0, "egress": 0}, 0
    for row in prefix:
        f = row.get("fields", {})
        if row.get("kind") in {"observer_fault", "capture_ended"}: errors.append("observer_interrupted")
        if row.get("tag") == "prns.trial" and f.get("stage") == "heartbeat":
            v = f.get("link_sequence", "")
            if not re.fullmatch(r"[0-9]{1,4}", v) or not watermark <= int(v) <= min(sequence, 8192): errors.append("link_watermark_invalid_or_tail_missing")
            else: watermark = int(v)
        if row.get("tag") == "prns.trial" and f.get("category") in {"output_truncated", "heartbeat_limit", "heartbeat_start_failed"}: errors.append("native_output_invalid")
        if row.get("tag") != "prns.links": continue
        if f == {"stage": "probe", "category": "installed"}:
            probe += 1
            if PHONE.epoch(row["host_observed_utc"]) > start: errors.append("late_probe")
            continue
        if not valid(f): errors.append("link_record_invalid"); continue
        if int(f["sequence"]) != sequence + 1 or int(f["source_sequence"]) != sources[f["family"]] + 1: errors.append("link_sequence_gap")
        sequence = int(f["sequence"]); sources[f["family"]] = int(f["source_sequence"])
        if any(int(f.get(k, "0")) for k in ("dropped_before", "producer_dropped", "sink_dropped")) or f["phase"] in {"producer_limit", "LimitReached"}: errors.append("link_events_dropped")
        if start <= PHONE.epoch(row["host_observed_utc"]) <= end:
            events.append({"host_observed_utc": row["host_observed_utc"], "capture_sequence": row["sequence"], **f})
    if probe != 1: errors.append("link_probe_missing_or_restarted")
    return {"qualified_coverage": not errors, "problems": sorted(set(errors)), "through_capture_sequence": horizon,
            "through_native_capture_sequence": anchor["sequence"], "sink_watermark": watermark, "observed_source_sequences": sources,
            "events": events, "limits": "Observed software boundaries only; absence at RF, exact group association, and durable delivery are not established."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path); parser.add_argument("--start", required=True); parser.add_argument("--end", required=True)
    args = parser.parse_args()
    start, end = PHONE.epoch(args.start), PHONE.epoch(args.end)
    if end <= start: parser.error("end must be after start")
    rows, info = ROUTES.snapshot(args.capture)
    result = analyze(rows, start, end)
    result["input"] = info
    if info["errors"]: result["qualified_coverage"] = False
    print(json.dumps(result, indent=2))


if __name__ == "__main__": main()
