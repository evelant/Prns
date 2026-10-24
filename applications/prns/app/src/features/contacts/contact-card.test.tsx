import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import { Link } from "expo-router";
import { type ReactNode, StrictMode } from "react";
import { Share } from "react-native";
import {
  identityReferenceKey,
  type NamedIdentity,
  type useNamedIdentity,
} from "@/native/named-identity-context";
import type { RawRouteParams } from "@/navigation/catalog";
import ImportContactCardRoute from "../../../app/(shell)/contacts/import";
import { ContactCardImporter, ContactCardImportScreen } from "./contact-card-import";
import { MyContactCardScreen } from "./my-contact-card-screen";

const mockCard: Bindings.ContactCard = {
  uri: "rns-contact://v1/YWJj",
  displayName: "Alice",
  identity: new Uint8Array(16).fill(0x55),
  destination: new Uint8Array(16).fill(0x66),
  serviceNameHash: new Uint8Array(10).fill(0x77),
};
const mockContact: Bindings.Contact = {
  destination: mockCard.destination,
  identity: mockCard.identity,
  alias: "Alice",
  announcedName: undefined,
  pinned: false,
  isMessaging: true,
};
const mockPreview = jest.fn<Promise<Bindings.ContactCardPreviewOutcome>, [string]>();
const mockImport = jest.fn<
  Promise<Bindings.ContactCardImportOutcome>,
  [Bindings.ImportContactCardInput]
>();
const mockExport = jest.fn<
  Promise<Bindings.ContactCardExportOutcome>,
  [Bindings.ExportContactCardInput]
>();
const baseRuntime = {
  previewContactCard: mockPreview,
  importContactCard: mockImport,
  exportContactCard: mockExport,
};
let mockRuntime = baseRuntime;
let mockGeneration = 1n;
let mockFocused = true;
let mockIdentity: Uint8Array = new Uint8Array(16).fill(0x11);
let mockNamedIdentity: Pick<
  NonNullable<ReturnType<typeof useNamedIdentity>>,
  "catalog" | "identity" | "ownerKey" | "activity"
> | null = null;
const mockReplace = jest.fn();
let mockRouteParams: RawRouteParams = {};
jest.mock("@/native/identity-navigation", () => ({
  ...jest.requireActual("@/native/identity-navigation"),
  useIdentityRouter: () => ({ replace: mockReplace }),
}));
jest.mock("@/native/named-identity-context", () => ({
  ...jest.requireActual("@/native/named-identity-context"),
  useNamedIdentity: () => mockNamedIdentity,
}));
jest.mock("@/native/contact-runtime-context", () => ({
  useContactRuntime: () => ({ runtime: mockRuntime }),
}));
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    phase: "ready",
    snapshot: {
      generationId: mockGeneration,
      primaryIdentity: { tag: "Present", inner: { identityHash: mockIdentity } },
    },
  }),
}));
jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
  useLocalSearchParams: () => mockRouteParams,
  useRouter: () => ({ replace: mockReplace }),
  useFocusEffect: (effect: () => undefined | (() => void)) => {
    const { useEffect } = jest.requireActual<typeof import("react")>("react");
    useEffect(() => (mockFocused ? effect() : undefined), [effect, mockFocused]);
  },
}));
jest.mock("react-native-qrcode-svg", () => ({
  __esModule: true,
  default: (props: object) => {
    const { View } = jest.requireActual<typeof import("react-native")>("react-native");
    return <View {...props} />;
  },
}));
jest.mock("./contact-qr-scanner", () => ({
  ContactQrScanner: ({
    onScan,
    onCancel,
  }: {
    onScan: (value: string) => void;
    onCancel: () => void;
  }) => {
    const { Button } = jest.requireActual<typeof import("@/ui/primitives")>("@/ui/primitives");
    return (
      <>
        <Button onPress={() => onScan(mockCard.uri)}>Read test QR</Button>
        <Button onPress={onCancel}>Cancel scanning</Button>
      </>
    );
  },
}));

beforeEach(() => {
  jest.clearAllMocks();
  mockIdentity = new Uint8Array(16).fill(0x11);
  mockNamedIdentity = null;
  mockGeneration = 1n;
  mockFocused = true;
  mockRuntime = baseRuntime;
  mockRouteParams = {};
  mockPreview.mockResolvedValue(Bindings.ContactCardPreviewOutcome.Valid.new({ card: mockCard }));
  mockImport.mockResolvedValue(
    Bindings.ContactCardImportOutcome.Saved.new({ contact: mockContact }),
  );
  mockExport.mockResolvedValue(Bindings.ContactCardExportOutcome.Ready.new({ card: mockCard }));
});

function namedImportFixture() {
  const identities: NamedIdentity[] = ["Personal", "Work"].map((label, index) => ({
    reference: {
      incarnation: new Uint8Array(16).fill(7),
      sequence: BigInt(index + 1),
      publicHash: new Uint8Array(16).fill(index + 0x11),
    },
    label,
    publishedName: undefined,
    state: index === 0 ? Bindings.NamedIdentityState.Enabled : Bindings.NamedIdentityState.Paused,
    ready: index === 0,
    transitioning: false,
  }));
  const personal = identities[0];
  const work = identities[1];
  if (personal === undefined || work === undefined) throw new Error("Missing identity fixture");
  const selectOwner = (identity: NamedIdentity) => {
    mockNamedIdentity = {
      catalog: { selected: personal.reference, identities },
      identity,
      ownerKey: identityReferenceKey(identity.reference),
      activity: null,
    };
    mockIdentity = identity.reference.publicHash;
    mockRuntime = { ...baseRuntime };
  };
  selectOwner(personal);
  return { personal, work, selectOwner };
}

it.each([
  ["valid", mockCard.uri, false],
  ["empty", "", false],
  ["partial", "rns-contact://v1/", false],
  ["rejected warm link", "", true],
] as const)(
  "keeps the %s draft usable through the real owner-selection route guard",
  async (_label, draft, invalid) => {
    const { personal, work, selectOwner } = namedImportFixture();
    mockRouteParams = invalid ? { card: mockCard.uri, invalid: "1" } : {};
    const screen = render(<ImportContactCardRoute />);
    if (!invalid) fireEvent.changeText(screen.getByLabelText("Contact link"), draft);
    fireEvent.press(screen.getByRole("button", { name: "Save contact to: Personal" }));
    fireEvent.press(screen.getByRole("radio", { name: "Work, Paused" }));
    const destination = mockReplace.mock.calls.at(-1)?.[0] as {
      pathname: string;
      params: RawRouteParams;
    };
    expect(destination.pathname).toBe("/contacts/import");
    expect(destination.params.owner).toBe(identityReferenceKey(work.reference));
    mockRouteParams = destination.params;
    selectOwner(work);
    screen.rerender(<ImportContactCardRoute />);
    expect(screen.queryByText("This page is not available")).toBeNull();
    expect(screen.getByLabelText("Contact link")).toHaveProp("value", draft);
    expect(mockNamedIdentity?.catalog?.selected).toBe(personal.reference);
    expect(mockImport).not.toHaveBeenCalled();
    if (draft === mockCard.uri) {
      await waitFor(() => expect(screen.getByText("Save to Work")).toBeTruthy());
      await act(async () => fireEvent.press(screen.getByText("Save to Work")));
      expect(mockImport).toHaveBeenCalledWith({
        uri: mockCard.uri,
        expectedLocalIdentity: work.reference.publicHash,
      });
    } else {
      expect(mockPreview).not.toHaveBeenCalled();
      expect(screen.queryByText("Save to Work")).toBeNull();
      expect(screen.queryByText(/This is not a supported contact card/)).toBeNull();
      if (invalid)
        expect(
          screen.getByText("This contact link is incomplete or uses an unsupported format."),
        ).toBeTruthy();
      else {
        expect(screen.queryByText(/This contact link is incomplete/)).toBeNull();
        if (draft !== "") {
          fireEvent.press(screen.getByText("Review contact card"));
          expect(screen.getByText(/This is not a supported contact card/)).toBeTruthy();
          expect(mockPreview).not.toHaveBeenCalled();
        }
      }
    }
  },
);

it("keeps a contact card when choosing a paused owner and saves only to that explicit owner", async () => {
  const { personal, work, selectOwner } = namedImportFixture();
  const screen = render(<ContactCardImportScreen initialURI={mockCard.uri} />);
  await waitFor(() => expect(screen.getByText("Save to Personal")).toBeTruthy());
  fireEvent.press(screen.getByRole("button", { name: "Save contact to: Personal" }));
  fireEvent.press(screen.getByRole("radio", { name: "Work, Paused" }));
  expect(mockReplace).toHaveBeenCalledWith({
    pathname: "/contacts/import",
    params: { owner: identityReferenceKey(work.reference), card: mockCard.uri, invalid: "0" },
  });
  expect(mockImport).not.toHaveBeenCalled();
  expect(mockNamedIdentity?.catalog?.selected).toBe(personal.reference);
  selectOwner(work);
  screen.rerender(<ContactCardImportScreen initialURI={mockCard.uri} />);
  await waitFor(() => expect(screen.getByText("Save to Work")).toBeTruthy());
  await act(async () => fireEvent.press(screen.getByText("Save to Work")));
  expect(mockImport).toHaveBeenCalledTimes(1);
  expect(mockImport).toHaveBeenCalledWith({
    uri: mockCard.uri,
    expectedLocalIdentity: work.reference.publicHash,
  });
});

it("preserves pasted input through owner selection and ignores the previous owner's late preview", async () => {
  const { work, selectOwner } = namedImportFixture();
  let finish!: (result: Bindings.ContactCardPreviewOutcome) => void;
  mockPreview.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const screen = render(<ContactCardImportScreen />);
  fireEvent.changeText(screen.getByLabelText("Contact link"), mockCard.uri);
  fireEvent.press(screen.getByText("Review contact card"));
  fireEvent.press(screen.getByRole("button", { name: "Save contact to: Personal" }));
  fireEvent.press(screen.getByRole("radio", { name: "Work, Paused" }));
  expect(mockReplace).toHaveBeenCalledWith({
    pathname: "/contacts/import",
    params: { owner: identityReferenceKey(work.reference), card: mockCard.uri, invalid: "0" },
  });
  selectOwner(work);
  screen.rerender(<ContactCardImportScreen initialURI={mockCard.uri} />);
  await waitFor(() => expect(screen.getByText("Save to Work")).toBeTruthy());
  await act(async () =>
    finish(
      Bindings.ContactCardPreviewOutcome.Valid.new({
        card: { ...mockCard, displayName: "Stale preview" },
      }),
    ),
  );
  expect(screen.queryByText("Stale preview")).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
});

it("pins scan-to-invite to its group owner without offering another identity", async () => {
  const { work, selectOwner } = namedImportFixture();
  selectOwner(work);
  const selected = jest.fn();
  const screen = render(<ContactCardImporter onImported={selected} />);
  expect(screen.getByText("Inviting as Work")).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Save contact to/ })).toBeNull();
  fireEvent.press(screen.getByText("Scan contact QR"));
  await act(async () => fireEvent.press(screen.getByText("Read test QR")));
  await act(async () => fireEvent.press(screen.getByText("Save and select contact")));
  expect(mockImport).toHaveBeenCalledWith({
    uri: mockCard.uri,
    expectedLocalIdentity: work.reference.publicHash,
  });
  expect(mockReplace).not.toHaveBeenCalled();
  expect(selected).toHaveBeenCalledTimes(1);
});

it("choosing another owner cannot revive an older card after a malformed warm link", () => {
  const { work } = namedImportFixture();
  const screen = render(<ContactCardImportScreen initialURI={mockCard.uri} invalid />);
  fireEvent.press(screen.getByRole("button", { name: "Save contact to: Personal" }));
  fireEvent.press(screen.getByRole("radio", { name: "Work, Paused" }));
  expect(mockReplace).toHaveBeenCalledWith({
    pathname: "/contacts/import",
    params: { owner: identityReferenceKey(work.reference), card: "", invalid: "1" },
  });
  expect(mockPreview).not.toHaveBeenCalled();
  expect(mockImport).not.toHaveBeenCalled();
});
afterEach(() => jest.restoreAllMocks());

it("previews an incoming link without saving, then saves only on explicit confirmation", async () => {
  const view = render(
    <StrictMode>
      <ContactCardImporter initialURI={mockCard.uri} />
    </StrictMode>,
  );
  await waitFor(() => expect(view.getByText("Alice")).toBeTruthy());
  expect(mockImport).not.toHaveBeenCalled();
  expect(view.getByText(/Signature checked/)).toBeTruthy();
  await act(async () => fireEvent.press(view.getByText("Save contact card")));
  expect(mockImport).toHaveBeenCalledWith({
    uri: mockCard.uri,
    expectedLocalIdentity: mockIdentity,
  });
  expect(view.getByText("Contact saved.")).toBeTruthy();
  expect(view.queryByText("Save contact card")).toBeNull();
});

it("lets a scanned card be reviewed and selected without sending an invitation", async () => {
  const selected = jest.fn();
  const view = render(<ContactCardImporter onImported={selected} />);
  fireEvent.press(view.getByText("Scan contact QR"));
  await act(async () => fireEvent.press(view.getByText("Read test QR")));
  expect(view.queryByText("Read test QR")).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
  expect(selected).not.toHaveBeenCalled();
  await act(async () => fireEvent.press(view.getByText("Save and select contact")));
  expect(selected).toHaveBeenCalledWith(mockCard, mockContact);
});

it("rejects malformed and oversized pasted input before native verification", async () => {
  const view = render(<ContactCardImporter />);
  for (const input of ["https://example.org/card", `rns-contact://v1/${"A".repeat(369)}`]) {
    fireEvent.changeText(view.getByLabelText("Contact link"), input);
    await act(async () => fireEvent.press(view.getByText("Review contact card")));
    expect(view.getByText(/not a supported contact card/)).toBeTruthy();
  }
  expect(mockPreview).not.toHaveBeenCalled();
  expect(mockImport).not.toHaveBeenCalled();
});

it.each([
  [
    Bindings.ContactCardPreviewOutcome.Invalid.new({ detail: "bad signature" }),
    /could not be verified/,
  ],
  [
    Bindings.ContactCardPreviewOutcome.UnsupportedService.new(),
    /service this app does not support/,
  ],
] as const)("does not allow saving failed verification %#", async (outcome, text) => {
  mockPreview.mockResolvedValue(outcome);
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(view.getByText(text)).toBeTruthy());
  expect(view.queryByText("Save contact card")).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
});

it("preserves a conflicting saved identity and never reports it selected", async () => {
  mockImport.mockResolvedValue(
    Bindings.ContactCardImportOutcome.IdentityConflict.new({
      existing: mockIdentity,
      attempted: mockCard.identity,
    }),
  );
  const selected = jest.fn();
  const view = render(<ContactCardImporter initialURI={mockCard.uri} onImported={selected} />);
  await waitFor(() => expect(view.getByText("Save and select contact")).toBeTruthy());
  await act(async () => fireEvent.press(view.getByText("Save and select contact")));
  expect(view.getByText(/was not replaced/)).toBeTruthy();
  expect(selected).not.toHaveBeenCalled();
});

it("serializes double confirmation and offers an idempotent retry after uncertain save", async () => {
  let resolve: ((result: Bindings.ContactCardImportOutcome) => void) | undefined;
  mockImport.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(view.getByText("Save contact card")).toBeTruthy());
  const save = view.getByText("Save contact card");
  act(() => {
    fireEvent.press(save);
    fireEvent.press(save);
  });
  expect(mockImport).toHaveBeenCalledTimes(1);
  await act(async () =>
    resolve?.(Bindings.ContactCardImportOutcome.Unavailable.new({ detail: "uncertain" })),
  );
  expect(view.getByText(/will not create a duplicate/)).toBeTruthy();
  mockImport.mockResolvedValue(
    Bindings.ContactCardImportOutcome.Existing.new({ contact: mockContact }),
  );
  await act(async () => fireEvent.press(view.getByText("Save contact card")));
  expect(view.getByText("Contact saved.")).toBeTruthy();
  expect(mockImport.mock.calls[0]).toEqual(mockImport.mock.calls[1]);
});

it.each(["identity", "generation", "runtime"])(
  "ignores a save completing after %s ownership changes",
  async (owner) => {
    let resolve: ((result: Bindings.ContactCardImportOutcome) => void) | undefined;
    mockImport.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const selected = jest.fn();
    const view = render(<ContactCardImporter onImported={selected} />);
    fireEvent.changeText(view.getByLabelText("Contact link"), mockCard.uri);
    await act(async () => fireEvent.press(view.getByText("Review contact card")));
    fireEvent.press(view.getByText("Save and select contact"));
    if (owner === "identity") mockIdentity = new Uint8Array(16).fill(0x22);
    if (owner === "generation") mockGeneration = 2n;
    if (owner === "runtime") mockRuntime = { ...baseRuntime };
    view.rerender(<ContactCardImporter onImported={selected} />);
    await act(async () =>
      resolve?.(Bindings.ContactCardImportOutcome.Saved.new({ contact: mockContact })),
    );
    expect(selected).not.toHaveBeenCalled();
    expect(view.queryByText("Contact saved.")).toBeNull();
    expect(view.queryByText("Alice")).toBeNull();
  },
);

it("does not offer to save your own card", async () => {
  mockIdentity = mockCard.identity;
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(view.getByText("This is your own contact card.")).toBeTruthy());
  fireEvent.press(view.getByText("Save contact card"));
  expect(mockImport).not.toHaveBeenCalled();
});

it("renders and shares exactly the native portable URI, only after a user action", async () => {
  const share = jest.spyOn(Share, "share").mockResolvedValue({ action: Share.sharedAction });
  const view = render(<MyContactCardScreen />);
  await waitFor(() => expect(view.getByTestId("my-contact-qr")).toBeTruthy());
  const segment = view.getByTestId("my-contact-qr").props.value[0] as {
    data: number[];
    mode: string;
  };
  expect(segment.mode).toBe("byte");
  expect(String.fromCharCode(...segment.data)).toBe(mockCard.uri);
  expect(view.queryByText("Device identity")).toBeNull();
  fireEvent.press(view.getByRole("button", { name: "About your contact card" }));
  expect(view.getByText("Device identity")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Close About your contact card" }));
  expect(share).not.toHaveBeenCalled();
  await act(async () => fireEvent.press(view.getByText("Share contact link")));
  expect(share).toHaveBeenCalledWith({ message: mockCard.uri });
  expect(mockExport).toHaveBeenCalledWith({ expectedLocalIdentity: mockIdentity });
});

it("returns from the contact QR to My profile with the same identity owner", async () => {
  const { work, selectOwner } = namedImportFixture();
  selectOwner(work);
  const view = render(<MyContactCardScreen />);
  await view.findByTestId("my-contact-qr");
  expect(view.getByRole("link", { name: "Back to Contacts" })).toBeTruthy();
  expect(view.UNSAFE_getByType(Link).props.href).toEqual({
    pathname: "/contacts",
    params: { section: "profile", owner: identityReferenceKey(work.reference) },
  });
});

it("encodes a maximum-size portable URI without a global TextEncoder on Hermes", () => {
  const encoder = jest.requireActual<{
    create: (
      value: { data: number[]; mode: string }[],
      options: { errorCorrectionLevel: string },
    ) => { modules: { size: number; data: Uint8Array } };
  }>("qrcode");
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "TextEncoder");
  Object.defineProperty(globalThis, "TextEncoder", { configurable: true, value: undefined });
  try {
    const uri = `rns-contact://v1/${"A".repeat(368)}`;
    const qr = encoder.create(
      [{ data: Array.from(uri, (character) => character.charCodeAt(0)), mode: "byte" }],
      { errorCorrectionLevel: "M" },
    );
    expect(qr.modules.size).toBeGreaterThan(0);
    expect(qr.modules.data.length).toBe(qr.modules.size ** 2);
  } finally {
    if (descriptor === undefined) Reflect.deleteProperty(globalThis, "TextEncoder");
    else Object.defineProperty(globalThis, "TextEncoder", descriptor);
  }
});

it("reviews an incoming link once its mounted route gains focus", async () => {
  mockFocused = false;
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  expect(view.getByLabelText("Contact link").props.value).toBe(mockCard.uri);
  expect(mockPreview).not.toHaveBeenCalled();
  mockFocused = true;
  view.rerender(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(view.getByText("Alice")).toBeTruthy());
  expect(mockPreview).toHaveBeenCalledTimes(1);
  expect(mockPreview).toHaveBeenCalledWith(mockCard.uri);
  expect(mockImport).not.toHaveBeenCalled();
});

it("ignores a preview that completes after blur and automatically re-reviews its link on return", async () => {
  let resolve: ((result: Bindings.ContactCardPreviewOutcome) => void) | undefined;
  mockPreview.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(mockPreview).toHaveBeenCalledTimes(1));
  mockFocused = false;
  view.rerender(<ContactCardImporter initialURI={mockCard.uri} />);
  await act(async () =>
    resolve?.(Bindings.ContactCardPreviewOutcome.Valid.new({ card: mockCard })),
  );
  expect(view.queryByText("Alice")).toBeNull();
  mockFocused = true;
  view.rerender(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(view.getByText("Alice")).toBeTruthy());
  expect(mockPreview).toHaveBeenCalledTimes(2);
  expect(mockImport).not.toHaveBeenCalled();
});

it("reviews a changed incoming URI without publishing the previous link's late result", async () => {
  let resolve: ((result: Bindings.ContactCardPreviewOutcome) => void) | undefined;
  mockPreview.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const view = render(<ContactCardImporter initialURI={mockCard.uri} />);
  await waitFor(() => expect(mockPreview).toHaveBeenCalledWith(mockCard.uri));
  const replacement = { ...mockCard, uri: "rns-contact://v1/ZGVm", displayName: "Bob" };
  mockPreview.mockResolvedValue(
    Bindings.ContactCardPreviewOutcome.Valid.new({ card: replacement }),
  );
  view.rerender(<ContactCardImporter initialURI={replacement.uri} />);
  await waitFor(() => expect(view.getByText("Bob")).toBeTruthy());
  await act(async () =>
    resolve?.(Bindings.ContactCardPreviewOutcome.Valid.new({ card: mockCard })),
  );
  expect(view.queryByText("Alice")).toBeNull();
  expect(view.getByText("Bob")).toBeTruthy();
  expect(view.getByLabelText("Contact link").props.value).toBe(replacement.uri);
  expect(mockPreview).toHaveBeenCalledTimes(2);
  expect(mockPreview).toHaveBeenLastCalledWith(replacement.uri);
  expect(mockImport).not.toHaveBeenCalled();
});

it("lets an admitted save finish without selecting a recipient after leaving the group route", async () => {
  let resolve: ((result: Bindings.ContactCardImportOutcome) => void) | undefined;
  mockImport.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const selected = jest.fn();
  const view = render(<ContactCardImporter initialURI={mockCard.uri} onImported={selected} />);
  await waitFor(() => expect(view.getByText("Save and select contact")).toBeTruthy());
  fireEvent.press(view.getByText("Save and select contact"));
  mockFocused = false;
  view.rerender(<ContactCardImporter initialURI={mockCard.uri} onImported={selected} />);
  await act(async () =>
    resolve?.(Bindings.ContactCardImportOutcome.Saved.new({ contact: mockContact })),
  );
  expect(mockImport).toHaveBeenCalledTimes(1);
  expect(selected).not.toHaveBeenCalled();
  expect(view.queryByText("Contact saved.")).toBeNull();
});

it.each(["manual", "retained link"])(
  "discards the previous %s preview when a malformed warm link arrives",
  async (source) => {
    const initial = source === "retained link" ? { initialURI: mockCard.uri } : {};
    const view = render(<ContactCardImporter {...initial} />);
    if (source === "manual") {
      fireEvent.changeText(view.getByLabelText("Contact link"), mockCard.uri);
      await act(async () => fireEvent.press(view.getByText("Review contact card")));
    }
    await waitFor(() => expect(view.getByText("Alice")).toBeTruthy());
    view.rerender(<ContactCardImporter {...initial} invalid />);
    await waitFor(() => expect(view.getByText(/contact link is incomplete/)).toBeTruthy());
    expect(view.queryByText("Alice")).toBeNull();
    expect(view.queryByText("Save contact card")).toBeNull();
    expect(view.getByLabelText("Contact link").props.value).toBe("");
    expect(mockPreview).toHaveBeenCalledTimes(1);
    expect(mockImport).not.toHaveBeenCalled();

    // A user may still deliberately replace the malformed link with a valid one.
    fireEvent.changeText(view.getByLabelText("Contact link"), mockCard.uri);
    await act(async () => fireEvent.press(view.getByText("Review contact card")));
    expect(view.getByText("Alice")).toBeTruthy();
    expect(mockPreview).toHaveBeenCalledTimes(2);
    expect(mockImport).not.toHaveBeenCalled();
  },
);

it("ignores a pending preview when a malformed warm link replaces its journey", async () => {
  let resolve: ((result: Bindings.ContactCardPreviewOutcome) => void) | undefined;
  mockPreview.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const view = render(<ContactCardImporter />);
  fireEvent.changeText(view.getByLabelText("Contact link"), mockCard.uri);
  fireEvent.press(view.getByText("Review contact card"));
  view.rerender(<ContactCardImporter invalid />);
  await act(async () =>
    resolve?.(Bindings.ContactCardPreviewOutcome.Valid.new({ card: mockCard })),
  );
  expect(view.getByText(/contact link is incomplete/)).toBeTruthy();
  expect(view.queryByText("Alice")).toBeNull();
  expect(view.queryByText("Save contact card")).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
});

it("does not select an admitted save's result in a new malformed-link journey", async () => {
  let resolve: ((result: Bindings.ContactCardImportOutcome) => void) | undefined;
  mockImport.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const selected = jest.fn();
  const view = render(<ContactCardImporter onImported={selected} />);
  fireEvent.changeText(view.getByLabelText("Contact link"), mockCard.uri);
  await act(async () => fireEvent.press(view.getByText("Review contact card")));
  fireEvent.press(view.getByText("Save and select contact"));
  view.rerender(<ContactCardImporter invalid onImported={selected} />);
  await act(async () =>
    resolve?.(Bindings.ContactCardImportOutcome.Saved.new({ contact: mockContact })),
  );
  expect(mockImport).toHaveBeenCalledTimes(1);
  expect(selected).not.toHaveBeenCalled();
  expect(view.getByText(/contact link is incomplete/)).toBeTruthy();
  expect(view.queryByText("Contact saved.")).toBeNull();
});
