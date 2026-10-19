import type { IdentityCreationOutcome } from "@prns-internal/expo";
import * as Bindings from "@prns-internal/expo";
import * as DocumentPicker from "expo-document-picker";
import { File } from "expo-file-system";
import { useRouter } from "expo-router";
import { useState } from "react";

import { formatBytes } from "@/features/nodes/format";
import { runtimeProvider } from "@/native/runtime-provider";
import { screenById } from "@/navigation/catalog";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import {
  Badge,
  BodyText,
  Button,
  Card,
  CardStack,
  KeyValue,
  Screen,
  ScreenHeading,
  Subheading,
} from "@/ui/primitives";
import { NotYetImplementedScreen } from "../placeholder-screen";

export type OnboardingStep =
  | "welcome"
  | "create"
  | "import"
  | "retention"
  | "provision"
  | "interfaces"
  | "pairing"
  | "complete";

const implementedSteps: readonly OnboardingStep[] = ["welcome", "create", "import"];

export function isOnboardingStep(value: string): value is OnboardingStep {
  return [
    "welcome",
    "create",
    "import",
    "retention",
    "provision",
    "interfaces",
    "pairing",
    "complete",
  ].includes(value);
}

export function OnboardingScreen({ step }: { readonly step: OnboardingStep }) {
  if (!implementedSteps.includes(step)) {
    return <NotYetImplementedScreen entry={screenById("installation.onboarding")} />;
  }
  if (step === "welcome") {
    return <Welcome />;
  }
  if (step === "create") {
    return <CreateIdentity />;
  }
  return <ImportIdentity />;
}

function Welcome() {
  const router = useRouter();
  return (
    <Screen>
      <Badge>Get started</Badge>
      <ScreenHeading>Welcome to prns</ScreenHeading>
      <BodyText>Connect nearby. Share a contact. Start a group.</BodyText>
      <Button onPress={() => router.push("/onboarding/create")}>Set up this phone</Button>
      <InfoDisclosure label="Existing identity or recovery">
        <BodyText>
          Your phone’s connection identity is separate from the named identities you use for
          messaging.
        </BodyText>
        <NavigationLink href="/onboarding/import">Import a phone identity</NavigationLink>
        <NavigationLink href="/recovery">Recovery options</NavigationLink>
        <BodyText>Key export and backup are not available in this preview.</BodyText>
      </InfoDisclosure>
    </Screen>
  );
}

function CreateIdentity() {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [outcome, setOutcome] = useState<IdentityCreationOutcome | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const unavailable = runtimeProvider.availability.type !== "available";

  const create = async () => {
    if (!("runtime" in runtimeProvider)) {
      return;
    }
    setPending(true);
    setFailure(null);
    try {
      setOutcome(await runtimeProvider.runtime.createGeneratedIdentity());
    } catch {
      setFailure("Identity could not be created. Try again.");
    } finally {
      setPending(false);
    }
  };

  return (
    <Screen>
      <Badge>Phone setup</Badge>
      <ScreenHeading>Make this phone yours</ScreenHeading>
      <BodyText>Create a private identity for this phone’s connections.</BodyText>
      <InfoDisclosure label="About your phone identity">
        <BodyText>
          The key stays on this phone. Your named messaging identities use separate keys and
          addresses.
        </BodyText>
      </InfoDisclosure>
      {unavailable ? <PlatformUnavailable /> : null}
      <CreationResult outcome={outcome} failure={failure} />
      <CardStack>
        {outcome?.tag === Bindings.IdentityCreationOutcome_Tags.Created ||
        outcome?.tag === Bindings.IdentityCreationOutcome_Tags.AlreadyExists ? (
          <Button onPress={() => router.replace("/setup")}>Connect this phone</Button>
        ) : (
          <Button disabled={pending || unavailable} onPress={() => void create()}>
            {pending ? "Creating…" : "Create phone identity"}
          </Button>
        )}
        <Button tone="secondary" onPress={() => router.back()}>
          Back
        </Button>
      </CardStack>
    </Screen>
  );
}

function ImportIdentity() {
  const router = useRouter();
  const [identity, setIdentity] = useState<Uint8Array | null>(null);
  const [previewHash, setPreviewHash] = useState<Uint8Array | null>(null);
  const [pending, setPending] = useState<"pick" | "import" | null>(null);
  const [outcome, setOutcome] = useState<IdentityCreationOutcome | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const unavailable = runtimeProvider.availability.type !== "available";

  const pick = async () => {
    if (!("runtime" in runtimeProvider)) {
      return;
    }
    let cachedCredential: File | null = null;
    setPending("pick");
    setFailure(null);
    setOutcome(null);
    try {
      const selected = await DocumentPicker.getDocumentAsync({
        copyToCacheDirectory: true,
        multiple: false,
      });
      if (selected.canceled) {
        return;
      }
      const asset = selected.assets[0];
      if (asset === undefined) {
        setFailure("No identity file was selected.");
        return;
      }
      cachedCredential = new File(asset.uri);
      const bytes = await cachedCredential.bytes();
      const preview = await runtimeProvider.runtime.previewIdentityImport(bytes);
      if (preview.tag === Bindings.IdentityImportPreviewOutcome_Tags.InvalidLength) {
        setIdentity(null);
        setPreviewHash(null);
        setFailure("The selected file is not a valid Reticulum identity.");
        return;
      }
      setIdentity(bytes);
      setPreviewHash(preview.inner.identityHash);
    } catch {
      setIdentity(null);
      setPreviewHash(null);
      setFailure("The identity file could not be read.");
    } finally {
      if (cachedCredential !== null) {
        try {
          cachedCredential.delete();
        } catch {
          setIdentity(null);
          setPreviewHash(null);
          setFailure("The temporary identity file could not be removed. Choose the file again.");
        }
      }
      setPending(null);
    }
  };

  const confirm = async () => {
    if (!("runtime" in runtimeProvider) || identity === null) {
      return;
    }
    setPending("import");
    setFailure(null);
    try {
      const result = await runtimeProvider.runtime.createImportedIdentity(identity);
      setOutcome(result);
      if (result.tag === Bindings.IdentityCreationOutcome_Tags.Created) {
        setIdentity(null);
      }
    } catch {
      setFailure("Identity could not be imported. Try again.");
    } finally {
      setPending(null);
    }
  };

  return (
    <Screen>
      <Badge>Phone setup</Badge>
      <ScreenHeading>Import a phone identity</ScreenHeading>
      <BodyText>Choose your Reticulum identity file, then confirm it.</BodyText>
      <InfoDisclosure label="What is imported?">
        <BodyText>
          This does not import a messaging identity, contacts or conversations. Named messaging
          identities are managed separately in More → Identities.
        </BodyText>
      </InfoDisclosure>
      {unavailable ? <PlatformUnavailable /> : null}
      {previewHash === null ? null : (
        <Card>
          <Subheading>Confirm node identity</Subheading>
          <KeyValue label="Node identity hash" value={formatBytes(previewHash)} />
        </Card>
      )}
      <CreationResult outcome={outcome} failure={failure} />
      <CardStack>
        {outcome?.tag === Bindings.IdentityCreationOutcome_Tags.Created ||
        outcome?.tag === Bindings.IdentityCreationOutcome_Tags.AlreadyExists ? (
          <Button onPress={() => router.replace("/setup")}>Connect this phone</Button>
        ) : (
          <>
            <Button disabled={pending !== null || unavailable} onPress={() => void pick()}>
              {pending === "pick" ? "Reading…" : "Choose identity file"}
            </Button>
            <Button
              disabled={identity === null || pending !== null}
              onPress={() => void confirm()}
              tone="secondary"
            >
              {pending === "import" ? "Importing…" : "Confirm and import"}
            </Button>
          </>
        )}
        <Button tone="secondary" onPress={() => router.back()}>
          Back
        </Button>
      </CardStack>
    </Screen>
  );
}

function CreationResult({
  failure,
  outcome,
}: {
  readonly failure: string | null;
  readonly outcome: IdentityCreationOutcome | null;
}) {
  if (failure !== null) {
    return (
      <Card>
        <Badge tone="warning">Identity action failed</Badge>
        <BodyText>{failure}</BodyText>
      </Card>
    );
  }
  if (outcome === null) {
    return null;
  }
  switch (outcome.tag) {
    case Bindings.IdentityCreationOutcome_Tags.Created:
      return (
        <Card>
          <Badge>Phone identity ready</Badge>
          <InfoDisclosure label="Phone identity details">
            <KeyValue label="Node identity hash" value={formatBytes(outcome.inner.identityHash)} />
          </InfoDisclosure>
        </Card>
      );
    case Bindings.IdentityCreationOutcome_Tags.AlreadyExists:
      return (
        <Card>
          <Badge>Your phone is already set up</Badge>
          <BodyText>Continue with your existing identity.</BodyText>
        </Card>
      );
    case Bindings.IdentityCreationOutcome_Tags.InvalidLength:
      return (
        <Card>
          <Badge tone="warning">Invalid identity file</Badge>
          <BodyText>Select a valid Reticulum identity file.</BodyText>
        </Card>
      );
    case Bindings.IdentityCreationOutcome_Tags.Unavailable:
      return (
        <Card>
          <Badge tone="warning">Identity storage unavailable</Badge>
          <BodyText>Identity storage is unavailable. Try again.</BodyText>
        </Card>
      );
    case Bindings.IdentityCreationOutcome_Tags.DevelopmentResetRequired:
      return (
        <Card>
          <Badge tone="warning">App reset required</Badge>
          <BodyText>Reset app data before continuing identity setup.</BodyText>
        </Card>
      );
  }
}

function PlatformUnavailable() {
  return (
    <Card>
      <Badge tone="warning">Identity setup unavailable</Badge>
      <BodyText>Identity setup is not available on this platform yet.</BodyText>
    </Card>
  );
}
