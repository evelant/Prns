"""Closed CRC-framed board Link metadata; CRC is corruption detection, not authentication."""
from __future__ import annotations
import importlib.util
from pathlib import Path
import re
import zlib

_spec = importlib.util.spec_from_file_location("link_schema", Path(__file__).with_name("link_schema.py"))
assert _spec and _spec.loader
LINK = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(LINK)
PREFIX = "prns.link.trace "
MAX_FRAME = 768
U64_MAX = 2**64 - 1
TOKEN = re.compile(r"[a-z][a-z0-9_]*=[A-Za-z0-9_().:+-]+\Z")
HEADER = {"v", "boot", "seq", "observed_ms"}
PROBE = {"stage", "category", "core_ms", "radio_before_ms", "radio_after_ms"}
HEALTH = {"stage", "core_seen", "lora_seen", "core_admitted", "lora_admitted",
          "core_published", "lora_published", "sink_dropped", "core_capped", "lora_capped", "healthy"}


def unsigned(value, maximum=U64_MAX):
    return isinstance(value, str) and re.fullmatch(r"[0-9]{1,20}", value) is not None and int(value) <= maximum


LORA_COUNTERS = {"attempted", "emitted", "producer_dropped", "sink_dropped", "queue_accepted",
                 "tx_started", "tx_complete", "tx_failed", "rx_frames", "rx_packets",
                 "rx_crc_errors", "rx_header_errors", "rx_radio_errors"}
LORA_FIELDS = {"stage", "family", "sequence", "source_sequence", "link", "has_link", "packet", "phase",
               "at_ms", "has_at", "age_ms", "has_age", "queue_bytes", "active", "frame_index", "frame_count"} | LORA_COUNTERS
LORA_PHASES = {"QueueActive", "QueuePending", "QueueActivated", "QueueRejected", "QueueCleared",
               "TxStarted", "TxFrameStarted", "TxFrameComplete", "TxComplete", "TxRadioError", "TxFramingError",
               "RxFrame", "RxAwaiting", "RxReplaced", "RxRejected", "RxPacket", "RxDelivered",
               "RxCrcError", "RxHeaderError", "RxRadioError", "RxReset", "LimitReached"} | {
                   f"QueueDropped({value})" for value in ("Expired", "Disabled", "PreResume", "MissingTime", "ClockRollback")
               } | {f"ActiveDropped({value})" for value in ("Contention", "Duty", "Disabled", "Changed", "Unconfigured")}


def valid_lora(fields):
    if (set(fields) != LORA_FIELDS or fields.get("stage") != "handshake" or fields.get("family") != "lora"
            or fields["phase"] not in LORA_PHASES or fields["packet"] not in {"Request", "Proof", "LinkData", "Other"}):
        return False
    for key, value in fields.items():
        if key in {"stage", "family", "phase", "packet"}: continue
        if key == "link":
            if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{16}", value) is None: return False
        elif key.startswith("has_") or key == "active":
            if value not in {"true", "false"}: return False
        elif not unsigned(value, 255 if key in {"frame_index", "frame_count"} else
                          65535 if key == "queue_bytes" else
                          2**32-1 if key in LORA_COUNTERS | {"sequence", "source_sequence"} else U64_MAX):
            return False
    return True


def parse(line):
    if len(line) > MAX_FRAME or not line.startswith(PREFIX): return None
    body, marker, checksum = line[len(PREFIX):].rpartition(" checksum=")
    if not marker or re.fullmatch(r"[0-9a-f]{8}", checksum) is None: return None
    try:
        raw = body.encode("ascii")
    except UnicodeEncodeError:
        return None
    if zlib.crc32(raw) != int(checksum, 16): return None
    values = {}
    for token in body.split(" "):
        if TOKEN.fullmatch(token) is None: return None
        key, value = token.split("=", 1)
        if key in values: return None
        values[key] = value
    if not HEADER <= set(values) or values["v"] != "1": return None
    if re.fullmatch(r"[0-9a-f]{16}", values["boot"]) is None: return None
    if not unsigned(values["seq"], 2**32-1) or int(values["seq"]) == 0 or not unsigned(values["observed_ms"]): return None
    fields = {k:v for k,v in values.items() if k not in HEADER}
    if fields.get("stage") == "probe":
        if (set(fields) != PROBE or fields["category"] != "installed"
                or not all(unsigned(fields[k]) for k in ("core_ms", "radio_before_ms", "radio_after_ms"))
                or int(fields["radio_after_ms"]) < int(fields["radio_before_ms"])): return None
        kind = "link_probe"
    elif fields == {"stage": "limit", "category": "health", "limit": "1800"}:
        kind = "link_limit"
    elif fields.get("stage") == "health":
        if set(fields) != HEALTH: return None
        for key, value in fields.items():
            if key == "stage": continue
            if key in {"core_capped", "lora_capped", "healthy"}:
                if value not in {"true", "false"}: return None
            elif not unsigned(value): return None
        kind = "link_health"
    elif fields.get("stage") == "handshake":
        if "sequence" in fields: return None
        fields = {**fields, "sequence": values["seq"]}
        if fields.get("family") == "core":
            if not LINK.valid(fields): return None
        elif fields.get("family") == "lora":
            if not valid_lora(fields): return None
        else: return None
        kind = "link_event"
    else: return None
    return {"kind":kind, "trace_version":1, "boot_token":values["boot"],
            "board_sequence":int(values["seq"]), "board_observed_ms":int(values["observed_ms"]),
            "fields":fields}
