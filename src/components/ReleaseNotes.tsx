import ReactMarkdown, { type Components } from "react-markdown";
import { openExternal } from "../lib/openExternal";

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
 */

// Styling overrides so we don't need @tailwindcss/typography.
const components: Components = {
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
  a: ({ href, children }) => (
    <a
      href={href}
      className="text-blue-600 underline hover:no-underline"
      onClick={(e) => {
        e.preventDefault();
        if (href) void openExternal(href);
      }}
    >
      {children}
    </a>
  ),
};

export function ReleaseNotes({ notes }: { notes: string | null | undefined }) {
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
      <ReactMarkdown components={components}>{trimmed}</ReactMarkdown>
    </div>
  );
}
