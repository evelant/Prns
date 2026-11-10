"""Check source availability for the exact published UniFFI npm runtimes."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.request

import react_native


def source_url(upstream):
    repo = upstream['repository']
    match = re.fullmatch(r'https://github\.com/([\w.-]+/[\w.-]+)\.git', repo)
    if not match or not re.fullmatch(r'[a-f0-9]{40}', upstream['revision']):
        raise ValueError('expected an immutable upstream GitHub source revision')
    return f"https://codeload.github.com/{match[1]}/tar.gz/{upstream['revision']}"


def upstream_archive(source):
    directory = react_native.cache_directory() / 'npm'
    directory.mkdir(parents=True, exist_ok=True)
    digest = source['upstream_source_sha256']
    if not re.fullmatch(r'[a-f0-9]{64}', digest):
        raise ValueError('invalid upstream source SHA-256')
    archive = directory / f'upstream-{digest}.tar.gz'
    if not archive.exists():
        with urllib.request.urlopen(source_url(source['upstream']), timeout=60) as response:
            contents = response.read()
        if hashlib.sha256(contents).hexdigest() != digest:
            raise ValueError('downloaded upstream source differs from reviewed SHA-256')
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as tmp:
            tmp.write(contents)
        Path(tmp.name).replace(archive)
    if react_native.digest(archive) != digest:
        raise ValueError('cached upstream source differs from reviewed SHA-256')
    return archive


def source_bytes(archive, name):
    member = archive.getmember(name)
    if not member.isfile():
        raise ValueError(f'expected an ordinary source file at {name}')
    return archive.extractfile(member).read()


def verify_corresponding_sources(source, archives):
    upstream = source['upstream']
    repository = upstream['repository'].removesuffix('.git')
    prefix = repository.rsplit('/', 1)[1] + '-' + upstream['revision']
    patch = react_native.ROOT / source['runtime_patch']['file']
    with tarfile.open(upstream_archive(source), 'r:gz') as originals:
        for package, path in archives:
            root = {'@ubjs/core': 'typescript', '@ubjs/react-native': 'runtimes/jsi'}[package['name']]
            with tarfile.open(path, 'r:gz') as packed:
                manifest = json.loads(source_bytes(originals, f'{prefix}/{root}/package.json'))
                if any(package[key] != manifest[key] for key in ('name', 'version', 'license')):
                    raise ValueError(f"{package['name']}: upstream source metadata differs")
                if source_bytes(originals, prefix + '/LICENSE') != (react_native.HERE / package['license_file']).read_bytes():
                    raise ValueError(f"{package['name']}: upstream notice differs")
                matches = 0
                for entry in packed.getmembers():
                    relative = entry.name.removeprefix('package/')
                    name = Path(relative)
                    if not entry.isfile() or relative == 'package.json' or 'dist' in name.parts or 'prebuilt' in name.parts:
                        continue
                    if source_bytes(packed, entry.name) != source_bytes(originals, f'{prefix}/{root}/{relative}'):
                        raise ValueError(f"{package['name']}: published source differs at {relative}")
                    matches += 1
                if not matches:
                    raise ValueError(f"{package['name']}: no matching published source")
                if package['name'] == '@ubjs/react-native':
                    # The registry package includes compiled Rust libraries for the native bridges.
                    # The files needed to edit these binaries are in the upstream repository.
                    for crate in ('core', 'jsi'):
                        for member in ('Cargo.toml', 'src/lib.rs'):
                            if not source_bytes(originals, f'{prefix}/runtimes/{crate}/{member}'):
                                raise ValueError(f'missing native runtime source: {crate}/{member}')
                    verify_runtime_patch(packed, patch)


def verify_runtime_patch(archive, patch):
    """Check the patch against the actual registry inputs, including XCFramework metadata."""
    prefix = 'node_modules/@ubjs/react-native/'
    paths = sorted(set(react_native.patch_paths(patch.read_text())))
    with tempfile.TemporaryDirectory(prefix='prns-runtime-source-') as tmp:
        for name in paths:
            if not name.startswith(prefix):
                raise ValueError('unexpected runtime patch target')
            destination = Path(tmp) / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source_bytes(archive, 'package/' + name.removeprefix(prefix)))
        result = subprocess.run(['git', 'apply', '--check', '--', str(patch)], cwd=tmp, capture_output=True)
        if result.returncode:
            raise ValueError('runtime source patch does not apply to the verified npm archive')
