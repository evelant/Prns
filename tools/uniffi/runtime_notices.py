"""Attribution for published UniFFI npm packages and the maintained text patches."""
import hashlib
from pathlib import Path
import sys
import tomllib

import react_native

ROOT = react_native.ROOT
sys.path.insert(0, str(ROOT / 'validation/security'))
import npm_lockfile


def input_paths():
    source = react_native.inputs()
    return tuple(sorted({
        Path(__file__).resolve(), Path(react_native.__file__).resolve(),
        react_native.LOCK, ROOT / 'deny.toml', Path(npm_lockfile.__file__).resolve(),
        ROOT / source['runtime_patch']['file'],
        *(react_native.HERE / item['file'] for item in source['patches']),
        *(react_native.HERE / item['license_file'] for item in source['packages']),
    }))


def evidence():
    source = react_native.inputs()
    allowed = tomllib.loads((ROOT / 'deny.toml').read_text())['licenses']['allow']
    packages = []
    for item in source['packages']:
        react_native.package_archive(item)
        notice = (react_native.HERE / item['license_file']).read_bytes()
        findings = []
        reviewed = npm_lockfile.reviewed_mobile_runtime_license(
            item['name'], item['version'], item['license'], item['integrity'], item['archive'])
        if item['license'] not in allowed and not reviewed:
            findings.append(f"No reviewed license policy entry for {item['name']} {item['version']} "
                            f"({item['license']}) at the supplied archive URL and integrity.")
        packages.append({
            'name': item['name'], 'version': item['version'],
            'archive': item['archive'], 'sha256': item['sha256'], 'integrity': item['integrity'],
            'license': item['license'], 'license_notice': notice.decode('utf-8'),
            'license_notice_sha256': hashlib.sha256(notice).hexdigest(),
            'upstream': source['upstream'], 'mobile_runtime_license_exception': reviewed,
            'unresolved_policy': findings,
        })
    patch = source['runtime_patch']
    return {
        'packages': packages, 'source': source,
        'source_lock_sha256': react_native.digest(react_native.LOCK),
        'source_patches': [{'file': patch['file'], 'sha256': patch['sha256'],
                            'patch': (ROOT / patch['file']).read_text()}],
        'scope': 'These exact published npm packages are required runtime peers. '
                 'The SDK supplies a text patch for the React Native runtime; consumers '
                 'apply it before native compilation. Prebuilt Rust binaries are unchanged.',
        'approval': 'Mobile runtime license exceptions are scoped to the package name, version, '
                    'license, registry archive URL and SHA-512 integrity. The source record retains '
                    'the upstream notices, revision and local patches.',
    }


def declaration_lines(report):
    lines = ['', '## Selected UniFFI npm packages', '', report['scope'], '', report['approval'], '']
    for item in report['packages']:
        lines.append(f"- `{item['name']} {item['version']}` — `{item['license']}`; "
                     f"published archive SHA-256 `{item['sha256']}`.")
        if item['mobile_runtime_license_exception']:
            lines.append('  Covered by the mobile runtime npm license exception for this archive.')
        lines.extend('  **Unresolved:** ' + finding for finding in item['unresolved_policy'])
    upstream = report['source']['upstream']
    lines.extend(['', f"Upstream source: {upstream['repository']} at `{upstream['revision']}`.",
                  f"Exact npm integrity, source licenses and patches are bound by configuration "
                  f"SHA-256 `{report['source_lock_sha256']}`."])
    return lines


def verify_policy(report):
    findings = [f"{item['name']}: {finding}" for item in report['packages']
                for finding in item['unresolved_policy']]
    if findings:
        raise ValueError('SDK runtime release review remains unresolved:\n' + '\n'.join(findings))
