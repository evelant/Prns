from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "mls_source_policy", ROOT / "validation/security/mls-source-policy.py"
)
assert SPEC is not None and SPEC.loader is not None
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


class MlsSourcePolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.write("deny.toml", f'[sources]\nunknown-git = "deny"\nallow-git = ["{policy.UPSTREAM}"]\n')
        self.provenance = {
            "upstream_repository": policy.UPSTREAM,
            "upstream_revision": policy.REVISION,
            "upstream_crate": "mls-rs", "upstream_version": policy.VERSION,
            "local_patch": "PRNS.patch", "patch_sha256": hashlib.sha256(b"fixture").hexdigest(),
        }
        self.write(policy.VENDOR / "PRNS.patch", "fixture")
        self.write(policy.VENDOR / "Cargo.toml", '[package]\nname = "mls-rs"\nversion = "0.56.0"\n')
        self.write(policy.VENDOR / "src/lib.rs", "// reviewed fixture source\n")
        self.provenance["vendored_files_sha256"] = {
            path.relative_to(self.root / policy.VENDOR).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in (self.root / policy.VENDOR).rglob("*") if path.is_file()
        }
        self.write(policy.VENDOR / "PROVENANCE.json", json.dumps(self.provenance))
        self.write("Cargo.toml", f'[workspace.dependencies]\nmls-rs-core = {{ git = "{policy.UPSTREAM}", rev = "{policy.REVISION}" }}\n')
        self.write("Cargo.lock", f'[[package]]\nname = "mls-rs-core"\nversion = "0.27.0"\nsource = "{policy.LOCK_SOURCE}"\n')

    def write(self, relative: str | Path, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_accepts_reviewed_pin_and_canonical_local_vendor(self) -> None:
        self.write("consumer/Cargo.toml", f'[dependencies]\nmls-rs = {{ path = "../{policy.VENDOR}" }}\nmls-rs-core.workspace = true\n')
        self.assertEqual(policy.check_policy(self.root), [])

    def test_rejects_revision_branch_tag_and_fork_changes(self) -> None:
        specs = [
            f'git = "{policy.UPSTREAM}", rev = "' + "0" * 40 + '"',
            f'git = "{policy.UPSTREAM}", branch = "main"',
            f'git = "{policy.UPSTREAM}", tag = "v0.56.0"',
            f'git = "https://github.com/other/mls-rs", rev = "{policy.REVISION}"',
        ]
        for spec in specs:
            with self.subTest(spec=spec):
                self.write("Cargo.toml", '[dependencies]\nmls-rs-core = {'+spec+'}\n')
                self.assertTrue(policy.check_policy(self.root))

    def test_rejects_registry_and_local_sibling_substitution(self) -> None:
        for spec in ['"0.27.0"', '{path = "local-core"}']:
            with self.subTest(spec=spec):
                self.write("Cargo.toml", '[dependencies]\nmls-rs-core = '+spec+'\n')
                self.assertTrue(policy.check_policy(self.root))

    def test_checks_aliases_target_dependencies_and_patches(self) -> None:
        for table in ['target.\'cfg(unix)\'.dependencies', 'patch.crates-io', 'build-dependencies']:
            with self.subTest(table=table):
                self.write("extra/Cargo.toml", f'[{table}]\nrenamed = {{ package = "mls-rs-core", git = "{policy.UPSTREAM}", branch = "main" }}\n')
                self.assertTrue(policy.check_policy(self.root))

    def test_checks_git_source_even_when_package_name_is_not_mls(self) -> None:
        self.write("extra/Cargo.toml", f'[dependencies]\nfuture_crate = {{git = "{policy.UPSTREAM}", branch = "main"}}\n')
        self.assertTrue(policy.check_policy(self.root))

    def test_checks_resolved_hash_and_requested_revision_in_every_lock(self) -> None:
        for source in [policy.LOCK_SOURCE[:-40]+"0"*40,
                       f'git+{policy.UPSTREAM}#{policy.REVISION}',
                       'registry+https://github.com/rust-lang/crates.io-index']:
            with self.subTest(source=source):
                self.write("consumer/Cargo.lock", f'[[package]]\nname = "mls-rs-core"\nversion = "0.27.0"\nsource = "{source}"\n')
                self.assertTrue(policy.check_policy(self.root))

    def test_rejects_vendor_path_substitution(self) -> None:
        self.write("consumer/Cargo.toml", '[dependencies]\nmls-rs = {path = "other-vendor"}\n')
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_changed_provenance_or_patch(self) -> None:
        self.provenance["upstream_revision"] = "0" * 40
        self.write(policy.VENDOR / "PROVENANCE.json", json.dumps(self.provenance))
        self.assertTrue(policy.check_policy(self.root))
        self.provenance["upstream_revision"] = policy.REVISION
        self.write(policy.VENDOR / "PROVENANCE.json", json.dumps(self.provenance))
        self.write(policy.VENDOR / "PRNS.patch", "changed patch")
        self.assertTrue(policy.check_policy(self.root))

    def test_ignores_only_validation_build_outputs(self) -> None:
        self.write(policy.VENDOR / "validation/target/debug/artifact", "build output")
        self.assertEqual(policy.check_policy(self.root), [])
        self.write(policy.VENDOR / "src/target/extra.rs", "unreviewed source")
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_direct_vendor_source_mutation(self) -> None:
        self.write(policy.VENDOR / "src/lib.rs", "// unreviewed change\n")
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_deleted_vendor_source(self) -> None:
        (self.root / policy.VENDOR / "src/lib.rs").unlink()
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_additional_vendor_source(self) -> None:
        self.write(policy.VENDOR / "src/extra.rs", "// unreviewed addition\n")
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_linked_vendor_files_and_directories(self) -> None:
        vendor = self.root / policy.VENDOR
        source = vendor / "src/lib.rs"
        contents = source.read_text()
        self.write("outside.rs", contents)
        source.unlink()
        source.symlink_to(self.root / "outside.rs")
        self.assertTrue(policy.check_policy(self.root))
        source.unlink()
        source.write_text(contents)
        (vendor / "linked").symlink_to(self.root, target_is_directory=True)
        self.assertTrue(policy.check_policy(self.root))

    def test_rejects_incomplete_or_escaping_vendor_inventory(self) -> None:
        original = self.provenance["vendored_files_sha256"]
        for inventory in [None, {}, {"../outside.rs": "0" * 64},
                          {"PROVENANCE.json": "0" * 64},
                          {**original, "PRNS.patch": "0" * 64}]:
            with self.subTest(inventory=inventory):
                self.provenance["vendored_files_sha256"] = inventory
                self.write(policy.VENDOR / "PROVENANCE.json", json.dumps(self.provenance))
                self.assertTrue(policy.check_policy(self.root))

    def test_rejects_missing_repository_allowlist_or_permissive_git_policy(self) -> None:
        for text in ['unknown-git = "deny"\nallow-git = []',
                     f'unknown-git = "allow"\nallow-git = ["{policy.UPSTREAM}"]']:
            with self.subTest(text=text):
                self.write("deny.toml", '[sources]\n'+text+'\n')
                self.assertTrue(policy.check_policy(self.root))

    def test_ignores_build_copies_but_does_not_skip_cargo_file_symlinks(self) -> None:
        self.write("target/Cargo.toml", "deliberately invalid TOML")
        self.assertEqual(policy.check_policy(self.root), [])
        path = self.root / "linked"
        path.mkdir()
        (path / "Cargo.toml").symlink_to(self.root / "Cargo.toml")
        self.assertTrue(policy.check_policy(self.root))


if __name__ == "__main__":
    unittest.main()
