import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StickyFooter } from "../ui/StickyFooter";
import { type ColumnDef } from "@tanstack/react-table";
import { DataTable } from "../ui/DataTable";

describe("StickyFooter", () => {
  it("renders children", () => {
    render(<StickyFooter><span>Footer content</span></StickyFooter>);
    expect(screen.getByText("Footer content")).toBeTruthy();
  });
});

describe("DataTable", () => {
  type TestRow = { id: number; name: string; value: number };
  const testData: TestRow[] = [
    { id: 1, name: "Alpha", value: 100 },
    { id: 2, name: "Beta", value: 200 },
  ];

  const columns: ColumnDef<TestRow, unknown>[] = [
    { accessorKey: "name", header: "Name" },
    { accessorKey: "value", header: "Value" },
  ];

  it("renders with data", () => {
    render(<DataTable data={testData} columns={columns} />);
    expect(screen.getByPlaceholderText("Search...")).toBeTruthy();
  });

  it("shows row count", () => {
    render(<DataTable data={testData} columns={columns} />);
    expect(screen.getByText("2 rows")).toBeTruthy();
  });

  it("renders with empty data", () => {
    render(<DataTable data={[]} columns={columns} />);
    expect(screen.getByText("0 rows")).toBeTruthy();
  });

  it("renders custom placeholder", () => {
    render(<DataTable data={testData} columns={columns} searchPlaceholder="Filter..." />);
    expect(screen.getByPlaceholderText("Filter...")).toBeTruthy();
  });
});
