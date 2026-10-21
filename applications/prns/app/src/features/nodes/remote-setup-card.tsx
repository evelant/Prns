import * as Bindings from "@prns-internal/expo";
import { useEffect, useRef, useState } from "react";

import {
  type DevelopmentRuntimeView,
  useDevelopmentRuntime,
} from "@/native/development-runtime-context";
import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Button, Card, KeyValue, Subheading } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { formatBytes, formatControllerAuthority, formatRequestKind } from "./format";

type Pending = "export" | "preview" | "import" | "check";
type Preview = { readonly text: string; readonly value: Bindings.TargetAccessCardPreview };
type Saved = {
  readonly target: Uint8Array;
  readonly permissions: readonly Bindings.RemoteControlRequestKind[];
};

export function RemoteSetupCard() {
  const runtime = useDevelopmentRuntime();
  const controller = runtime.snapshot?.controllerIdentityFingerprint;
  const owner = `${controller === undefined ? "missing" : formatBytes(controller)}:${runtime.snapshot?.generationId ?? "none"}:${runtime.phase}:${runtime.snapshot?.runtime}:${runtime.stoppingNode}`;
  return <SetupCardSession key={owner} runtime={runtime} controller={controller} />;
}

function SetupCardSession({
  runtime,
  controller,
}: {
  readonly runtime: DevelopmentRuntimeView;
  readonly controller: Uint8Array | undefined;
}) {
  const [exported, setExported] = useState<Bindings.ControllerSetupCard | null>(null);
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<Preview | null>(null);
  const [saved, setSaved] = useState<Saved | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const [connection, setConnection] = useState<string | null>(null);
  const request = useRef({ active: true, sequence: 0, pending: null as Pending | null });
  const ready =
    controller !== undefined &&
    runtime.phase === "ready" &&
    !runtime.stoppingNode &&
    runtime.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Running;
  const recipientMatches =
    controller !== undefined &&
    preview !== null &&
    formatBytes(preview.value.recipientControllerFingerprint) === formatBytes(controller);

  useEffect(() => {
    const owner = request.current;
    owner.active = true;
    return () => {
      owner.active = false;
      owner.sequence += 1;
    };
  }, []);

  const begin = (operation: Pending) => {
    if (!ready || request.current.pending !== null) return null;
    request.current.pending = operation;
    setPending(operation);
    setFeedback(null);
    return ++request.current.sequence;
  };
  const isCurrent = (sequence: number) =>
    request.current.active && request.current.sequence === sequence;
  const finish = (sequence: number) => {
    if (!isCurrent(sequence)) return false;
    request.current.pending = null;
    setPending(null);
    return true;
  };

  const exportCard = async () => {
    const sequence = begin("export");
    if (sequence === null) return;
    try {
      const result = await runtime.exportControllerSetupCard();
      if (!finish(sequence)) return;
      if (
        result.type === "outcome" &&
        result.outcome.tag === "Ready" &&
        controller !== undefined &&
        formatBytes(result.outcome.inner.card.controllerIdentityFingerprint) ===
          formatBytes(controller)
      ) {
        setExported(result.outcome.inner.card);
      } else setFeedback("Controller details are unavailable. Check that this phone is running.");
    } catch {
      if (finish(sequence)) setFeedback("Controller details could not be loaded. Try again.");
    }
  };

  const changeText = (value: string) => {
    if (request.current.pending === "import") return;
    request.current.sequence += 1;
    request.current.pending = null;
    setPending(null);
    setText(value);
    setPreview(null);
    setSaved(null);
    setUncertain(false);
    setFeedback(null);
    setConnection(null);
  };

  const previewCard = async () => {
    const sequence = begin("preview");
    if (sequence === null) return;
    const card = text.trim();
    setPreview(null);
    setSaved(null);
    setConnection(null);
    try {
      const result = await runtime.previewTargetAccessCard(card);
      if (!finish(sequence)) return;
      if (result.type === "outcome" && result.outcome.tag === "Valid") {
        setPreview({ text: card, value: result.outcome.inner.preview });
      } else
        setFeedback("This access card could not be read. Ask the setup tool for a supported card.");
    } catch {
      if (finish(sequence)) setFeedback("This access card could not be read. Try again.");
    }
  };

  const importCard = async () => {
    if (preview === null || controller === undefined || !recipientMatches || uncertain) return;
    const sequence = begin("import");
    if (sequence === null) return;
    try {
      const result = await runtime.importTargetAccessCard({
        card: preview.text,
        expectedControllerIdentity: controller,
        trustedSourceConfirmed: true,
      });
      if (!finish(sequence)) return;
      if (result.type === "operationFailure") {
        setUncertain(true);
        setFeedback("The save outcome is unknown. Check Nodes before trying to import again.");
      } else if (result.outcome.tag === "Saved" || result.outcome.tag === "Existing") {
        if (
          formatBytes(result.outcome.inner.targetIdentityFingerprint) !==
          formatBytes(preview.value.targetIdentityFingerprint)
        ) {
          setUncertain(true);
          setFeedback("The saved target could not be confirmed. Check Nodes before continuing.");
          return;
        }
        setSaved({
          target: result.outcome.inner.targetIdentityFingerprint,
          permissions: preview.value.permissions,
        });
        setFeedback(null);
      } else {
        setUncertain(result.outcome.tag === "OutcomeUnknown");
        setFeedback(importFailure(result.outcome));
      }
    } catch {
      if (finish(sequence)) {
        setUncertain(true);
        setFeedback("The save outcome is unknown. Check Nodes before trying to import again.");
      }
    }
  };

  const checkConnection = async () => {
    if (saved === null) return;
    const sequence = begin("check");
    if (sequence === null) return;
    setConnection(null);
    try {
      const result = await runtime.describeTarget({ targetIdentityFingerprint: saved.target });
      if (!finish(sequence)) return;
      if (
        result.type === "outcome" &&
        result.outcome.tag === "Described" &&
        result.outcome.inner.snapshot.generationId === runtime.snapshot?.generationId &&
        controller !== undefined &&
        formatBytes(result.outcome.inner.target.controllerIdentityFingerprint) ===
          formatBytes(controller) &&
        formatBytes(result.outcome.inner.target.targetIdentityFingerprint) ===
          formatBytes(saved.target)
      ) {
        setConnection(
          "Connection checked. The node accepted an authenticated information request.",
        );
      } else
        setConnection(
          "Connection could not be checked. Saved access is kept; the node may need a route or local radio setup.",
        );
    } catch {
      if (finish(sequence)) setConnection("Connection could not be checked. Saved access is kept.");
    }
  };

  return (
    <Card>
      <Subheading>Pair using a setup tool</Subheading>
      <BodyText>
        To use a setup tool or an existing administrator, share your public controller details. Ask
        them to authorize this controller and return its target access card.
      </BodyText>
      <Button
        disabled={!ready || pending !== null}
        onPress={() => void exportCard()}
        tone="secondary"
      >
        {pending === "export" ? "Loading controller details…" : "Show my controller details"}
      </Button>
      {exported === null ? null : (
        <>
          <KeyValue
            label="Controller fingerprint"
            value={formatBytes(exported.controllerIdentityFingerprint)}
          />
          <KeyValue label="Controller card" value={exported.text} />
          <KeyValue label="Public key for compatible tools" value={exported.publicKeyHex} />
          <BodyText muted>
            These are public details. Your private key stays on this device.
          </BodyText>
        </>
      )}
      <TextField
        label="Target access card"
        value={text}
        onChangeText={changeText}
        multiline
        autoCapitalize="none"
        autoCorrect={false}
        maxLength={1024}
        editable={ready && pending !== "import"}
        placeholder="Paste the card from your setup tool"
      />
      <Button
        disabled={!ready || pending !== null || text.trim().length === 0 || uncertain}
        onPress={() => void previewCard()}
        tone="secondary"
      >
        {pending === "preview" ? "Reading card…" : "Review access card"}
      </Button>
      {preview === null || saved !== null ? null : (
        <>
          <Badge tone="warning">Source not verified</Badge>
          <KeyValue
            label="Target fingerprint"
            value={formatBytes(preview.value.targetIdentityFingerprint)}
          />
          <KeyValue
            label="Recipient controller"
            value={formatBytes(preview.value.recipientControllerFingerprint)}
          />
          <KeyValue label="Access" value={formatControllerAuthority(preview.value.authority)} />
          {preview.value.permissions.map((permission) => (
            <BodyText key={permission}>{formatRequestKind(permission)}</BodyText>
          ))}
          {preview.value.authority === Bindings.RemoteControlControllerAuthority.Administrator ? (
            <BodyText>
              Administrator access also allows managing and revoking controller grants.
            </BodyText>
          ) : null}
          {recipientMatches ? (
            <>
              <BodyText>
                Verify these details with the setup tool or administrator you trust. Saving this
                card pins the target on this device; it does not confirm the target has granted
                access.
              </BodyText>
              <Button
                disabled={!ready || pending !== null || uncertain}
                onPress={() => void importCard()}
              >
                {pending === "import" ? "Saving access…" : "Trust this source and save access"}
              </Button>
            </>
          ) : (
            <BodyText>
              This card is for a different controller. Ask for a card addressed to this device.
            </BodyText>
          )}
        </>
      )}
      {saved === null ? null : (
        <>
          <Badge>Access saved locally</Badge>
          <BodyText>
            {connection === null ? "Control has not been checked. " : "Your saved access is kept. "}
            You can finish now and keep the current radio settings.
          </BodyText>
          {saved.permissions.includes(Bindings.RemoteControlRequestKind.Describe) ? (
            <Button
              disabled={!ready || pending !== null}
              onPress={() => void checkConnection()}
              tone="secondary"
            >
              {pending === "check" ? "Checking connection…" : "Check connection"}
            </Button>
          ) : null}
          {connection === null ? null : <BodyText>{connection}</BodyText>}
          <NavigationLink
            href={{
              pathname: "/nodes/managed/[nodeId]",
              params: { nodeId: formatBytes(saved.target) },
            }}
          >
            Configure radio or view node settings
          </NavigationLink>
          <NavigationLink href="/nodes">Keep current settings</NavigationLink>
          <NavigationLink href="/inbox">Do this later</NavigationLink>
        </>
      )}
      {feedback === null ? null : <BodyText>{feedback}</BodyText>}
      {uncertain ? <NavigationLink href="/nodes">Check saved nodes</NavigationLink> : null}
    </Card>
  );
}

function importFailure(outcome: Bindings.TargetAccessCardImportOutcome): string {
  switch (outcome.tag) {
    case Bindings.TargetAccessCardImportOutcome_Tags.TrustRequired:
      return "Confirm that you trust the setup tool or administrator before saving.";
    case Bindings.TargetAccessCardImportOutcome_Tags.ControllerChanged:
      return "The controller changed. Export its current details and request a new card.";
    case Bindings.TargetAccessCardImportOutcome_Tags.RecipientMismatch:
      return "This card is for a different controller. Request a card for this device.";
    case Bindings.TargetAccessCardImportOutcome_Tags.Conflict:
      return "Different access is already saved for this target. Review it in Nodes; it has not been replaced.";
    case Bindings.TargetAccessCardImportOutcome_Tags.CapacityExhausted:
      return "There is no space for another saved target. Review your saved nodes first.";
    case Bindings.TargetAccessCardImportOutcome_Tags.Invalid:
      return "This access card is invalid or unsupported.";
    case Bindings.TargetAccessCardImportOutcome_Tags.Busy:
      return "Another node operation is still running. Try again when it finishes.";
    case Bindings.TargetAccessCardImportOutcome_Tags.LocalNodeStopped:
      return "Start this phone’s node before saving access.";
    case Bindings.TargetAccessCardImportOutcome_Tags.PersistenceUnavailable:
      return "Durable storage is unavailable. Access has not been activated.";
    case Bindings.TargetAccessCardImportOutcome_Tags.PersistenceFailed:
      return "Access could not be saved durably. Check this phone’s storage.";
    case Bindings.TargetAccessCardImportOutcome_Tags.Unavailable:
      return "Access import is unavailable on this device.";
    case Bindings.TargetAccessCardImportOutcome_Tags.OutcomeUnknown:
      return "The save outcome is unknown. Check Nodes before trying to import again.";
    case Bindings.TargetAccessCardImportOutcome_Tags.Saved:
    case Bindings.TargetAccessCardImportOutcome_Tags.Existing:
      return "Access is saved locally.";
  }
}
