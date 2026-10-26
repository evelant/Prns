# Bounded catalog encoding

`prns-identities-codec` encodes the lifecycle catalog for storage without `std`,
allocation or unsafe code. Its only dependency is `prns-identities-core`.

- `max_encoded_len::<I, O, S>()` gives a conservative buffer/read bound, or `None`
  if the capacities or size cannot be represented.
- `encode(&catalog, &mut bytes)` validates the catalog and returns bytes written.
- `decode(&bytes, &mut catalog)` fills caller memory and validates it. On **any**
  error the output remains invalid and must not authorize effects. There is no
  implicit empty-catalog fallback.

The caller owns buffers and durable publication. Integrity checks, atomic writes,
locking, encryption and rollback protection belong to the storage adapter.
The codec alone cannot detect a bit change that produces another valid catalog.

## Format 1

This is a versioned local persistence format, independent of Reticulum packets,
contact cards and Rust struct layout. Integers use big-endian encoding. Capacity
values must exactly match the receiving catalog; changing storage bounds requires
an explicit new-store decision, not silent truncation. Legacy app-data migration
is outside this initial implementation.

| Offset | Field |
| --- | --- |
| 0 | Eight magic bytes `PRNSIDC1` |
| 8 | Codec version, `u16`, currently 1 |
| 10 | Identity, operation and service capacities, three `u32` values |
| 22 | Core catalog format, `u16` |
| 24 | Catalog incarnation, 16 bytes |
| 40 | Catalog revision, `u64` |
| 48 | Session, `u64` |
| 56 | Issued operation sequence, `u64` |
| 64 | Completed replay floor, `u64` |
| 72 | Exactly I optional identity records, then O optional operation records |

Option tags are 0 for absent and 1 followed by the value for present. Booleans
accept only 0 or 1. Arrays preserve every slot, including empty service slots.
Text uses a `u16` byte length followed by bounded UTF-8 without control characters.
Record fields and enum discriminants are explicit in [the codec](src/lib.rs);
changing their interpretation requires a codec version change. Unknown versions,
tags, invalid lengths/text, trailing bytes and invalid lifecycle states fail.

Tests round-trip actual lifecycle checkpoints, reject every truncated prefix,
verify the endian header, and cover cleanup/retirement result variants. The
native adapter adds its own checksum envelope and crash-safe file publication.
