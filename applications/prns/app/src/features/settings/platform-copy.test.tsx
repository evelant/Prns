import { fireEvent, render } from "@testing-library/react-native";

import { AboutScreen } from "./about-screen";
import { HelpScreen } from "./help-screen";

describe("unsupported-platform capability copy", () => {
  it("identifies the web preview and keeps unavailable native sharing out of the flow", () => {
    const view = render(<AboutScreen />);
    expect(view.getByText("Web preview")).toBeTruthy();
    expect(
      view.getByText(/Messaging and node connections are available in the iOS and Android apps/u),
    ).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Diagnostics" }));
    expect(
      view.getByText(/Diagnostic sharing is available in the iOS and Android apps/u),
    ).toBeTruthy();
    expect(view.queryByRole("button", { name: "Share diagnostics" })).toBeNull();
  });

  it("limits web help to the interactions that web supports", () => {
    const view = render(<HelpScreen />);
    expect(view.getByText(/This web preview shows the app layout/u)).toBeTruthy();
    expect(view.queryByRole("link", { name: "Start a group" })).toBeNull();
    expect(view.queryByText(/NomadNet|planned pages|Coming later/iu)).toBeNull();
  });
});
