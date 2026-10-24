import * as Bindings from "@prns-internal/expo";
import { useRouter } from "expo-router";
import { useState } from "react";
import { Alert } from "react-native";

import { runtimeProvider } from "@/native/runtime-provider";
import { useScaffoldState } from "@/state/scaffold-state-context";
import { NavigationLink } from "@/ui/navigation-link";
import {
  BodyText,
  Button,
  Card,
  CardStack,
  Screen,
  ScreenHeading,
  Subheading,
} from "@/ui/primitives";
import { Disclosure } from "@/ui/disclosure";

export function SettingsScreen() {
  const router = useRouter();
  const { resetDevelopmentData } = useScaffoldState();
  const [resetting, setResetting] = useState(false);
  const [resetFailure, setResetFailure] = useState<string | null>(null);

  const reset = async () => {
    if (!("runtime" in runtimeProvider)) {
      setResetFailure("Reset is not available on this platform yet.");
      return;
    }
    setResetting(true);
    setResetFailure(null);
    try {
      const outcome = await runtimeProvider.runtime.resetDevelopmentData();
      if (outcome.tag === Bindings.DevelopmentNodeStopOutcome_Tags.Failed) {
        setResetFailure("App data could not be reset. Try again.");
        return;
      }
      await resetDevelopmentData();
      router.replace("/onboarding/welcome");
    } catch {
      setResetFailure("App data could not be reset. Try again.");
    } finally {
      setResetting(false);
    }
  };

  const confirmReset = () => {
    Alert.alert(
      "Reset app data?",
      "This permanently removes all identities, contacts, groups, messages, pending actions, paired-node access and app preferences from this device. This cannot be undone.",
      [
        { text: "Cancel", style: "cancel" },
        { text: "Reset", style: "destructive", onPress: () => void reset() },
      ],
    );
  };

  return (
    <Screen>
      <ScreenHeading>Settings</ScreenHeading>
      <CardStack>
        <Subheading>App information</Subheading>
        <NavigationLink href="/more/settings/about">About prns</NavigationLink>
        <NavigationLink href="/more/help">Help</NavigationLink>
      </CardStack>
      <Card>
        <Disclosure label="App data">
          <BodyText muted>
            Reset permanently removes every identity and its contacts, groups, messages and pending
            actions, along with paired-node access and app preferences. This cannot be undone.
          </BodyText>
          <CardStack>
            <Button
              disabled={resetting || runtimeProvider.availability.type !== "available"}
              tone="destructive"
              onPress={confirmReset}
            >
              {resetting ? "Resetting…" : "Reset app data"}
            </Button>
          </CardStack>
          {resetFailure === null ? null : <BodyText>{resetFailure}</BodyText>}
        </Disclosure>
      </Card>
    </Screen>
  );
}
