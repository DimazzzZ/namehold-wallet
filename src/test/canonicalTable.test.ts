/**
 * Self-tests for the canonical-table assertion helpers.
 *
 * These verify that the checkers themselves work correctly BEFORE we
 * apply them to real components. If someone accidentally weakens
 * `assertCanonicalTable` (say, deletes the "font-semibold on <th> is
 * banned" check), these fail — the real per-component tests would
 * otherwise silently start green.
 */
import { describe, it, expect } from "vitest";
import { assertCanonicalTable, assertVirtualCanonicalTable } from "./canonicalTable";

function makeTable(html: string): HTMLTableElement {
  const div = document.createElement("div");
  div.innerHTML = html.trim();
  const table = div.querySelector("table");
  if (!table) throw new Error("test fixture must contain a <table>");
  return table as HTMLTableElement;
}

const GOOD = `
  <table class="w-full text-sm">
    <thead>
      <tr class="text-left text-gray-500 border-b">
        <th class="py-1 pr-4">Name</th>
        <th class="py-1 pr-4">Height</th>
        <th class="py-1">Action</th>
      </tr>
    </thead>
    <tbody>
      <tr class="border-t border-gray-100 hover:bg-gray-50">
        <td class="py-1 pr-4 text-xs font-mono">.foo</td>
        <td class="py-1 pr-4 text-xs text-gray-500 font-mono">#123</td>
        <td class="py-1 text-right">btn</td>
      </tr>
    </tbody>
  </table>
`;

describe("assertCanonicalTable — positive case", () => {
  it("passes on a fully canonical table", () => {
    expect(() => assertCanonicalTable(makeTable(GOOD))).not.toThrow();
  });
});

describe("assertCanonicalTable — catches deviations", () => {
  it("catches missing w-full on <table>", () => {
    const bad = GOOD.replace('class="w-full text-sm"', 'class="text-sm"');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/w-full/);
  });

  it("catches missing text-sm on <table>", () => {
    const bad = GOOD.replace('class="w-full text-sm"', 'class="w-full"');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/text-sm/);
  });

  it("catches missing text-gray-500 on header row", () => {
    const bad = GOOD.replace(
      'class="text-left text-gray-500 border-b"',
      'class="text-left border-b"',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/text-gray-500/);
  });

  it("catches font-medium on <th>", () => {
    const bad = GOOD.replace('class="py-1 pr-4">Name', 'class="py-1 pr-4 font-medium">Name');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/font-medium/);
  });

  it("catches non-last <th> missing pr-4", () => {
    const bad = GOOD.replace('class="py-1 pr-4">Name', 'class="py-1">Name');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/pr-4/);
  });

  it("catches py-2 on <th>", () => {
    const bad = GOOD.replace('class="py-1 pr-4">Name', 'class="py-2 pr-4">Name');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/py-1|py-2/);
  });

  it("catches px-3 on <th>", () => {
    const bad = GOOD.replace('class="py-1 pr-4">Name', 'class="px-3 py-1 pr-4">Name');
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/px-3/);
  });

  it("catches <tbody> row missing hover:bg-gray-50", () => {
    const bad = GOOD.replace(
      'class="border-t border-gray-100 hover:bg-gray-50"',
      'class="border-t border-gray-100"',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/hover:bg-gray-50/);
  });

  it("catches inverted border-b on <tbody> row", () => {
    const bad = GOOD.replace(
      'class="border-t border-gray-100 hover:bg-gray-50"',
      'class="border-b border-gray-50 hover:bg-gray-50"',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/border-t|border-b/);
  });

  it("catches font-semibold on a <td>", () => {
    const bad = GOOD.replace(
      'class="py-1 pr-4 text-xs font-mono">.foo',
      'class="py-1 pr-4 text-xs font-mono font-semibold">.foo',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/font-semibold/);
  });

  it("catches text-gray-400 on a <td>", () => {
    const bad = GOOD.replace(
      'class="py-1 pr-4 text-xs text-gray-500 font-mono">#123',
      'class="py-1 pr-4 text-xs text-gray-400 font-mono">#123',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/text-gray-400/);
  });

  it("catches px-3 py-2 on a <td>", () => {
    const bad = GOOD.replace(
      'class="py-1 pr-4 text-xs font-mono">.foo',
      'class="px-3 py-2 text-xs font-mono">.foo',
    );
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/py-1|px-3|py-2/);
  });

  it("errors when the tbody has no rows (fixture must supply data)", () => {
    const bad = GOOD.replace(/<tr class="border-t[\s\S]*?<\/tr>/, "");
    expect(() => assertCanonicalTable(makeTable(bad))).toThrow(/no rows/);
  });
});

const VIRTUAL_GOOD = `
  <table class="w-full text-sm">
    <thead>
      <tr>
        <th class="px-3 py-1.5 text-left text-gray-500 cursor-pointer">Name</th>
        <th class="px-3 py-1.5 text-left text-gray-500 cursor-pointer">Height</th>
      </tr>
    </thead>
    <tbody>
      <tr class="absolute w-full border-t border-gray-100 hover:bg-gray-50 cursor-pointer">
        <td class="px-3 py-1.5 whitespace-nowrap">.foo</td>
        <td class="px-3 py-1.5 whitespace-nowrap">#123</td>
      </tr>
    </tbody>
  </table>
`;

describe("assertVirtualCanonicalTable", () => {
  it("passes on a canonical virtualized table (px-3 allowed)", () => {
    expect(() => assertVirtualCanonicalTable(makeTable(VIRTUAL_GOOD))).not.toThrow();
  });

  it("catches font-medium on a virtual <th>", () => {
    const bad = VIRTUAL_GOOD.replace(
      'class="px-3 py-1.5 text-left text-gray-500 cursor-pointer">Name',
      'class="px-3 py-1.5 text-left text-gray-500 font-medium cursor-pointer">Name',
    );
    expect(() => assertVirtualCanonicalTable(makeTable(bad))).toThrow(/font-medium/);
  });

  it("catches text-gray-600 on a virtual <th> (must be text-gray-500)", () => {
    const bad = VIRTUAL_GOOD.replace("text-gray-500", "text-gray-600");
    expect(() => assertVirtualCanonicalTable(makeTable(bad))).toThrow(/text-gray-600|text-gray-500/);
  });

  it("catches py-2 on a virtual <th>", () => {
    const bad = VIRTUAL_GOOD.replace("py-1.5", "py-2");
    expect(() => assertVirtualCanonicalTable(makeTable(bad))).toThrow(/py-2|compact/);
  });

  it("catches missing hover:bg-gray-50 on a virtual row", () => {
    const bad = VIRTUAL_GOOD.replace(" hover:bg-gray-50", "");
    expect(() => assertVirtualCanonicalTable(makeTable(bad))).toThrow(/hover:bg-gray-50/);
  });
});
