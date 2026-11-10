"""Verify the corresponding sources supplied for crates selected under MPL-2.0."""
import hashlib
from pathlib import Path
import tarfile


REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'
REVIEWS = Path('release/licenses/rust-mpl-sources.json')


def crate_source(package, cargo_home, reviews):
    name, version = package['name'], package['version']
    review = reviews.get(f'{name} {version}')
    if package.get('source') != REGISTRY or not review:
        raise ValueError(f'{name} {version}: verify and record the exact MPL source and notices')
    stem = f'{name}-{version}'
    source = Path(package['manifest_path']).resolve().parent
    registry = cargo_home.resolve() / 'registry'
    if source.parent.parent != registry / 'src' or source.name != stem:
        raise ValueError(f'{stem}: selected Rust source differs from the public registry archive')
    archive = registry / 'cache' / source.parent.name / (stem + '.crate')
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    if checksum != review['sha256']:
        raise ValueError(f'{stem}: source archive differs from the reviewed SHA-256')
    # Cargo fetched these bytes with --locked in an isolated home. Check that its
    # actual build sources match the archive offered to recipients, including notices.
    archived = set()
    with tarfile.open(archive, 'r:gz') as crate:
        for entry in crate.getmembers():
            parts = Path(entry.name).parts
            if (not parts or parts[0] != stem or '..' in parts
                    or not (entry.isfile() or entry.isdir())):
                raise ValueError(f'{stem}: unsupported source archive entry {entry.name}')
            if entry.isfile():
                path = source.joinpath(*parts[1:])
                archived.add(path)
                if path.is_symlink() or path.read_bytes() != crate.extractfile(entry).read():
                    raise ValueError(f'{stem}: local source differs from published {entry.name}')
        for member in ('Cargo.toml', review['notice']):
            if not crate.getmember(f'{stem}/{member}').isfile():
                raise ValueError(f'{stem}: missing published {member}')
    unexpected = [path for path in source.rglob('*')
                  if not path.is_dir() and path not in archived and path != source / '.cargo-ok']
    if unexpected:
        raise ValueError(f'{stem}: installed sources include unpublished files: {unexpected}')
    return (f'https://static.crates.io/crates/{name}/{stem}.crate', checksum, review['notice'])
