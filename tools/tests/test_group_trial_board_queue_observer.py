"""Closed-schema, independently bounded positive evidence of LoRa queue drops."""
from tools.tests import test_group_trial_board_observer as base
import unittest

BOARD, Recorder, line = base.BOARD, base.Recorder, base.line


def queue_line(seq=1, observed=100, source="runtime_lane", reason="expired", age="Some(120001)"):
    return (f"INFO - prns.lora.queue seq={seq} observed_ms={observed} phase=drop "
            f"source={source} reason={reason} age_ms={age}")


class BoardQueueObserverTests(unittest.TestCase):
    def test_closed_schema_retains_only_reason_source_and_age(self):
        for source in ("runtime_lane", "driver_queue", "active"):
            for reason in ("expired", "disabled", "pre_resume", "missing_time", "clock_rollback"):
                with self.subTest(source=source, reason=reason):
                    event = BOARD.parse_safe_line(queue_line(source=source, reason=reason))
                    self.assertEqual(event, {"kind": "queue_event", "board_sequence": 1,
                        "board_observed_ms": 100, "phase": "drop", "source": source,
                        "reason": reason, "age_ms": 120001})
        self.assertIsNone(BOARD.parse_safe_line(queue_line(age="None"))["age_ms"])

    def test_unknown_values_contents_and_integer_overflow_are_rejected(self):
        for value in (queue_line(source="PRIVATE"), queue_line(reason="PRIVATE"),
                      queue_line() + " payload=PRIVATE", queue_line(seq=513),
                      queue_line(seq=0), queue_line(observed=1 << 64),
                      queue_line(age=f"Some({1 << 64})"), queue_line(age="-1"),
                      queue_line().replace("phase=drop", "phase=sent"),
                      "PRIVATE " + queue_line(), queue_line() + queue_line()):
            self.assertIsNone(BOARD.parse_safe_line(value))

    def test_queue_cap_and_sequences_are_independent_of_route_events(self):
        recorder = Recorder()
        observation = BOARD.Observation(recorder)
        for value in (line(), queue_line(), line(seq=2), queue_line(seq=2)):
            observation.accept(BOARD.parse_safe_line(value))
        self.assertFalse(observation.problems)
        cap = BOARD.parse_safe_line("INFO - prns.lora.queue limit_reached=512")
        self.assertEqual(cap, {"kind": "queue_diagnostics_limit", "limit": 512})
        observation.accept(cap)
        self.assertEqual(observation.problems, {"queue_diagnostic_budget_exhausted"})
        self.assertIsNone(BOARD.parse_safe_line("INFO - prns.lora.queue limit_reached=512 PRIVATE"))

    def test_gaps_clock_regression_and_explicit_boot_are_visible(self):
        recorder = Recorder()
        observation = BOARD.Observation(recorder)
        for seq, now in ((3, 100), (5, 200), (1, 10)):
            observation.accept(BOARD.parse_safe_line(queue_line(seq=seq, observed=now)))
        self.assertTrue({"queue_events_before_first_observation", "queue_event_gap",
                         "queue_sequence_regressed", "queue_clock_regressed"} <= observation.problems)
        observation.accept({"kind": "boot_boundary", "source": "rom_banner"})
        before = len([row for row in recorder.rows if row["kind"] == "observation_gap"])
        observation.accept(BOARD.parse_safe_line(queue_line(seq=1, observed=1)))
        self.assertEqual(before, len([row for row in recorder.rows if row["kind"] == "observation_gap"]))

    def test_capture_persists_positive_drops_and_counts_rejections_without_raw_text(self):
        harness = base.BoardObserverTests()
        _, rows, text, _ = harness.run_capture(seconds=4, chunks=[
            (queue_line() + "\n").encode(),
            (queue_line() + " PRIVATE\n").encode(),
        ])
        self.assertNotIn("PRIVATE", text)
        self.assertEqual(sum(row["kind"] == "queue_event" for row in rows), 1)
        self.assertEqual(rows[-1]["rejected_queue_lines"], 1)
        self.assertIn("rejected_queue_lines", rows[-1]["problems"])
        self.assertFalse(rows[-1]["silence_proves_no_event"])

    def assert_one_inferred_boot(self, queue_first):
        recorder = Recorder()
        observation = BOARD.Observation(recorder)
        for value in (line(), queue_line(), line(seq=2), queue_line(seq=2)):
            observation.accept(BOARD.parse_safe_line(value))
        restarted = [queue_line(seq=1, observed=1), line(seq=1, observed=1)]
        if not queue_first:
            restarted.reverse()
        for value in restarted:
            observation.accept(BOARD.parse_safe_line(value))
        boundaries = [row for row in recorder.rows if row["kind"] == "boot_boundary"]
        self.assertEqual(len(boundaries), 1)
        self.assertEqual(boundaries[0]["source"], "inferred_queue_sequence_restart" if queue_first else "inferred_sequence_restart")
        current = [row for row in recorder.rows if row["kind"] in ("queue_event", "route_event")][-2:]
        self.assertEqual([row["board_segment"] for row in current], [1, 1])
        # Subsequent observations in both independent families share the segment
        # and do not compare against the other family's pre-restart baseline.
        before = len(recorder.rows)
        for value in (line(seq=2, observed=2), queue_line(seq=2, observed=2)):
            observation.accept(BOARD.parse_safe_line(value))
        self.assertTrue(all(row["kind"] != "observation_gap" for row in recorder.rows[before:]))

    def test_inferred_route_restart_resets_queue_baseline(self):
        self.assert_one_inferred_boot(queue_first=False)

    def test_inferred_queue_restart_resets_route_baseline(self):
        self.assert_one_inferred_boot(queue_first=True)


if __name__ == "__main__":
    unittest.main()
