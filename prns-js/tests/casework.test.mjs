import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { createRequire } from "node:module";
import { test } from "node:test";

import {
  Tag,
  from,
  match,
  match_into,
} from "../dist/casework.js";

const require = createRequire(import.meta.url);
const commonjs = require("../dist-cjs/casework.js");

test("ESM and CommonJS expose identical casework behavior", () => {
  const value = Tag("Active", { peers: 3 });
  assert.deepEqual(commonjs.Tag("Active", { peers: 3 }), value);
  assert.equal(
    match(value, {
      Active: ({ peers }) => peers,
    }),
    3,
  );
  assert.equal(
    match_into().from(value, {
      Active: ({ peers }) => peers,
    }),
    3,
  );
});

test("case constructors retain the declared tagged shape", () => {
  const tagged = from().MakeTag("Settled", { command: 4n });
  assert.deepEqual(tagged, {
    tag: "Settled",
    data: { command: 4n },
  });
});

test("both facades preserve the published Casework dispatch contract", async () => {
  // The oracle is compiled only in the test process. The published runtime is
  // compiled from src/casework.ts by the same tsc jobs as the rest of the SDK.
  const source = readFileSync(require.resolve("casework/briefcase"), "utf8");
  const reference = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  });
  const upstream = await import("data:text/javascript;base64," + Buffer.from(reference.outputText).toString("base64"));

  for (const facade of [{ Tag, from, match, match_into }, commonjs]) {
    for (const payload of [undefined, null, false, 0, "text", { peers: 4 }]) {
      assert.deepEqual(facade.Tag("State", payload), upstream.Tag("State", payload));
      assert.deepEqual(facade.from().MakeTag("State", payload), upstream.from().MakeTag("State", payload));
    }
    const handlers = {
      UNTAGGED(...args) { return [this === handlers, "untagged", ...args]; },
      State(...args) { return [this === handlers, "state", ...args]; },
    };
    const tagged = upstream.Tag("State", null);
    for (const value of ["State", tagged, null, undefined, true, 6, 0n, [], {},
      { tag: "State" }, { ...tagged, extra: 1 }, { tag: 3, data: false }]) {
      assert.deepEqual(facade.match(value, handlers), upstream.match(value, handlers));
    }
    assert.deepEqual(facade.match_into().from(tagged, handlers), upstream.match_into().from(tagged, handlers));
    for (const value of ["Absent", upstream.Tag("Absent")]) {
      assert.throws(() => facade.match(value, handlers), TypeError);
      assert.throws(() => upstream.match(value, handlers), TypeError);
    }
  }
});

test("dispatch requires an own callable branch for literal, tagged and fallback values", () => {
  for (const facade of [{ Tag, match, match_into }, commonjs]) {
    const paths = [
      ["Ready", (handlers) => facade.match("Ready", handlers), []],
      ["Ready", (handlers) => facade.match(facade.Tag("Ready", 7), handlers), [7]],
      ["Ready", (handlers) => facade.match_into().from(facade.Tag("Ready", 7), handlers), [7]],
      ["UNTAGGED", (handlers) => facade.match(null, handlers), [null]],
    ];
    for (const [key, dispatch, expected] of paths) {
      const inherited = { [key]() { assert.fail("inherited handlers must not run"); } };
      const handlers = Object.create(inherited);
      assert.throws(() => dispatch(handlers), TypeError);
      assert.throws(() => dispatch(Object.create(null)), TypeError);
      handlers[key] = 12;
      assert.throws(() => dispatch(handlers), TypeError);
      handlers[key] = function (...args) {
        assert.equal(this, handlers);
        return args;
      };
      assert.deepEqual(dispatch(handlers), expected);
    }
  }
});
