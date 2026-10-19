import { fireEvent, render } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { Alert } from "react-native";

import { MoreScreen } from "@/features/more/more-screen";
import { AboutScreen } from "./about-screen";
import { HelpScreen } from "./help-screen";
import { SettingsScreen } from "./settings-screen";

const mockReplace = jest.fn();
const mockResetNativeData = jest.fn();
const mockResetScaffoldData = jest.fn();
const mockUpdateScaffoldState = jest.fn();

jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
  useRouter: () => ({ replace: mockReplace }),
}));

jest.mock("@/native/runtime-provider", () => ({
  runtimeProvider: {
    availability: { type: "available", platform: "ios" },
    runtime: { resetDevelopmentData: () => mockResetNativeData() },
  },
}));

jest.mock("@/state/scaffold-state-context", () => ({
  useScaffoldState: () => ({
    resetDevelopmentData: mockResetScaffoldData,
    state: { scaffoldSchema: 2, showUnavailableFeatures: true },
    updateScaffoldState: mockUpdateScaffoldState,
  }),
}));

describe("preview capability copy", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("describes available features without implementation or board jargon", () => {
    const about = render(<AboutScreen />);
    expect(
      about.getByText(/group messaging, node management and network inspection/u),
    ).toBeTruthy();
    expect(JSON.stringify(about.toJSON())).not.toMatch(
      /E290|signed availability|upstream RemoteControl/iu,
    );
    about.unmount();

    const help = render(<HelpScreen />);
    expect(help.getByRole("link", { name: "Start a group" })).toBeTruthy();
    expect(help.getByRole("link", { name: "Manage identities" })).toBeTruthy();
    expect(help.queryByText(/NomadNet|Coming later|identity management,/iu)).toBeNull();
    expect(help.queryByText(/Keep prns open on both phones/u)).toBeNull();
    fireEvent.press(help.getByRole("button", { name: "A message is waiting" }));
    expect(help.getByText(/Keep the original message/u)).toBeTruthy();
    expect(JSON.stringify(help.toJSON())).not.toMatch(
      /E290|signed availability|upstream RemoteControl/iu,
    );
  });

  it("introduces More without narrating scaffold state", () => {
    const view = render(<MoreScreen />);

    expect(view.getByText("Messaging")).toBeTruthy();
    expect(view.getByRole("header", { name: "Network" })).toBeTruthy();
    expect(view.getByText("App")).toBeTruthy();
    expect(view.queryByText("Notifications")).toBeNull();
  });

  it("keeps the preview marker out of routine screens", () => {
    for (const RoutineScreen of [MoreScreen, SettingsScreen, HelpScreen]) {
      const view = render(<RoutineScreen />);
      expect(view.queryByText("Development preview")).toBeNull();
      view.unmount();
    }

    const about = render(<AboutScreen />);
    expect(about.getByText("Maintainer alpha")).toBeTruthy();
  });

  it("discloses every durable data category removed by development reset", () => {
    const alert = jest.spyOn(Alert, "alert").mockImplementation();
    const view = render(<SettingsScreen />);

    expect(view.queryByRole("button", { name: "Reset app data" })).toBeNull();
    expect(view.queryByText("Show upcoming features")).toBeNull();
    fireEvent.press(view.getByRole("button", { name: "App data" }));
    fireEvent.press(view.getByRole("button", { name: "Reset app data" }));

    expect(alert).toHaveBeenCalledWith(
      "Reset app data?",
      "This permanently removes all identities, contacts, groups, messages, pending actions, paired-node access and app preferences from this device. This cannot be undone.",
      expect.any(Array),
    );
    expect(JSON.stringify(view.toJSON())).not.toMatch(
      /E290|signed availability|upstream RemoteControl/iu,
    );
    expect(mockResetNativeData).not.toHaveBeenCalled();
    expect(mockResetScaffoldData).not.toHaveBeenCalled();
    alert.mockRestore();
  });
});
