import { runtimeProvider } from "@/native/runtime-provider";
import { Disclosure } from "@/ui/disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { BodyText, Card, CardStack, Screen, ScreenHeading, Subheading } from "@/ui/primitives";

export function HelpScreen() {
  const available = runtimeProvider.availability.type === "available";
  return (
    <Screen>
      <ScreenHeading>Help</ScreenHeading>
      {available ? (
        <>
          <CardStack>
            <Subheading>Get started</Subheading>
            <NavigationLink href="/setup">Connect this phone</NavigationLink>
            <NavigationLink href="/contacts">Share or add a contact</NavigationLink>
            <NavigationLink href="/inbox/group/new">Start a group</NavigationLink>
          </CardStack>
          <CardStack>
            <Subheading>Manage</Subheading>
            <NavigationLink href="/more/identities">Manage identities</NavigationLink>
            <NavigationLink href="/nodes">Manage nodes</NavigationLink>
            <NavigationLink href="/more/interfaces">Interface settings</NavigationLink>
            <Card>
              <Disclosure label="Using named identities">
                <BodyText>
                  Choose an identity from the top of the screen. Contacts and conversations stay
                  with their original identity, and other active identities can keep receiving.
                </BodyText>
                <BodyText>
                  Pausing stops messaging for that identity while preserving its history. Resume it
                  from Identities to continue.
                </BodyText>
              </Disclosure>
            </Card>
          </CardStack>
          <CardStack>
            <Subheading>Troubleshooting</Subheading>
            <NavigationLink href="/more/activity">Inspect the network</NavigationLink>
            <NavigationLink href="/more/settings/about">
              Build details and diagnostics
            </NavigationLink>
            <Card>
              <Disclosure label="A message is waiting">
                <BodyText>
                  Keep prns open on both phones and check Interfaces. Saved messages retry when a
                  path becomes available; a receipt confirms that a device received them.
                </BodyText>
                <BodyText>
                  Open Delivery details in the conversation to inspect progress. Keep the original
                  message instead of sending a replacement or clearing app data.
                </BodyText>
                <NavigationLink href="/more/interfaces">Check interfaces</NavigationLink>
              </Disclosure>
            </Card>
            <Card>
              <Disclosure label="Maintainer demo limits">
                <BodyText>
                  Use test messages and keep prns open. Background delivery, protected phone storage
                  and backup recovery are not ready.
                </BodyText>
                <BodyText>
                  Use one prns app variant with Bluetooth enabled per phone and the same discovery
                  group on nearby phones.
                </BodyText>
                <BodyText>
                  Each identity supports four groups, with up to eight devices per group. Reuse a
                  demo group; removal and ownership transfer are not available.
                </BodyText>
                <BodyText>
                  Enroll everyone before sending. New members receive future messages, and adding
                  members can interrupt unfinished delivery.
                </BodyText>
              </Disclosure>
            </Card>
          </CardStack>
        </>
      ) : (
        <>
          <BodyText>
            This web preview shows the app layout. Use the iOS or Android app to set up identities,
            connect nodes and exchange messages.
          </BodyText>
          <NavigationLink href="/more/settings/about">Build details and diagnostics</NavigationLink>
        </>
      )}
    </Screen>
  );
}
