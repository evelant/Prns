import * as Bindings from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { BodyText, Button, Card, Subheading } from "@/ui/primitives";

const choices = [
  { minutes: undefined, label: "Manual only" },
  { minutes: 15, label: "Every 15 minutes" },
  { minutes: 30, label: "Every 30 minutes" },
  { minutes: 60, label: "Every hour" },
  { minutes: 180, label: "Every 3 hours" },
  { minutes: 360, label: "Every 6 hours" },
] as const;

export function GroupAnnounceSettings() {
  const development = useDevelopmentRuntime();
  const { getGroupAnnounceSettings, setGroupAnnounceSettings, announceGroups } = development;
  const available = development.availability.type === "available";
  const ready = available && development.phase === "ready";
  const running =
    ready &&
    development.canPublish !== false &&
    development.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Running;
  const owner = useMemo(
    () => ({
      available,
      phase: development.phase,
      generation: development.snapshot?.generationId,
      identity: development.ownerKey,
    }),
    [available, development.phase, development.snapshot?.generationId, development.ownerKey],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const mounted = useRef(false);
  const operation = useRef(0);
  const inFlight = useRef<object | null>(null);
  const loadedOwner = useRef<object | null>(null);
  const [saved, setSaved] = useState<{ owner: object; minutes: number | undefined } | null>(null);
  const [pending, setPending] = useState<object | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const busy = pending === owner;
  const loaded = saved?.owner === owner;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      operation.current += 1;
    };
  }, []);

  const settings = useCallback(
    async (update?: Bindings.GroupAnnounceSettings) => {
      if (!ready || inFlight.current === owner) return;
      inFlight.current = owner;
      const ticket = ++operation.current;
      const current = () =>
        mounted.current && currentOwner.current === owner && operation.current === ticket;
      setPending(owner);
      try {
        const result =
          update === undefined
            ? await getGroupAnnounceSettings()
            : await setGroupAnnounceSettings(update);
        if (!current()) return;
        if (result.type !== "outcome" || result.outcome.tag !== "Ready") {
          setFeedback(
            "Announcement settings could not be confirmed. Reload to check the saved choice.",
          );
          return;
        }
        setSaved({ owner, minutes: result.outcome.inner.settings.intervalMinutes });
        setFeedback(
          update === undefined
            ? null
            : "Announcement preference saved. Applies while the node is running.",
        );
        if (update !== undefined) setExpanded(false);
      } catch {
        if (current())
          setFeedback(
            "Announcement settings could not be confirmed. Reload to check the saved choice.",
          );
      } finally {
        if (inFlight.current === owner) inFlight.current = null;
        if (current()) setPending(null);
      }
    },
    [getGroupAnnounceSettings, setGroupAnnounceSettings, owner, ready],
  );

  useEffect(() => {
    if (!ready || loadedOwner.current === owner) return;
    // Load once per native owner; failures require explicit reload.
    loadedOwner.current = owner;
    void settings();
  }, [owner, ready, settings]);

  async function announce() {
    if (!running || inFlight.current === owner) return;
    inFlight.current = owner;
    const ticket = ++operation.current;
    const current = () =>
      mounted.current && currentOwner.current === owner && operation.current === ticket;
    setPending(owner);
    try {
      const result = await announceGroups();
      if (!current()) return;
      if (result.type !== "outcome") {
        setFeedback("Announcement request could not be confirmed.");
        return;
      }
      switch (result.outcome.tag) {
        case "Queued":
          setFeedback(
            "Announcement queued behind another identity. Keep prns open; all identities share the announcement budget.",
          );
          break;
        case "Requested":
          setFeedback(
            "Group announcements requested. This does not confirm that another device received them.",
          );
          break;
        case "Throttled":
          setFeedback(
            `Recently announced. Try again in ${Math.max(1, Math.ceil(Number(result.outcome.inner.retryAfterMillis) / 1000))} seconds.`,
          );
          break;
        case "Busy":
          setFeedback("An announcement is already pending. Try again shortly.");
          break;
        case "Unavailable":
          setFeedback(
            "Group announcements could not be confirmed. Check that the node is running.",
          );
          break;
      }
    } catch {
      if (current()) setFeedback("Announcement request could not be confirmed.");
    } finally {
      if (inFlight.current === owner) inFlight.current = null;
      if (current()) setPending(null);
    }
  }

  if (!available) return null;
  return (
    <Card>
      <Subheading>Group visibility</Subheading>
      <BodyText muted>
        Announcements help other devices find this phone for groups and invitations. They do not
        send group messages.
      </BodyText>
      <BodyText>
        {loaded
          ? `Automatic announcements: ${choices.find((choice) => choice.minutes === saved.minutes)?.label ?? "Unknown"}.`
          : "Announcement preference not loaded."}
      </BodyText>
      <Button tone="secondary" disabled={!running || busy} onPress={() => void announce()}>
        Announce groups now
      </Button>
      <Button
        tone="secondary"
        disabled={!ready || busy || !loaded}
        onPress={() => setExpanded(!expanded)}
      >
        {expanded ? "Hide announce intervals" : "Change announce interval"}
      </Button>
      {expanded ? (
        <>
          <BodyText muted>
            Timings vary by up to 25% to spread network traffic. Short intervals use more shared
            radio bandwidth; 15 and 30 minutes are intended for temporary discovery needs. Manual
            only also disables startup announcements.
          </BodyText>
          {choices.map((choice) => (
            <Button
              key={choice.label}
              tone="secondary"
              accessibilityState={{ selected: loaded && saved.minutes === choice.minutes }}
              disabled={!ready || busy}
              onPress={() => void settings({ intervalMinutes: choice.minutes })}
            >
              {choice.label}
            </Button>
          ))}
        </>
      ) : null}
      {feedback === null ? null : <BodyText>{feedback}</BodyText>}
      {!loaded || feedback?.includes("Reload") ? (
        <Button tone="secondary" disabled={!ready || busy} onPress={() => void settings()}>
          Reload announce preference
        </Button>
      ) : null}
    </Card>
  );
}
