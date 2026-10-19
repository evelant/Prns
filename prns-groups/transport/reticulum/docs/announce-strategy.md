# Group destination announces

Announcements establish destination reachability and public keys. They are
separate from inventory synchronization, membership distribution, receipts and
custody. Fresh content uses peer-directed synchronization, not network-wide
announcements.

## Current policy

Two destinations serve each identity: `prns.groups` admits enrolled peers, and
`prns.groups.enrollment` admits identified bootstrap requests. They multiplex all
that identity's groups, with empty announce application data. Observers can
associate the two services with the same public identity, but learn no group list,
membership or inventory head from their announce payloads.

Application owners supply periodic and manual announcement scheduling. The
carrier itself does not start a timer or store preferences. The following native
runner integration supplies a startup/six-hour default, positive jitter and a
bounded manual lane; app-wide identity coordination is separate work.

Do not announce per message, receipt, membership change or failed synchronization.
All groups under one identity use the same service destinations. Adding groups
does not add destinations; adding identities increases the potential service load.

Sparse refresh helps maintain discoverable cached routes across intermittent
transport contacts. Unknown-destination path requests are not recursively
forwarded in every topology/interface mode. We therefore do not copy `lxmd`'s
silent-by-default delivery address, and do not send announcements per message,
receipt, membership revision, poll or failed synchronization. Positive jitter
spreads periodic work; there is no announce-on-every-reconnect loop.

## Comparison with Reticulum applications

| Reference examined | Observed strategy |
| --- | --- |
| [Reticulum 1.5.5 manual: announce rate control](https://reticulum.network/manual/interfaces.html#announce-rate-control) | Recommends infrequent application announcements, generally startup and roughly six-hour refresh. Describes announce bandwidth caps and penalties for excessive repeats. |
| [LXMF 1.2.0 `LXMRouter`](https://github.com/markqvist/LXMF/blob/1.2.0/LXMF/LXMRouter.py) | Delivery announcement is an explicit method; the library does not impose a periodic delivery timer. Propagation enable/disable announces the changed service state. |
| [LXMF 1.2.0 `lxmd`](https://github.com/markqvist/LXMF/blob/1.2.0/LXMF/Utilities/lxmd.py) | Generated configuration uses startup and 360-minute announcements for propagation service; its delivery address has startup and periodic announces disabled by default. The daemon owns these timers. |
| [NomadNet public `master` source](https://github.com/markqvist/NomadNet/blob/master/nomadnet/NomadNetworkApp.py), retrieved 2026-10-13 | Generated client and node configuration enables startup announces and uses 360-minute intervals. This is an application choice, not an LXMF protocol requirement. |
| [Sideband public `master` source](https://github.com/markqvist/Sideband/blob/master/sbapp/sideband/core.py), retrieved 2026-10-13 | When automatic announces are enabled, startup is delayed eight seconds and subsequent intervals are randomized between 90 and 300 minutes. Interface changes can also trigger announcements, with a separate debounce. |

The manual and application `master` links are dated observations, not immutable
version pins. These application choices inform our policy; none is an LXMF
protocol requirement or a guarantee of discovery on a disconnected path.

## Reachability and ownership

`connect_identified` requests a missing path and verifies the destination public
key against the expected enrolled/invited identity before establishing a Link.
It does not wait for the periodic timer. Registered SINGLE destinations answer
requested discovery; transport caches and interface modes determine which
multi-hop requests can succeed.

Endpoint Link establishment failure can retire the exact attempted route and
request discovery, guarded against discarding newer authenticated activity or a
replacement route. Requested repair shares the existing destination throttle; it
is separate from unsolicited service refresh. Stock
[Reticulum path requests](https://github.com/markqvist/Reticulum/blob/1.5.5/RNS/Transport.py)
also distinguish path responses from unsolicited announcements.

Destination registration does not start a second timer. Requested path
responses remain outside application scheduling and pass through ordinary
interface pacing.

## Qualification

Carrier tests exercise requested discovery and identified Links without
unsolicited announces. Timer, fairness and restart behavior belong to the
consumer integration; neither establishes physical airtime or battery use.

For an [isolated LoRa trial](lora-qualification.md), count unsolicited announces
separately from requested path responses and measure bytes at startup, idle,
reconnect and repair. The aggregate radio load also includes forwarding, retries
and other services; configured announce intervals do not bound total traffic.
