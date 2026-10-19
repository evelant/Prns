import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import NotFoundRoute from "../../../app/+not-found";
import BootstrapRoute from "../../../app/index";
const mockInspectDevelopmentIdentity = jest.fn();
const mockReplace = jest.fn();
let mockScaffoldStatus = "ready";
jest.mock("expo-router", () => ({
  useRouter: () => ({ replace: mockReplace }),
}));
jest.mock("@/native/runtime-provider", () => ({
  runtimeProvider: {
    availability: { type: "available", platform: "ios" },
    runtime: {
      inspectDevelopmentIdentity: () => mockInspectDevelopmentIdentity(),
    },
  },
}));
jest.mock("@/state/scaffold-state-context", () => ({
  useScaffoldState: () => ({ status: mockScaffoldStatus }),
}));
describe("bootstrap identity routing", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockScaffoldStatus = "ready";
  });
  it.each([
    [Bindings.PrimaryIdentityState.Missing.new(), "/onboarding/welcome"],
    [
      Bindings.PrimaryIdentityState.Present.new({
        identityHash: new Uint8Array(16),
      }),
      "/inbox",
    ],
    [
      Bindings.PrimaryIdentityState.Unavailable.new({
        detail: "identity directory is locked",
      }),
      "/recovery",
    ],
    [
      Bindings.PrimaryIdentityState.DevelopmentResetRequired.new({
        reason: "primary identity has 63 bytes",
      }),
      "/recovery",
    ],
  ] as const)("routes %s without collapsing identity state", async (identity, destination) => {
    mockInspectDevelopmentIdentity.mockResolvedValue(identity);
    const view = render(<BootstrapRoute />);
    await waitFor(() => expect(mockReplace).toHaveBeenCalledWith(destination));
    view.unmount();
  });
  it("routes bridge inspection failure to visible recovery", async () => {
    mockInspectDevelopmentIdentity.mockRejectedValue(new Error("native bridge detached"));
    const view = render(<BootstrapRoute />);
    await waitFor(() => expect(mockReplace).toHaveBeenCalledWith("/recovery"));
    view.unmount();
  });

  it("waits for saved app state before selecting a launch destination", async () => {
    mockScaffoldStatus = "loading";
    mockInspectDevelopmentIdentity.mockResolvedValue(
      Bindings.PrimaryIdentityState.Present.new({ identityHash: new Uint8Array(16) }),
    );
    const view = render(<BootstrapRoute />);
    expect(mockInspectDevelopmentIdentity).not.toHaveBeenCalled();
    expect(mockReplace).not.toHaveBeenCalled();

    mockScaffoldStatus = "ready";
    view.rerender(<BootstrapRoute />);
    await waitFor(() => expect(mockReplace).toHaveBeenCalledWith("/inbox"));
  });

  it("does not replace a newly opened destination after bootstrap unmounts", async () => {
    let completeInspection: ((value: Bindings.PrimaryIdentityState) => void) | undefined;
    mockInspectDevelopmentIdentity.mockReturnValue(
      new Promise<Bindings.PrimaryIdentityState>((resolve) => {
        completeInspection = resolve;
      }),
    );
    const view = render(<BootstrapRoute />);
    view.unmount();
    await act(async () => {
      completeInspection?.(
        Bindings.PrimaryIdentityState.Present.new({ identityHash: new Uint8Array(16) }),
      );
    });
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("returns obsolete or unknown links through identity-aware bootstrap", () => {
    const view = render(<NotFoundRoute />);
    fireEvent.press(view.getByRole("button", { name: "Return to a safe screen" }));
    expect(mockReplace).toHaveBeenCalledWith("/");
  });
});
