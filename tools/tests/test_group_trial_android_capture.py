"""The optional Android startup replay stays bounded, PID-pinned and filtered."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).resolve().parents[1] / "device/group-trial/observer.py"
SPEC = importlib.util.spec_from_file_location("group_trial_android_capture", MODULE_PATH)
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)


class AndroidCaptureArguments(unittest.TestCase):
    def args(self, *extra):
        return observer.parser().parse_args([
            "capture", "--android", "test-device", "--pid", "1234",
            "--output", "unused.jsonl", *extra,
        ])

    def test_default_preserves_one_line_replay(self):
        command = observer.android_capture_command(self.args())
        self.assertEqual(command[command.index("-T") + 1], "1")

    def test_replay_accepts_only_bounded_integer_counts(self):
        for count in (1, 5000, 10000):
            with self.subTest(count=count):
                self.assertEqual(self.args("--android-replay-lines", str(count)).android_replay_lines, count)
        for count in ("0", "-1", "10001", "1.5", "unbounded"):
            with self.subTest(count=count), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    self.args("--android-replay-lines", count)
                self.assertEqual(error.exception.code, 2)

    def test_replay_keeps_exact_pid_and_six_tag_filters(self):
        self.assertEqual(observer.android_capture_command(self.args("--android-replay-lines", "5000")), [
            "adb", "-s", "test-device", "logcat", "-v", "epoch", "-T", "5000",
            "--pid", "1234", "prns.groups:I", "prns.bluetooth:I", "prns.trial:I",
            "prns.routes:I", "prns.schedule:I", "prns.links:I", "*:S",
        ])

    def test_direct_call_cannot_bypass_replay_bounds(self):
        args = self.args()
        args.android_replay_lines = 10001
        with self.assertRaises(observer.argparse.ArgumentTypeError):
            observer.android_capture_command(args)

    def test_missing_pid_is_rejected_before_output_or_process(self):
        args = self.args("--android-replay-lines", "5000")
        args.pid = None
        with mock.patch.object(observer, "Journal") as journal, mock.patch.object(observer.subprocess, "Popen") as process:
            with self.assertRaisesRegex(ValueError, "freshly verified app --pid"):
                observer.capture(args)
            journal.assert_not_called()
            process.assert_not_called()

    def test_replay_does_not_weaken_parser_pid_and_tag_filter(self):
        self.assertIsNone(observer.parse_safe_line("123.0 4321 20 I prns.schedule: stage=logger category=schedule_probe", 1234))
        self.assertIsNone(observer.parse_safe_line("123.0 1234 20 I unrelated: stage=logger category=schedule_probe", 1234))
        self.assertIsNotNone(observer.parse_safe_line("123.0 1234 20 I prns.schedule: stage=logger category=schedule_probe", 1234))


if __name__ == "__main__":
    unittest.main()
