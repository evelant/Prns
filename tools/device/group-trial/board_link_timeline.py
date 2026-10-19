#!/usr/bin/env python3
"""Qualify a fixed board Link observation window and retain positive observations.

A complete software trace does not prove RF absence, peer reception, group-message
identity, or durable delivery. Counts at publication are not event-time snapshots.
"""
from __future__ import annotations
import argparse
import importlib.util
import json
from pathlib import Path
import re


def sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ROUTES = sibling("route_timeline")
# A bounded board journal can contain 16,384 enriched rows.
ROUTES.MAX_INPUT = 32 * 1024 * 1024
SCHEMA = ROUTES.BOARD.LINK
PHONE = ROUTES.PHONE
KINDS = {"link_probe", "link_health", "link_event", "link_limit"}
COUNTERS = SCHEMA.HEALTH - {"stage", "core_capped", "lora_capped", "healthy"}
LIMITS = ("Observed software boundaries only. TX completion is the local radio driver's success; "
          "RX delivery is a software ingress handoff. Neither establishes RF absence, peer reception, "
          "exact group-message association, or durable delivery. Counter snapshots can straddle events.")


def valid_record(row):
    if (row.get("trace_version") != 1 or not isinstance(row.get("boot_token"), str)
            or re.fullmatch(r"[0-9a-f]{16}", row["boot_token"]) is None): return False
    for name, maximum in (("board_sequence", 2**32-1), ("board_observed_ms", 2**64-1)):
        value = row.get(name)
        if type(value) is not int or not 0 <= value <= maximum: return False
    if row["board_sequence"] == 0: return False
    f = row.get("fields")
    if not isinstance(f, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k,v in f.items()): return False
    if row["kind"] == "link_probe":
        return (set(f) == SCHEMA.PROBE and f["stage"] == "probe" and f["category"] == "installed"
                and all(SCHEMA.unsigned(f[k]) for k in ("core_ms", "radio_before_ms", "radio_after_ms"))
                and int(f["radio_after_ms"]) >= int(f["radio_before_ms"]))
    if row["kind"] == "link_limit": return f == {"stage":"limit", "category":"health", "limit":"1800"}
    if row["kind"] == "link_health":
        return (set(f) == SCHEMA.HEALTH and f["stage"] == "health"
                and all(SCHEMA.unsigned(f[k]) for k in COUNTERS)
                and all(f[k] in {"true", "false"} for k in ("core_capped", "lora_capped", "healthy")))
    if row["kind"] != "link_event" or f.get("sequence") != str(row["board_sequence"]): return False
    return SCHEMA.LINK.valid(f) if f.get("family") == "core" else SCHEMA.valid_lora(f)


def clock_anchor_review(rows):
    """Reject an origin that places a source event after its own publication.

    Persistence can resume the core at a different logical origin. A probe of
    an obsolete copied timebase must not turn that difference into queue delay.
    The offset already includes millisecond rounding uncertainty, so only an
    event whose entire interval is in the future proves an inconsistency.
    """
    candidates = {}
    for row in rows:
        if row.get("kind") == "link_probe" and valid_record(row):
            candidates.setdefault(row["boot_token"], []).append(row["fields"])
    anchors = {boot:{"core_ms":int(values[0]["core_ms"]),
                  "radio_before_ms":int(values[0]["radio_before_ms"]),
                  "radio_after_ms":int(values[0]["radio_after_ms"]),
                  "radio_minus_core_ms":[int(values[0]["radio_before_ms"])-int(values[0]["core_ms"])-1,
                                         int(values[0]["radio_after_ms"])-int(values[0]["core_ms"])+1]}
            for boot,values in candidates.items() if len(values)==1}
    problems = []
    for row in rows:
        if row.get("kind") != "link_event" or not valid_record(row): continue
        f = row["fields"]; boot = row["boot_token"]; anchor = anchors.get(boot)
        if not anchor or f.get("family") != "core": continue
        past_fields = (["at_ms"] if f.get("has_at") == "true" else [])
        if "requested_at_ms" in f: past_fields.append("requested_at_ms")
        for key in past_fields:
            interval = [int(f[key]) + offset for offset in anchor["radio_minus_core_ms"]]
            if interval[0] > row["board_observed_ms"]:
                problems.append({"category":"clock_anchor_inconsistent", "boot_token":boot,
                                 "capture_sequence":row["sequence"], "core_field":key, "core_ms":int(f[key]),
                                 "rejected_radio_interval_ms":interval,
                                 "board_publication_ms":row["board_observed_ms"], "probe":anchor})
                # Suppress every derived conversion for this boot; retain source
                # timestamps and positive events without guessing another offset.
                del anchors[boot]
                break
    return anchors, problems


def clock_anchors(rows):
    """Consistent same-board offsets only; no cross-device clock alignment."""
    return clock_anchor_review(rows)[0]


def core_clock_intervals(row, anchors):
    f=row.get("fields",{});anchor=anchors.get(row.get("boot_token"))
    if not anchor or f.get("family")!="core":return {}
    offsets=anchor["radio_minus_core_ms"]
    return {key:[int(f[key])+value for value in offsets] for key in ("at_ms","requested_at_ms","timeout_at_ms")
            if key in f and (key!="at_ms" or f.get("has_at")=="true")}


def analyze(rows, start, end, host_gap=12.0, board_gap=6.0):
    errors, events = [], []
    anchors, clock_problems = clock_anchor_review(rows)
    # Positive evidence survives missing anchors and unqualified coverage.
    for row in rows:
        if row.get("kind") == "link_event" and valid_record(row) and start <= PHONE.epoch(row["host_observed_utc"]) <= end:
            events.append({"capture_sequence":row["sequence"], "host_observed_utc":row["host_observed_utc"],
                           "host_monotonic_ns":row["host_monotonic_ns"], "boot_token":row["boot_token"],
                           "board_observed_ms":row["board_observed_ms"], **row["fields"],
                           "core_in_radio_clock_ms":core_clock_intervals(row, anchors)})
    health = [r for r in rows if r.get("kind") == "link_health" and valid_record(r)]
    host = [r for r in rows if r.get("kind") == "capture_health"]
    board_anchor = next((r for r in health if PHONE.epoch(r["host_observed_utc"]) >= end), None)
    host_anchor = next((r for r in host if PHONE.epoch(r["host_observed_utc"]) >= end), None)
    if not board_anchor: errors.append("closing_board_heartbeat_missing")
    if not host_anchor: errors.append("closing_host_heartbeat_missing")
    first_probe_sequence = next((r["sequence"] for r in rows if r.get("kind") == "link_probe" and valid_record(r)), None)
    faults=[]
    allowed_faults={"panic_banner","panic_location","backtrace_header","stack_overflow","stack_guard","exception","double_exception","breakpoint"}
    for row in rows:
        at=PHONE.epoch(row["host_observed_utc"])
        if at>end:continue
        if row.get("kind")=="firmware_fault" and row.get("category") in allowed_faults:
            faults.append({"capture_sequence":row["sequence"],"host_observed_utc":row["host_observed_utc"],
                           "category":row["category"],"window_relation":"before" if at<start else "within"})
            if first_probe_sequence is not None and row["sequence"]>first_probe_sequence:errors.append("firmware_fault_observed")
        if row.get("kind")=="firmware_fault_limit":errors.append("firmware_fault_budget_exhausted")
    result = {"qualified_coverage":False, "problems":sorted(set(errors)), "events":events,
              "clock_anchors":anchors, "clock_conversion_qualified":bool(anchors) and not clock_problems,
              "clock_problems":clock_problems,
              "clock_limits":"Clock consistency is separate from capture coverage. An inconsistent anchor suppresses all conversions for that boot; no replacement offset is inferred. A consistent anchor alone does not verify which runtime clock was sampled.",
              "firmware_faults":faults, "limits":LIMITS}
    if not board_anchor or not host_anchor: return result
    horizon = max(board_anchor["sequence"], host_anchor["sequence"])
    prefix = [r for r in rows if r["sequence"] <= horizon]
    frames = [r for r in prefix if r.get("kind") in KINDS and r["sequence"] <= board_anchor["sequence"]]
    valid_host = [r for r in host if r["sequence"] <= host_anchor["sequence"] and r.get("read_loop_alive") is True and r.get("port_open") is True]
    if not PHONE.continuous(valid_host, start, end, host_gap): errors.append("host_coverage_gap")
    if not PHONE.continuous([r for r in health if r["sequence"] <= board_anchor["sequence"]], start, end, board_gap): errors.append("board_coverage_gap")
    if not rows or rows[0].get("kind") != "capture_started" or rows[0].get("link_heartbeat_expected") is not True:
        errors.append("link_capture_contract_missing")
    probe = 0; sequence = 0; previous_ms = 0; boot = None
    sources = {"core":0, "lora":0}; published = {"core":0, "lora":0}
    prior_health = {k:0 for k in COUNTERS}; prior_lora = {k:0 for k in SCHEMA.LORA_COUNTERS}
    observed_health = None
    for row in frames:
        if not valid_record(row): errors.append("link_record_invalid"); continue
        if row["board_sequence"] != sequence + 1: errors.append("link_sink_sequence_gap")
        sequence = row["board_sequence"]
        if boot is not None and row["boot_token"] != boot: errors.append("link_boot_changed")
        boot = row["boot_token"]
        if row["board_observed_ms"] < previous_ms: errors.append("link_board_clock_regressed")
        previous_ms = row["board_observed_ms"]
        f = row["fields"]
        if row["kind"] == "link_probe":
            probe += 1
            if sequence != 1 or PHONE.epoch(row["host_observed_utc"]) > start: errors.append("link_probe_late")
        elif row["kind"] == "link_limit": errors.append("link_health_budget_exhausted")
        elif row["kind"] == "link_event":
            family = f["family"]
            if int(f["source_sequence"]) != sources[family] + 1: errors.append("link_source_sequence_gap")
            sources[family] = int(f["source_sequence"]); published[family] += 1
            if sources[family] > (4096 if family == "core" else 8192): errors.append("link_producer_budget_exhausted")
            if f["phase"] in {"producer_limit", "LimitReached"} or any(int(f.get(k,"0")) for k in ("dropped_before", "producer_dropped", "sink_dropped")):
                errors.append("link_events_dropped")
            if family == "lora":
                for key in SCHEMA.LORA_COUNTERS:
                    value = int(f[key])
                    if value < prior_lora[key]: errors.append("lora_counter_regressed")
                    prior_lora[key] = value
        elif row["kind"] == "link_health":
            current = {k:int(f[k]) for k in COUNTERS}
            if any(current[k] < prior_health[k] for k in COUNTERS): errors.append("link_health_counter_regressed")
            prior_health = current
            if f["core_capped"] != "false" or f["lora_capped"] != "false" or current["sink_dropped"]:
                errors.append("link_events_dropped")
            for family in sources:
                seen, admitted, emitted = (current[family+suffix] for suffix in ("_seen", "_admitted", "_published"))
                if emitted != published[family] or not emitted <= admitted <= seen:
                    errors.append("link_health_counter_mismatch")
                if seen < sources[family]: errors.append("link_health_source_watermark_regressed")
            pending = any(current[family+"_admitted"] != current[family+"_published"] for family in sources)
            if f["healthy"] != "true" and not pending: errors.append("link_health_unhealthy_without_pending_output")
            # Pending items may reconcile at a later health record. The closing
            # watermark must account for every callback through the interval.
            observed_health = f
    if probe != 1: errors.append("link_probe_missing_or_restarted")
    if observed_health is None or observed_health["healthy"] != "true": errors.append("closing_link_health_unhealthy")
    else:
        for family in sources:
            if not all(int(observed_health[family+suffix]) == sources[family] == published[family]
                       for suffix in ("_seen", "_admitted", "_published")):
                errors.append("closing_link_watermark_pending_or_missing")
    first_probe = next((r["sequence"] for r in frames if r.get("kind") == "link_probe"), None)
    for row in prefix:
        kind = row.get("kind")
        if kind in {"capture_ended", "observer_fault"}: errors.append("capture_interrupted")
        if kind == "firmware_fault_limit": errors.append("firmware_fault_budget_exhausted")
        if kind == "firmware_fault" and first_probe is not None and row["sequence"] > first_probe: errors.append("firmware_fault_observed")
        if kind == "boot_boundary" and first_probe is not None and row["sequence"] > first_probe: errors.append("board_restarted_after_probe")
        if kind == "observation_gap" and row.get("reason", "").startswith("link_"): errors.append("link_observation_gap")
        if kind in {"capture_health", "capture_ended"}:
            if any(row.get(k,0) != 0 for k in ("rejected_link_lines", "undecodable_lines", "oversized_lines")):
                errors.append("capture_discarded_link_evidence")
    result.update(qualified_coverage=not errors, problems=sorted(set(errors)),
                  through_capture_sequence=horizon, through_board_capture_sequence=board_anchor["sequence"],
                  observed_source_sequences=sources, board_boot_token=boot, board_sink_sequence=sequence)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path); parser.add_argument("--start", required=True); parser.add_argument("--end", required=True)
    args = parser.parse_args(); start, end = PHONE.epoch(args.start), PHONE.epoch(args.end)
    if end <= start: parser.error("end must be after start")
    rows, info = ROUTES.snapshot(args.capture)
    result = analyze(rows,start,end); result["input"] = info
    if info["errors"]: result["qualified_coverage"] = False
    print(json.dumps(result,indent=2))


if __name__ == "__main__": main()
