import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  flexRender,
  type ColumnDef,
  type SortingState,
  type Row,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useState, useRef, useMemo } from "react";
import { cn } from "../../lib/utils";
import { nameMatches } from "../../lib/idn";
import { Input } from "./Input";

/**
 * Global filter that matches BOTH the raw string form and the
 * `displayName()`-decoded form of every cell value in a row. This means a
 * user typing the Unicode label they see (e.g. `сбер`, `münchen`) finds
 * rows whose stored value is the raw punycode (`xn--90ai7ab`), and a user
 * typing `xn--` still finds rows via the raw form.
 *
 * Delegates to the shared `nameMatches()` helper (idn.ts) for the actual
 * punycode-aware comparison, so the matching logic lives in one place.
 */
function globalPunyFilter<T>(row: Row<T>, _columnId: string, filterValue: unknown): boolean {
  if (typeof filterValue !== "string") return true;
  const q = filterValue.trim().toLowerCase();
  if (!q) return true;
  const values = row.getAllCells().map((c) => c.getValue());
  return values.some((v) => {
    if (v == null) return false;
    const s = String(v);
    // nameMatches handles both raw substring matching AND punycode-decoded
    // matching in one call. For non-name columns (numbers, dates) it still
    // works as a plain case-insensitive substring check on the stringified
    // value.
    return nameMatches(s, q);
  });
}

interface DataTableProps<T> {
  data: T[];
  columns: ColumnDef<T, unknown>[];
  searchPlaceholder?: string;
  onRowClick?: (row: T) => void;
  enableRowSelection?: boolean;
  selectedIds?: Set<number>;
  onSelectionChange?: (ids: Set<number>) => void;
  height?: number;
}

export function DataTable<T extends { id: number }>({
  data,
  columns,
  searchPlaceholder = "Search...",
  onRowClick,
  enableRowSelection = false,
  selectedIds,
  onSelectionChange,
  height = 600,
}: DataTableProps<T>) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [globalFilter, setGlobalFilter] = useState("");
  const parentRef = useRef<HTMLDivElement>(null);

  const tableColumns = useMemo(() => {
    if (!enableRowSelection) return columns;
    const selectCol: ColumnDef<T, unknown> = {
      id: "select",
      size: 40,
      header: ({ table }) => (
        <input
          type="checkbox"
          checked={table.getIsAllRowsSelected()}
          onChange={table.getToggleAllRowsSelectedHandler()}
        />
      ),
      cell: ({ row }) => (
        <input
          type="checkbox"
          checked={row.getIsSelected()}
          onChange={row.getToggleSelectedHandler()}
        />
      ),
    };
    return [selectCol, ...columns];
  }, [columns, enableRowSelection]);

  const table = useReactTable({
    data,
    columns: tableColumns,
    state: { sorting, globalFilter },
    onSortingChange: setSorting,
    onGlobalFilterChange: setGlobalFilter,
    globalFilterFn: globalPunyFilter,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    onRowSelectionChange: (updater) => {
      if (!onSelectionChange) return;
      const newSelection =
        typeof updater === "function"
          ? updater(
              Object.fromEntries(
                Array.from(selectedIds || []).map((id) => [id, true]),
              ) as Record<string, boolean>,
            )
          : updater;
      const ids = new Set(
        Object.entries(newSelection)
          .filter(([, v]) => v)
          .map(([k]) => Number(k)),
      );
      onSelectionChange(ids);
    },
    getRowId: (row) => String(row.id),
  });

  const rows = table.getRowModel().rows;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 40,
    overscan: 10,
  });

  return (
    <div className="flex flex-col gap-2">
      <Input
        inputSize="md"
        className="w-full max-w-sm"
        placeholder={searchPlaceholder}
        value={globalFilter}
        onChange={(e) => setGlobalFilter(e.target.value)}
      />
      <div
        ref={parentRef}
        className="overflow-auto border border-gray-200 rounded"
        style={{ height: `${height}px`, maxWidth: "100%" }}
      >
        <table className="w-full text-sm" style={{ minWidth: "800px" }}>
          <thead className="sticky top-0 bg-gray-50 z-10">
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((h) => (
                  <th
                    key={h.id}
                    className="px-3 py-1.5 text-left text-gray-500 cursor-pointer select-none whitespace-nowrap"
                    style={{ width: h.getSize() }}
                    onClick={h.column.getToggleSortingHandler()}
                  >
                    {flexRender(h.column.columnDef.header, h.getContext())}
                    {
                      { asc: " \u2191", desc: " \u2193" }[
                        h.column.getIsSorted() as string
                      ]
                    }
                  </th>
                ))}
              </tr>
            ))}
          </thead>
          <tbody
            style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}
          >
            {virtualizer.getVirtualItems().map((vi) => {
              const row = rows[vi.index]!;
              return (
                <tr
                  key={row.id}
                  className={cn(
                    "absolute w-full border-t border-gray-100 hover:bg-gray-50 cursor-pointer",
                    selectedIds?.has(row.original.id) && "bg-blue-50",
                  )}
                  style={{
                    height: `${vi.size}px`,
                    transform: `translateY(${vi.start}px)`,
                  }}
                  onClick={() => onRowClick?.(row.original)}
                >
                  {row.getVisibleCells().map((cell) => (
                    <td key={cell.id} className="px-3 py-1.5 whitespace-nowrap">
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="text-xs text-gray-500">
        {rows.length} rows
      </div>
    </div>
  );
}
