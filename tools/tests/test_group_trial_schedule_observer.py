"""Independent retry-schedule evidence must fail closed without changing delivery."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.tests import test_group_trial_observer as base

OBSERVER = base.OBSERVER
GROUP, PEER = "1111111111111111", "2222222222222222"


def order(data):
    data.sort(key=lambda row: row["host_monotonic_ns"])
    for index, row in enumerate(data, 1):
        row["sequence"] = index
    return data


def event(second, tag, fields):
    return {"schema": 1, "capture_id": "capture", "kind": "event", "tag": tag, "fields": fields,
            "host_observed_utc": base.stamp(second), "host_monotonic_ns": int(second * 1e9)}


def records(**options):
    data = base.rows(**options)
    for row in data:
        fields = row.get("fields", {})
        if "group" in fields:
            fields["group"] = GROUP
        if fields.get("stage") == "heartbeat":
            seconds = int(fields["elapsed_ms"]) // 1000
            fields.update(schedule_ready="true", schedule_sequence=str(sum(seconds >= t for t in (6, 14, 19))))
    data.append(event(0, "prns.schedule", {"stage": "logger", "category": "schedule_probe"}))
    for sequence, (second, category) in enumerate(((6, "completed"), (14, "fresh_work"), (19, "admitted")), 1):
        data.append(event(second, "prns.schedule", {
            "stage": "schedule", "category": category, "sequence": str(sequence), "group": GROUP, "peer": PEER,
            "activity": "failed" if sequence == 1 else "none", "due_in_ms": "120000" if sequence == 1 else "0",
            "floor_in_ms": "0", "failures": "4", "pending": "unknown" if sequence == 1 else "true",
            "fresh": "true", "repair": "false", "accelerated": "true" if sequence == 2 else "false"}))
    return order(data)


def specification():
    spec = base.specification()
    spec["gate_kind"] = "schedule"
    spec["checks"] = [{"name": "fresh work shortened retry", "stream": "sender", "where": {
        "stage": "schedule", "category": "fresh_work", "group": GROUP, "peer": PEER, "accelerated": "true"}},
        {"name": "attempt admitted", "stream": "sender", "where": {
            "stage": "schedule", "category": "admitted", "group": GROUP, "peer": PEER}}]
    return spec


class ScheduleObserverTests(unittest.TestCase):
    def evaluate(self, data=None, spec=None, *, changed_ui=False, binding_group=GROUP):
        data = records() if data is None else order(data)
        spec = specification() if spec is None else spec
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "sender.jsonl").write_text("".join(json.dumps(row) + "\n" for row in data))
            ui = root / "ui.json"
            ui.write_text('{"single_submission":true}')
            commit = next(row for row in data if row.get("fields", {}).get("stage") == "local_send_result")
            binding = {"case_id": "case-one", "capture_id": "capture", "session": "phone-session",
                       "group": binding_group, "record": "record-tag", "first_capture_sequence": commit["sequence"],
                       "ui_evidence": "ui.json", "ui_evidence_sha256": hashlib.sha256(ui.read_bytes()).hexdigest()}
            path = root / "binding.json"
            path.write_text(json.dumps(binding))
            if spec.get("correlation_evidence"):
                spec["correlation_evidence"][0]["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            if changed_ui:
                ui.write_text("changed")
            return OBSERVER.evaluate(spec, root)

    def test_schedule_has_independent_complete_gate_and_host_measurements(self):
        result = self.evaluate()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["gate_kind"], "schedule")
        self.assertEqual(result["checks"][1]["host_seconds_after_start"], 9)
        self.assertEqual(result["streams"]["sender"]["schedule_watermark"], 3)

    def recovery_records(self):
        data = records()
        for row in data:
            fields = row.get("fields", {})
            if fields.get("stage") == "schedule" and int(fields["sequence"]) >= 2:
                fields["sequence"] = str(int(fields["sequence"]) + 2)
            if fields.get("stage") == "heartbeat" and int(fields["elapsed_ms"]) >= 15000:
                fields["schedule_sequence"] = str(int(fields["schedule_sequence"]) + 2)
        data.extend(event(second, "prns.schedule", {"stage": "recovery", "category": category,
                    "sequence": str(sequence), "group": GROUP, "peer": PEER})
                    for second, sequence, category in ((12, 2, "offered"), (13, 3, "peer_active")))
        return order(data)

    def test_raw_hint_disposition_shares_complete_ordering_without_implying_acceleration(self):
        data = self.recovery_records()
        result = self.evaluate(data)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["streams"]["sender"]["schedule_watermark"], 5)
        spec = specification()
        spec["checks"].append({"name": "raw hint ignored while peer active", "stream": "sender", "where": {
            "stage": "recovery", "category": "peer_active", "group": GROUP, "peer": PEER}})
        self.assertEqual(self.evaluate(data, spec)["status"], "passed")
        spec["checks"][-1]["where"]["category"] = "consumed_accelerated"
        self.assertEqual(self.evaluate(data, spec)["status"], "failed_deadline")

    def test_missing_raw_hint_is_a_schedule_coverage_gap(self):
        data = [row for row in self.recovery_records() if row.get("fields", {}).get("category") != "offered"]
        result = self.evaluate(data)
        self.assertEqual(result["status"], "unqualified")
        self.assertIn("schedule_sequence_gap", result["streams"]["sender"]["problems"])

    def test_recovery_schema_rejects_unknown_private_and_fake_schedule_fields(self):
        for key, value in (("body", "private"), ("category", "mystery"), ("due_in_ms", "0"),
                           ("sequence", "1025"), ("peer", "identity")):
            with self.subTest(key=key):
                data = self.recovery_records()
                row = next(row for row in data if row.get("fields", {}).get("category") == "offered")
                row["fields"][key] = value
                self.assertEqual(self.evaluate(data)["status"], "unqualified")
        spec = specification()
        spec["checks"][0]["where"].update(stage="recovery", category="offered")
        with self.assertRaises(ValueError):  # Retains accelerated=true, which raw hints never claim.
            self.evaluate(self.recovery_records(), spec)

    def test_missing_or_late_decision_is_failure_only_with_complete_coverage(self):
        data = records()
        row = next(row for row in data if row.get("fields", {}).get("category") == "admitted")
        row["fields"]["category"] = "completed"
        self.assertEqual(self.evaluate(data)["status"], "failed_deadline")
        spec = specification()
        spec["checks"][1]["max_after_start_seconds"] = 8
        self.assertEqual(self.evaluate(spec=spec)["status"], "failed_deadline")

    def test_missing_tail_is_unqualified_even_without_a_later_schedule_event(self):
        data = [row for row in records() if row.get("fields", {}).get("category") != "admitted"]
        result = self.evaluate(data)
        self.assertEqual(result["status"], "unqualified")
        self.assertIn("schedule_tail_missing", result["streams"]["sender"]["problems"])

    def test_interior_loss_duplicate_and_reorder_fail_closed(self):
        for mode in ("loss", "duplicate", "reorder"):
            with self.subTest(mode=mode):
                data = records()
                schedule = [row for row in data if row.get("fields", {}).get("stage") == "schedule"]
                if mode == "loss":
                    data.remove(schedule[0])
                elif mode == "duplicate":
                    schedule[1]["fields"]["sequence"] = "1"
                else:
                    schedule[0]["fields"]["sequence"], schedule[1]["fields"]["sequence"] = "2", "1"
                self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_probe_and_schedule_readiness_required_without_weakening_old_delivery(self):
        for mode in ("probe", "missing_ready", "false_ready", "missing_watermark"):
            with self.subTest(mode=mode):
                data = records()
                if mode == "probe":
                    data = [row for row in data if row.get("fields", {}).get("category") != "schedule_probe"]
                else:
                    row = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "15000")
                    if mode == "false_ready":
                        row["fields"]["schedule_ready"] = "false"
                    else:
                        row["fields"].pop("schedule_ready" if mode == "missing_ready" else "schedule_sequence")
                self.assertEqual(self.evaluate(data)["status"], "unqualified")
        self.assertEqual(base.ObserverTests().evaluate()["status"], "passed")

    def test_watermark_regression_invalid_value_and_unhealthy_lost_cap_fail_closed(self):
        for mode in ("regression", "invalid", "unhealthy"):
            data = records()
            row = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "20000")
            if mode == "unhealthy":
                row["fields"]["schedule_ready"] = "false"  # Cap marker itself was lost.
            else:
                row["fields"]["schedule_sequence"] = "0" if mode == "regression" else "1025"
            self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_output_before_an_older_heartbeat_snapshot_is_safe(self):
        data = records()
        row = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "20000")
        row["fields"]["schedule_sequence"] = "2"
        self.assertEqual(self.evaluate(data)["status"], "passed")

    def test_later_exhaustion_cannot_change_an_already_complete_window(self):
        data = records()
        data.append(event(22, "prns.schedule", {"stage": "diagnostics", "category": "schedule_limit", "family": "schedule"}))
        for row in data:
            if row.get("fields", {}).get("stage") == "heartbeat" and int(row["fields"]["elapsed_ms"]) >= 25000:
                row["fields"]["schedule_ready"] = "false"
        self.assertEqual(self.evaluate(data)["status"], "passed")

    def test_every_schedule_cap_unavailable_truncation_and_unknown_marker_invalidates(self):
        for category in ("schedule_limit", "schedule_line_limit", "schedule_unavailable", "output_truncated", "correlation_limit", "future_limit"):
            with self.subTest(category=category):
                data = records()
                data.append(event(5, "prns.schedule", {"stage": "diagnostics", "category": category, "family": "schedule"}))
                self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_independent_delivery_attempt_route_ble_caps_do_not_claim_schedule_loss(self):
        for tag, category in (("prns.groups", "delivery_line_limit"), ("prns.groups", "correlation_limit"),
                              ("prns.routes", "line_limit"), ("prns.bluetooth", "family_limit")):
            data = records()
            data.append(event(14, tag, {"stage": "diagnostics", "category": category}))
            result = self.evaluate(data)
            self.assertEqual(result["status"], "passed")
            self.assertIn(category, result["streams"]["sender"]["independent_limits"])

    def test_schedule_cap_does_not_invalidate_an_existing_delivery_gate(self):
        data = base.rows()
        data.append(event(14, "prns.schedule", {"stage": "diagnostics", "category": "schedule_limit", "family": "schedule"}))
        self.assertEqual(base.ObserverTests().evaluate(order(data))["status"], "passed")

    def test_host_native_session_heartbeat_and_restart_fail_closed(self):
        for change in ("host_gap", "native_gap", "session_change", "restart"):
            with self.subTest(change=change):
                data = records(**{change: True})
                if change == "restart":
                    row = next(row for row in data if row.get("kind") == "capture_ended")
                    row.update(host_observed_utc=base.stamp(18), host_monotonic_ns=int(18e9))
                self.assertEqual(self.evaluate(data)["status"], "unqualified")
        data = records()
        row = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "15000")
        row["fields"]["sequence"] = "2"
        self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_changed_mapping_evidence_wrong_group_peer_and_unpinned_session(self):
        self.assertEqual(self.evaluate(changed_ui=True)["status"], "unqualified")
        self.assertEqual(self.evaluate(binding_group="3333333333333333")["status"], "unqualified")
        spec = specification()
        spec["checks"][0]["where"]["peer"] = "3333333333333333"
        self.assertEqual(self.evaluate(spec=spec)["status"], "failed_deadline")
        spec = specification()
        spec["streams"]["sender"].pop("session")
        self.assertEqual(self.evaluate(spec=spec)["status"], "unqualified")

    def test_closed_schedule_record_schema_rejects_extra_private_or_malformed_fields(self):
        for key, value in (("body", "private"), ("pending", "perhaps"), ("due_in_ms", "-1"), ("peer", "identity"), ("failures", str(2**65))):
            with self.subTest(key=key):
                data = records()
                row = next(row for row in data if row.get("fields", {}).get("category") == "fresh_work")
                row["fields"][key] = value
                self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_schedule_check_cannot_be_used_for_delivery_or_unbounded_targets(self):
        for mode in ("record", "category", "bound", "missing_mapping", "unknown_gate"):
            spec = specification()
            if mode == "record":
                spec["checks"][0]["where"]["record"] = "record-tag"
            elif mode == "category":
                spec["checks"][0]["where"]["category"] = "authenticated_receipt_applied"
            elif mode == "bound":
                spec["checks"][0]["max_after_start_seconds"] = 10000
            elif mode == "unknown_gate":
                spec["gate_kind"] = "typo"
            else:
                spec["correlation_evidence"] = []
            with self.assertRaises(ValueError):
                self.evaluate(spec=spec)

    def test_schedule_tag_supported_on_ios_and_pinned_android_only(self):
        line = "[prns.schedule] stage=logger category=schedule_probe"
        self.assertEqual(OBSERVER.parse_safe_line(line)["tag"], "prns.schedule")
        android = "1791937000.123  31763  31764 I prns.schedule: " + line
        self.assertIsNotNone(OBSERVER.parse_safe_line(android, 31763))
        self.assertIsNone(OBSERVER.parse_safe_line(android, 3971))
        self.assertIsNone(OBSERVER.parse_safe_line(line, 31763))


if __name__ == "__main__":
    unittest.main()
