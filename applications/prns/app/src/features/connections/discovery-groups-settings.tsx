import * as Bindings from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { ActionRow, BodyText, Button, Card, KeyValue, Subheading } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";

export function DiscoveryGroupsSettings() {
  const runtime = useDevelopmentRuntime();
  const [expanded, setExpanded] = useState(false);
  if (runtime.availability.type !== "available") return null;
  return (
    <Card>
      <Subheading>Advanced Bluetooth discovery</Subheading>
      <Button tone="secondary" onPress={() => setExpanded(!expanded)}>
        {expanded ? "Hide discovery groups" : "Change discovery groups"}
      </Button>
      {expanded ? <DiscoveryGroupsForm /> : null}
    </Card>
  );
}

function DiscoveryGroupsForm() {
  const runtime = useDevelopmentRuntime();
  const { readBluetoothDiscoveryGroups, setBluetoothDiscoveryGroups } = runtime;
  const ready = runtime.phase !== "starting" && !runtime.stoppingNode;
  const nodeState = runtime.snapshot?.runtime;
  const bluetoothEnabled = runtime.snapshot?.bluetooth.desiredEnabled;
  const bluetoothState = runtime.snapshot?.bluetooth.state.tag;
  const canBeApplied =
    ready &&
    nodeState === Bindings.DevelopmentNodeRuntime.Running &&
    bluetoothEnabled === true &&
    (bluetoothState === "Connecting" ||
      bluetoothState === "WaitingForPeers" ||
      bluetoothState === "Connected");
  const owner = useMemo(
    () => ({
      generation: runtime.snapshot?.generationId,
      phase: runtime.phase,
      readBluetoothDiscoveryGroups,
      ready,
      nodeState,
      bluetoothEnabled,
      canBeApplied,
    }),
    [
      runtime.snapshot?.generationId,
      runtime.phase,
      readBluetoothDiscoveryGroups,
      ready,
      nodeState,
      bluetoothEnabled,
      canBeApplied,
    ],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const mounted = useRef(false);
  const inFlight = useRef<{ owner: object; operation: number } | null>(null);
  const ticket = useRef(0);
  const [pending, setPending] = useState<object | null>(null);
  const [saved, setSaved] = useState<{
    owner: object;
    settings: Bindings.LocalBluetoothDiscoveryGroupsSettings;
  } | null>(null);
  const [invalidSaved, setInvalidSaved] = useState<{
    owner: object;
    ownerId: bigint;
    generationId: bigint;
    detail: string;
  } | null>(null);
  const [draft, setDraft] = useState<{ owner: object; text: string; edited: boolean } | null>(null);
  const [proposal, setProposal] = useState<{
    owner: object;
    input: Bindings.SetBluetoothDiscoveryGroupsInput;
  } | null>(null);
  const [feedback, setFeedback] = useState<{ owner: object; text: string } | null>(null);
  const settings = saved?.owner === owner ? saved.settings : null;
  const repair = invalidSaved?.owner === owner ? invalidSaved : null;
  const text = draft?.owner === owner ? draft.text : "";
  const confirmation = proposal?.owner === owner ? proposal : null;
  const busy = pending === owner;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      ticket.current += 1;
      inFlight.current = null;
    };
  }, []);

  const execute = useCallback(
    async (input?: Bindings.SetBluetoothDiscoveryGroupsInput) => {
      if (!ready || inFlight.current?.owner === owner) return;
      const operation = ++ticket.current;
      const active = { owner, operation };
      inFlight.current = active;
      const current = () =>
        mounted.current && currentOwner.current === owner && operation === ticket.current;
      const publish = (
        next: Bindings.LocalBluetoothDiscoveryGroupsSettings,
        resetDraft: boolean,
      ) => {
        setSaved({ owner, settings: next });
        setInvalidSaved(null);
        setDraft((previous) =>
          !resetDraft && previous?.owner === owner && previous.edited
            ? previous
            : { owner, text: next.groups.join("\n"), edited: false },
        );
      };
      const publishInvalid = (invalid: {
        ownerId: bigint;
        generationId: bigint;
        detail: string;
      }) => {
        setSaved(null);
        setInvalidSaved({ owner, ...invalid });
        setFeedback(null);
      };
      setPending(owner);
      setProposal(null);
      setFeedback(null);
      try {
        const result =
          input === undefined
            ? await readBluetoothDiscoveryGroups()
            : await setBluetoothDiscoveryGroups(input);
        if (!current()) return;
        if (result.type === "outcome" && result.outcome.tag === "Ready") {
          publish(result.outcome.inner.settings, input !== undefined);
          return;
        }
        if (result.type === "outcome" && result.outcome.tag === "Invalid") {
          setFeedback({ owner, text: result.outcome.inner.detail });
          return;
        }
        if (result.type === "outcome" && result.outcome.tag === "InvalidSaved") {
          publishInvalid(result.outcome.inner);
          return;
        }
        const uncertain = input !== undefined;
        setSaved(null);
        setInvalidSaved(null);
        setFeedback({
          owner,
          text: uncertain
            ? "The save result could not be confirmed. Checking the saved groups before another change."
            : "Discovery groups could not be loaded. Refresh to try again.",
        });
        if (uncertain) {
          // A timed-out caller cannot assume a native-owned settings write failed.
          const reread = await readBluetoothDiscoveryGroups();
          if (!current()) return;
          if (reread.type === "outcome" && reread.outcome.tag === "Ready") {
            publish(reread.outcome.inner.settings, false);
            setFeedback({
              owner,
              text: "The save result was not confirmed. The saved groups below have been checked; review them before trying again.",
            });
          } else if (reread.type === "outcome" && reread.outcome.tag === "InvalidSaved") {
            publishInvalid(reread.outcome.inner);
          } else {
            setFeedback({
              owner,
              text: "The save result is still unconfirmed. Refresh before trying another change.",
            });
          }
        }
      } catch {
        if (current()) {
          setSaved(null);
          setInvalidSaved(null);
          setFeedback({
            owner,
            text: "Discovery groups could not be confirmed. Refresh before trying another change.",
          });
        }
      } finally {
        if (inFlight.current === active) inFlight.current = null;
        if (current()) setPending(null);
      }
    },
    [owner, ready, readBluetoothDiscoveryGroups, setBluetoothDiscoveryGroups],
  );

  useEffect(() => {
    if (ready) void execute();
  }, [execute, ready]);

  const propose = (groups: string[]) => {
    const fences = settings ?? repair;
    if (!ready || busy || fences === null) return;
    setProposal({
      owner,
      input: { ownerId: fences.ownerId, generationId: fences.generationId, groups },
    });
  };
  const applied =
    canBeApplied &&
    settings?.appliedGroups !== undefined &&
    settings.groups.length === settings.appliedGroups.length &&
    settings.groups.every((group, index) => group === settings.appliedGroups?.[index]);
  const stopped =
    runtime.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Stopped ||
    runtime.snapshot?.bluetooth.desiredEnabled === false;

  return (
    <>
      <BodyText muted>
        Nearby devices need a discovery group in common. These names do not encrypt connections or
        grant access to a node.
      </BodyText>
      {settings === null ? (
        repair === null ? (
          <BodyText>Discovery groups not loaded.</BodyText>
        ) : (
          <>
            <BodyText>{repair.detail}</BodyText>
            <BodyText>
              Saved discovery groups could not be read. Restore the default to repair them.
            </BodyText>
          </>
        )
      ) : (
        <>
          <KeyValue label="Saved discovery groups" value={settings.groups.join(", ")} />
          <KeyValue
            label="Currently applied groups"
            value={
              (canBeApplied ? settings.appliedGroups?.join(", ") : undefined) ?? "Not confirmed"
            }
          />
          <BodyText>
            {applied
              ? "Saved and applied to Bluetooth. This does not confirm a connection."
              : stopped
                ? "Saved for the next Bluetooth start."
                : "Saved. Application to Bluetooth is not yet confirmed; refresh to check."}
          </BodyText>
        </>
      )}
      <TextField
        label="Discovery groups (one per line)"
        value={text}
        onChangeText={(value) => setDraft({ owner, text: value, edited: true })}
        editable={ready && !busy && settings !== null && confirmation === null}
        autoCapitalize="none"
        autoCorrect={false}
        multiline
        maxLength={132}
      />
      <BodyText muted>Enter one to four distinct names, one per line. Default: reticulum.</BodyText>
      <ActionRow>
        <Button
          tone="secondary"
          disabled={!ready || busy || settings === null || confirmation !== null}
          onPress={() => propose(text === "" ? [] : text.split("\n"))}
        >
          Save discovery groups
        </Button>
        <Button
          tone="secondary"
          disabled={
            !ready || busy || (settings === null && repair === null) || confirmation !== null
          }
          onPress={() => propose(["reticulum"])}
        >
          Restore default discovery group
        </Button>
      </ActionRow>
      {confirmation === null ? null : (
        <>
          <BodyText>
            Changing discovery groups disconnects nearby devices as Bluetooth reconnects. Set a
            matching group on your board first or you may lose access to it. Your identity and
            messages stay saved.
          </BodyText>
          <KeyValue label="New discovery groups" value={confirmation.input.groups.join(", ")} />
          <ActionRow>
            <Button
              tone="destructive"
              disabled={!ready || busy}
              onPress={() => void execute(confirmation.input)}
            >
              Save and reconnect
            </Button>
            <Button tone="secondary" disabled={busy} onPress={() => setProposal(null)}>
              Cancel
            </Button>
          </ActionRow>
        </>
      )}
      {feedback?.owner === owner ? <BodyText>{feedback.text}</BodyText> : null}
      <Button tone="secondary" disabled={!ready || busy} onPress={() => void execute()}>
        {busy ? "Checking discovery groups…" : "Refresh discovery groups"}
      </Button>
    </>
  );
}
