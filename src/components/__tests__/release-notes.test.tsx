/**
 * ReleaseNotes — Markdown rendering with external-link interception.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { ReleaseNotes } from "../ReleaseNotes";

// Mock only openExternal so we can verify it's called without actually opening
// URLs. The other exports (resolveReleaseNotesHref, constants) stay real so
// relative-link resolution is tested end-to-end.
vi.mock("../../lib/openExternal", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/openExternal")>();
  return {
    ...actual,
    openExternal: vi.fn(),
  };
});

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

  it("rewrites relative doc-links to the repo's blob/<tag> URL when `version` is set", () => {
    render(
      <ReleaseNotes
        version="0.6.0"
        notes='See [docs/RECOVER_LOST_BIDS.md](docs/RECOVER_LOST_BIDS.md) for the full guide.'
      />,
    );
    const link = screen.getByRole("link", { name: /RECOVER_LOST_BIDS/ });
    expect(link).toHaveAttribute(
      "href",
      "https://github.com/DimazzzZ/namehold-wallet/blob/v0.6.0/docs/RECOVER_LOST_BIDS.md",
    );
    fireEvent.click(link);
    expect(openExternal).toHaveBeenCalledWith(
      "https://github.com/DimazzzZ/namehold-wallet/blob/v0.6.0/docs/RECOVER_LOST_BIDS.md",
    );
  });

  it("falls back to blob/HEAD when `version` is omitted", () => {
    render(<ReleaseNotes notes='See [docs/x.md](docs/x.md).' />);
    const link = screen.getByRole("link", { name: /docs\/x\.md/ });
    expect(link).toHaveAttribute(
      "href",
      "https://github.com/DimazzzZ/namehold-wallet/blob/HEAD/docs/x.md",
    );
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
