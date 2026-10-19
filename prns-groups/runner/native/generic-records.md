# Generic native records

`Client` publishes application-defined inline records on the same serialized
storage lane, MLS transaction and Reticulum runner used by text messages. It
creates no node, database or executor. `SendText` remains a convenience profile;
its encoding and retained request IDs are unchanged.

## API

- `admit_publish(PublishRecord)` synchronously admits a job and returns a result
  waiter. Dropping the waiter does not cancel the admitted operation. `publish`
  is its asynchronous convenience wrapper.
- Publications supply a group, retry token, namespace/schema, opaque body and
  strictly sorted unique receipt targets. Targets select delivery obligations,
  **not confidentiality**: all MLS readers can decrypt. Applications assign their
  own namespace/schema; the carrier does not interpret their payload.
- Exact retained retries return the original record without another encryption
  or change event. Reusing the token with different namespace, schema, body or
  targets fails with `IdempotencyConflict`. The clock may differ on retry.
  Retention and retry horizon still come from the enrolled native profile.
- `records` pages retained records newest-first across every profile. `record`
  retrieves one record. Both expose authenticated metadata, original targets,
  local insertion sequence and device delivery evidence. The service retains a
  record's Published/Received change until that record is retired in the same
  maintenance transaction; projections validate this invariant. Text history filters
  and decodes the text profile over this same projection.
- `changes` reads the durable bounded feed. Its `ChangeToken` binds group,
  storage incarnation and position. Persist that token with an application
  projection; refuse tokens from a replacement store. `history_pruned` means
  the cursor crossed retired events or a maintenance invalidation gap. Rebuild
  from retained records even when the returned event list is empty.
  This is not an eternal audit log or an exactly-once external effect protocol.
- `delivery_progress` reconstructs authenticated receipt counts, pending/expired
  targets and membership interruption from committed state. Missing retired
  records return `None`, never successful delivery. Device receipt does not mean
  a human read the text or an actuator executed a command.

Absolute native admission ceilings (16 KiB body, eight targets) are checked before
queueing; each enrolled group's persisted limits can be smaller. Oversized or
unsorted input returns `InvalidInput`, not `StorageFull`; empty targets return
`NoRecipients`. Actual capacity exhaustion remains distinguishable. Incompatible
groups and invalid page sizes are rejected.
Errors retain their typed capacity, retry, clock, storage and protocol variants;
`Unavailable` or a dropped result never authorizes minting a replacement request
ID. A new retry token is created only for a new application operation.

The generic core and text codec remain allocation-free `no_std`. This native
API owns vectors on its caller's native lane; the existing MLS/service layers
remain bounded `no_std + alloc`. The API does not add a requirement for a native
executor or database to the portable core.

## Rebuilding a consumer projection

Record pages are live queries, not a frozen snapshot. Before rebuilding, capture
`changes(group, None, 1).head`; then page retained records into a new projection,
upserting entries by record ID. Replay changes **after that captured
head**, idempotently, before saving the resulting cursor with the projection.
Using the last record page's revision as the starting cursor could skip records
inserted while pagination was in progress. If `history_pruned` recurs during the
rebuild, retry with a fresh head under an application-defined bounded retry policy;
report incomplete reconstruction rather than claiming a complete snapshot.

Expiry maintenance removes records and associated feed entries **and advances the
feed head in the same transaction**. The new position has no event: its gap makes
`history_pruned` true for a consumer crossing it, including one already caught up
before cleanup. It requires no unbounded deletion log. Maintenance that only
retires internal recovery evidence may conservatively invalidate a projection
too. An exact maintenance retry does not advance the head again.

Rebuild into a replacement projection, so records absent from retained pages are
removed; upserting into an old projection alone would keep deleted entries. Save
the rebuilt projection and cursor together only after replay succeeds. On an
error or exhausted rebuild budget, keep the previous cursor/projection and report
incomplete refresh. A page can lose a record before a subsequent read; `record`
returning `None` is authoritative at that query and must remove that projected ID.
Continue polling the feed after a successful refresh: no live read is a promise
of permanent retention. This contract does not provide an atomic historical
snapshot, individual deletion events, or exactly-once external effects.

## Telemetry example and recovery evidence

Run from the repository root:

```sh
CARGO_INCREMENTAL=0 cargo run --locked --manifest-path prns-groups/Cargo.toml \
  -p prns-groups-runner-native --example telemetry
```

The example encodes an eight-byte sensor reading, publishes it through the
generic API, consumes the durable change feed at a second independently
credentialed endpoint, and waits for its authenticated receipt. Its in-process
controlled carrier still traverses real Reticulum nodes, Links, requests and
Resources. Fixture enrollment and temporary plaintext native stores are demo
setup, not production provisioning or protected storage.

Its small `examples/support/telemetry_projection.rs` consumer stages a replacement
map and cursor, captures the feed head before paging, and retries invalidated
rebuilds at most three times. Each pass is limited to 32 pages of 16 items and a
caller-selected cache capacity capped at 256 readings. Exhausting a bound returns
`Incomplete` and keeps the previous map and cursor; errors also leave them intact.
This helper is native application code, not an allocation requirement on the core.
The demo also runs expiry maintenance and verifies that the consumer removes the
expired reading after observing the durable invalidation.

`generic_records` tests lost-result admission, database reopen before delivery,
exact retry/conflict checks, scoped/pruned change cursors, mixed telemetry/text
pagination and receipt convergence between independent Reticulum nodes. They also
cover caught-up consumers after a lost maintenance result and database reopen,
unchanged cache/cursor on incomplete refresh, and deterministic insertion or
retirement between record paging and feed replay, including exhausted rebuild
attempts under repeated retirement. Service `maintenance_recovery`
tests prove that deletion and the invalidation gap survive the same crash cuts,
and exact maintenance replay does not advance the head twice. Existing
`lane_recovery` and session checkpoint tests also exercise abrupt process exit
at durable send/receive/receipt boundaries. These establish host recovery, not
phone suspension, physical power-loss behavior or protected at-rest storage.
