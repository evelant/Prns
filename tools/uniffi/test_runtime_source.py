import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import react_native
import runtime_notices
import runtime_source


def write_archive(path, files):
    with tarfile.open(path, 'w:gz') as archive:
        for name, contents in files.items():
            member = tarfile.TarInfo(name)
            raw = contents.encode()
            member.size = len(raw)
            archive.addfile(member, io.BytesIO(raw))


class RuntimeSourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.enterContext(patch.multiple(react_native, ROOT=self.root, HERE=self.root))
        self.patch = self.root / 'change.patch'
        path = 'node_modules/@ubjs/react-native/cpp/example.cpp'
        self.patch.write_text(f'diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n'
                              '@@ -1,1 +1,1 @@\n-original\n+modified\n')
        (self.root / 'LICENSE').write_text('MPL notice')
        package = {'name': '@ubjs/react-native', 'version': '1.2.3', 'license': 'MPL-2.0'}
        self.files = {'package.json': json.dumps(package), 'cpp/example.cpp': 'original\n'}
        self.packed = self.root / 'npm.tgz'
        write_archive(self.packed, {'package/' + name: text for name, text in self.files.items()})
        self.revision = 'a' * 40
        prefix = 'upstream-' + self.revision
        self.source = {'upstream': {'repository': 'https://github.com/org/upstream.git', 'revision': self.revision},
                       'runtime_patch': {'file': 'change.patch'}}
        self.upstream = self.root / 'upstream.tgz'
        files = {prefix + '/runtimes/jsi/' + name: text for name, text in self.files.items()}
        for crate in ('core', 'jsi'):
            files[f'{prefix}/runtimes/{crate}/src/lib.rs'] = 'pub fn bridge() {}'
            files[f'{prefix}/runtimes/{crate}/Cargo.toml'] = '[package]'
        write_archive(self.upstream, {**files, prefix + '/LICENSE': 'MPL notice'})
        self.enterContext(patch.object(runtime_source, 'upstream_archive', return_value=self.upstream))
        self.inputs = [(dict(package, license_file='LICENSE'), self.packed)]

    def test_verified_sources_and_applicable_patch(self):
        runtime_source.verify_corresponding_sources(self.source, self.inputs)

    def test_mismatched_published_source_or_notice_fails(self):
        write_archive(self.packed, {'package/' + name: ('changed' if name.endswith('.cpp') else text)
                                    for name, text in self.files.items()})
        with self.assertRaisesRegex(ValueError, 'published source differs'):
            runtime_source.verify_corresponding_sources(self.source, self.inputs)
        (self.root / 'LICENSE').write_text('different notice')
        with self.assertRaisesRegex(ValueError, 'upstream notice differs'):
            runtime_source.verify_corresponding_sources(self.source, self.inputs)

    def test_inapplicable_patch_fails_and_recipient_text_keeps_exact_patch(self):
        contents = self.patch.read_text().replace('-original\n', '-wrong\n')
        self.patch.write_text(contents)
        with self.assertRaisesRegex(ValueError, 'does not apply'):
            runtime_source.verify_corresponding_sources(self.source, self.inputs)
        report = {'packages': [dict(self.inputs[0][0], unresolved_policy=[], archive='https://registry.npmjs.org/example',
                                     sha256='a' * 64, license_notice='MPL notice')],
                  'source': dict(self.source, upstream_source_sha256='b' * 64),
                  'source_patches': [{'sha256': hashlib.sha256(contents.encode()).hexdigest(), 'patch': contents}]}
        lines = '\n'.join(runtime_notices.recipient_source_lines(report, {'@ubjs/react-native'}))
        encoded = lines.split("\n```base64\n")[1].split("\n```")[0]
        self.assertEqual(base64.b64decode(encoded).decode(), contents)
        self.assertIn(runtime_source.source_url(self.source['upstream']), lines)
        with self.assertRaisesRegex(ValueError, 'missing selected'):
            runtime_notices.recipient_source_lines(report, {'unknown'})
