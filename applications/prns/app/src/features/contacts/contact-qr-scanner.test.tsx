import { act, fireEvent, render } from "@testing-library/react-native";
import { AppState, Linking } from "react-native";
import { ContactQrScanner } from "./contact-qr-scanner";

let mockPermission = { granted: true, canAskAgain: true };
const mockRequestPermission = jest.fn(async () => mockPermission);
let mockForeground: ((state: "active" | "background" | "inactive") => void) | undefined;
let mockFocused = true;
jest.mock("expo-camera", () => ({
  CameraView: (props: object) => {
    const { View } = jest.requireActual<typeof import("react-native")>("react-native");
    return <View {...props} />;
  },
  useCameraPermissions: () => [mockPermission, mockRequestPermission],
}));
jest.mock("expo-router", () => ({
  useFocusEffect: (effect: () => undefined | (() => void)) => {
    const { useEffect } = jest.requireActual<typeof import("react")>("react");
    useEffect(() => (mockFocused ? effect() : undefined), [effect, mockFocused]);
  },
}));

beforeEach(() => {
  jest.clearAllMocks();
  mockPermission = { granted: true, canAskAgain: true };
  mockFocused = true;
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" });
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    mockForeground = listener;
    return { remove: jest.fn() };
  });
});
afterEach(() => jest.restoreAllMocks());

it("requests permission on demand and keeps paste available after denial", async () => {
  mockPermission = { granted: false, canAskAgain: true };
  const view = render(<ContactQrScanner onScan={jest.fn()} onCancel={jest.fn()} />);
  expect(mockRequestPermission).not.toHaveBeenCalled();
  expect(view.queryByTestId("contact-qr-camera")).toBeNull();
  await act(async () => fireEvent.press(view.getByText("Allow camera")));
  expect(mockRequestPermission).toHaveBeenCalledTimes(1);
  expect(view.getByText(/paste a contact link instead/)).toBeTruthy();
});

it("offers settings for a permanent denial without requesting microphone access", async () => {
  mockPermission = { granted: false, canAskAgain: false };
  const openSettings = jest.spyOn(Linking, "openSettings").mockResolvedValue();
  const view = render(<ContactQrScanner onScan={jest.fn()} onCancel={jest.fn()} />);
  await act(async () => fireEvent.press(view.getByText("Open camera settings")));
  expect(openSettings).toHaveBeenCalledTimes(1);
  expect(mockRequestPermission).not.toHaveBeenCalled();
});

it("accepts one QR frame and ignores non-QR or repeated callbacks", () => {
  const scan = jest.fn();
  const view = render(<ContactQrScanner onScan={scan} onCancel={jest.fn()} />);
  const camera = view.getByTestId("contact-qr-camera");
  expect(camera.props.barcodeScannerSettings).toEqual({ barcodeTypes: ["qr"] });
  const frame = camera.props.onBarcodeScanned as (value: { type: string; data: string }) => void;
  act(() => {
    frame({ type: "ean13", data: "123" });
    frame({ type: "qr", data: "rns-contact://v1/YWJj" });
    frame({ type: "qr", data: "rns-contact://v1/YWJj" });
  });
  expect(scan).toHaveBeenCalledTimes(1);
});

it("unmounts the camera in the background and on blur, rejecting queued frames", () => {
  const scan = jest.fn();
  const cancel = jest.fn();
  const view = render(<ContactQrScanner onScan={scan} onCancel={cancel} />);
  const frame = view.getByTestId("contact-qr-camera").props.onBarcodeScanned;
  act(() => mockForeground?.("background"));
  expect(view.queryByTestId("contact-qr-camera")).toBeNull();
  act(() => frame({ type: "qr", data: "late" }));
  expect(scan).not.toHaveBeenCalled();
  act(() => mockForeground?.("active"));
  expect(view.getByTestId("contact-qr-camera")).toBeTruthy();
  act(() => frame({ type: "qr", data: "old camera frame after resume" }));
  expect(scan).not.toHaveBeenCalled();
  mockFocused = false;
  view.rerender(<ContactQrScanner onScan={scan} onCancel={cancel} />);
  expect(view.queryByTestId("contact-qr-camera")).toBeNull();
  act(() => frame({ type: "qr", data: "late" }));
  expect(scan).not.toHaveBeenCalled();
});

it("rejects a frame queued during cancellation or after unmount", () => {
  const scan = jest.fn();
  const cancel = jest.fn();
  const view = render(<ContactQrScanner onScan={scan} onCancel={cancel} />);
  const frame = view.getByTestId("contact-qr-camera").props.onBarcodeScanned;
  fireEvent.press(view.getByText("Cancel scanning"));
  act(() => frame({ type: "qr", data: "late" }));
  expect(cancel).toHaveBeenCalledTimes(1);
  expect(scan).not.toHaveBeenCalled();
  view.unmount();
  act(() => frame({ type: "qr", data: "later" }));
  expect(scan).not.toHaveBeenCalled();
});
