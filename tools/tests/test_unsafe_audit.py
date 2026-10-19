from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "unsafe_audit", ROOT / "validation/security/unsafe-audit.py"
)
assert SPEC is not None and SPEC.loader is not None
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class UnsafeAuditTests(unittest.TestCase):
    def test_group_library_graphs_cover_their_owned_workspaces(self) -> None:
        graphs = {name: (manifest, target) for name, manifest, target in audit.GRAPHS}
        self.assertEqual(graphs["group-libraries"], ("prns-groups/Cargo.toml", "x86_64-unknown-linux-gnu"))
        self.assertNotIn("group-libraries", audit.GRAPH_FEATURES)

    def test_app_graphs_audit_the_shipped_platform_features(self) -> None:
        graphs = {name: (manifest, target) for name, manifest, target in audit.GRAPHS}
        self.assertEqual(len(graphs), len(audit.GRAPHS))
        self.assertEqual(
            audit.GRAPH_FEATURES,
            {"prns-app-android": ("android",), "prns-app-ios": ("apple",)},
        )
        manifest = "applications/prns/native-composition/Cargo.toml"
        self.assertEqual(graphs["prns-app-android"], (manifest, "aarch64-linux-android"))
        self.assertEqual(graphs["prns-app-ios"], (manifest, "aarch64-apple-ios"))
        self.assertNotIn("expo-host-linux", audit.GRAPH_FEATURES)
        self.assertNotIn("engine", audit.GRAPH_FEATURES)

    def test_metadata_keeps_defaults_and_adds_only_requested_product_features(self) -> None:
        completed = subprocess.CompletedProcess([], 0, json.dumps({"packages": []}), "")
        with mock.patch.object(audit.subprocess, "run", return_value=completed) as run:
            audit.run_metadata("example/Cargo.toml", "aarch64-linux-android", ("android",))
            command = run.call_args.args[0]
            self.assertEqual(command[-2:], ["--features", "android"])
            self.assertIn("--locked", command)
            self.assertNotIn("--all-features", command)
            self.assertNotIn("--no-default-features", command)
            audit.run_metadata("example/Cargo.toml", "aarch64-apple-darwin")
            self.assertNotIn("--features", run.call_args.args[0])

    def test_snapshot_records_product_feature_selection(self) -> None:
        with mock.patch.object(audit, "run_metadata", return_value={"packages": []}):
            snapshot = audit.build_snapshot()
        graphs = {graph["name"]: graph for graph in snapshot["graphs"]}
        self.assertEqual(graphs["prns-app-android"]["features"], ["android"])
        self.assertEqual(graphs["prns-app-ios"]["features"], ["apple"])
        self.assertNotIn("features", graphs["expo-host-linux"])


if __name__ == "__main__":
    unittest.main()
