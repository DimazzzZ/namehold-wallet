/**
 * ReleaseNotes — Markdown rendering with external-link interception.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { ReleaseNotes } from "../ReleaseNotes";

// Mock openExternal so we can verify it's called without actually opening URLs.
vi.mock("../../lib/openExternal", () => ({
  openExternal: vi.fn(),
}));

import { openExternal } from "../../lib/openExternal";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ReleaseNotes", () => {
  it("renders empty state when notes are null", () => {
    render(<ReleaseNotes notes={null} />);
    expect(screen.getByTestId("release-notes-empty")).toHaveTextContent(
      /No release notes were provided/,
    );
  });

  it("renders empty state when notes are empty string", () => {
    render(<ReleaseNotes notes="" />);
    expect(screen.getByTestId("release-notes-empty")).toHaveTextContent(
      /No release notes were provided/,
    );
  });

  it("renders markdown headings", () => {
    render(<ReleaseNotes notes={"## What's Changed\n\n### Bug Fixes"} />);
    expect(screen.getByText(/What's Changed/)).toBeInTheDocument();
    expect(screen.getByText(/Bug Fixes/)).toBeInTheDocument();
  });

  it("renders bullet lists", () => {
    render(
      <ReleaseNotes
        notes={"## Changes\n\n- Fixed sync footer\n- Added pagination\n- Updated docs"}
      />,
    );
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(3);
    expect(items[0]).toHaveTextContent("Fixed sync footer");
    expect(items[1]).toHaveTextContent("Added pagination");
    expect(items[2]).toHaveTextContent("Updated docs");
  });

  it("renders links and intercepts clicks to call openExternal", () => {
    render(
      <ReleaseNotes
        notes='Check out [this PR](https://github.com/example/repo/pull/123) for details.'
      />,
    );
    const link = screen.getByRole("link", { name: /this PR/ });
    expect(link).toHaveAttribute("href", "https://github.com/example/repo/pull/123");

    fireEvent.click(link);
    expect(openExternal).toHaveBeenCalledWith("https://github.com/example/repo/pull/123");
  });

  it("renders inline code with styling", () => {
    render(<ReleaseNotes notes="Use `formatDate()` for timestamps." />);
    const code = screen.getByText("formatDate()");
    expect(code.tagName).toBe("CODE");
    expect(code).toHaveClass("rounded", "bg-gray-100");
  });

  it("renders complex markdown with mixed content", () => {
    const notes = `## What's Changed

### Features
- Added [release notes modal](https://github.com/example)
- Improved \`openExternal\` handling

### Fixes
- Fixed date format rendering

See the [full changelog](https://github.com/example/compare) for details.`;

    render(<ReleaseNotes notes={notes} />);

    // Check headings
    expect(screen.getByText(/What's Changed/)).toBeInTheDocument();
    expect(screen.getByText(/Features/)).toBeInTheDocument();
    expect(screen.getByText(/Fixes/)).toBeInTheDocument();

    // Check list items
    const items = screen.getAllByRole("listitem");
    expect(items.length).toBeGreaterThanOrEqual(3);

    // Check links
    const links = screen.getAllByRole("link");
    expect(links.length).toBeGreaterThanOrEqual(2);

    // Check inline code
    expect(screen.getByText("openExternal")).toBeInTheDocument();
  });
});
