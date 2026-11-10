"""Verify Android runtime custody, apply the app-specific review, and package its notices."""
from __future__ import annotations
import argparse
from collections import defaultdict
import hashlib
import gzip
import io
import importlib.util
import json
import os
import re
import shutil
import subprocess
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

BASELINE = 'applications/tools/native-bindings/android-maven-baseline.json'
EVIDENCE = 'release/licenses/app-maven/pom-license-evidence.json'
ARTIFACTS = 'release/licenses/app-maven/artifact-notices.json'
REVIEW = 'release/licenses/app-maven/release-review.json'
ORIGINS = 'release/licenses/app-maven/artifact-origins.json'
ASSET = 'applications/prns/app/assets/licenses/android-maven.txt'
RELEASE = 'releaseRuntimeClasspath'
MAX_NOTICE_BYTES = 4 * 1024 * 1024


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'Duplicate Maven evidence key: {key}')
        result[key] = value
    return result


def _source(root: Path, source: dict) -> bytes:
    directory = (root / EVIDENCE).parent
    path = directory / source['path']
    if not path.resolve().is_relative_to(directory.resolve()):
        raise ValueError('Maven notice source escapes its evidence directory')
    packed = path.read_bytes()
    if len(packed) > MAX_NOTICE_BYTES or hashlib.sha256(packed).hexdigest() != source['file_sha256']:
        raise ValueError('Maven notice source file changed or exceeds its bound')
    if source['encoding'] == 'gzip':
        with gzip.GzipFile(fileobj=io.BytesIO(packed)) as stream:
            raw = stream.read(MAX_NOTICE_BYTES + 1)
    elif source['encoding'] == 'plain':
        raw = packed
    else:
        raise ValueError('Unknown Maven notice encoding')
    if (len(raw) > MAX_NOTICE_BYTES or len(raw) != source['bytes']
            or hashlib.sha256(raw).hexdigest() != source['sha256']):
        raise ValueError('Maven notice source bytes changed')
    return raw


def notice_ranges(index: bytes, contents: bytes) -> list[tuple[str, str]]:
    """Read the archive's byte offsets, without treating component labels as licenses."""
    entries = json.loads(index, object_pairs_hook=_unique_object)
    if not isinstance(entries, dict):
        raise ValueError('Maven notice index must be an object')
    result, intervals = [], []
    for name, item in sorted(entries.items()):
        if not name or not isinstance(item, dict) or set(item) != {'start', 'length'}:
            raise ValueError('Malformed Maven notice range')
        start, length = item['start'], item['length']
        if (type(start) is not int or type(length) is not int or start < 0
                or length <= 0 or start + length > len(contents)):
            raise ValueError('Maven notice range lies outside source bytes')
        intervals.append((start, start + length))
        result.append((name, contents[start:start + length].decode('utf-8')))
    ordered = sorted(intervals)
    if any(left[1] > right[0] for left, right in zip(ordered, ordered[1:])):
        raise ValueError('Maven notice ranges overlap')
    if not entries and contents.strip():
        raise ValueError('Maven notice text has no declared ranges')
    return result


def packaged_notices(root: Path, baseline: dict, documents: dict) -> dict:
    directory = (root / EVIDENCE).parent
    manifest = json.loads((root / ARTIFACTS).read_text(), object_pairs_hook=_unique_object)
    if manifest.get('schema') != 1:
        raise ValueError('Unsupported Maven artifact notice schema')
    bound = {}
    for kind in ('resolution', 'pom_origins'):
        item = manifest[kind]
        path = directory / item['path']
        if not path.resolve().is_relative_to(directory.resolve()):
            raise ValueError('Maven custody source escapes its evidence directory')
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != item['sha256']:
            raise ValueError(f'Maven {kind} custody source changed')
        bound[kind] = json.loads(raw, object_pairs_hook=_unique_object)
    resolution, origins = bound['resolution'], bound['pom_origins']
    if resolution['configurations'] != baseline:
        raise ValueError('Maven artifact resolution differs from selected baseline')
    if (origins.get('schema') != 1 or origins['pom_evidence_sha256']
            != hashlib.sha256((root / EVIDENCE).read_bytes()).hexdigest()):
        raise ValueError('Maven origin evidence does not bind the preserved POMs')
    pom_origins = {}
    for origin in origins['documents']:
        coordinate = origin['coordinate']
        if coordinate in pom_origins or coordinate not in documents:
            raise ValueError('Duplicate or foreign Maven POM origin')
        pom_origins[coordinate] = origin
        document = documents[coordinate]
        if origin['status'] == 'exact-pom-match':
            verified = origin['verified']
            group, artifact, version = coordinate.split(':')
            suffix = '/'.join((group.replace('.', '/'), artifact, version, f'{artifact}-{version}.pom'))
            allowed = {host + suffix for host in (
                'https://dl.google.com/dl/android/maven2/', 'https://repo.maven.apache.org/maven2/')}
            if (verified['url'] not in allowed or verified['resolved_url'] not in allowed
                    or verified['sha256'] != document['pom_sha256']):
                raise ValueError('Maven POM origin coordinate or bytes disagree')
        elif origin['status'] == 'existing-integrity-bound-npm-origin':
            if (origin['source'] != document['source']
                    or origin['pom_sha256'] != document['pom_sha256']):
                raise ValueError('Maven npm origin disagrees with preserved evidence')
        else:
            raise ValueError('Unverified Maven POM origin')
    if set(pom_origins) != set(documents):
        raise ValueError('Maven POM origin coverage is incomplete')
    expected = {}
    for profile, artifacts in resolution['artifacts'].items():
        if profile not in baseline:
            raise ValueError('Maven artifact has an unselected profile')
        seen = set()
        for artifact in artifacts:
            key = artifact['coordinate'], artifact['file_name'], artifact['sha256']
            if key in seen or key[0] not in baseline[profile]:
                raise ValueError('Duplicate or foreign resolved Maven artifact')
            seen.add(key)
            expected.setdefault(key, set()).add(profile)
    if set(resolution['artifacts']) != set(baseline):
        raise ValueError('Missing selected Maven artifact profile')
    custody = json.loads((root / ORIGINS).read_text(), object_pairs_hook=_unique_object)
    if custody.get('schema') != 1:
        raise ValueError('Unsupported Maven binary origin schema')
    origins = {}
    for origin in custody['artifacts']:
        key = origin['coordinate'], origin['file_name'], origin['sha256']
        if key in origins or key not in expected:
            raise ValueError('Maven binary origin differs from the resolved artifact')
        origins[key] = origin
    if set(origins) != set(expected):
        raise ValueError('Maven binary origin coverage is incomplete')
    seen, notices = set(), []
    for artifact in manifest['artifacts']:
        key = artifact['coordinate'], artifact['file_name'], artifact['sha256']
        if key in seen or key not in expected or set(artifact['profiles']) != expected[key]:
            raise ValueError('Maven notice artifact differs from resolved custody')
        seen.add(key)
        origin = origins[key]
        if artifact['source_observation'] == 'integrity-checked npm archive':
            source = documents[key[0]]['source']
            group, artifact_name, version = key[0].split(':')
            expected_member = '/'.join(('package/local-maven-repo', group.replace('.', '/'),
                                        artifact_name, version, key[1]))
            if (source['kind'] != 'npm-local-maven-repository'
                    or artifact['npm_integrity'] != source['integrity']
                    or artifact['npm_resolved'] != source['resolved']
                    or origin.get('url') != source['resolved']
                    or origin.get('member') != artifact['npm_member']
                    or artifact['npm_member'] != expected_member):
                raise ValueError('Maven packaged notice npm custody disagrees')
        elif artifact['source_observation'] != 'existing Gradle cache':
            raise ValueError('Unknown Maven artifact observation')
        else:
            parent = pom_origins[key[0]]['verified']['url'].rsplit('/', 1)[0]
            filename = key[1]
            if 'module_file' in origin:
                module_hash = origin['module_sha256']
                if len(module_hash) != 64 or any(c not in '0123456789abcdef' for c in module_hash):
                    raise ValueError('Invalid Maven module digest')
                raw = (directory / 'artifact-notice-sources' / f'{module_hash}.module').read_bytes()
                if hashlib.sha256(raw).hexdigest() != module_hash:
                    raise ValueError('Maven module metadata differs from its observed digest')
                files = [f for v in json.loads(raw)['variants'] for f in v.get('files', [])]
                filename = origin['module_file']
                if Path(filename).name != filename or not any(f['name'] == key[1] and f['url'] == filename
                        and f.get('sha256') == key[2] for f in files):
                    raise ValueError('Published Maven variant does not bind the resolved artifact')
            if origin['url'] != f'{parent}/{filename}' or 'member' in origin:
                raise ValueError('Maven binary origin must match the official coordinate and filename')
            if 'resolved_url' in origin:
                expected_url = origin['url'].replace('https://repo.maven.apache.org/', 'https://repo.reactnative.dev/')
                if not key[0].startswith('com.facebook.react:react-android:') or origin['resolved_url'] != expected_url:
                    raise ValueError('Unrecognized Maven binary redirect')
        sources = {}
        for item in artifact['notices']:
            path = item['archive_path']
            if path in sources:
                raise ValueError('Duplicate Maven archive notice path')
            sources[path] = _source(root, item['source'])
        pairs = []
        if 'third_party_licenses.json' in sources:
            if 'third_party_licenses.txt' not in sources:
                raise ValueError('Maven notice range index is missing its text')
            pairs.extend(notice_ranges(sources.pop('third_party_licenses.json'),
                                       sources.pop('third_party_licenses.txt')))
        for path, raw in sorted(sources.items()):
            if raw.strip():
                pairs.append((path, raw.decode('utf-8')))
        for name, text in pairs:
            notices.append({'coordinate': key[0], 'artifact': key[1], 'name': name,
                            'text': text, 'profiles': sorted(expected[key])})
    if seen != set(expected):
        raise ValueError('Maven artifact notice coverage is incomplete')
    return {'artifacts': len(seen), 'pom_origins': len(pom_origins),
            'release_artifacts': [dict(item, origin=origins[(item['coordinate'], item['file_name'], item['sha256'])]['url'])
                                  for item in resolution['artifacts'][RELEASE]],
            'notices': sorted(notices, key=lambda item: (item['coordinate'], item['artifact'], item['name']))}


def inputs(root: Path) -> tuple[Path, ...]:
    return (root / BASELINE, root / '.gitattributes', root / 'validation/security/app_maven.py', root / 'applications/tools/native-bindings/android_maven.py',
            *(p for p in sorted((root/'release/licenses/app-maven').rglob('*')) if p.is_file()))


def report(root: Path) -> dict:
    spec = importlib.util.spec_from_file_location('app_android_maven', root/'applications/tools/native-bindings/android_maven.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    baseline = module.coordinates(json.loads((root/BASELINE).read_text()))
    union = sorted(set().union(*baseline.values()))
    evidence = json.loads((root/EVIDENCE).read_text())
    if evidence.get('schema_version') != 1 or evidence.get('runtime_coordinates') != union:
        raise ValueError('App Maven POM evidence differs from the resolved runtime baseline')
    documents = evidence['documents']
    for coordinate, item in documents.items():
        path = (root/EVIDENCE).parent / item['source']['evidence_copy']
        if not path.resolve().is_relative_to((root/EVIDENCE).parent.resolve()):
            raise ValueError('Maven evidence path escapes its directory')
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != item['pom_sha256']:
            raise ValueError(f'Maven POM evidence changed: {coordinate}')
        xml = ET.fromstring(raw)
        parent = xml.find('{*}parent')
        parent_parts = [parent.findtext('{*}'+key, default='').strip() if parent is not None else ''
                        for key in ('groupId', 'artifactId', 'version')]
        parent_coordinate = ':'.join(parent_parts) if parent is not None else None
        own_parts = [(xml.findtext('{*}groupId') or parent_parts[0]).strip(),
                     (xml.findtext('{*}artifactId') or '').strip(),
                     (xml.findtext('{*}version') or parent_parts[2]).strip()]
        if not all(own_parts) or ':'.join(own_parts) != coordinate or item['coordinate'] != coordinate:
            raise ValueError(f'Maven coordinate disagrees with preserved POM: {coordinate}')
        if parent_coordinate != item['parent']:
            raise ValueError(f'Maven parent disagrees with preserved POM: {coordinate}')
        licenses = [{key: value for key in ('name', 'url', 'distribution', 'comments')
                     if (value := node.findtext(f'{{*}}{key}')) is not None}
                    for node in xml.findall('{*}licenses/{*}license')]
        if licenses != item['declared_licenses']:
            raise ValueError(f'Maven license declarations disagree with preserved POM: {coordinate}')
    rows = []
    for coordinate in union:
        current, visited = coordinate, set()
        while True:
            if current in visited or current not in documents:
                raise ValueError(f'Missing/cyclic Maven license parent: {coordinate}')
            visited.add(current)
            item = documents[current]
            if item['declared_licenses']:
                break
            current = item['parent']
        if documents[coordinate]['nearest_license_declarations_at'] != current:
            raise ValueError(f'Maven license inheritance changed: {coordinate}')
        rows.append({'coordinate':coordinate,'declarations':item['declared_licenses'],
                     'declared_at':current,'pom_sha256':documents[coordinate]['pom_sha256'],
                     'provenance':documents[coordinate]['source']})
    attribution = packaged_notices(root, baseline, documents)
    reviewed = release_review(root, baseline[RELEASE], rows, attribution)
    return {'schema': 3, 'configurations': baseline, 'packages': rows,
            'artifact_evidence': {key: attribution[key] for key in ('artifacts', 'pom_origins')},
            'notices': attribution['notices'], 'release_review': reviewed}


def declared_decision(row: dict) -> str | None:
    names = [term['name'] for term in row['declarations']]
    if row['coordinate'] == 'net.java.dev.jna:jna:5.18.1' and set(names) == {'LGPL-2.1-or-later', 'Apache-2.0'}:
        return 'JNA / Apache-2.0'
    if names == ['ML Kit Terms of Service']:
        return 'Google ML Kit'
    if names == ['Android Software Development Kit License']:
        return 'Google Android SDK client terms'
    if row['coordinate'] == 'androidx.camera:camera-core:1.6.0' and names == [
            'The Apache Software License, Version 2.0', 'BSD-3-Clause']:
        return 'Apache-2.0 AND BSD-3-Clause'
    if row['coordinate'] == 'com.parse.bolts:bolts-tasks:1.4.0' and names == ['BSD License']:
        return 'BSD-3-Clause'
    if len(names) == 1:
        name = names[0].lower()
        if name in {'mit', 'mit license', 'the mit license'}:
            return 'MIT'
        if name in {'apache-2', 'apache-2.0', 'apache 2.0', 'apache license, version 2.0',
                    'the apache license, version 2.0', 'the apache software license, version 2.0'}:
            return 'Apache-2.0'
    return None


def release_review(root: Path, release: list[str], rows: list[dict], attribution: dict) -> dict:
    policy = json.loads((root / REVIEW).read_text(), object_pairs_hook=_unique_object)
    if policy.get('schema') != 1 or policy.get('profile') != RELEASE:
        raise ValueError('Unsupported Maven release review')
    artifacts = attribution['release_artifacts']
    packages = [row for row in rows if row['coordinate'] in release]
    # Bind source and license declarations, including inherited declarations, not just group names.
    closure = {'artifacts': artifacts, 'declarations': [
        [r['coordinate'], r['pom_sha256'], r['declared_at'], r['declarations']] for r in packages]}
    digest = hashlib.sha256(json.dumps(closure, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    issues = []
    if digest != policy['release_inputs_sha256']:
        issues.append('The selected release artifacts, origins or POM declarations have changed.')
    by_coordinate = defaultdict(list)
    for artifact in artifacts:
        by_coordinate[artifact['coordinate']].append(artifact)
    decisions = []
    for row in packages:
        key = row['coordinate']
        decision = policy['coordinates'].get(key)
        if not by_coordinate[key]:
            classification = 'resolution-metadata'
        else:
            classification = declared_decision(row)
            if classification is None:
                classification = 'unreviewed'
                issues.append(f'{key}: no reviewed release license declaration')
        if decision != classification:
            issues.append(f'{key}: release decision does not match the artifact and POM declarations')
        decisions.append({'coordinate': key, 'decision': classification, 'artifacts': by_coordinate[key]})
    if set(policy['coordinates']) != set(release):
        issues.append('Recorded Maven review coordinates differ from the release profile.')
    if any(row['decision'] not in policy['decisions'] for row in decisions):
        issues.append('A release coordinate has no matching review basis.')
    for row in decisions:
        description = policy['decisions'].get(row['decision'], {})
        if description.get('status') == 'pending':
            issues.append(f"{row['coordinate']}: {description['basis']}")
        elif description.get('status') != 'reviewed':
            issues.append(f"{row['coordinate']}: unrecognized release decision")
    jna = 'net.java.dev.jna:jna:5.18.1'
    if jna in release and not any(item['coordinate'] == jna and RELEASE in item['profiles']
            and 'SPDX-License-Identifier: Apache-2.0 OR LGPL-2.1-or-later' in item['text']
            for item in attribution['notices']):
        issues.append(f'{jna}: archive is missing its explicit dual license')
    supplement_digest = hashlib.sha256(json.dumps(policy['supplemental_notices'], sort_keys=True,
                                                  separators=(',', ':')).encode()).hexdigest()
    if supplement_digest != policy['supplemental_notices_sha256']:
        raise ValueError('Maven supplemental attribution inventory changed')
    supplements = []
    for item in policy['supplemental_notices']:
        text = _source(root, item['source']).decode('utf-8')
        if not item['coordinates'] or not set(item['coordinates']) <= set(by_coordinate):
            raise ValueError('Maven supplemental notice has unknown coordinates')
        supplements.append({'name': item['name'], 'coordinates': item['coordinates'], 'url': item['url'], 'text': text})
    return {'profile': RELEASE, 'inputs_sha256': digest, 'decisions': decisions,
            'bases': policy['decisions'], 'supplemental_notices': supplements, 'issues': issues}


def audit(report: dict) -> None:
    review = report['release_review']
    if not review['decisions']:
        raise ValueError('Missing Android Maven release decisions')
    if review['issues']:
        raise ValueError('Android Maven release review pending:\n' + '\n'.join(review['issues']))


def notice_bytes(report: dict) -> bytes:
    review = report['release_review']
    lines = ['PRNS Chat for Android', 'Third-party Android runtime notices', '']
    for item in review['decisions']:
        if item['artifacts']:
            lines.append(f"{item['coordinate']} — {item['decision']}")
    lines.append('')
    for key, decision in review['bases'].items():
        if key != 'resolution-metadata':
            lines.extend([key, decision['basis'], *decision['sources'], ''])
    grouped = defaultdict(set)
    for item in report['notices']:
        if RELEASE in item['profiles']:
            grouped[item['text'].strip()].add(f"{item['coordinate']} / {item['name']}")
    for item in review['supplemental_notices']:
        grouped[item['text'].strip()].add(f"{', '.join(item['coordinates'])} / {item['name']} / {item['url']}")
    for text, owners in sorted(grouped.items(), key=lambda item: sorted(item[1])):
        lines.extend([*sorted(owners), '', text, ''])
    # Keep original custody bytes in the evidence store. Render stable LF text for Android.
    normalized = (line.rstrip() for block in lines for line in (block.splitlines() or ['']))
    return ('\n'.join(normalized).rstrip() + '\n').encode('utf-8')


def apk_raw_resources(apk: Path) -> dict[str, str]:
    # R8/AAPT can shorten physical entries to res/XX.txt. The raw resource
    # table preserves the names Metro and Expo use to load the notice offline.
    binary = shutil.which('aapt2')
    if not binary:
        sdk = os.environ.get('ANDROID_HOME') or os.environ.get('ANDROID_SDK_ROOT')
        builds = [] if not sdk else [p for p in (Path(sdk) / 'build-tools').glob('*/aapt2')
                                      if re.fullmatch(r'\d+\.\d+\.\d+', p.parent.name)]
        if not builds:
            raise ValueError('Android build-tools aapt2 is required to read the packaged resource table')
        binary = max(builds, key=lambda p: tuple(map(int, p.parent.name.split('.'))))
    table = subprocess.check_output([binary, 'dump', 'resources', apk], text=True)
    return dict(re.findall(
        r'^\s*resource 0x[0-9a-f]+ raw/(\w+)\s*\n\s*\(\) \(file\) (res/\S+)\s*$',
        table, re.MULTILINE))


def verify_apk(apk: Path, expected: bytes) -> dict:
    digest = hashlib.sha256(expected).hexdigest()
    with zipfile.ZipFile(apk) as archive:
        raw = apk_raw_resources(apk) if 'resources.arsc' in archive.namelist() else None
        seen = set()
        for member in archive.infolist():
            if member.filename in seen:
                raise ValueError('Duplicate ZIP member in Android APK')
            seen.add(member.filename)
        for member in archive.infolist():
            resource_directory = member.filename.split('/')[:-1]
            raw_resource = (len(resource_directory) == 2 and resource_directory[0] == 'res'
                            and resource_directory[1].split('-')[0] == 'raw')
            selected = (member.filename == raw.get('assets_licenses_androidmaven') if raw is not None
                        else member.filename.startswith('assets/') or raw_resource)
            if selected and member.file_size == len(expected) and archive.read(member) == expected:
                return {'member': member.filename, 'sha256': digest}
    raise ValueError(f'Android APK does not contain the generated release notices ({digest})')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--write-notices', action='store_true')
    mode.add_argument('--check-notices', action='store_true')
    mode.add_argument('--verify-apk', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    data = notice_bytes(report(root))
    asset = root / ASSET
    if args.write_notices:
        asset.parent.mkdir(parents=True, exist_ok=True)
        asset.write_bytes(data)
    elif not asset.is_file() or asset.read_bytes() != data:
        raise ValueError(f'{ASSET} differs from the selected Android release notices')
    if args.verify_apk:
        print(json.dumps(verify_apk(args.verify_apk, data)))


if __name__ == '__main__':
    main()
