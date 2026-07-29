import { useState, useMemo } from "react";

/**
 * Client-side pagination over an in-memory list. Manages the current page
 * as internal state, clamps it against the derived page count (so a shrunk
 * list can't leave the caller stuck on a now-empty page), and returns the
 * exact slice for the current page along with the metadata a pager UI
 * needs.
 *
 * The returned `page` is ALREADY CLAMPED — render it directly in the pager
 * without re-clamping. `setPage` accepts any integer; the clamp is applied
 * on the next render. Callers that need to reset to page 1 on some external
 * event (e.g. a search-box change) should call `setPage(1)` explicitly.
 *
 * `totalPages` is always ≥ 1 (an empty list still has "page 1 of 1"), so
 * pager UIs can render without special-casing empty state.
 */
export function usePagination<T>(items: T[], pageSize: number) {
  const [page, setPage] = useState(1);

  return useMemo(() => {
    const totalRows = items.length;
    const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
    const clampedPage = Math.min(Math.max(1, page), totalPages);
    const pageStart = (clampedPage - 1) * pageSize;
    const pageEnd = Math.min(pageStart + pageSize, totalRows);
    const pageRows = items.slice(pageStart, pageEnd);
    return {
      page: clampedPage,
      setPage,
      pageRows,
      totalRows,
      totalPages,
      pageStart,
      pageEnd,
    };
  }, [items, pageSize, page]);
}
