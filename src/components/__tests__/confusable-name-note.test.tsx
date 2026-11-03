import { describe, expect, it } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { ConfusableNameNote } from "../ConfusableNameNote";
import { toAceName } from "../../lib/idnEncode";

describe("ConfusableNameNote", () => {
  it("says what the name reads like and shows the name it really is", () => {
    const name = toAceName("аpple");
    render(<ConfusableNameNote name={name} />);
    const note = screen.getByTestId("confusable-name-note");
    expect(note).toHaveTextContent(".apple");
    expect(note).toHaveTextContent(`.${name}`);
  });

  it("renders nothing for a plain name", () => {
    const { container } = render(<ConfusableNameNote name="apple" />);
    expect(container).toBeEmptyDOMElement();
  });
});
