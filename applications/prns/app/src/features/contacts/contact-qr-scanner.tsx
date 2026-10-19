import { CameraView, useCameraPermissions } from "expo-camera";
import { useFocusEffect } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, Linking, StyleSheet, View } from "react-native";

import { BodyText, Button } from "@/ui/primitives";

export function ContactQrScanner({
  onScan,
  onCancel,
}: {
  readonly onScan: (value: string) => void;
  readonly onCancel: () => void;
}) {
  const [permission, requestPermission] = useCameraPermissions();
  const [focused, setFocused] = useState(false);
  const [foreground, setForeground] = useState(AppState.currentState === "active");
  const [failure, setFailure] = useState<string | null>(null);
  const [requesting, setRequesting] = useState(false);
  const accepted = useRef(false);
  const active = useRef(false);
  const mounted = useRef(true);
  const cameraEpoch = useRef(0);
  const frameEpoch = cameraEpoch.current;
  active.current = focused && foreground && permission?.granted === true && failure === null;

  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      return () => {
        active.current = false;
        cameraEpoch.current += 1;
        setFocused(false);
      };
    }, []),
  );
  useEffect(() => {
    mounted.current = true;
    const listener = AppState.addEventListener("change", (state) => {
      if (state !== "active") {
        active.current = false;
        cameraEpoch.current += 1;
      }
      setForeground(state === "active");
    });
    return () => {
      mounted.current = false;
      active.current = false;
      listener.remove();
    };
  }, []);
  const request = async () => {
    setRequesting(true);
    setFailure(null);
    try {
      await requestPermission();
    } catch {
      if (mounted.current) setFailure("Camera permission could not be requested. Try again.");
    } finally {
      if (mounted.current) setRequesting(false);
    }
  };
  return (
    <View style={styles.container}>
      <BodyText>Point the camera at a contact QR code. You will review it before saving.</BodyText>
      {permission === null ? <BodyText>Checking camera permission…</BodyText> : null}
      {permission !== null && !permission.granted ? (
        <>
          <BodyText>Camera access is only used while this scanner is open.</BodyText>
          {permission.canAskAgain ? (
            <Button disabled={requesting} onPress={() => void request()}>
              Allow camera
            </Button>
          ) : (
            <Button
              tone="secondary"
              onPress={() =>
                void Linking.openSettings().catch(() =>
                  setFailure("Open this app's camera permission in your phone settings."),
                )
              }
            >
              Open camera settings
            </Button>
          )}
          <BodyText muted>You can cancel and paste a contact link instead.</BodyText>
        </>
      ) : null}
      {permission?.granted && focused && foreground && failure === null ? (
        <CameraView
          testID="contact-qr-camera"
          style={styles.camera}
          facing="back"
          barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
          onMountError={() => setFailure("The camera could not start. Cancel and try again.")}
          onBarcodeScanned={({ data, type }) => {
            if (
              !active.current ||
              accepted.current ||
              type !== "qr" ||
              cameraEpoch.current !== frameEpoch
            )
              return;
            accepted.current = true;
            onScan(data);
          }}
        />
      ) : null}
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      <Button
        tone="secondary"
        onPress={() => {
          accepted.current = true;
          active.current = false;
          onCancel();
        }}
      >
        Cancel scanning
      </Button>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: 12 },
  camera: { width: "100%", height: 280, borderRadius: 12, overflow: "hidden" },
});
