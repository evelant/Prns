"""Regression tests for false-pass risks in physical group trial measurement."""
from __future__ import annotations

import datetime as dt
import argparse
import contextlib
import io
import importlib.util
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "device/group-trial/observer.py"
SPEC = importlib.util.spec_from_file_location("group_trial_observer", SCRIPT)
assert SPEC and SPEC.loader
OBSERVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(OBSERVER)
BASE = dt.datetime(2026, 10, 14, tzinfo=dt.timezone.utc)


def stamp(seconds):
    return (BASE + dt.timedelta(seconds=seconds)).isoformat()


def rows(*, receipt_time=30, host_gap=False, native_gap=False, session_change=False,
         cap=False, wrong_record=False, restart=False):
    entries = [(0, "capture_started", {})]
    for second in range(0, 111, 5):
        if not (host_gap and 15 <= second <= 35):
            entries.append((second, "observer_heartbeat", {"child_alive": True}))
        if not (native_gap and 15 <= second <= 35):
            entries.append((second, "event", {"tag": "prns.trial", "fields": {
                "stage": "heartbeat", "sequence": str(second // 5 + 1),
                "session": "different" if session_change and second >= 20 else "phone-session",
                "elapsed_ms": str(second * 1000), "groups_ready": "true", "bluetooth_ready": "true", "routes_ready": "true"}}))
    entries.append((12, "event", {"tag": "prns.groups", "fields": {
        "stage": "local_send_result", "category": "durable_observed", "group": "group-tag", "record": "record-tag"}}))
    if receipt_time is not None:
        entries.append((receipt_time, "event", {"tag": "prns.groups", "fields": {
            "stage": "receipt_transition", "category": "authenticated_receipt_applied",
            "group": "group-tag", "record": "unrelated" if wrong_record else "record-tag"}}))
    if cap:
        entries.append((14, "event", {"tag": "prns.groups", "fields": {
            "stage": "delivery_limit", "family": "receipt", "limit": "256"}}))
    if restart:
        entries.append((22, "capture_ended", {"returncode": 0}))
    entries.sort(key=lambda item: item[0])
    return [{"schema": 1, "capture_id": "capture", "sequence": n,
             "host_observed_utc": stamp(second), "host_monotonic_ns": int(second * 1e9),
             "kind": kind, **fields}
            for n, (second, kind, fields) in enumerate(entries, 1)]


def specification():
    return {"schema": 1, "case_id": "case-one", "start_utc": stamp(10), "deadline_utc": stamp(100),
            "max_observer_gap_seconds": 15,
            "correlation_evidence": [{"stream": "sender", "path": "binding.json", "sha256": "filled-by-fixture"}],
            "streams": {"sender": {"capture": "sender.jsonl", "session": "phone-session"}},
            "checks": [{"name": "durable receipt", "stream": "sender", "kind": "authenticated_receipt",
                        "where": {"stage": "receipt_transition", "category": "authenticated_receipt_applied",
                                  "group": "group-tag", "record": "record-tag"}}]}


class ObserverTests(unittest.TestCase):
    def evaluate(self, value=None, spec=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            data = value if value is not None else rows()
            (path / "sender.jsonl").write_text("".join(json.dumps(row) + "\n" for row in data))
            evidence = path / "ui.json"
            evidence.write_text('{"single_submission": true}')
            commit = next(row for row in data if row.get("fields", {}).get("stage") == "local_send_result")
            binding = {"case_id": "case-one", "capture_id": "capture", "session": "phone-session",
                       "group": "group-tag", "record": "record-tag", "first_capture_sequence": commit["sequence"],
                       "ui_evidence": "ui.json", "ui_evidence_sha256": hashlib.sha256(evidence.read_bytes()).hexdigest()}
            (path / "binding.json").write_text(json.dumps(binding))
            selected = spec or specification()
            if selected.get("correlation_evidence"):
                selected["correlation_evidence"][0]["sha256"] = hashlib.sha256((path / "binding.json").read_bytes()).hexdigest()
            return OBSERVER.evaluate(selected, path)

    def test_exact_native_receipt_with_continuous_capture_passes(self):
        result = self.evaluate()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["checks"][0]["first_observed_utc"], stamp(30))

    def test_complete_coverage_and_missing_receipt_is_observed_deadline_failure(self):
        self.assertEqual(self.evaluate(rows(receipt_time=None))["status"], "failed_deadline")

    def test_receipt_after_deadline_never_changes_failure_to_pass(self):
        self.assertEqual(self.evaluate(rows(receipt_time=101))["status"], "failed_deadline")

    def test_receipt_from_different_record_cannot_satisfy_case(self):
        self.assertEqual(self.evaluate(rows(wrong_record=True))["status"], "failed_deadline")

    def test_host_native_sequence_session_caps_and_restart_fail_closed(self):
        for change, problem in [("host_gap", "host_observer_coverage_gap"),
                                ("native_gap", "native_stream_coverage_gap"),
                                ("session_change", "native_session_changed"),
                                ("cap", "diagnostic_budget_exhausted"),
                                ("restart", "observer_interrupted")]:
            with self.subTest(change=change):
                result = self.evaluate(rows(**{change: True}))
                self.assertEqual(result["status"], "unqualified")
                self.assertIn(problem, result["streams"]["sender"]["problems"])

    def test_missing_observation_at_deadline_is_not_claimed_delivery_failure(self):
        early = [row for row in rows(receipt_time=None) if OBSERVER.epoch(row["host_observed_utc"]) < OBSERVER.epoch(stamp(70))]
        result = self.evaluate(early)
        self.assertEqual(result["status"], "unqualified")
        self.assertIn("host_observer_coverage_gap", result["streams"]["sender"]["problems"])

    def test_console_liveness_without_group_logger_readiness_cannot_qualify(self):
        for state in ("false", None):
            data = rows()
            heartbeat = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "20000")
            if state is None:
                heartbeat["fields"].pop("groups_ready")
            else:
                heartbeat["fields"]["groups_ready"] = state
            result = self.evaluate(data)
            self.assertEqual(result["status"], "unqualified")
            self.assertIn("native_diagnostics_not_ready", result["streams"]["sender"]["problems"])

    def test_route_enums_are_captured_but_route_caps_invalidate_required_evidence(self):
        line = "[prns.routes] stage=path kind=Some(BluetoothPeer) mode=Some(Full) disposition=Ignored(NoRoute) phase=request_ingress"
        parsed = OBSERVER.parse_safe_line(line)
        self.assertEqual(parsed["fields"]["disposition"], "Ignored(NoRoute)")
        self.assertIsNone(OBSERVER.parse_safe_line(line.replace("prns.routes", "prns.groups")))
        data = rows()
        data.append({**data[0], "kind": "event", "tag": "prns.routes", "fields": {
            "stage": "diagnostics", "category": "line_limit", "limit": "2048"},
            "host_observed_utc": stamp(14), "host_monotonic_ns": int(14e9)})
        data.sort(key=lambda row: row["host_monotonic_ns"])
        for index, row in enumerate(data, 1):
            row["sequence"] = index
        self.assertEqual(self.evaluate(data)["status"], "passed")
        spec = specification()
        spec["streams"]["sender"]["required_tags"] = ["prns.groups", "prns.trial", "prns.routes"]
        spec["streams"]["sender"]["required_readiness"] = ["groups_ready", "routes_ready"]
        self.assertEqual(self.evaluate(data, spec)["status"], "unqualified")

    def test_native_heartbeat_duplicate_is_a_sequence_fault(self):
        data = rows()
        heartbeat = next(row for row in data if row.get("fields", {}).get("elapsed_ms") == "20000")
        heartbeat["fields"]["sequence"] = "4"
        result = self.evaluate(data)
        self.assertEqual(result["status"], "unqualified")
        self.assertIn("native_heartbeat_sequence_gap", result["streams"]["sender"]["problems"])

    def test_only_explicit_independent_attempt_cap_can_qualify_delivery(self):
        for family in (None, "local_send_result", "unknown", "sync_attempt"):
            with self.subTest(family=family):
                data = rows()
                fields = {"stage": "diagnostics", "category": "correlation_limit", "limit": "128"}
                if family is not None:
                    fields["family"] = family
                data.append({**data[0], "kind": "event", "tag": "prns.groups", "fields": fields,
                             "host_observed_utc": stamp(14), "host_monotonic_ns": int(14e9)})
                data.sort(key=lambda row: row["host_monotonic_ns"])
                for index, row in enumerate(data, 1):
                    row["sequence"] = index
                result = self.evaluate(data)
                self.assertEqual(result["status"], "passed" if family == "sync_attempt" else "unqualified")
                if family == "sync_attempt":
                    self.assertEqual(result["streams"]["sender"]["diagnostic_limits"],
                                     ["sync_attempt_correlation_exhausted"])
                    spec = specification()
                    spec["streams"]["sender"]["require_sync_attempt_correlation"] = True
                    self.assertEqual(self.evaluate(data, spec)["status"], "unqualified")
                    spec = specification()
                    spec["checks"].append({"name": "attempt", "stream": "sender", "where": {
                        "stage": "sync_attempt", "phase": "start"}})
                    self.assertEqual(self.evaluate(data, spec)["status"], "unqualified")

    def test_attempt_cap_does_not_hide_a_delivery_deadline_failure(self):
        data = rows(receipt_time=None)
        row = next(row for row in data if row.get("fields", {}).get("stage") == "local_send_result")
        limit = {**row, "host_observed_utc": stamp(14), "host_monotonic_ns": int(14e9), "fields": {
            "stage": "diagnostics", "category": "correlation_limit", "family": "sync_attempt", "limit": "128"}}
        data.append(limit)
        data.sort(key=lambda row: row["host_monotonic_ns"])
        for index, row in enumerate(data, 1):
            row["sequence"] = index
        self.assertEqual(self.evaluate(data)["status"], "failed_deadline")
        limit["tag"] = "prns.bluetooth"
        self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_wall_clock_jump_invalidates_deadline_measurement(self):
        data = rows()
        data[5]["host_observed_utc"] = stamp(500)
        self.assertEqual(self.evaluate(data)["status"], "unqualified")

    def test_duplicate_receipt_observations_are_not_unique_commit_counts(self):
        data = rows()
        receipt = next(row for row in data if row.get("fields", {}).get("category") == "authenticated_receipt_applied")
        data.append({**receipt, "host_observed_utc": stamp(109), "host_monotonic_ns": int(109e9)})
        data.sort(key=lambda row: row["host_monotonic_ns"])
        for index, row in enumerate(data, 1):
            row["sequence"] = index
        result = self.evaluate(data)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["checks"][0]["matching_observations"], 1)  # Second was after deadline.

    def test_carrier_acceptance_cannot_be_used_as_authenticated_receipt(self):
        spec = specification()
        spec["checks"][0]["where"]["category"] = "accepted"
        with self.assertRaisesRegex(ValueError, "authenticated receipt"):
            self.evaluate(spec=spec)

    def test_missing_message_mapping_and_unpinned_process_fail_closed(self):
        spec = specification()
        spec["correlation_evidence"] = []
        with self.assertRaisesRegex(ValueError, "mapping"):
            self.evaluate(spec=spec)
        spec = specification()
        del spec["streams"]["sender"]["session"]
        self.assertEqual(self.evaluate(spec=spec)["status"], "unqualified")

    def test_parser_discards_unrelated_output_free_text_and_wrong_android_pid(self):
        for value in ["a private payload", "[prns.groups] stage=error message=secret text",
                      "[prns.groups] stage=error path=/private/container/key", "[prns.groups] stage=a stage=b",
                      "[prns.groups.extra] stage=a"]:
            with self.subTest(value=value):
                self.assertIsNone(OBSERVER.parse_safe_line(value))
        line = "1791937000.123  31763  31764 I prns.groups: [prns.groups] stage=push phase=finish accepted_receipts=1"
        self.assertIsNotNone(OBSERVER.parse_safe_line(line, 31763))
        self.assertIsNone(OBSERVER.parse_safe_line(line, 3971))
        self.assertIsNone(OBSERVER.parse_safe_line("[prns.groups] stage=push", 31763))

    def test_capture_writes_durable_filtered_rows_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "argv.json"
            source.write_text(json.dumps([sys.executable, "-c", "import time; print('private user content', flush=True); print('[prns.trial] stage=heartbeat session=fixture sequence=1 elapsed_ms=0', flush=True); time.sleep(0.06)"]))
            output = root / "capture.jsonl"
            argv = [sys.executable, str(SCRIPT), "capture", "--command-json", str(source),
                    "--output", str(output), "--heartbeat-seconds", "0.01", "--detach-safe"]
            result = subprocess.run(argv, text=True, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            text = output.read_text()
            self.assertNotIn("private user content", text)
            loaded, errors = OBSERVER.read_journal(output)
            self.assertEqual(errors, [])
            self.assertGreaterEqual(sum(row["kind"] == "observer_heartbeat" for row in loaded), 2)
            self.assertEqual(sum(row["kind"] == "event" for row in loaded), 1)
            again = subprocess.run(argv, text=True, capture_output=True, timeout=5)
            self.assertNotEqual(again.returncode, 0)
            self.assertEqual(output.read_text(), text)

    def test_message_binding_requires_unique_record_and_unchanged_ui_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            captured = root / "sender.jsonl"
            captured.write_text("".join(json.dumps(row) + "\n" for row in rows()))
            ui = root / "ui.json"
            ui.write_text('{"unique_rows": 1, "submissions": 1}')
            binding_path = root / "binding.json"
            args = argparse.Namespace(capture=captured, output=binding_path, case_id="case-one",
                                      kind="local-send", start_utc=stamp(10), end_utc=stamp(15),
                                      marker_sha256="f" * 64, ui_evidence=ui, group=None)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(OBSERVER.bind_message(args), 0)
            spec = specification()
            spec["correlation_evidence"][0]["sha256"] = hashlib.sha256(binding_path.read_bytes()).hexdigest()
            self.assertEqual(OBSERVER.evaluate(spec, root)["status"], "passed")
            ui.write_text('{"unique_rows": 2}')
            self.assertEqual(OBSERVER.evaluate(spec, root)["status"], "unqualified")
            changed = rows()
            sent = next(row for row in changed if row.get("fields", {}).get("stage") == "local_send_result")
            other = {**sent, "fields": {**sent["fields"], "record": "second-record"},
                     "host_observed_utc": stamp(13), "host_monotonic_ns": int(13e9)}
            changed.append(other)
            changed.sort(key=lambda row: row["host_monotonic_ns"])
            for index, row in enumerate(changed, 1):
                row["sequence"] = index
            captured.write_text("".join(json.dumps(row) + "\n" for row in changed))
            args.output = root / "ambiguous.json"
            with self.assertRaisesRegex(ValueError, "exactly one distinct"):
                OBSERVER.bind_message(args)
            self.assertFalse(args.output.exists())

    def test_live_partial_append_does_not_become_a_complete_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "capture.jsonl"
            initial = rows()[:5]
            target.write_text("".join(json.dumps(row) + "\n" for row in initial) + '{"kind": "event"')
            loaded, errors = OBSERVER.read_journal(target)
            self.assertEqual(errors, [])
            self.assertEqual(loaded, initial)

    def test_deadline_watcher_finishes_from_continuous_capture_without_agent_polling(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            producer = """import time
for n in range(40):
 print(f'[prns.trial] stage=heartbeat session=phone-session sequence={n} elapsed_ms={n*50} groups_ready=true', flush=True)
 if n == 4: print('[prns.groups] stage=local_send_result category=durable_observed group=group-tag record=record-tag', flush=True)
 if n == 16: print('[prns.groups] stage=receipt_transition category=authenticated_receipt_applied group=group-tag record=record-tag', flush=True)
 time.sleep(0.05)
"""
            command = root / "command.json"
            command.write_text(json.dumps([sys.executable, "-c", producer]))
            journal = root / "sender.jsonl"
            recorder = subprocess.Popen([sys.executable, str(SCRIPT), "capture", "--command-json", str(command),
                                         "--output", str(journal), "--heartbeat-seconds", "0.05", "--detach-safe"],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                limit = time.monotonic() + 2
                while time.monotonic() < limit:
                    captured, _ = OBSERVER.read_journal(journal)
                    commits = [row for row in captured if row.get("fields", {}).get("stage") == "local_send_result"]
                    if commits:
                        break
                    time.sleep(0.01)
                self.assertTrue(commits)
                heartbeat = next(row for row in captured if row.get("fields", {}).get("stage") == "heartbeat")
                ui = root / "ui.json"
                ui.write_text('{"unique_rows": 1, "submissions": 1}')
                binding_path = root / "binding.json"
                with contextlib.redirect_stdout(io.StringIO()):
                    OBSERVER.bind_message(argparse.Namespace(capture=journal, output=binding_path, case_id="case-one",
                                          kind="local-send", start_utc=heartbeat["host_observed_utc"],
                                          end_utc=commits[0]["host_observed_utc"], marker_sha256="f" * 64,
                                          ui_evidence=ui, group=None))
                spec = specification()
                spec["start_utc"] = heartbeat["host_observed_utc"]
                spec["deadline_utc"] = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(seconds=2)).isoformat()
                spec["max_observer_gap_seconds"] = 1
                spec["correlation_evidence"][0]["sha256"] = hashlib.sha256(binding_path.read_bytes()).hexdigest()
                spec_path = root / "case.json"
                spec_path.write_text(json.dumps(spec))
                result_path = root / "result.json"
                # No polling, UI reads, or assistant interaction once this gate starts.
                gate = subprocess.run([sys.executable, str(SCRIPT), "evaluate", "--spec", str(spec_path),
                                       "--output", str(result_path), "--watch", "--grace-seconds", "0"],
                                      text=True, capture_output=True, timeout=5)
                self.assertEqual(gate.returncode, 0, gate.stderr)
                result = json.loads(result_path.read_text())
                self.assertEqual(result["status"], "passed")
                self.assertTrue(result["finalized"])
                recorder.communicate(timeout=3)
                self.assertEqual(recorder.returncode, 0)
            finally:
                if recorder.poll() is None:
                    recorder.terminate()
                    recorder.communicate(timeout=3)

    def test_finalized_deadline_result_cannot_be_rewritten_as_late_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            spec = root / "case.json"
            spec.write_text(json.dumps(specification()))
            result = root / "result.json"
            result.write_text('{"status":"failed_deadline","finalized":true}')
            original = result.read_bytes()
            args = argparse.Namespace(spec=spec, output=result, watch=False, grace_seconds=0)
            with self.assertRaisesRegex(ValueError, "finalized gate"):
                OBSERVER.run_evaluation(args)
            self.assertEqual(result.read_bytes(), original)

    def test_capture_duration_limit_is_durable_and_stops_safe_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "argv.json"
            source.write_text(json.dumps([sys.executable, "-c", "import time; time.sleep(10)"]))
            output = root / "capture.jsonl"
            result = subprocess.run([sys.executable, str(SCRIPT), "capture", "--command-json", str(source),
                                     "--output", str(output), "--max-seconds", "0.05",
                                     "--heartbeat-seconds", "0.01", "--detach-safe"],
                                    text=True, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2, result.stderr)
            loaded, _ = OBSERVER.read_journal(output)
            self.assertTrue(any(row.get("reason") == "capture_duration_limit" for row in loaded))


if __name__ == "__main__":
    unittest.main()
