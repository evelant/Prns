import type { Contact, ContactCard } from "@prns-internal/expo";
import { useFocusEffect } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { Keyboard } from "react-native";

import { IdentityPicker } from "@/features/identities/identity-picker";
import { useIdentityRouter } from "@/native/identity-navigation";
import { identityReferenceKey, useNamedIdentity } from "@/native/named-identity-context";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { BodyText, Button, Card, KeyValue, Screen, ScreenHeading } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { isContactCardUriShape, MAX_CONTACT_CARD_URI_LENGTH } from "./contact-card-links";
import { useContactCardSession } from "./contact-card-session";
import { ContactQrScanner } from "./contact-qr-scanner";
import { formatContactHash } from "./format";

export function ContactCardImportScreen({
  initialURI,
  invalid = false,
}: {
  readonly initialURI?: string;
  readonly invalid?: boolean;
}) {
  const identity = useNamedIdentity();
  const router = useIdentityRouter();
  const draft = useRef(initialURI ?? "");
  useEffect(() => {
    draft.current = invalid ? "" : (initialURI ?? "");
  }, [initialURI, invalid]);
  return (
    <Screen>
      <ScreenHeading>Add a contact card</ScreenHeading>
      {identity === null ? null : (
        <IdentityPicker
          label="Save contact to"
          selectedKey={identity.ownerKey}
          onSelect={(chosen) => {
            const owner = identityReferenceKey(chosen.reference);
            if (owner === identity.ownerKey) return;
            router.replace({
              pathname: "/contacts/import",
              // Explicit values also clear retained params from a previous warm link.
              params: {
                owner,
                card: draft.current.slice(0, MAX_CONTACT_CARD_URI_LENGTH + 1),
                invalid: invalid && draft.current === "" ? "1" : "0",
              },
            });
          }}
        />
      )}
      <ContactCardImporter
        {...(initialURI === undefined ? {} : { initialURI })}
        invalid={invalid}
        onDraftChange={(value) => {
          draft.current = value;
        }}
        {...(identity?.identity === undefined || identity.identity === null
          ? {}
          : { saveLabel: `Save to ${identity.identity.label}` })}
      />
      <NavigationLink href="/contacts" direction="back">
        Back to Contacts
      </NavigationLink>
    </Screen>
  );
}

export function ContactCardImporter({
  initialURI,
  invalid = false,
  onImported,
  onCancel,
  onDraftChange,
  saveLabel,
}: {
  readonly initialURI?: string;
  readonly invalid?: boolean;
  readonly onImported?: (card: ContactCard, contact: Contact) => void;
  readonly onCancel?: () => void;
  readonly onDraftChange?: (value: string) => void;
  readonly saveLabel?: string;
}) {
  const session = useContactCardSession();
  // A rejected incoming link starts a fresh journey, even if navigation retained
  // an earlier valid card parameter. Its old preview/save cannot select a contact.
  const acceptedInitialURI = invalid ? undefined : initialURI;
  return (
    <ImportJourney
      key={`${session.key}:${invalid}:${acceptedInitialURI ?? ""}`}
      {...(acceptedInitialURI === undefined ? {} : { initialURI: acceptedInitialURI })}
      invalid={invalid}
      {...(onImported === undefined ? {} : { onImported })}
      {...(onCancel === undefined ? {} : { onCancel })}
      {...(onDraftChange === undefined ? {} : { onDraftChange })}
      {...(saveLabel === undefined ? {} : { saveLabel })}
    />
  );
}

function ImportJourney({
  initialURI,
  invalid,
  onImported,
  onCancel,
  onDraftChange,
  saveLabel,
}: {
  readonly initialURI?: string;
  readonly invalid: boolean;
  readonly onImported?: (card: ContactCard, contact: Contact) => void;
  readonly onCancel?: () => void;
  readonly onDraftChange?: (value: string) => void;
  readonly saveLabel?: string;
}) {
  const session = useContactCardSession();
  const owner = useNamedIdentity()?.identity;
  const draftCallback = useRef(onDraftChange);
  draftCallback.current = onDraftChange;
  const currentSession = useRef(session);
  currentSession.current = session;
  const mounted = useRef(false);
  const focused = useRef(false);
  const operation = useRef(0);
  const busy = useRef(false);
  const [input, setInput] = useState(initialURI ?? "");
  const [card, setCard] = useState<ContactCard | null>(null);
  const [scanning, setScanning] = useState(false);
  const [pending, setPending] = useState(false);
  const [saved, setSaved] = useState(false);
  const [failure, setFailure] = useState<string | null>(
    invalid ? "This contact link is incomplete or uses an unsupported format." : null,
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      operation.current += 1;
      busy.current = false;
    };
  }, []);

  const review = useCallback(
    async (value: string) => {
      if (busy.current || !focused.current) return;
      Keyboard.dismiss();
      setScanning(false);
      setCard(null);
      setSaved(false);
      setInput(value.length <= MAX_CONTACT_CARD_URI_LENGTH ? value : "");
      draftCallback.current?.(value.length <= MAX_CONTACT_CARD_URI_LENGTH ? value : "");
      setFailure(null);
      if (!isContactCardUriShape(value)) {
        setFailure(
          "This is not a supported contact card. Ask the other person to share their contact link or QR code.",
        );
        return;
      }
      if (session.runtime === null) {
        setFailure("Contact cards are available in the iOS and Android app.");
        return;
      }
      busy.current = true;
      const ticket = ++operation.current;
      setPending(true);
      try {
        const outcome = await session.runtime.previewContactCard(value);
        if (
          !mounted.current ||
          !focused.current ||
          currentSession.current !== session ||
          operation.current !== ticket
        )
          return;
        switch (outcome.tag) {
          case "Valid":
            setCard(outcome.inner.card);
            break;
          case "UnsupportedService":
            setFailure("This signed card is for a service this app does not support.");
            break;
          case "Invalid":
            setFailure(
              "This contact card could not be verified. It may be damaged or changed. Ask for a new copy.",
            );
            break;
        }
      } catch {
        if (
          mounted.current &&
          focused.current &&
          currentSession.current === session &&
          operation.current === ticket
        )
          setFailure("The card could not be checked. Try reviewing it again.");
      } finally {
        if (
          mounted.current &&
          focused.current &&
          currentSession.current === session &&
          operation.current === ticket
        ) {
          busy.current = false;
          setPending(false);
        }
      }
    },
    [session],
  );

  useFocusEffect(
    useCallback(() => {
      focused.current = true;
      // External navigation can mount this route before it becomes focused.
      // Preview complete links here, but let owner changes carry unfinished input
      // without treating that selection as a request to validate the draft.
      if (initialURI !== undefined && isContactCardUriShape(initialURI)) void review(initialURI);
      return () => {
        focused.current = false;
        operation.current += 1;
        busy.current = false;
        // Native writes already admitted still complete. Do not select their result
        // in a group journey the user has left; re-reviewing is safe and idempotent.
        if (mounted.current) {
          setPending(false);
          Keyboard.dismiss();
          setScanning(false);
          setCard(null);
          setSaved(false);
        }
      };
    }, [initialURI, review]),
  );

  const save = async () => {
    if (
      busy.current ||
      !focused.current ||
      saved ||
      card === null ||
      session.runtime === null ||
      session.identity === null
    )
      return;
    busy.current = true;
    setPending(true);
    setFailure(null);
    const ticket = ++operation.current;
    try {
      const outcome = await session.runtime.importContactCard({
        uri: card.uri,
        expectedLocalIdentity: session.identity,
      });
      if (
        !mounted.current ||
        !focused.current ||
        currentSession.current !== session ||
        operation.current !== ticket
      )
        return;
      switch (outcome.tag) {
        case "Saved":
        case "Updated":
        case "Existing":
          setSaved(true);
          onImported?.(card, outcome.inner.contact);
          break;
        case "IdentityConflict":
          setFailure(
            "This card conflicts with the identity already saved for this address. The saved contact was not replaced.",
          );
          break;
        case "IdentityChanged":
          setFailure("Your identity changed. Open and review the contact card again.");
          break;
        case "MissingIdentity":
          setFailure("Create your own identity before saving a contact.");
          break;
        case "UnsupportedService":
          setFailure("This signed card is for a service this app does not support.");
          break;
        case "Invalid":
          setFailure("This contact card could not be verified. Ask for a new copy.");
          break;
        case "Busy":
          setFailure("Another contact update is in progress. Try saving this card again.");
          break;
        case "DevelopmentResetRequired":
          setFailure("App storage needs attention. Open recovery before saving this contact.");
          break;
        case "Unavailable":
          setFailure(
            "The save could not be confirmed. Try saving this same card again; it will not create a duplicate contact.",
          );
          break;
      }
    } catch {
      if (
        mounted.current &&
        focused.current &&
        currentSession.current === session &&
        operation.current === ticket
      )
        setFailure(
          "The save could not be confirmed. Try saving this same card again; it will not create a duplicate contact.",
        );
    } finally {
      if (
        mounted.current &&
        focused.current &&
        currentSession.current === session &&
        operation.current === ticket
      ) {
        busy.current = false;
        setPending(false);
      }
    }
  };

  const self =
    card !== null &&
    session.identity !== null &&
    formatContactHash(card.identity) === formatContactHash(session.identity);
  return (
    <Card>
      {onImported === undefined || owner === undefined || owner === null ? null : (
        <BodyText muted>Inviting as {owner.label}</BodyText>
      )}
      {scanning ? (
        <ContactQrScanner
          onScan={(value) => void review(value)}
          onCancel={() => setScanning(false)}
        />
      ) : (
        <>
          <BodyText>Scan a contact QR code or paste a shared contact link.</BodyText>
          <Button
            disabled={pending || session.runtime === null}
            onPress={() => {
              Keyboard.dismiss();
              setCard(null);
              setSaved(false);
              setFailure(null);
              setScanning(true);
            }}
          >
            Scan contact QR
          </Button>
          <TextField
            label="Contact link"
            testID="contact-card-link-input"
            value={input}
            maxLength={MAX_CONTACT_CARD_URI_LENGTH + 1}
            autoCapitalize="none"
            autoCorrect={false}
            editable={!pending}
            onChangeText={(value) => {
              const bounded = value.slice(0, MAX_CONTACT_CARD_URI_LENGTH + 1);
              setInput(bounded);
              draftCallback.current?.(bounded);
              setCard(null);
              setSaved(false);
              setFailure(null);
            }}
          />
          <Button
            disabled={pending || input.trim() === ""}
            tone="secondary"
            onPress={() => void review(input.trim())}
          >
            {pending && card === null ? "Checking card…" : "Review contact card"}
          </Button>
        </>
      )}
      {card === null ? null : (
        <>
          <KeyValue label="Contact name" value={card.displayName || "Unnamed contact"} />
          <BodyText muted>Signature checked. Confirm who shared this card.</BodyText>
          <InfoDisclosure label="Contact identity details">
            <KeyValue label="Device identity" value={formatContactHash(card.identity)} />
            <KeyValue label="Messaging address" value={formatContactHash(card.destination)} />
            <BodyText muted>
              The name was supplied by this identity. Saving does not grant group membership or
              change a saved private name.
            </BodyText>
          </InfoDisclosure>
          {self ? <BodyText>This is your own contact card.</BodyText> : null}
          {saved ? (
            <>
              <BodyText>Contact saved.</BodyText>
              <NavigationLink
                href={{
                  pathname: "/contacts/[destination]",
                  params: { destination: formatContactHash(card.destination) },
                }}
              >
                Open saved contact
              </NavigationLink>
            </>
          ) : (
            <Button
              disabled={pending || self || session.identity === null}
              onPress={() => void save()}
            >
              {pending
                ? "Saving contact…"
                : onImported === undefined
                  ? (saveLabel ?? "Save contact card")
                  : "Save and select contact"}
            </Button>
          )}
        </>
      )}
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      {session.identity === null ? (
        <BodyText>Set up your identity before saving contacts.</BodyText>
      ) : null}
      {onCancel === undefined ? null : (
        <Button tone="secondary" disabled={pending} onPress={onCancel}>
          Back to saved contacts
        </Button>
      )}
    </Card>
  );
}
