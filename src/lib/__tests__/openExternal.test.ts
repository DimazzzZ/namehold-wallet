import { describe, it, expect } from "vitest";
import { resolveReleaseNotesHref, GITHUB_REPO_URL } from "../openExternal";

describe("resolveReleaseNotesHref", () => {
  it("rewrites a relative markdown-doc href to blob/<tag>", () => {
    expect(resolveReleaseNotesHref("docs/RECOVER_LOST_BIDS.md", "v0.6.0")).toBe(
      `${GITHUB_REPO_URL}/blob/v0.6.0/docs/RECOVER_LOST_BIDS.md`,
    );
  });

  it("strips a leading './' before joining", () => {
    expect(resolveReleaseNotesHref("./README.md", "v0.6.0")).toBe(
      `${GITHUB_REPO_URL}/blob/v0.6.0/README.md`,
    );
  });

  it("strips a leading '/' before joining (repo-root absolute path)", () => {
    expect(resolveReleaseNotesHref("/CHANGELOG.md", "v0.6.0")).toBe(
      `${GITHUB_REPO_URL}/blob/v0.6.0/CHANGELOG.md`,
    );
  });

  it("uses HEAD when no tag is provided", () => {
    expect(resolveReleaseNotesHref("docs/X.md")).toBe(
      `${GITHUB_REPO_URL}/blob/HEAD/docs/X.md`,
    );
  });

  it("leaves absolute https URLs untouched", () => {
    const url = "https://github.com/foo/bar/pull/42";
    expect(resolveReleaseNotesHref(url, "v0.6.0")).toBe(url);
  });

  it("leaves mailto: links untouched", () => {
    expect(resolveReleaseNotesHref("mailto:a@b.com", "v0.6.0")).toBe(
      "mailto:a@b.com",
    );
  });

  it("leaves anchor-only hrefs untouched", () => {
    expect(resolveReleaseNotesHref("#section", "v0.6.0")).toBe("#section");
  });

  it("leaves protocol-relative URLs untouched", () => {
    expect(resolveReleaseNotesHref("//example.com/x", "v0.6.0")).toBe(
      "//example.com/x",
    );
  });

  it("returns empty for empty input", () => {
    expect(resolveReleaseNotesHref("", "v0.6.0")).toBe("");
  });
});
