import * as Bindings from "@prns-internal/expo";
import { useEffect, useState } from "react";
import {
  identityReferenceKey,
  type NamedIdentity,
  useNamedIdentity,
} from "@/native/named-identity-context";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import {
  ActionRow,
  Badge,
  BodyText,
  Button,
  Card,
  CardHeader,
  KeyValue,
  Screen,
  ScreenHeading,
} from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { IdentityActivityLabel } from "./identity-activity";

export function IdentitiesScreen() {
  const identities = useNamedIdentity();
  const [label, setLabel] = useState("");
  useEffect(() => {
    if (identities?.pendingCreationLabel) setLabel(identities.pendingCreationLabel);
  }, [identities?.pendingCreationLabel]);
  if (identities === null)
    return (
      <Screen>
        <ScreenHeading>Identities</ScreenHeading>
        <BodyText>Identities are available in the installed app.</BodyText>
      </Screen>
    );
  const selected = identities.catalog?.selected;
  const selectedKey = selected === undefined ? null : identityReferenceKey(selected);
  return (
    <Screen>
      <ScreenHeading
        trailing={
          <InfoDisclosure label="About identities">
            <BodyText>
              Your default opens in Inbox and Contacts. Other active identities keep receiving. Each
              identity has separate contacts and conversations.
            </BodyText>
          </InfoDisclosure>
        }
      >
        Identities
      </ScreenHeading>
      {identities.failure === null ? null : (
        <Card>
          <BodyText>{identities.failure}</BodyText>
          <Button tone="secondary" onPress={() => void identities.refresh()}>
            Retry
          </Button>
        </Card>
      )}
      {identities.activityFailure === null || identities.activityFailure === undefined ? null : (
        <Card>
          <BodyText>{identities.activityFailure}</BodyText>
          <Button tone="secondary" onPress={() => void identities.refreshActivity()}>
            Refresh activity
          </Button>
        </Card>
      )}
      {identities.catalog === null ? (
        <BodyText muted>Loading identities…</BodyText>
      ) : (
        identities.catalog.identities.map((identity) => {
          const key = identityReferenceKey(identity.reference);
          return (
            <Card key={key}>
              <CardHeader title={identity.label} />
              <Badge>
                {key === selectedKey ? "Default · " : ""}
                {identity.transitioning
                  ? identity.state === Bindings.NamedIdentityState.Paused
                    ? "Pausing…"
                    : "Starting…"
                  : identity.state === Bindings.NamedIdentityState.Retired
                    ? "Retired"
                    : identity.state === Bindings.NamedIdentityState.Paused
                      ? "Paused"
                      : identity.ready
                        ? "Active"
                        : "Starting"}
              </Badge>
              <IdentityActivityLabel ownerKey={key} />
              <NavigationLink
                href={{ pathname: "/more/identities/[identityId]", params: { identityId: key } }}
              >
                Manage {identity.label}
              </NavigationLink>
              {key === selectedKey ? null : (
                <Button
                  tone="secondary"
                  disabled={
                    identities.loading || identity.state === Bindings.NamedIdentityState.Retired
                  }
                  onPress={() =>
                    void identities.change(
                      identity.reference,
                      Bindings.NamedIdentityChange.Select.new(),
                    )
                  }
                >
                  Use {identity.label} by default
                </Button>
              )}
              <NavigationLink href={{ pathname: "/inbox", params: { owner: key } }}>
                Open {identity.label} Inbox
              </NavigationLink>
              <NavigationLink href={{ pathname: "/contacts", params: { owner: key } }}>
                Open {identity.label} Contacts
              </NavigationLink>
            </Card>
          );
        })
      )}
      <Card>
        <CardHeader title="Create an identity" />
        <BodyText muted>A private name, such as Work or Friends.</BodyText>
        <TextField
          label="Identity name"
          value={label}
          onChangeText={setLabel}
          maxLength={64}
          editable={!identities.loading && identities.pendingCreationLabel === null}
          placeholder="For example, Work or Friends"
        />
        <Button
          disabled={identities.loading || !identities.creationReady || label.trim().length === 0}
          onPress={() =>
            void identities.create(label.trim()).then((created) => {
              if (created) setLabel("");
            })
          }
        >
          {identities.loading
            ? "Saving…"
            : identities.pendingCreationLabel === null
              ? "Create identity"
              : "Finish saved creation"}
        </Button>
      </Card>
      <NavigationLink href="/more" direction="back">
        Back to More
      </NavigationLink>
    </Screen>
  );
}

export function IdentityDetailScreen({ identityId }: { readonly identityId: string }) {
  const identities = useNamedIdentity();
  const identity = identities?.catalog?.identities.find(
    (entry) => identityReferenceKey(entry.reference) === identityId,
  );
  if (identity === undefined)
    return (
      <Screen>
        <ScreenHeading>Identity</ScreenHeading>
        <BodyText>This identity is not available. Choose one from your saved identities.</BodyText>
        <NavigationLink href="/more/identities" direction="back">
          Back to identities
        </NavigationLink>
      </Screen>
    );
  return <IdentityDetail key={identityId} identity={identity} />;
}
function IdentityDetail({ identity }: { readonly identity: NamedIdentity }) {
  const identities = useNamedIdentity();
  const [label, setLabel] = useState(identity.label);
  if (identities === null) return null;
  const paused = identity.state === Bindings.NamedIdentityState.Paused;
  const retired = identity.state === Bindings.NamedIdentityState.Retired;
  const transitionLabel = identity.transitioning ? (paused ? "Pausing…" : "Starting…") : null;
  const actionLabel = identity.transitioning
    ? paused
      ? "Finish pausing"
      : "Finish starting"
    : paused
      ? "Resume identity"
      : "Pause identity";
  const lifecycleChange = identity.transitioning
    ? paused
      ? Bindings.NamedIdentityChange.Pause.new()
      : Bindings.NamedIdentityChange.Resume.new()
    : paused
      ? Bindings.NamedIdentityChange.Resume.new()
      : Bindings.NamedIdentityChange.Pause.new();
  const key = identityReferenceKey(identity.reference);
  return (
    <Screen>
      <ScreenHeading>{identity.label}</ScreenHeading>
      <Badge>
        {transitionLabel ??
          (paused ? "Paused" : retired ? "Retired" : identity.ready ? "Active" : "Starting")}
      </Badge>
      {identities.failure === null ? null : (
        <Card>
          <BodyText>{identities.failure}</BodyText>
        </Card>
      )}
      <Card>
        <CardHeader title="Private name" />
        <InfoDisclosure label="About this name">
          <BodyText>
            Changing this private name does not announce your identity or change the name shared
            with contacts.
          </BodyText>
        </InfoDisclosure>
        <TextField
          label="Identity name"
          value={label}
          onChangeText={setLabel}
          maxLength={64}
          editable={!identities.loading && !retired}
        />
        <Button
          disabled={
            identities.loading ||
            retired ||
            label.trim().length === 0 ||
            label.trim() === identity.label
          }
          onPress={() =>
            void identities.change(
              identity.reference,
              Bindings.NamedIdentityChange.Rename.new({ label: label.trim() }),
            )
          }
        >
          Save name
        </Button>
      </Card>
      <Card>
        <CardHeader
          title={transitionLabel ?? (paused ? "Resume this identity" : "Pause this identity")}
        />
        {identity.transitioning ? (
          <BodyText>{paused ? "Finishing work before pausing…" : "Reconnecting…"}</BodyText>
        ) : null}
        {paused && !identity.transitioning ? (
          <BodyText>Saved contacts, messages and groups remain here.</BodyText>
        ) : null}
        <InfoDisclosure label="About pausing">
          <BodyText>
            Pausing stops sending, receiving and announcing after work in progress settles. Saved
            history and pending deliveries are kept. Other identities and managed nodes continue
            working. Resume to reconnect and continue pending deliveries.
          </BodyText>
        </InfoDisclosure>
        <Button
          disabled={identities.loading || retired}
          tone="secondary"
          onPress={() => void identities.change(identity.reference, lifecycleChange)}
        >
          {actionLabel}
        </Button>
      </Card>
      <ActionRow>
        <NavigationLink href={{ pathname: "/inbox", params: { owner: key } }}>
          Open Inbox
        </NavigationLink>
        <NavigationLink href={{ pathname: "/contacts", params: { owner: key } }}>
          Open Contacts
        </NavigationLink>
      </ActionRow>
      <InfoDisclosure label="Identity details">
        <KeyValue
          label="Public identity"
          value={Array.from(identity.reference.publicHash, (byte) =>
            byte.toString(16).padStart(2, "0"),
          ).join("")}
        />
        <BodyText muted>
          Identity separation does not hide that these identities use the same phone or network
          connection.
        </BodyText>
      </InfoDisclosure>
      <NavigationLink href="/more/identities" direction="back">
        Back to identities
      </NavigationLink>
    </Screen>
  );
}
