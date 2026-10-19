from pathlib import Path
import json
import subprocess
import tempfile
import unittest
from unittest import mock

import working_tree
from test_run import mobility


class WorkingTreeTests(unittest.TestCase):
    def test_tilde_versions_are_not_filesystem_paths(self):
        self.assertIsNone(mobility.npm_local_path('~57.0.24'))
        self.assertEqual(mobility.npm_local_path('~/local'), '~/local')

    def test_sdk_exception_is_exact_and_identity_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app, sdk = root / 'applications', root / 'prns-react-native'
            app.mkdir(); sdk.mkdir()
            (sdk / 'package.json').write_text('{"name":"personal-rns-expo"}')
            self.assertTrue(mobility.shared_sdk_package(sdk, app, 'personal-rns-expo'))
            self.assertFalse(mobility.shared_sdk_package(sdk, app, 'unrelated'))
            self.assertFalse(mobility.shared_sdk_package(sdk / 'src', app))
            (sdk / 'package.json').write_text('{"name":"unreviewed"}')
            self.assertFalse(mobility.shared_sdk_package(sdk, app))

    def test_recorded_metadata_must_exist_in_actual_pinned_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(mobility.QualificationFailure, 'recorded Prns revision lacks SDK metadata'):
                mobility.stage_sdk_metadata(root / 'recorded', root / 'export')

    def test_current_snapshot_keeps_dirty_and_new_source_but_excludes_builds(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'repository'; root.mkdir()
            (root / 'applications').mkdir()
            old = root / 'applications/source.txt'; old.write_text('old')
            subprocess.run(['git', 'init', '--quiet', root], check=True)
            subprocess.run(['git', '-C', root, 'add', '.'], check=True)
            subprocess.run(['git', '-C', root, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                            'commit', '--quiet', '-m', 'fixture'], check=True)
            old.write_text('current')
            (root / 'applications/new.txt').write_text('new')
            (root / 'applications/build').mkdir()
            (root / 'applications/build/cache').write_text('excluded')
            (root / 'unrelated').mkdir(); (root / 'unrelated/private').write_text('excluded')
            work = Path(temporary) / 'output'; work.mkdir()
            exported, receipt = working_tree.snapshot(root, work, {'sourceRoots': ['applications']})
            self.assertEqual((exported / 'applications/source.txt').read_text(), 'current')
            self.assertTrue((exported / 'applications/new.txt').is_file())
            self.assertFalse((exported / 'applications/build').exists())
            self.assertFalse((exported / 'unrelated').exists())
            self.assertFalse(receipt['releaseQualified'])
            self.assertEqual(receipt['kind'], 'working-tree-source')
            self.assertEqual(len(receipt['snapshotSha256']), 64)

    def test_snapshot_keeps_exact_vendor_build_sources_and_excludes_generated_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'repository'; root.mkdir()
            included = {
                'vendor/object-0.39.1/src/build/module.rs',
                'vendor/object-0.39.1/tests/build/module.rs',
            }
            excluded = {
                'applications/build/cache',
                'vendor/other/src/build/module.rs',
                'vendor/object-0.39.1/build/cache',
                'vendor/object-0.39.1/src/build/target/cache',
                'vendor/object-0.39.1/src/build/build/cache',
            }
            for relative in included | excluded:
                path = root / relative; path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative)
            subprocess.run(['git', 'init', '--quiet', root], check=True)
            subprocess.run(['git', '-C', root, 'add', '-f', '.'], check=True)
            subprocess.run(['git', '-C', root, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                            'commit', '--quiet', '-m', 'fixture'], check=True)
            work = Path(temporary) / 'output'; work.mkdir()
            export, receipt = working_tree.snapshot(root, work, {'sourceRoots': [
                'applications', 'vendor/object-0.39.1', 'vendor/other']})
            self.assertEqual(set(receipt['files']), included)
            for relative in included:
                self.assertEqual((export / relative).read_text(), relative)
            for relative in excluded:
                self.assertFalse((export / relative).exists())

    def test_current_snapshot_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'applications').mkdir()
            (root / 'applications/escape').symlink_to('/etc/hosts')
            with mock.patch.object(working_tree.subprocess, 'run', return_value=subprocess.CompletedProcess([],0,b'applications/escape\0')):
                with self.assertRaisesRegex(ValueError, 'escapes|symlink'):
                    working_tree.inventory(root, {'sourceRoots': ['applications']})

    def test_cargo_optional_platform_dependency_cannot_escape_policy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            apps = root / 'applications'; apps.mkdir()
            other = root / 'unreviewed'; other.mkdir()
            (apps / 'Cargo.toml').write_text('[package]\nname="app"\nversion="0.1.0"\n[target.x.dependencies]\nevil={path="../unreviewed",optional=true}\n')
            (other / 'Cargo.toml').write_text('[package]\nname="evil"\nversion="0.1.0"\n')
            with self.assertRaisesRegex(ValueError, 'escapes source snapshot'):
                working_tree.validate_dependencies(root, {'sourceRoots':['applications'],
                    'cargoEntrypoints':['applications/Cargo.toml'],'npmPackages':{}}, mobility)

    def test_root_local_patch_requires_exact_reviewed_source_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            apps = root / 'applications'; apps.mkdir()
            vendor = root / 'vendor/object-0.39.1'; vendor.mkdir(parents=True)
            (apps / 'Cargo.toml').write_text('[package]\nname="app"\nversion="0.1.0"\n')
            (vendor / 'Cargo.toml').write_text('[package]\nname="object"\nversion="0.39.1"\n')
            (root / 'Cargo.toml').write_text('[workspace]\nmembers=["unrelated-firmware"]\n'
                '[patch.crates-io]\nobject={path="vendor/object-0.39.1"}\n')
            policy = {'sourceRoots': ['Cargo.toml', 'applications'],
                      'cargoEntrypoints': ['applications/Cargo.toml'], 'npmPackages': {}}
            with self.assertRaisesRegex(ValueError, 'Cargo dependency object escapes source snapshot'):
                working_tree.validate_dependencies(root, policy, mobility)
            policy['sourceRoots'].append('vendor/object-0.39.1')
            packages = working_tree.validate_dependencies(root, policy, mobility)
            self.assertEqual(packages['object'], 'vendor/object-0.39.1/Cargo.toml')
            self.assertNotIn('unrelated-firmware', packages)
            # Reviewing one vendor does not implicitly admit other vendor roots.
            other = root / 'vendor/unreviewed'; other.mkdir()
            (other / 'Cargo.toml').write_text('[package]\nname="other"\nversion="0.1.0"\n')
            (vendor / 'Cargo.toml').write_text('[package]\nname="object"\nversion="0.39.1"\n'
                '[dependencies]\nother={path="../unreviewed"}\n')
            with self.assertRaisesRegex(ValueError, 'Cargo dependency other escapes source snapshot'):
                working_tree.validate_dependencies(root, policy, mobility)

    def test_entrypoint_patch_alias_is_identity_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            apps = root / 'applications'; apps.mkdir()
            vendor = root / 'vendor/object-0.39.1'; vendor.mkdir(parents=True)
            (vendor / 'Cargo.toml').write_text('[package]\nname="object"\nversion="0.39.1"\n')
            (apps / 'Cargo.toml').write_text('[package]\nname="app"\nversion="0.1.0"\n'
                '[patch.crates-io]\naliased={package="object",path="../vendor/object-0.39.1"}\n')
            policy = {'sourceRoots': ['applications', 'vendor/object-0.39.1'],
                      'cargoEntrypoints': ['applications/Cargo.toml'], 'npmPackages': {}}
            self.assertIn('object', working_tree.validate_dependencies(root, policy, mobility))
            (vendor / 'Cargo.toml').write_text('[package]\nname="substitute"\nversion="0.39.1"\n')
            with self.assertRaisesRegex(ValueError, 'Cargo dependency identity mismatch: aliased'):
                working_tree.validate_dependencies(root, policy, mobility)

    def test_sdk_jni_core_dependencies_use_same_recorded_git_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            app = root / 'applications'; app.mkdir()
            core = root / 'personal-rns'; core.mkdir()
            jni = root / 'prns-react-native/platform/android'; jni.mkdir(parents=True)
            (core / 'Cargo.toml').write_text('[package]\nname="personal-rns"\nversion="0.0.0"\n')
            (jni / 'Cargo.toml').write_text('[package]\nname="prns-expo-android"\nversion="0.0.0"\n[dependencies]\npersonal-rns={path="../../../personal-rns"}\n')
            (app / 'Cargo.toml').write_text('[package]\nname="app"\nversion="0.0.0"\n[dependencies]\nprns-expo-android={path="../prns-react-native/platform/android"}\npersonal-rns={path="../personal-rns"}\n')
            compatibility = {"prns": {"rustPackages": {"direct": ["personal-rns"], "source": [
                {"name": "personal-rns", "path": "personal-rns", "version": "0.0.0"}]}}}
            rewrites = mobility.cargo_rewrite_plan(app, root, compatibility)
            self.assertEqual(len(rewrites), 2)
            mobility.rewrite_cargo_dependencies(app, rewrites, 'file:///recorded', '1' * 40)
            mobility.reject_external_cargo_paths(app)
            self.assertIn('git = "file:///recorded"', (jni / 'Cargo.toml').read_text())
            self.assertIn('path="../prns-react-native/platform/android"', (app / 'Cargo.toml').read_text())

    def test_checked_policy_covers_facade_native_android_jni_and_identities(self):
        policy = working_tree.load_policy(mobility.REPOSITORY_ROOT)
        packages = working_tree.validate_dependencies(mobility.REPOSITORY_ROOT, policy, mobility)
        for name in ('prns-host-uniffi','prns-host-native','prns-expo-android','prns-app-native'):
            self.assertIn(name, packages)
        for component in ('core', 'codec', 'runtime', 'native'):
            with self.subTest(component=component):
                self.assertEqual(packages[f'prns-identities-{component}'],
                                 f'prns-identities/{component}/Cargo.toml')

    def test_checked_policy_rejects_missing_identity_source_root(self):
        policy = working_tree.load_policy(mobility.REPOSITORY_ROOT)
        policy['sourceRoots'].remove('prns-identities')
        with self.assertRaisesRegex(ValueError, r'Cargo dependency prns-identities-.* escapes source snapshot'):
            working_tree.validate_dependencies(mobility.REPOSITORY_ROOT, policy, mobility)

    def test_checked_policy_covers_group_native_dependency_closure(self):
        policy = working_tree.load_policy(mobility.REPOSITORY_ROOT)
        packages = working_tree.validate_dependencies(mobility.REPOSITORY_ROOT, policy, mobility)
        for name, manifest in (
            ('prns-groups-runner-native', 'prns-groups/runner/native/Cargo.toml'),
            ('prns-groups-reticulum', 'prns-groups/transport/reticulum/Cargo.toml'),
            ('prns-groups-service', 'prns-groups/service/Cargo.toml'),
        ):
            with self.subTest(package=name):
                self.assertEqual(packages[name], manifest)

    def test_checked_policy_rejects_missing_group_source_root(self):
        policy = working_tree.load_policy(mobility.REPOSITORY_ROOT)
        policy['sourceRoots'].remove('prns-groups')
        with self.assertRaisesRegex(ValueError, r'Cargo dependency prns-groups.* escapes source snapshot'):
            working_tree.validate_dependencies(mobility.REPOSITORY_ROOT, policy, mobility)

    def test_workspace_projection_preserves_inherited_policy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.toml').write_text('''[workspace]
resolver = "2"
members = [
    "personal-rns",
    "unrelated-firmware",
]
default-members = [
    "personal-rns",
    "unrelated-firmware",
]
[workspace.package]
version = "0.3.7"
[workspace.lints.rust]
unsafe_code = "forbid"
[profile.release]
lto = true
[patch.crates-io]
object = { path = "vendor/object-0.39.1" }
''')
            receipt = {}
            working_tree.project_workspace(root, {'personal-rns': 'personal-rns/Cargo.toml'}, receipt)
            result = working_tree.tomllib.loads((root / 'Cargo.toml').read_text())
            self.assertEqual(result['workspace']['members'], ['personal-rns'])
            self.assertEqual(result['workspace']['default-members'], ['personal-rns'])
            self.assertEqual(result['workspace']['lints']['rust']['unsafe_code'], 'forbid')
            self.assertTrue(result['profile']['release']['lto'])
            self.assertEqual(result['patch']['crates-io']['object'], {'path': 'vendor/object-0.39.1'})
            self.assertEqual(receipt['workspaceProjection']['omitted']['members'], ['unrelated-firmware'])
            self.assertNotEqual(receipt['workspaceProjection']['sourceSha256'], receipt['workspaceProjection']['projectedSha256'])

    def test_current_artifact_policy_selects_staged_aggregate_without_exporting_builds(self):
        policy = working_tree.load_policy(mobility.REPOSITORY_ROOT)
        self.assertEqual(policy['npmPackages']['personal-rns-expo'], 'prns-react-native')
        self.assertEqual(policy['npmArtifacts']['personal-rns-expo'], 'applications/target/react-native-sdk')
        self.assertNotIn('applications/target/react-native-sdk', policy['sourceRoots'])
