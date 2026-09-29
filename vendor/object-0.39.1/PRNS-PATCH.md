# object 0.39.1 compatibility patch

This is the published `object` 0.39.1 crate, with only the optional `ruzstd`
dependency changed from `0.8.1` to `0.9.1` in `Cargo.toml` and `Cargo.toml.orig`.
`PROVENANCE.json` records the verified crates.io archive checksum, upstream
commit, original file hashes and the two patched manifest hashes. Rust source,
licenses and feature selection are unchanged.

`espflash` 4.5.0 requires the object 0.39 API. Published object 0.39.1 requires
ruzstd 0.8, whose decompression output bound is vulnerable to
[RUSTSEC-2026-0344](https://rustsec.org/advisories/RUSTSEC-2026-0344.html).
Object 0.40 accepts the fixed decoder but changes the reader API; espflash
4.6 still selects object 0.39 and raises the compiler requirement. This narrow
manifest patch keeps the existing firmware image tooling and its compressed
section support, using the upstream fixed decoder without an advisory exception.

The root workspace patch applies to the builder, flasher and resource tools.
It does not change embedded firmware dependencies. The root lockfile selects
ruzstd 0.9.1; its Rust 1.87 minimum is within the repository's Rust 1.90 minimum.

The resource integration tests exercise valid zstd sections, concatenated and
skippable frames, truncated input, and a ten-byte RLE frame whose 1,025-byte
block exceeds its 1,024-byte window. That last case is bounded in memory even
under the old decoder and must be rejected with the fixed dependency.

Remove this patch when the supported espflash release selects a fixed decoder
without requiring a compatibility patch. Do not edit the vendored Rust source
or broaden the patch without recording provenance and requalification.
