# Application npm license evidence

`provenance.json` binds each fallback to an exact package/version/archive integrity. Normal package notices come directly from integrity-checked npm archives; these preserved texts cover packages whose archives omit a standalone license file. Full archive checks use the lockfiles. The production bundle capture uses the installed packages and staged SDK.

Most fallbacks use the registry's exact published Git revision. `bplist-parser` and `imurmurhash` reproduce complete license sections from their published READMEs. For `boolbase`, the later maintainer license commit still declares 1.0.0 and its implementation is byte-identical to the published archive. For `structured-headers`, the first license commit still declares 0.4.1 and all six published TypeScript source files match; the file-by-file hashes are preserved separately. These are explicit source-equivalence findings, not claims that the original tarballs contained the later license files.

The published `@ubjs/core` and `@ubjs/react-native` `0.31.0-6` archives omit their
standalone license file. Their registry `gitHead` identifies upstream revision
`3dd1cd1353750dc3bfbbf4bb4d4ccde26b0fbd2e`; the fallback preserves that revision's
exact `LICENSE` text and its link to the MPL 2.0 terms. The app and SDK use the same
MPL-2.0 source verification for these two registry archives. The audit checks each
name, version, archive URL, integrity hash, and license. The SDK notice records
the upstream revision and the patch applied to the React Native runtime.

The preserved fallback reviews for `casework` and `jimp-compact` remain unresolved: `casework` has a license declaration but no complete attributed text at the published revision; `jimp-compact` reproduces its explicitly attributed Jimp 0.16.1 terms, but its bundled dependencies and wrapper are not fully reviewed. The full declaration report retains these findings. An app release selects a package for distribution review when its code or assets occur in the production export. No copyright holder or legal text is inferred.

The collector records absent required peers separately in `unresolved_required_peers`; they contribute no selected archive bytes, but remain failures in the release audit and are displayed in the notice bundle. Missing required runtime dependencies still stop collection. An optional peer declaration on a parent package does not waive a child's required peer. Installed optional peers of registry packages remain in the declaration inventory. The app installs its testing library under a development-only alias. A clean production install leaves Expo Router’s optional testing-library peer, Jest, and deepmerge out of the graph.

Android and iOS each have a checked production inventory in `mobile-bundles.json`.
Their mapped modules and emitted assets select exact registry archives. Source
headers and `embedded-sources.json` supply additional notices for third-party
code contained inside those archives. Both Metro export and native release
qualification check these inputs. A new import of a previously unused package
brings that package into the distribution gate.

`npm-production-audit.py` checks the full reachable closure for integrity,
required peers and advisories and writes its diagnostics. It evaluates the
SPDX expressions and required attribution on the per-platform shipped sets.
For MPL code, source availability, local patches and the notice delivered to
the recipient are verified separately. The website and each published SDK
have their own qualification. See `docs/release-dependency-audit.md` for
the app regeneration commands.
