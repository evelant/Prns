import * as Bindings from "@prns-internal/expo";
import { fireEvent, render, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { OnboardingScreen } from "./onboarding-screen";

const mockBack = jest.fn();
const mockPush = jest.fn();
const mockReplace = jest.fn();
const mockCreateGeneratedIdentity = jest.fn();
const mockCreateImportedIdentity = jest.fn();
const mockPreviewIdentityImport = jest.fn();
const mockGetDocumentAsync = jest.fn();
const mockReadBytes = jest.fn();
const mockDelete = jest.fn();
jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
  useRouter: () => ({
    back: mockBack,
    push: mockPush,
    replace: mockReplace,
  }),
}));
jest.mock("expo-document-picker", () => ({
  getDocumentAsync: (...input: readonly unknown[]) => mockGetDocumentAsync(...input),
}));
jest.mock("expo-file-system", () => ({
  File: class {
    async bytes() {
      return mockReadBytes();
    }
    delete() {
      mockDelete();
    }
  },
}));
jest.mock("@/native/runtime-provider", () => ({
  runtimeProvider: {
    availability: { type: "available", platform: "ios" },
    runtime: {
      createGeneratedIdentity: () => mockCreateGeneratedIdentity(),
      createImportedIdentity: (identity: Uint8Array) => mockCreateImportedIdentity(identity),
      previewIdentityImport: (identity: Uint8Array) => mockPreviewIdentityImport(identity),
    },
  },
}));
beforeEach(() => {
  jest.clearAllMocks();
});
describe("identity onboarding", () => {
  it("continues a new phone into guided connection setup", async () => {
    mockCreateGeneratedIdentity.mockResolvedValue(
      Bindings.IdentityCreationOutcome.Created.new({
        identityHash: new Uint8Array(16).fill(0x11),
      }),
    );
    const view = render(<OnboardingScreen step="create" />);
    fireEvent.press(view.getByRole("button", { name: "Create phone identity" }));
    await waitFor(() =>
      expect(view.getByRole("button", { name: "Connect this phone" })).toBeTruthy(),
    );
    fireEvent.press(view.getByRole("button", { name: "Connect this phone" }));
    expect(mockReplace).toHaveBeenCalledWith("/setup");
  });
  it("previews and imports the same bytes before guided setup", async () => {
    const credential = new Uint8Array(64).fill(0x22);
    const identityHash = new Uint8Array(16).fill(0x33);
    mockGetDocumentAsync.mockResolvedValue({
      canceled: false,
      assets: [{ uri: "file:///identity.raw" }],
    });
    mockReadBytes.mockResolvedValue(credential);
    mockPreviewIdentityImport.mockResolvedValue(
      Bindings.IdentityImportPreviewOutcome.Valid.new({
        identityHash,
      }),
    );
    mockCreateImportedIdentity.mockResolvedValue(
      Bindings.IdentityCreationOutcome.Created.new({
        identityHash,
      }),
    );
    const view = render(<OnboardingScreen step="import" />);
    fireEvent.press(view.getByRole("button", { name: "Choose identity file" }));
    await waitFor(() => expect(view.getByText("33333333333333333333333333333333")).toBeTruthy());
    fireEvent.press(view.getByRole("button", { name: "Confirm and import" }));
    await waitFor(() =>
      expect(view.getByRole("button", { name: "Connect this phone" })).toBeTruthy(),
    );
    fireEvent.press(view.getByRole("button", { name: "Connect this phone" }));
    expect(mockPreviewIdentityImport).toHaveBeenCalledWith(credential);
    expect(mockCreateImportedIdentity).toHaveBeenCalledWith(credential);
    expect(mockGetDocumentAsync).toHaveBeenCalledWith({
      copyToCacheDirectory: true,
      multiple: false,
    });
    expect(mockDelete).toHaveBeenCalledTimes(1);
    expect(mockReplace).toHaveBeenCalledWith("/setup");
  });
});

it("keeps onboarding concise and explains separate identities on demand", () => {
  const welcome = render(<OnboardingScreen step="welcome" />);
  expect(welcome.getByRole("button", { name: "Set up this phone" })).toBeTruthy();
  expect(welcome.queryByText(/connection identity is separate/)).toBeNull();
  fireEvent.press(welcome.getByRole("button", { name: "Existing identity or recovery" }));
  expect(welcome.getByText(/connection identity is separate/)).toBeTruthy();
  welcome.unmount();
  const importing = render(<OnboardingScreen step="import" />);
  expect(importing.queryByText(/This does not import a messaging identity/)).toBeNull();
  fireEvent.press(importing.getByRole("button", { name: "What is imported?" }));
  expect(
    importing.getByText(/This does not import a messaging identity, contacts or conversations/),
  ).toBeTruthy();
});

it("keeps the existing installation identity and continues setup", async () => {
  mockCreateGeneratedIdentity.mockResolvedValue(
    Bindings.IdentityCreationOutcome.AlreadyExists.new(),
  );
  const view = render(<OnboardingScreen step="create" />);
  fireEvent.press(view.getByRole("button", { name: "Create phone identity" }));
  await waitFor(() => expect(view.getByText("Your phone is already set up")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Connect this phone" }));
  expect(mockReplace).toHaveBeenCalledWith("/setup");
  expect(mockCreateImportedIdentity).not.toHaveBeenCalled();
});
