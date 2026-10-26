import type { ContactCard } from "@prns-internal/expo";
import { useFocusEffect } from "expo-router";
import { type ComponentType, useCallback, useRef, useState } from "react";
import { Share, StyleSheet, View } from "react-native";
import QRCode, { type QRCodeProps } from "react-native-qrcode-svg";

// Upstream supports node-qrcode byte segments but its declaration still says string.
const ByteQRCode = QRCode as unknown as ComponentType<
  Omit<QRCodeProps, "value"> & {
    value: { data: number[]; mode: "byte" }[];
  }
>;

import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { BodyText, Button, Card, KeyValue, Screen, ScreenHeading } from "@/ui/primitives";
import { useContactCardSession } from "./contact-card-session";
import { formatContactHash } from "./format";

export function MyContactCardScreen() {
  const session = useContactCardSession();
  return <MyContactCardJourney key={session.key} />;
}

function MyContactCardJourney() {
  const session = useContactCardSession();
  const currentSession = useRef(session);
  currentSession.current = session;
  const active = useRef(false);
  const ticket = useRef(0);
  const [card, setCard] = useState<ContactCard | null>(null);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [qrFailed, setQrFailed] = useState(false);

  const refresh = useCallback(async () => {
    const operation = ++ticket.current;
    setCard(null);
    setFailure(null);
    setQrFailed(false);
    if (session.runtime === null || session.identity === null) return;
    const current = () =>
      active.current && currentSession.current === session && ticket.current === operation;
    setPending(true);
    try {
      const result = await session.runtime.exportContactCard({
        expectedLocalIdentity: session.identity,
      });
      if (!current()) return;
      switch (result.tag) {
        case "Ready":
          setCard(result.inner.card);
          break;
        case "IdentityChanged":
          setFailure("Your identity changed. Reopen your contact card.");
          break;
        case "MissingIdentity":
          setFailure("Create your identity before sharing a contact card.");
          break;
        case "Busy":
          setFailure("Another contact update is in progress. Try again.");
          break;
        case "DevelopmentResetRequired":
          setFailure("App storage needs attention. Open recovery before sharing.");
          break;
        case "Unavailable":
          setFailure("Your contact card could not be loaded. Try again.");
          break;
      }
    } catch {
      if (current()) setFailure("Your contact card could not be loaded. Try again.");
    } finally {
      if (current()) setPending(false);
    }
  }, [session]);

  useFocusEffect(
    useCallback(() => {
      active.current = true;
      void refresh();
      return () => {
        active.current = false;
        ticket.current += 1;
      };
    }, [refresh]),
  );

  const share = async () => {
    if (card === null || !active.current || currentSession.current !== session) return;
    try {
      await Share.share({ message: card.uri });
    } catch {
      if (active.current && currentSession.current === session)
        setFailure("The contact link could not be shared. Try again.");
    }
  };

  return (
    <Screen>
      <ScreenHeading
        trailing={
          <InfoDisclosure label="About your contact card">
            <BodyText>
              This public card includes your name and addresses. Anyone with it can save your
              contact; it does not invite them to a group.
            </BodyText>
            {card === null ? null : (
              <>
                <KeyValue label="Device identity" value={formatContactHash(card.identity)} />
                <KeyValue label="Messaging address" value={formatContactHash(card.destination)} />
              </>
            )}
          </InfoDisclosure>
        }
      >
        My contact card
      </ScreenHeading>
      <Card>
        {pending ? <BodyText>Loading contact card…</BodyText> : null}
        {card === null ? null : (
          <>
            <KeyValue label="Contact name" value={card.displayName || "Unnamed contact"} />
            {qrFailed ? (
              <BodyText>The QR code could not be displayed. You can still share the link.</BodyText>
            ) : (
              <View style={styles.qr} accessible accessibilityLabel="Your public contact QR code">
                <ByteQRCode
                  testID="my-contact-qr"
                  // The canonical URI is ASCII. Byte segments avoid a global TextEncoder requirement on Hermes.
                  value={[
                    {
                      data: Array.from(card.uri, (character) => character.charCodeAt(0)),
                      mode: "byte",
                    },
                  ]}
                  size={224}
                  quietZone={16}
                  ecl="M"
                  color="#000000"
                  backgroundColor="#ffffff"
                  onError={() => setQrFailed(true)}
                />
              </View>
            )}
            <Button onPress={() => void share()}>Share contact link</Button>
          </>
        )}
        {session.runtime === null ? (
          <BodyText>Contact cards are available in the iOS and Android app.</BodyText>
        ) : null}
        {session.identity === null ? (
          <BodyText>Create your identity before sharing a contact card.</BodyText>
        ) : null}
        {failure === null ? null : <BodyText>{failure}</BodyText>}
        {failure === null ? null : (
          <Button tone="secondary" disabled={pending} onPress={() => void refresh()}>
            Reload contact card
          </Button>
        )}
      </Card>
      <NavigationLink
        href={{ pathname: "/contacts", params: { section: "profile" } }}
        direction="back"
      >
        Back to Contacts
      </NavigationLink>
    </Screen>
  );
}

const styles = StyleSheet.create({
  qr: { alignSelf: "center", padding: 16, backgroundColor: "#ffffff", borderRadius: 12 },
});
