import ReactMarkdown, { type Components } from "react-markdown";
import { openExternal, resolveReleaseNotesHref } from "../lib/openExternal";

/**
 * Renders a release-notes Markdown string (the GitHub release body that
 * flows through `latest.json` → `UpdateMetadata.notes`) with compact,
 * app-consistent styling.
 *
 * `react-markdown` does NOT render raw HTML unless `rehype-raw` is added —
 * it isn't — so the GitHub-authored notes render safely as text + standard
 * Markdown. Links are intercepted and handed to the OS browser via
 * `openExternal`, because the notes contain PR/compare URLs that must never
 * drive the in-app react-router (which would break the SPA shell).
 *
 * Relative links in release bodies (e.g. `[docs/X.md](docs/X.md)`) are
 * rewritten to `github.com/<repo>/blob/<tag>/<path>` — GitHub's own web UI
 * does this at render time, but the raw API body we receive keeps the bare
 * relative path, which the OS browser can't open. `version` is the release
 * tag those relative links resolve against; when omitted, they resolve to
 * `HEAD`.
 */

function buildComponents(version: string | undefined): Components {
  return {
  h1: ({ children }) => <h3 className="text-sm font-semibold mt-3 first:mt-0">{children}</h3>,
  h2: ({ children }) => <h3 className="text-sm font-semibold mt-3 first:mt-0">{children}</h3>,
  h3: ({ children }) => <h4 className="text-xs font-semibold mt-2 first:mt-0">{children}</h4>,
  p: ({ children }) => <p className="my-1">{children}</p>,
  ul: ({ children }) => <ul className="list-disc pl-5 space-y-0.5 my-1">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal pl-5 space-y-0.5 my-1">{children}</ol>,
  li: ({ children }) => <li>{children}</li>,
  code: ({ children }) => (
    <code className="rounded bg-gray-100 px-1 py-0.5 font-mono text-[0.9em]">{children}</code>
  ),
  pre: ({ children }) => (
    <pre className="rounded bg-gray-100 p-2 overflow-auto font-mono text-xs my-2">{children}</pre>
  ),
  a: ({ href, children }) => {
    const resolved = href ? resolveReleaseNotesHref(href, version ? `v${version}` : "HEAD") : undefined;
    return (
      <a
        href={resolved}
        className="text-blue-600 underline hover:no-underline cursor-pointer"
        onClick={(e) => {
          e.preventDefault();
          if (resolved) void openExternal(resolved);
        }}
      >
        {children}
      </a>
    );
  },
  };
}

export function ReleaseNotes({
  notes,
  version,
}: {
  notes: string | null | undefined;
  version?: string;
}) {
  const trimmed = (notes ?? "").trim();
  if (!trimmed) {
    return (
      <div className="text-xs text-gray-500 italic" data-testid="release-notes-empty">
        No release notes were provided for this version.
      </div>
    );
  }
  return (
    <div className="text-sm text-gray-700 leading-relaxed" data-testid="release-notes">
      <ReactMarkdown components={buildComponents(version)}>{trimmed}</ReactMarkdown>
    </div>
  );
}
