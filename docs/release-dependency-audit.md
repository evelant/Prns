# Public-Release Dependency Audit

This is the release policy and evidence map for the shipped Prns engine, daemon, desktop/mobile
apps, firmware, WebAssembly module, and npm package. The per-build evidence artifact records the exact Git
commit and hashes of the checked baselines.

## Reproducible graph matrix

The [dependency audit](../validation/security/deps-audit.sh) owns the exact
manifest, target and feature matrix. It covers shipped engine, service, app,
SDK, firmware, browser and tool graphs. Use that list rather than a copied table
when adding a target or changing features. The registered `dependency-audit`
suite runs it with `--locked` and `--exclude-dev`, checking advisories, licenses,
sources and bans; its entry in the [validation registry](../validation/manifest.toml)
defines the platform and artifact location.

## Dependency and license policy

Historical advisory fixes belong in commit history. Run the audit against the
candidate's locked graphs and current advisory data to establish its result.
Pinned audit-tool versions are enforced by the audit entrypoint.

The audit enforces `ctutils 0.4.4` or later for
[RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html) and
Tokio `1.53.3` or later for
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html).
These minimums do not replace checking current advisory data. Unix supplied
pipes and Linux HaLow sockets use a sealed owned-descriptor wrapper in the FFI
boundary to satisfy Tokio's registration safety contract; consuming crates
retain their unsafe-code forbids.

[`deny.toml`](../deny.toml) owns the reviewed license allowlist, exact package
exceptions and permitted sources. Unlisted license expressions fail unless the
selected package and version have an explicit exception. GPL, LGPL, AGPL and
unknown licenses are not accepted. MPL-2.0 is permitted when recipients receive the relevant notices and
access to the corresponding source, including local changes. For Rust, the notice
generator checks each selected MPL archive against its reviewed digest in
`release/licenses/rust-mpl-sources.json`, checks the unpacked sources, and places
a download link and checksum beside the license. Review a new archive's source
coverage and notices before recording its digest. The selected mobile SDK uses
the upstream revision, exact registry archives and verified source patch described
in its own recipient notice.

## Unsafe enforcement

The [unsafe audit](../validation/security/unsafe-audit.py) owns the reviewed
package exceptions and lint requirements. Other shipped first-party targets
must declare `#![forbid(unsafe_code)]`. Exceptions cover narrow native, FFI and
embedded boundaries; they do not grant downstream packages permission to add
unsafe code. Review the exception sets in that file when changing ownership.

The deterministic
`validation/security/unsafe-audit.py` combines locked Cargo metadata with a nested-comment/string/raw-string-
aware Rust token scan. `audits/unsafe-snapshot.json` records package version, source, enabled
features, graph membership, and unsafe token classes; unexplained drift fails CI.

Cargo-geiger 0.13.0 remains supplemental evidence only. Its logs are always uploaded, and parser
failure is reported as an incomplete inventory rather than accepted as a green gate. The compiler
forbids and the reviewed metadata/token snapshot are the enforcement mechanisms.

## Notices and non-Cargo graphs

`THIRD_PARTY_NOTICES.md` is generated and byte-checked with cargo-about 0.9.1. Each complete locked
manifest closure is fetched first; cargo-about then target-filters and reads only the packaged
license material in offline mode, so mutable network responses cannot change signed release
evidence. The bundle canonicalizes presentation-only whitespace (line endings, trailing space, and
repeated blank lines) without changing legal words, then deduplicates those texts across the graph
matrix. It includes bundled native-code notices, reproduces the Nordic terms, and appends the
checked Android Maven and hosted JavaScript closures. The browser graph pins
`esptool-js 0.6.0`, `spark-md5 3.0.2` (MIT alternative), and their exact transitive runtime graph.
Playwright 1.61.1 and axe 4.12.1 are exact-pinned development-only browser/accessibility tools.
Their locked registry sources, integrity hashes, and permissive/MPL licenses are audited, while
production source and bundle scans prevent them from entering shipped output; because they are not
distributed, they are intentionally absent from the shipped notice bundle.
Firmware builds copy the notices beside every hosted image; CLI archives and the website carry the
same checked bundle.

Android's `releaseRuntimeClasspath` must exactly match
`personal-hopspot/mobile/android/dependencies/release-runtime.tsv`. The npm production graph must
remain empty for `prns-wasm`; the website graph must exactly match the explicit browser-flasher
allowlist and cannot contain a runtime CDN or the legacy ESP Web Tools engine.

The website CI job runs `validation/security/npm-production-audit.py --scope web`
to check the website and WebAssembly graphs. The release dependency audit uses
the default `all` scope, which also checks the selected mobile dependencies.
Mobile policy findings therefore fail the release audit without failing the
website job.

For PRNS Chat, the default audit checks registry origin, integrity, required peers
and advisories across the full app declaration graph. The Android and iOS npm
license gates select the files in each production Metro export, including
assets, embedded upstream code and required notices. A moved import, new
package, changed archive or changed release input triggers requalification.
After staging the selected SDK, installing the application dependencies and
applying the runtime patch, refresh both production inventories with
`python3 validation/security/app_mobile_npm.py --write`. Then run
`python3 validation/security/npm-production-audit.py --write-app-notices`.
The normal CI commands compare all generated evidence and notices.

The Android release check verifies the resolved Maven artifacts, the packaged
notices and the final JavaScript against the release build. Maven decisions
apply to the exact artifacts and product in `release/licenses/app-maven/`.
The iOS production build checks the target’s CocoaPods acknowledgements and
JavaScript and native notices in the app bundle. Both platforms expose these
materials under About. Refresh the native and Maven assets with
`python3 validation/security/app_native_sources.py --write` and
`python3 validation/security/app_maven.py --write-notices`.

When the npm audit produces reports, CI uploads
`validation-artifacts/dependency-audit/*.json` as `dependency-audit-diagnostics`,
including when the audit fails. These reports describe findings, not policy
approval. The separate `release-dependency-audit` artifact, including the exact
commit attestation, is produced only after the audit and generated-notice checks
pass.

## Pinned tools and remaining physical checks

- cargo-deny 0.19.8
- cargo-about 0.9.1
- cargo-geiger 0.13.0 (advisory evidence)

OS/hardware acceptance—including the complete web/CLI all-board matrix and every published CLI
architecture—must be signed off against the exact signed candidate. The protected promotion job
validates `acceptance.json`; those physical observations cannot be inferred from CI.
