"""Attribution for published UniFFI npm packages and the maintained text patches."""
import base64
import hashlib
from pathlib import Path
import sys
import tomllib

import react_native
import runtime_source

ROOT = react_native.ROOT
sys.path.insert(0, str(ROOT / 'validation/security'))
import npm_lockfile


def input_paths():
    source = react_native.inputs()
    return tuple(sorted({
        Path(__file__).resolve(), Path(react_native.__file__).resolve(),
        Path(runtime_source.__file__).resolve(),
        react_native.LOCK, ROOT / 'deny.toml', Path(npm_lockfile.__file__).resolve(),
        ROOT / source['runtime_patch']['file'],
        *(react_native.HERE / item['file'] for item in source['patches']),
        *(react_native.HERE / item['license_file'] for item in source['packages']),
    }))


def evidence():
    source = react_native.inputs()
    allowed = tomllib.loads((ROOT / 'deny.toml').read_text())['licenses']['allow']
    packages = []
    archives = []
    for item in source['packages']:
        archives.append((item, react_native.package_archive(item)))
        notice = (react_native.HERE / item['license_file']).read_bytes()
        findings = []
        reviewed = npm_lockfile.reviewed_mobile_runtime_license(
            item['name'], item['version'], item['license'], item['integrity'], item['archive'])
        if not reviewed or item['license'] not in allowed:
            findings.append(f"No reviewed license policy entry for {item['name']} {item['version']} "
                            f"({item['license']}) at the supplied archive URL and integrity.")
        packages.append({
            'name': item['name'], 'version': item['version'],
            'archive': item['archive'], 'sha256': item['sha256'], 'integrity': item['integrity'],
            'license': item['license'], 'license_notice': notice.decode('utf-8'),
            'license_notice_sha256': hashlib.sha256(notice).hexdigest(),
            'upstream': source['upstream'], 'reviewed_source_archive': reviewed,
            'unresolved_policy': findings,
        })
    runtime_source.verify_corresponding_sources(source, archives)
    patch = source['runtime_patch']
    return {
        'packages': packages, 'source': source,
        'source_lock_sha256': react_native.digest(react_native.LOCK),
        'source_patches': [{'file': patch['file'], 'sha256': patch['sha256'],
                            'patch': (ROOT / patch['file']).read_text()}],
        'scope': 'These exact published npm packages are required runtime peers. '
                 'The SDK supplies a text patch for the React Native runtime; consumers '
                 'apply it before native compilation. Prebuilt Rust binaries are unchanged.',
        'approval': 'MPL-2.0 distribution requires matching published source, upstream notices and '
                    'the local runtime changes for each selected package.',
    }


def recipient_source_lines(report, names=None):
    """Recipient-ready source access for the selected runtime archives."""
    packages = [item for item in report['packages'] if names is None or item['name'] in names]
    if names is not None and {item['name'] for item in packages} != set(names):
        raise ValueError('missing selected runtime source or notice')
    if not packages:
        return []
    verify_policy(dict(report, packages=packages))
    upstream_url = runtime_source.source_url(report['source']['upstream'])
    lines = ['', '## UniFFI mobile runtime sources', '',
             'The source code and the PRNS changes below are available under MPL-2.0.', '',
             f"Download the [upstream source for revision {report['source']['upstream']['revision']}]({upstream_url}) "
             f"(SHA-256 `{report['source']['upstream_source_sha256']}`).", '']
    for item in packages:
        lines.extend([f"- [{item['name']} {item['version']} registry archive]({item['archive']}) "
                      f"(SHA-256 `{item['sha256']}`)."])
    lines.extend(['', 'Original upstream notice', '', '```text',
                  packages[0]['license_notice'].rstrip(), '```', ''])
    if any(item['name'] == '@ubjs/react-native' for item in packages):
        patch, = report['source_patches']
        encoded = base64.encodebytes(patch['patch'].encode()).decode('ascii').rstrip()
        lines.extend(['To reproduce the modified source, unpack the React Native registry archive '
                      'with its top-level `package` directory renamed to `node_modules/@ubjs/react-native`. '
                      'The SDK contains the patch in `patches/@ubjs+react-native+0.31.0-6.patch`. '
                      'This notice also includes the complete patch below. Save the block as '
                      '`runtime.patch.b64`, run `python3 -m base64 -d runtime.patch.b64 > runtime.patch`, '
                      'then run `git apply runtime.patch` from the directory containing `node_modules`.',
                      '', f"Patch SHA-256: `{patch['sha256']}`.", '', '```base64', encoded, '```', ''])
    return lines


def declaration_lines(report):
    lines = ['', '## Selected UniFFI npm packages', '', report['scope'], '', report['approval']]
    for item in report['packages']:
        lines.extend('  **Unresolved:** ' + finding for finding in item['unresolved_policy'])
    return [*lines, *recipient_source_lines(report)]


def verify_policy(report):
    findings = [f"{item['name']}: {finding}" for item in report['packages']
                for finding in item['unresolved_policy']]
    if findings:
        raise ValueError('SDK runtime release review remains unresolved:\n' + '\n'.join(findings))
