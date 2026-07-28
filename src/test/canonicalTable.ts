/**
 * Shared assertions for the canonical data-table design used across the app.
 *
 * A data table in this repo is expected to follow ONE shared visual contract
 * so different screens (Owned Names, Activity, Auctions, Renewals, Batches,
 * Namebase dashboard, DNS records, TLD inventory, etc.) all feel like one
 * product.
 *
 * The contract:
 *
 *   <table class="w-full text-sm">
 *     <thead>
 *       <tr class="text-left text-gray-500 border-b">
 *         <th class="py-1 pr-4">...</th>   ← every non-last header
 *         <th class="py-1">...</th>        ← last header (no pr-4)
 *       </tr>
 *     </thead>
 *     <tbody>
 *       <tr class="border-t border-gray-100 hover:bg-gray-50">...</tr>
 *     </tbody>
 *   </table>
 *
 * Additional rules verified here:
 *
 *   - Every <th> uses "py-1" (compact density) — never "py-2" / "px-2 py-1" /
 *     "px-3 py-2".
 *   - No <th> is bold: no "font-medium" or "font-semibold" on headers.
 *   - Non-last <th> has "pr-4" (column gutter).
 *   - Every <tbody> <tr> carries "border-t border-gray-100" and
 *     "hover:bg-gray-50" (never the inverted "border-b border-gray-50").
 *   - Every <td> has "py-1" (never "py-2" / "px-*").
 *   - No <td> is bold ("font-semibold" is banned on cells — dynamic tone
 *     classes like text-red-600 stay).
 *   - Muted / secondary <td>s use "text-gray-500", never "text-gray-400"
 *     (checked opportunistically: any <td> that names a gray tone must be
 *     -500, not -400).
 *
 * The virtualized `DataTable` (src/components/ui/DataTable.tsx) uses absolute-
 * positioned virtual rows and CANNOT drop its symmetric horizontal padding.
 * For it a separate, looser assertion (`assertVirtualCanonicalTable`) exists
 * below — it enforces the visual properties that DO apply (compact rows, no
 * bold headers, gray-500 headers, hover, correct border) but permits `px-3`.
 */

function classList(el: Element): string[] {
  return (el.getAttribute("class") ?? "").split(/\s+/).filter(Boolean);
}

function hasClass(el: Element, cls: string): boolean {
  return classList(el).includes(cls);
}

/** Assert that `table` follows the canonical (non-virtualized) design. */
export function assertCanonicalTable(
  table: HTMLTableElement,
  opts: { name?: string } = {},
): void {
  const tag = opts.name ? `[${opts.name}] ` : "";

  // Table itself: w-full text-sm.
  const tclasses = classList(table);
  if (!tclasses.includes("w-full")) {
    throw new Error(`${tag}<table> missing "w-full": ${tclasses.join(" ")}`);
  }
  if (!tclasses.includes("text-sm")) {
    throw new Error(`${tag}<table> missing "text-sm": ${tclasses.join(" ")}`);
  }

  // Header row: text-left text-gray-500 border-b (allow the last one to be
  // any single tr; some tables render a single header group).
  const thead = table.querySelector("thead");
  if (!thead) throw new Error(`${tag}<table> missing <thead>`);
  const headerRow = thead.querySelector("tr");
  if (!headerRow) throw new Error(`${tag}<thead> missing <tr>`);
  const hrClasses = classList(headerRow);
  for (const required of ["text-left", "text-gray-500", "border-b"]) {
    if (!hrClasses.includes(required)) {
      throw new Error(
        `${tag}<thead><tr> missing "${required}": ${hrClasses.join(" ")}`,
      );
    }
  }

  // Header cells: py-1, no font-medium / font-semibold, non-last has pr-4.
  const headerCells = Array.from(headerRow.querySelectorAll("th"));
  if (headerCells.length === 0) {
    throw new Error(`${tag}<thead><tr> has no <th> cells`);
  }
  headerCells.forEach((th, i) => {
    const cls = classList(th);
    const isLast = i === headerCells.length - 1;
    if (!cls.includes("py-1")) {
      throw new Error(
        `${tag}<th> #${i} (${th.textContent?.trim() || "empty"}) missing "py-1": ${cls.join(" ")}`,
      );
    }
    for (const banned of ["py-2", "px-2", "px-3", "font-medium", "font-semibold"]) {
      if (cls.includes(banned)) {
        throw new Error(
          `${tag}<th> #${i} (${th.textContent?.trim() || "empty"}) has banned class "${banned}": ${cls.join(" ")}`,
        );
      }
    }
    if (!isLast && !cls.includes("pr-4")) {
      throw new Error(
        `${tag}<th> #${i} (${th.textContent?.trim() || "empty"}) is not the last column but missing "pr-4": ${cls.join(" ")}`,
      );
    }
  });

  // Body rows: border-t border-gray-100 + hover:bg-gray-50; no inverted
  // "border-b border-gray-50".
  const tbody = table.querySelector("tbody");
  if (!tbody) throw new Error(`${tag}<table> missing <tbody>`);
  const bodyRows = Array.from(tbody.querySelectorAll(":scope > tr"));
  if (bodyRows.length === 0) {
    throw new Error(`${tag}<tbody> has no rows to check (fixture must include \u2265 1 row)`);
  }
  bodyRows.forEach((tr, i) => {
    const cls = classList(tr);
    for (const required of ["border-t", "border-gray-100", "hover:bg-gray-50"]) {
      if (!cls.includes(required)) {
        throw new Error(
          `${tag}<tbody> row #${i} missing "${required}": ${cls.join(" ")}`,
        );
      }
    }
    if (cls.includes("border-b")) {
      throw new Error(
        `${tag}<tbody> row #${i} uses inverted "border-b" instead of "border-t": ${cls.join(" ")}`,
      );
    }
  });

  // Body cells: py-1, no py-2 / px-* / font-semibold; no text-gray-400 on
  // <td> (only text-gray-500 is allowed for muted).
  bodyRows.forEach((tr, rowIdx) => {
    const cells = Array.from(tr.querySelectorAll(":scope > td"));
    cells.forEach((td, colIdx) => {
      const cls = classList(td);
      if (!cls.includes("py-1")) {
        throw new Error(
          `${tag}<td> row=${rowIdx} col=${colIdx} missing "py-1": ${cls.join(" ")}`,
        );
      }
      for (const banned of ["py-2", "px-2", "px-3", "font-semibold"]) {
        if (cls.includes(banned)) {
          throw new Error(
            `${tag}<td> row=${rowIdx} col=${colIdx} has banned class "${banned}": ${cls.join(" ")}`,
          );
        }
      }
      if (cls.includes("text-gray-400")) {
        throw new Error(
          `${tag}<td> row=${rowIdx} col=${colIdx} uses "text-gray-400" (must be text-gray-500 for muted): ${cls.join(" ")}`,
        );
      }
    });
  });
}

/**
 * Looser assertion for the virtualized `DataTable`: rows are absolutely
 * positioned inside <tbody> and NEED symmetric horizontal padding (`px-3`),
 * so we allow that. We still enforce:
 *   - table: w-full text-sm
 *   - headers: gray-500, no font-medium / font-semibold, py-1.5 or py-1
 *   - rows: border-t border-gray-100, hover:bg-gray-50
 *   - cells: no font-semibold, no text-gray-400
 */
export function assertVirtualCanonicalTable(
  table: HTMLTableElement,
  opts: { name?: string } = {},
): void {
  const tag = opts.name ? `[${opts.name}] ` : "";

  const tclasses = classList(table);
  if (!tclasses.includes("w-full")) {
    throw new Error(`${tag}<table> missing "w-full"`);
  }
  if (!tclasses.includes("text-sm")) {
    throw new Error(`${tag}<table> missing "text-sm"`);
  }

  const thead = table.querySelector("thead");
  if (!thead) throw new Error(`${tag}<table> missing <thead>`);
  const headerCells = Array.from(thead.querySelectorAll("th"));
  if (headerCells.length === 0) {
    throw new Error(`${tag}<thead> has no <th> cells`);
  }
  headerCells.forEach((th, i) => {
    const cls = classList(th);
    for (const banned of ["font-medium", "font-semibold", "text-gray-600"]) {
      if (cls.includes(banned)) {
        throw new Error(
          `${tag}virtual <th> #${i} has banned class "${banned}": ${cls.join(" ")}`,
        );
      }
    }
    if (!cls.includes("text-gray-500")) {
      throw new Error(
        `${tag}virtual <th> #${i} missing "text-gray-500": ${cls.join(" ")}`,
      );
    }
    // Compact density — accept py-1 or py-1.5, reject py-2/py-3.
    const compact = cls.includes("py-1") || cls.includes("py-1.5");
    if (!compact) {
      throw new Error(
        `${tag}virtual <th> #${i} not compact (need py-1 or py-1.5): ${cls.join(" ")}`,
      );
    }
    for (const banned of ["py-2", "py-3"]) {
      if (cls.includes(banned)) {
        throw new Error(
          `${tag}virtual <th> #${i} uses non-compact "${banned}"`,
        );
      }
    }
  });

  const tbody = table.querySelector("tbody");
  if (!tbody) throw new Error(`${tag}<table> missing <tbody>`);
  const bodyRows = Array.from(tbody.querySelectorAll(":scope > tr"));
  if (bodyRows.length === 0) {
    throw new Error(`${tag}virtual <tbody> has no rows`);
  }
  bodyRows.forEach((tr, i) => {
    const cls = classList(tr);
    for (const required of ["border-t", "border-gray-100", "hover:bg-gray-50"]) {
      if (!cls.includes(required)) {
        throw new Error(
          `${tag}virtual <tbody> row #${i} missing "${required}": ${cls.join(" ")}`,
        );
      }
    }
  });

  // Cells: no font-semibold, no text-gray-400, no py-2/py-3.
  bodyRows.forEach((tr, rowIdx) => {
    const cells = Array.from(tr.querySelectorAll(":scope > td"));
    cells.forEach((td, colIdx) => {
      const cls = classList(td);
      for (const banned of ["font-semibold", "text-gray-400", "py-2", "py-3"]) {
        if (cls.includes(banned)) {
          throw new Error(
            `${tag}virtual <td> row=${rowIdx} col=${colIdx} has banned class "${banned}": ${cls.join(" ")}`,
          );
        }
      }
    });
  });
}

// Exported for use by consumers that want raw checks.
export const _testOnly = { classList, hasClass };
