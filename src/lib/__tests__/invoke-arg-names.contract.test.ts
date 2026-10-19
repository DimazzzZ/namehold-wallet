/**
 * Contract: the argument names of a Tauri command are camelCase.
 *
 * Tauri 2 matches the parameters of a plain `#[tauri::command]` by their
 * camelCase name (`api_key` on the Rust side is `apiKey` on the wire). A
 * snake_case key at the top level of the `invoke` arguments is therefore never
 * delivered: the Rust parameter arrives as `None` and the command silently runs
 * without it (the Test connection button once ignored the typed API key this
 * way). This test parses every non-test source file under `src/` with the
 * TypeScript compiler and fails on such a key, naming file:line.
 *
 * What is checked: the TOP-LEVEL keys of the second argument of `invoke(...)` /
 * `tauriInvoke(...)`. Nested objects are ignored on purpose: a field inside a
 * struct argument (`request: { some_field }`) is named by the struct's serde
 * attributes, not by Tauri, and may legitimately be snake_case.
 *
 * How the argument is resolved:
 *  - an object literal: its own keys, plus whatever its `...spread` entries resolve to;
 *  - an identifier, `x as T` or `...x`: followed inside the same file to a `const`
 *    object literal, or to the type of a parameter / `const` (type literal, or an
 *    interface / type alias declared in the same file);
 *  - anything else (an import, a call result, an `any`/`Record<string, unknown>`
 *    type) cannot be resolved statically. Those are collected in `unchecked` and
 *    pinned in UNCHECKED_CALLS below, so a new one shows up in review; they are
 *    not silently trusted.
 */
import { describe, it, expect } from "vitest";
import ts from "typescript";

// Every source file under src/, as text (the path keys are like "/src/lib/x.ts").
const ALL_SOURCES = import.meta.glob("/src/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const INVOKE_NAMES = new Set(["invoke", "tauriInvoke"]);
const SNAKE_CASE = /^[A-Za-z0-9]+(_[A-Za-z0-9]+)+$/;

/** Non-test sources: not in a `__tests__` / `test` directory, not `*.test.*`. */
function sourceFiles(): [string, string][] {
  return Object.entries(ALL_SOURCES)
    .filter(
      ([path]) => !/\/(__tests__|test)\//.test(path) && !/\.(test|spec)\.(ts|tsx)$/.test(path),
    )
    .map(([path, text]): [string, string] => [path.slice(1), text]);
}

export interface Finding {
  where: string;
  key: string;
}

function propName(name: ts.PropertyName): string | null {
  if (ts.isIdentifier(name) || ts.isStringLiteralLike(name) || ts.isNumericLiteral(name)) {
    return name.text;
  }
  return null; // computed key: not statically known
}

/** Scan the source text and return the offending keys and the unresolved calls. */
export function scanInvokeArgs(
  text: string,
  fileName: string,
): { offenders: Finding[]; unchecked: string[]; calls: number } {
  const sf = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const offenders: Finding[] = [];
  const unchecked: string[] = [];
  let calls = 0;

  const at = (n: ts.Node) =>
    `${fileName}:${sf.getLineAndCharacterOfPosition(n.getStart()).line + 1}`;

  // Interfaces and type aliases by name, file-wide (a collision resolves to the first).
  const types = new Map<string, ts.TypeNode | ts.InterfaceDeclaration>();
  const collect = (n: ts.Node) => {
    if (ts.isInterfaceDeclaration(n)) {
      if (!types.has(n.name.text)) types.set(n.name.text, n);
    } else if (ts.isTypeAliasDeclaration(n)) {
      if (!types.has(n.name.text)) types.set(n.name.text, n.type);
    }
    ts.forEachChild(n, collect);
  };
  collect(sf);

  /** The nearest declaration of `name` visible from `from`: a function parameter
   *  or a `const`/`let` in an enclosing block, innermost first. */
  const lookup = (
    name: string,
    from: ts.Node,
  ): { init?: ts.Expression; type?: ts.TypeNode } | null => {
    for (let n: ts.Node | undefined = from.parent; n; n = n.parent) {
      if (ts.isFunctionLike(n)) {
        for (const p of n.parameters) {
          if (ts.isIdentifier(p.name) && p.name.text === name) {
            return { init: p.initializer, type: p.type };
          }
        }
      }
      if (ts.isBlock(n) || ts.isSourceFile(n) || ts.isModuleBlock(n)) {
        for (const st of n.statements) {
          if (!ts.isVariableStatement(st)) continue;
          for (const d of st.declarationList.declarations) {
            if (ts.isIdentifier(d.name) && d.name.text === name) {
              return { init: d.initializer, type: d.type };
            }
          }
        }
      }
    }
    return null;
  };

  const strip = (e: ts.Expression): ts.Expression => {
    while (
      ts.isParenthesizedExpression(e) ||
      ts.isAsExpression(e) ||
      ts.isSatisfiesExpression(e) ||
      ts.isNonNullExpression(e)
    ) {
      e = e.expression;
    }
    return e;
  };

  // Each resolver returns false when it hit something it cannot see through.
  const keysOfType = (t: ts.TypeNode | ts.InterfaceDeclaration, depth: number): boolean => {
    if (depth > 6) return false;
    if (ts.isInterfaceDeclaration(t)) {
      if (t.heritageClauses?.length) return false;
      return keysOfMembers(t.members, t);
    }
    if (ts.isTypeLiteralNode(t)) return keysOfMembers(t.members, t);
    if (ts.isParenthesizedTypeNode(t)) return keysOfType(t.type, depth + 1);
    if (ts.isIntersectionTypeNode(t)) {
      return t.types.map((x) => keysOfType(x, depth + 1)).every(Boolean);
    }
    if (ts.isTypeReferenceNode(t) && ts.isIdentifier(t.typeName) && !t.typeArguments) {
      const decl = types.get(t.typeName.text);
      return decl ? keysOfType(decl, depth + 1) : false;
    }
    return false;
  };
  const keysOfMembers = (members: ts.NodeArray<ts.TypeElement>, anchor: ts.Node): boolean => {
    let ok = true;
    for (const m of members) {
      if (ts.isPropertySignature(m)) {
        const k = propName(m.name);
        if (k === null) ok = false;
        else if (SNAKE_CASE.test(k)) offenders.push({ where: at(m), key: k });
      } else {
        ok = false; // index signature, method, ...
      }
    }
    void anchor;
    return ok;
  };

  const keysOfExpr = (e: ts.Expression, depth: number): boolean => {
    if (depth > 6) return false;
    e = strip(e);
    if (ts.isObjectLiteralExpression(e)) {
      let ok = true;
      for (const p of e.properties) {
        if (ts.isSpreadAssignment(p)) {
          if (!keysOfExpr(p.expression, depth + 1)) ok = false;
        } else if (
          ts.isPropertyAssignment(p) ||
          ts.isShorthandPropertyAssignment(p) ||
          ts.isMethodDeclaration(p)
        ) {
          const k = propName(p.name);
          if (k === null) ok = false;
          else if (SNAKE_CASE.test(k)) offenders.push({ where: at(p), key: k });
        } else {
          ok = false;
        }
      }
      return ok;
    }
    if (ts.isIdentifier(e)) {
      const decl = lookup(e.text, e);
      if (!decl) return false;
      if (decl.init && keysOfExpr(decl.init, depth + 1)) return true;
      return decl.type ? keysOfType(decl.type, depth + 1) : false;
    }
    return false;
  };

  const visit = (n: ts.Node) => {
    if (
      ts.isCallExpression(n) &&
      ts.isIdentifier(n.expression) &&
      INVOKE_NAMES.has(n.expression.text) &&
      n.arguments.length >= 2
    ) {
      calls += 1;
      const arg = n.arguments[1];
      if (arg && !keysOfExpr(arg, 0)) unchecked.push(at(arg));
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  // A type reached from several calls reports its keys once.
  const seen = new Set<string>();
  const unique = offenders.filter((o) => {
    const id = `${o.where}|${o.key}`;
    return seen.has(id) ? false : (seen.add(id), true);
  });
  return { offenders: unique, unchecked, calls };
}

/**
 * Calls whose arguments the scan cannot see through (file:line is not pinned,
 * since lines move; the file and a count are). A new entry means a new place
 * where a snake_case argument name could hide: look at it by hand, then add it.
 */
const UNCHECKED_CALLS: Record<string, number> = {
  // The wrapper itself: forwards whatever its caller passed.
  "src/lib/invoke.ts": 1,
  // useWalletMutation forwards its caller's args. Its call sites were read by
  // hand: they pass `{ label, network, kind }` or a mapper returning
  // `{ walletProfileId }`, all camelCase.
  "src/queries/wallet.ts": 1,
};

describe("invoke argument names are camelCase", () => {
  const files = sourceFiles();
  const results = files.map(([rel, text]) => ({ rel, ...scanInvokeArgs(text, rel) }));

  it("scans the whole source tree", () => {
    expect(files.length).toBeGreaterThan(50);
    expect(results.reduce((n, r) => n + r.calls, 0)).toBeGreaterThan(80);
  });

  it("finds no snake_case top-level key in an invoke argument object", () => {
    const offenders = results.flatMap((r) => r.offenders.map((o) => `${o.where}  ${o.key}`));
    expect(offenders, `snake_case invoke argument names:\n${offenders.join("\n")}`).toEqual([]);
  });

  it("has no unresolved invoke argument beyond the pinned list", () => {
    const seen: Record<string, number> = {};
    for (const r of results) if (r.unchecked.length) seen[r.rel] = r.unchecked.length;
    expect(
      seen,
      `unresolved calls: ${JSON.stringify(results.flatMap((r) => r.unchecked))}`,
    ).toEqual(UNCHECKED_CALLS);
  });
});

describe("scanInvokeArgs (the scanner itself)", () => {
  it("reports a snake_case top-level key with its line", () => {
    const src = `invoke("c", {\n  url,\n  api_key: k,\n});`;
    const r = scanInvokeArgs(src, "x.ts");
    expect(r.offenders).toEqual([{ where: "x.ts:3", key: "api_key" }]);
    expect(r.unchecked).toEqual([]);
  });

  it("ignores snake_case keys of nested objects", () => {
    const r = scanInvokeArgs(`invoke("c", { request: { some_field: 1 }, ok: 2 });`, "x.ts");
    expect(r.offenders).toEqual([]);
  });

  it("follows a const, a spread and a typed parameter", () => {
    const src = [
      `const a = { good: 1, bad_one: 2 };`,
      `invoke("c", a);`,
      `invoke("c", { ...a });`,
      `type T = { fine: string; bad_two: string };`,
      `const f = (t: T) => invoke("c", { ...t });`,
    ].join("\n");
    const r = scanInvokeArgs(src, "x.ts");
    expect(r.offenders.map((o) => o.key)).toEqual(["bad_one", "bad_two"]);
    expect(r.unchecked).toEqual([]);
  });

  it("lists an argument it cannot see through as unchecked", () => {
    const r = scanInvokeArgs(`import { a } from "./a";\ninvoke("c", a);`, "x.ts");
    expect(r.unchecked).toEqual(["x.ts:2"]);
  });
});
