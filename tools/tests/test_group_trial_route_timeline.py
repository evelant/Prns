"""Prevent false correlation or timing claims from route diagnostic journals."""
from __future__ import annotations

import datetime as dt
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "device/group-trial/route_timeline.py"
SPEC = importlib.util.spec_from_file_location("group_trial_route_timeline", SCRIPT)
assert SPEC and SPEC.loader
ROUTES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ROUTES)
BASE = dt.datetime(2026, 10, 14, tzinfo=dt.timezone.utc)
NONCE = "abcdef1234567890"
DEST = "0123456789abcdef"


def stamp(second):
    return (BASE + dt.timedelta(seconds=second)).isoformat()


def journal(events, capture="session"):
    entries = [(0, {"kind": "capture_started"}), *events]
    return [{"schema": 1, "capture_id": capture, "sequence": n,
             "host_observed_utc": stamp(second), "host_monotonic_ns": int(second * 1e9), **row}
            for n, (second, row) in enumerate(entries, 1)]


def phone(phase, nonce=NONCE, destination=DEST, disposition=None):
    fields = {"stage": "path", "phase": phase, "request": nonce, "has_request": str(nonce != ROUTES.ZERO).lower(),
              "destination": destination, "has_destination": "true", "interface": DEST,
              "has_interface": "true", "kind": "Some(BluetoothPeer)"}
    if disposition:
        fields["disposition"] = disposition
    return {"kind": "event", "tag": "prns.routes", "fields": fields}


def board(phase, nonce=NONCE, destination=DEST, disposition=None, segment=0):
    value = {"kind": "route_event", "phase": phase, "request": nonce, "destination": destination,
             "interface": DEST, "board_sequence": 1, "board_observed_ms": 10, "core_ms": 10,
             "board_segment": segment}
    if disposition:
        value["disposition"] = disposition
    return value


class TimelineTests(unittest.TestCase):
    def analyze(self, values, **kwargs):
        with tempfile.TemporaryDirectory() as directory:
            paths = {}
            for label, rows in values.items():
                path = Path(directory) / (label + ".jsonl")
                path.write_text("".join(json.dumps(row) + "\n" for row in rows))
                paths[label] = path
            return ROUTES.analyze(paths, **kwargs)

    def test_shared_request_nonce_correlates_positive_phases(self):
        value = self.analyze({
            "phone": journal([(1, phone("local_request")), (2, phone("request_egress", disposition="Dispatched"))]),
            "board": journal([(3, board("request_ingress", disposition="RecursiveForward"))])}, same_host=True)
        request = value["request_threads"][0]
        self.assertEqual(request["sources_observed"], ["board", "phone"])
        self.assertEqual(request["event_ids"], ["phone:2", "phone:3", "board:2"])
        self.assertEqual([v["milliseconds"] for v in request["adjacent_host_observation_gaps_ms"]], [1000, 1000])
        self.assertEqual(value["analysis_kind"], "positive_route_observations_only")
        self.assertNotIn("passed", json.dumps(value))

    def test_response_has_only_local_destination_context_not_nonce_correlation(self):
        value = self.analyze({"phone": journal([
            (1, phone("local_request")),
            (2, phone("response_ingress", nonce=ROUTES.ZERO, disposition="Accepted")),
        ])})
        self.assertEqual(value["request_threads"][0]["event_ids"], ["phone:2"])
        context = value["local_destination_contexts"][0]
        self.assertEqual(context["event_ids"], ["phone:2", "phone:3"])
        self.assertEqual(context["response_event_ids"], ["phone:3"])
        self.assertNotIn("adjacent_host_observation_gaps_ms", value["request_threads"][0])

    def test_same_destination_token_is_never_joined_across_sources_or_boots(self):
        value = self.analyze({
            "a": journal([(1, board("local_request")), (2, board("local_request", segment=1))]),
            "b": journal([(1, board("local_request"))]),
        })
        self.assertEqual(len(value["local_destination_contexts"]), 3)

    def test_multiple_nonces_stay_distinct_with_one_local_destination(self):
        value = self.analyze({"a": journal([
            (1, phone("local_request")),
            (2, phone("local_request", nonce="1111111111111111")),
            (3, phone("response_ingress", nonce=ROUTES.ZERO, disposition="Accepted")),
        ])})
        self.assertEqual(len(value["request_threads"]), 2)
        self.assertEqual(len(value["local_destination_contexts"][0]["request_tokens"]), 2)
        for request in value["request_threads"]:
            self.assertNotIn("a:4", request["event_ids"])

    def test_zero_or_absent_request_is_not_correlated(self):
        value = self.analyze({"a": journal([(1, phone("local_request", nonce=ROUTES.ZERO))]),
                              "b": journal([(2, board("recursive_timeout", nonce=ROUTES.ZERO))])})
        self.assertEqual(value["request_threads"], [])

    def test_time_filter_preserves_whole_input_snapshot_metadata(self):
        value = self.analyze({"a": journal([(1, phone("local_request")), (3, phone("local_request"))])},
                             start=stamp(2), end=stamp(4))
        self.assertEqual(value["route_event_count"], 1)
        self.assertEqual(value["inputs"]["a"]["rows"], 3)
        self.assertEqual(value["inputs"]["a"]["errors"], [])

    def test_selected_fields_do_not_copy_unrelated_console_or_arbitrary_extra(self):
        event = phone("local_request")
        event["fields"]["private"] = "PRIVATE"
        value = self.analyze({"a": journal([(1, event), (2, {"kind": "event", "tag": "other", "raw": "PRIVATE"})])})
        self.assertNotIn("PRIVATE", json.dumps(value))
        self.assertEqual(value["route_event_count"], 1)

    def test_unknown_disposition_is_rejected_and_counted(self):
        value = self.analyze({"a": journal([(1, phone("request_ingress", disposition="PRIVATE"))])})
        self.assertEqual(value["route_event_count"], 0)
        self.assertEqual(value["inputs"]["a"]["rejected_route_rows"], 1)
        self.assertNotIn("PRIVATE", json.dumps(value))

    def test_partial_snapshot_keeps_hash_of_exact_read_and_ignores_incomplete_row(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.jsonl"
            data = b"".join((json.dumps(row) + "\n").encode() for row in journal([])) + b'{"partial":'
            path.write_bytes(data)
            rows, info = ROUTES.snapshot(path)
            self.assertEqual(len(rows), 1)
            self.assertEqual(info["sha256"], hashlib.sha256(data).hexdigest())
            self.assertTrue(info["partial_final_row_unobserved"])
            self.assertEqual(path.read_bytes(), data)

    def test_capture_sequence_or_clock_fault_stops_at_valid_prefix(self):
        for fault in ("sequence", "clock"):
            rows = journal([(1, phone("local_request")), (2, phone("local_request"))])
            if fault == "sequence":
                rows[2]["sequence"] = 10
            else:
                rows[2]["host_observed_utc"] = stamp(100)
            value = self.analyze({"a": rows})
            self.assertEqual(value["route_event_count"], 1)
            self.assertIn("invalid_journal_row", value["inputs"]["a"]["errors"])

    def test_caps_and_observation_gaps_remain_visible(self):
        value = self.analyze({"a": journal([
            (1, {"kind": "observation_gap", "reason": "board_event_gap", "previous": 1, "current": 3}),
            (2, {"kind": "board_diagnostics_limit", "limit": 2048}),
            (3, {"kind": "capture_ended", "reason": "duration_complete", "problems": ["board_event_gap"]}),
        ])})
        self.assertEqual(value["route_event_count"], 0)
        self.assertEqual(value["capture_markers"][1]["marker_reason"], "board_event_gap")
        self.assertTrue(value["inputs"]["a"]["terminal_observed"])

    def test_window_filter_keeps_prior_and_later_fault_context_distinct(self):
        value = self.analyze({"a": journal([
            (1, {"kind": "board_diagnostics_limit", "limit": 2048}),
            (2, {"kind": "boot_boundary", "source": "firmware_banner", "board_segment": 1}),
            (3, {"kind": "capture_health"}),
            (4, phone("local_request")),
            (5, {"kind": "observation_gap", "reason": "board_event_gap"}),
            (6, {"kind": "event", "tag": "prns.routes", "fields": {
                "stage": "diagnostics", "category": "line_limit"}}),
            (7, {"kind": "capture_ended", "reason": "duration_complete"}),
        ])}, start=stamp(4), end=stamp(5))
        self.assertEqual(value["route_event_count"], 1)
        self.assertEqual([(v["kind"], v["window_relation"]) for v in value["capture_markers"]], [
            ("board_diagnostics_limit", "before"), ("boot_boundary", "before"),
            ("observation_gap", "within"), ("route_diagnostics_status", "after"),
            ("capture_ended", "after"),
        ])


if __name__ == "__main__":
    unittest.main()
