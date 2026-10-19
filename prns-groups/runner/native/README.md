# Native durable group owner

This slice exposes `GroupManager`, `StoreSession`, `Client` and `Job` for a
caller-owned exclusive database lane. It creates no worker, runtime or network
task. The caller implements `Submit`; each admitted job runs to a durable result
even if its reply waiter disappears. Exact request tokens survive reopen and
must be reused after ambiguous outcomes.

`GroupManager` provides authenticated enrollment, generic records, text profiles,
bounded maintenance and scoped history in the same native store. Enrollment work
is returned as bounded request/response data: a caller supplies an authenticated
peer identity to `handle` and correlates each completion with its original work.
No claimed identity inside payload data substitutes for transport authentication.
The Reticulum transport below this crate supplies the existing identified-Link
primitive; automatic polling, synchronization, announcements and headless network
execution arrive in the next slice.

All enrollment transitions are present here, including owner-local pending-send
checks immediately before Add, exact Welcome replay, Join/KeyPackage persistence,
retirement fences and interrupted-operation recovery. Delayed Join retries cannot
bypass the Add guard. The guard does not establish that unknown remote sends have
drained. See `enrollment.md` for the protocol and its limits.

The generic record APIs (`publish`, `records`, `changes`, `delivery_progress`)
and the retained text projection work offline. `Clock`/`SystemClock` provide the
same time semantics to durable request IDs and subsequent network scheduling.
The native adapter uses `std` and bounded allocation; it imposes neither on the
allocation-free `no_std` generic core. Native storage is not protected at rest.

Run `cargo test -p prns-groups-runner-native` from the group workspace. The owner
tests cover dropped replies, exact replay, maintenance, page invalidation,
checkpoint recovery and safe membership changes. No network runner is linked
into this slice. Development fixture generation remains an optional test tool;
authenticated creation/invitation/joining do not require it.
