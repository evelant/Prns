import hashlib
from pathlib import Path
import tarfile
import tempfile
import unittest

from validation.security import mpl_sources


class MplCrateSourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.home = Path(temporary.name)
        cache_path = self.home / 'registry/cache/test-index/example-1.0.0.crate'
        cache_path.parent.mkdir(parents=True)
        source = self.home / 'registry/src/test-index/example-1.0.0'
        source.mkdir(parents=True)
        self.notice = source / 'LICENSE'
        (source / 'Cargo.toml').write_text('[package]\nname="example"\n')
        self.notice.write_text('Original MPL notice')
        (source / 'lib.rs').write_text('pub const ANSWER: usize = 42;')
        with tarfile.open(cache_path, 'w:gz') as crate:
            for path in source.iterdir():
                crate.add(path, arcname='example-1.0.0/' + path.name)
        self.digest = hashlib.sha256(cache_path.read_bytes()).hexdigest()
        self.reviews = {'example 1.0.0': {'sha256': self.digest, 'notice': 'LICENSE'}}
        self.package = {'name': 'example', 'version': '1.0.0', 'source': mpl_sources.REGISTRY,
                        'manifest_path': str(source / 'Cargo.toml')}

    def test_source_offer_matches_reviewed_archive_and_notice(self):
        url, digest, notice = mpl_sources.crate_source(self.package, self.home, self.reviews)
        self.assertEqual(url, 'https://static.crates.io/crates/example/example-1.0.0.crate')
        self.assertEqual(digest, self.digest)
        self.assertEqual(notice, 'LICENSE')

    def test_unknown_patched_or_changed_source_fails(self):
        cases = [
            (self.package, {}),
            (dict(self.package, version='1.0.1'), self.reviews),
            (dict(self.package, source=None), self.reviews),
            (dict(self.package, source='git+https://example.org/fork'), self.reviews),
            (self.package, {'example 1.0.0': {'sha256': '0' * 64, 'notice': 'LICENSE'}}),
        ]
        for package, reviews in cases:
            with self.subTest(package=package, reviews=reviews), self.assertRaises(ValueError):
                mpl_sources.crate_source(package, self.home, reviews)
        extra = self.notice.with_name('build.rs')
        extra.write_text('fn main() {}')
        with self.assertRaisesRegex(ValueError, 'unpublished files'):
            mpl_sources.crate_source(self.package, self.home, self.reviews)
        extra.unlink()
        self.notice.write_text('locally altered notice')
        with self.assertRaisesRegex(ValueError, 'local source differs'):
            mpl_sources.crate_source(self.package, self.home, self.reviews)


if __name__ == '__main__':
    unittest.main()
