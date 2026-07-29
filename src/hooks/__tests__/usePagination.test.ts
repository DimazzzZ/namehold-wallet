import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { usePagination } from "../usePagination";

describe("usePagination", () => {
  const items = Array.from({ length: 53 }, (_, i) => i);

  it("returns the first page of items by default", () => {
    const { result } = renderHook(() => usePagination(items, 10));
    expect(result.current.page).toBe(1);
    expect(result.current.pageRows).toEqual(items.slice(0, 10));
    expect(result.current.totalPages).toBe(6);
    expect(result.current.totalRows).toBe(53);
    expect(result.current.pageStart).toBe(0);
    expect(result.current.pageEnd).toBe(10);
  });

  it("navigates to a specific page", () => {
    const { result } = renderHook(() => usePagination(items, 10));
    act(() => result.current.setPage(3));
    expect(result.current.page).toBe(3);
    expect(result.current.pageRows).toEqual(items.slice(20, 30));
    expect(result.current.pageStart).toBe(20);
    expect(result.current.pageEnd).toBe(30);
  });

  it("clamps page to totalPages when items shrink", () => {
    const { result, rerender } = renderHook(
      ({ data }) => usePagination(data, 10),
      { initialProps: { data: items } },
    );
    act(() => result.current.setPage(6)); // last page
    expect(result.current.page).toBe(6);

    // Shrink items to 15 (2 pages)
    rerender({ data: items.slice(0, 15) });
    expect(result.current.page).toBe(2); // clamped from 6 → 2
    expect(result.current.pageRows).toEqual([10, 11, 12, 13, 14]);
  });

  it("returns page 1 of 1 for an empty list", () => {
    const { result } = renderHook(() => usePagination([], 10));
    expect(result.current.page).toBe(1);
    expect(result.current.totalPages).toBe(1);
    expect(result.current.pageRows).toEqual([]);
    expect(result.current.totalRows).toBe(0);
  });

  it("clamps page below 1 to 1", () => {
    const { result } = renderHook(() => usePagination(items, 10));
    act(() => result.current.setPage(-5));
    expect(result.current.page).toBe(1);
  });
});
