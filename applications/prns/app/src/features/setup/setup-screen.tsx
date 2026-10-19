import * as Bindings from "@prns-internal/expo";
import { useRouter } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { identityReferenceKey, useNamedIdentity } from "@/native/named-identity-context";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Button, Card, CardHeader, Screen, ScreenHeading } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { NearbySetupStep } from "./nearby-setup";

type Stage = "nearby" | "identity" | "complete";
export function SetupScreen() {
  const runtime = useDevelopmentRuntime();
  const identities = useNamedIdentity();
  const [stage, setStage] = useState<Stage>("nearby");
  const [completedOwner, setCompletedOwner] = useState<string | null>(null);
  const running =
    runtime.phase === "ready" &&
    !runtime.stoppingNode &&
    runtime.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Running;
  const missing = runtime.snapshot?.primaryIdentity.tag === "Missing";
  return (
    <Screen>
      <Badge>
        {!running
          ? "1 of 3 · This phone"
          : stage === "nearby"
            ? "2 of 3 · Connections"
            : stage === "identity"
              ? "3 of 3 · Your identity"
              : "Setup complete"}
      </Badge>
      <ScreenHeading>
        {stage === "complete" && running ? "Ready to say hello" : "Let’s get connected"}
      </ScreenHeading>
      {!running ? (
        <Card>
          <CardHeader title="Start this phone" />
          <BodyText>
            {runtime.phase === "starting"
              ? "Getting your phone ready…"
              : missing
                ? "Create this phone’s identity to continue."
                : "Your phone needs to be running to connect and open your identities."}
          </BodyText>
          {missing ? (
            <NavigationLink href="/onboarding/create">Set up this phone</NavigationLink>
          ) : (
            <Button
              disabled={!runtime.canStartNode || runtime.stoppingNode}
              onPress={runtime.startNode}
            >
              {runtime.phase === "starting" ? "Starting…" : "Start this phone"}
            </Button>
          )}
          {runtime.phase === "failed" || runtime.lifecycleFailure !== null ? (
            <BodyText>This phone could not start. Try again.</BodyText>
          ) : null}
          <InfoDisclosure label="Connection details">
            <BodyText>
              Starting the shared node keeps your existing identities, connections and paired radio
              nodes.
            </BodyText>
            {runtime.lifecycleFailure === null ? null : (
              <BodyText>{runtime.lifecycleFailure}</BodyText>
            )}
            <NavigationLink href="/nodes/local">This phone’s details</NavigationLink>
          </InfoDisclosure>
        </Card>
      ) : stage === "nearby" ? (
        <NearbySetupStep onContinue={() => setStage("identity")} />
      ) : stage === "identity" ? (
        <>
          {identities?.catalog === null || identities === null ? (
            <Card>
              <BodyText>
                {identities?.failure
                  ? "Your identities could not be loaded."
                  : "Loading your identities…"}
              </BodyText>
              <Button tone="secondary" onPress={() => void identities?.refresh()}>
                Try again
              </Button>
            </Card>
          ) : (
            <IdentitySetup
              key={identities.ownerKey}
              onComplete={(owner) => {
                setCompletedOwner(owner);
                setStage("complete");
              }}
            />
          )}
          <Button tone="secondary" onPress={() => setStage("nearby")}>
            Back to connections
          </Button>
        </>
      ) : (
        <Card>
          <CardHeader title="Meet your first contact" />
          <BodyText>Share your contact card, or scan theirs to start a conversation.</BodyText>
          <NavigationLink
            href={{
              pathname: "/contacts/card",
              params: { owner: completedOwner ?? identities?.ownerKey },
            }}
          >
            Share my contact
          </NavigationLink>
          <NavigationLink
            href={{
              pathname: "/contacts/import",
              params: { owner: completedOwner ?? identities?.ownerKey },
            }}
          >
            Scan a contact
          </NavigationLink>
          <NavigationLink
            href={{
              pathname: "/inbox/group/new",
              params: { owner: completedOwner ?? identities?.ownerKey },
            }}
          >
            Create a group
          </NavigationLink>
          <InfoDisclosure label="Inviting people">
            <BodyText>
              Create a group, then use Invite to add a saved contact or scan their card. Keep both
              phones open for your first exchange.
            </BodyText>
          </InfoDisclosure>
        </Card>
      )}
      <NavigationLink href="/inbox">
        {stage === "complete" ? "Open inbox" : "Finish later"}
      </NavigationLink>
    </Screen>
  );
}

function IdentitySetup({ onComplete }: { readonly onComplete: (owner: string) => void }) {
  const router = useRouter();
  const identities = useNamedIdentity();
  const runtime = identities?.runtime;
  const identity = identities?.identity;
  const [name, setName] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [pending, setPending] = useState(false);
  const [selecting, setSelecting] = useState(false);
  const busy = pending || selecting;
  const [failure, setFailure] = useState<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const load = useCallback(async () => {
    if (!runtime) return;
    setFailure(null);
    try {
      const result = await runtime.readMessagingProfile();
      if (!mounted.current) return;
      if (result.tag === "Ready" || result.tag === "SavedButNotApplied") {
        setName(result.inner.profile.displayName);
        setLoaded(true);
      } else setFailure("Your name could not be loaded. Try again.");
    } catch {
      if (mounted.current) setFailure("Your name could not be loaded. Try again.");
    }
  }, [runtime]);
  // This component is keyed by immutable owner; a late reply cannot retarget it.
  useEffect(() => {
    void load();
  }, [load]);
  const save = async () => {
    if (!runtime || !identities || busy || !loaded) return;
    setPending(true);
    setFailure(null);
    try {
      const result = await runtime.setMessagingName(name.trim());
      if (!mounted.current) return;
      if (result.tag === "Ready") {
        onComplete(identities.ownerKey);
        void identities.refresh();
      } else if (result.tag === "SavedButNotApplied") {
        setFailure("Your name is saved. Try again to finish applying it.");
      } else
        setFailure(
          result.tag === "InvalidInput"
            ? "Choose a shorter name without line breaks."
            : "Your name could not be saved. Try again.",
        );
    } catch {
      if (mounted.current) setFailure("Your name could not be saved. Try again.");
    } finally {
      if (mounted.current) setPending(false);
    }
  };
  const select = async (reference: Bindings.NamedIdentityReference) => {
    if (!identities || busy) return;
    setSelecting(true);
    setFailure(null);
    try {
      const selected = await identities.change(
        reference,
        Bindings.NamedIdentityChange.Select.new(),
      );
      if (!mounted.current) return;
      if (selected) {
        // Setup can be entered with an explicit owner. Change that route owner
        // as well as the default, retaining the current step in this screen.
        router.setParams({ owner: identityReferenceKey(reference) });
      } else setFailure("That identity could not be selected. Try again.");
    } catch {
      if (mounted.current) setFailure("That identity could not be selected. Try again.");
    } finally {
      if (mounted.current) setSelecting(false);
    }
  };
  if (!identities || !identity)
    return (
      <Card>
        <BodyText>Choose an identity to continue.</BodyText>
        <NavigationLink href="/more/identities">Open identities</NavigationLink>
      </Card>
    );
  return (
    <Card>
      <CardHeader title="Choose how you appear" />
      <BodyText>Use {identity.label} for messages and groups.</BodyText>
      {!identities.canPublish ? (
        <Badge tone="warning">
          {identity.transitioning
            ? "Updating identity…"
            : identity.state === Bindings.NamedIdentityState.Paused
              ? "Paused"
              : "Starting identity…"}
        </Badge>
      ) : null}
      {identities.catalog?.identities
        .filter((entry) => entry.state !== Bindings.NamedIdentityState.Retired)
        .map((entry) => {
          const selected = identityReferenceKey(entry.reference) === identities.ownerKey;
          return (
            <Button
              key={identityReferenceKey(entry.reference)}
              tone={selected ? "primary" : "secondary"}
              disabled={busy || identities.loading || selected}
              onPress={() => void select(entry.reference)}
            >
              {selected ? `${entry.label} · Selected` : entry.label}
            </Button>
          );
        })}
      <TextField
        label="Name others see"
        value={name}
        onChangeText={setName}
        editable={loaded && !busy}
        autoCapitalize="words"
        placeholder="Your name (optional)"
      />
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      {!loaded ? (
        <Button tone="secondary" onPress={() => void load()}>
          Reload name
        </Button>
      ) : null}
      {identity.state === Bindings.NamedIdentityState.Paused ? (
        <Button
          disabled={busy || identities.loading || identity.transitioning}
          onPress={() =>
            void identities.change(identity.reference, Bindings.NamedIdentityChange.Resume.new())
          }
        >
          Resume {identity.label}
        </Button>
      ) : null}
      {identities.failure ? <BodyText>{identities.failure}</BodyText> : null}
      <Button disabled={busy || !loaded || !identities.canPublish} onPress={() => void save()}>
        {pending ? "Saving…" : "Save and continue"}
      </Button>
      <InfoDisclosure label="About identities">
        <BodyText>
          Your display name is shared with contacts. Your identity label is private. Each identity
          keeps its own contacts and conversations.
        </BodyText>
        <NavigationLink href="/more/identities">Manage or create an identity</NavigationLink>
      </InfoDisclosure>
    </Card>
  );
}
