"""Closed, metadata-only Link diagnostic schema shared by native and board tools."""
from __future__ import annotations
import re

CORE_BASE = {"stage", "family", "sequence", "source_sequence", "dropped_before", "link", "has_link", "interface", "has_interface", "command", "has_command", "at_ms", "has_at", "phase"}
CORE_PHASES = {
    "request_pending": {"requested_at_ms", "timeout_at_ms", "hops", "bitrate", "has_bitrate", "first_hop_floor_ms", "has_floor", "computed_timeout_ms", "route_generation"},
    "request_rejected": {"detail"}, "request_ingress": {"detail"}, "proof_ingress": {"detail"},
    "proof_prepared": set(), "proof_rejected": {"detail"}, "proof_completion": {"detail"},
    "initiator_timeout": {"requested_at_ms", "route_generation", "detail"}, "responder_timeout": set(),
    "relay_pending": {"timeout_at_ms", "remaining_hops", "taken_hops", "bitrate", "has_bitrate", "route_generation"},
    "relay_timeout": {"timeout_at_ms", "route_generation"}, "producer_limit": {"limit", "dropped_at_least"},
}
EGRESS_FIELDS = {"stage", "family", "sequence", "source_sequence", "link", "has_link", "interface", "has_interface", "packet", "phase", "queue_age_ms", "has_age", "attempted", "emitted", "producer_dropped", "sink_dropped"}
EGRESS_PHASES = {"Enqueued", "Deferred", "Unavailable", "DroppedFull", "LaneMissing", "IfacRejected", "Flushed", "PendingDiscarded", "LimitReached"}
# Exact closed diagnostic vocabulary; never persist arbitrary error formatting.
IGNORE = {"Consumed", "Malformed", "UnhandledContext", "Duplicate", "Superseded", "NotForUs", "NoRoute", "HopLimitReached", "LoopPrevented", "RouteUnresponsive", "OtherInstance", "UnknownLink", "LinkPhaseMismatch", "DecryptFailed", "ProofInvalid", "UnknownIdentity", "LinkRequestsRefused", "PermissionDenied", "RateLimited", "CapacityExhausted", "RequestTooLarge", "StrategyDeclined", "UnmatchedResponse", "IfacRefused"}
DETAILS = {"RouteVanished", "Serialize", "LinkTableFull", "DuplicateLinkId", "IdentityNotHeld", "Accepted", "VerificationPending", "Forwarded", "Activated", "InvalidSignature", "EgressDisabled", "WriteRejected(NotPending)", "WriteRejected(Serialize)", "Removed", "Preserved", "TransportMarkedUnresponsive", "TransportPreserved"} | {f"Ignored({value})" for value in IGNORE} | {f"Ignored(LinkRttError({value}))" for value in ("Malformed", "InvalidToken", "BufferTooShort")}


def valid(fields):
    if not isinstance(fields, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k,v in fields.items()): return False
    if fields.get("stage") != "handshake": return False
    if fields.get("family") == "core":
        extras = CORE_PHASES.get(fields.get("phase"))
        if extras is None or set(fields) != CORE_BASE | extras: return False
    elif fields.get("family") == "egress":
        if set(fields) != EGRESS_FIELDS or fields["phase"] not in EGRESS_PHASES or fields["packet"] not in {"none", "request", "proof"}: return False
    else: return False
    for key, value in fields.items():
        if key in {"stage", "family", "phase", "packet"}: continue
        if key in {"link", "interface"}:
            if not re.fullmatch(r"[0-9a-f]{16}", value): return False
        elif key.startswith("has_"):
            if value not in {"true", "false"}: return False
        elif key == "detail":
            if value not in DETAILS: return False
        elif not re.fullmatch(r"[0-9]{1,20}", value) or int(value) > 2**64 - 1: return False
    return True
