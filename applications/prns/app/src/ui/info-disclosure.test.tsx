import { fireEvent, render } from "@testing-library/react-native";
import { Text } from "react-native";
import { InfoDisclosure } from "./info-disclosure";

test("keeps optional information out of the journey until requested and supports dismissal", () => {
  const screen = render(
    <InfoDisclosure label="About group messaging">
      <Text>Optional explanation</Text>
    </InfoDisclosure>,
  );
  expect(screen.queryByText("Optional explanation")).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "About group messaging" }));
  expect(screen.getByText("Optional explanation")).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Close About group messaging" }));
  expect(screen.queryByText("Optional explanation")).toBeNull();
});
