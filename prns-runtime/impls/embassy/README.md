# Personal RNS (Prns)

This crate is one package in the Personal RNS public Rust graph. Quick overviews, the complete feature guide, API documentation, examples, and the cross-language SDK overview are available at [prns.dev](https://prns.dev) or [reticulum.rs](https://reticulum.rs), and in the [source repository](https://github.com/KenAKAFrosty/Prns).

All public packages use the same engine, release version, and dual MIT/Apache-2.0 license.

## Bound route-journal replay work

`EmbeddedCompactionPolicy::with_route_replay_compaction_threshold` enables idle
maintenance when retained route upserts exceed the configured threshold and a
snapshot would at least halve the replay work. Generic policies keep it disabled;
`hopspot` requests it after 256 route upserts. Maintenance uses the existing
incremental journal transaction and persisted 24-hour attempt budget. It preserves
critical state and only resets the replay counter after a confirmed replacement
commit. The journal format is unchanged. The first boot still validates the old
journal; later boots benefit from the completed compaction.

Nine focused regressions cover trigger policy, retained predecessors, cooldown,
failed writes, reboot, torn I/O and authorization arriving during maintenance.
Historical E290 diagnostic evidence recorded 918 route upserts compacted to two
and a later firmware-clock restore of 223 ms, versus the earlier 28,756 ms.
Those observations belong to the original working-tree images, not this PR's
rebased binary. The persistence implementation source matched SHA-256
`3825dbc10477d1d5be18e199d8a7635a16a6e1eaba33c24dd4149058f3854d8d`;
new baseline test results and exact-image hardware qualification are separate.
