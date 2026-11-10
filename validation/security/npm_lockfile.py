"""Verify licenses and provenance, including children inside hashed npm archives."""

import base64
import binascii


# These reviewed source records identify exact published bytes independently of the input lockfiles.
_REVIEWED_MOBILE_RUNTIME_ARCHIVES = {
    "@ubjs/core": (
        "https://registry.npmjs.org/@ubjs/core/-/core-0.31.0-6.tgz",
        "sha512-pscv6Zj8mVmd5+ikb1Zr6XZ0yqYwKGeIV8HU8rf9sTvs7GW34OARAp03xFJ7rE9XE4R6LgwQhzYW9knAuz5yxA==",
    ),
    "@ubjs/react-native": (
        "https://registry.npmjs.org/@ubjs/react-native/-/react-native-0.31.0-6.tgz",
        "sha512-vD1033OqUFy968F/3yUKFfN/Im7sfSM5NXRZcUlpUWwuzC9vQn81vtX9P1Fz/xuLxNc88MnNym3zari7PkClNQ==",
    ),
}


def reviewed_mobile_runtime_license(
    name: str, version: str, license_expression: str, integrity: str, archive_url: str,
) -> bool:
    """Whether this archive matches a mobile runtime with verified corresponding source."""
    return (
        version == "0.31.0-6"
        and license_expression == "MPL-2.0"
        and _REVIEWED_MOBILE_RUNTIME_ARCHIVES.get(name) == (archive_url, integrity)
    )


def has_registry_integrity(metadata: dict) -> bool:
    resolved = metadata.get("resolved", "")
    integrity = metadata.get("integrity", "")
    if (
        not isinstance(resolved, str)
        or not resolved.startswith("https://registry.npmjs.org/")
        or not isinstance(integrity, str)
        or not integrity.startswith("sha512-")
    ):
        return False
    try:
        return len(base64.b64decode(integrity.removeprefix("sha512-"), validate=True)) == 64
    except (ValueError, binascii.Error):
        return False


def verify_development_packages(packages: dict, allowed_licenses: set[str]) -> None:
    for path, metadata in packages.items():
        if not path or metadata.get("dev") is not True:
            continue
        if metadata.get("license") not in allowed_licenses:
            raise ValueError(f"website test/build dependency lacks a reviewed license: {path}")
        archive = metadata
        if metadata.get("inBundle") is True:
            parent_path, separator, name = path.rpartition("/node_modules/")
            parent = packages.get(parent_path, {})
            bundled = parent.get("bundleDependencies")
            if (
                not separator
                or parent.get("dev") is not True
                or parent.get("inBundle") is True
                or not isinstance(bundled, list)
                or name not in bundled
            ):
                raise ValueError(f"bundled dependency lacks its declared development archive: {path}")
            archive = parent
        if not has_registry_integrity(archive):
            raise ValueError(f"website test/build dependency lacks registry source/integrity: {path}")
