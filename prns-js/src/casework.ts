import type * as Casework from "casework/briefcase";

type LiteralTag<Name extends string> = string extends Name ? never : Name;
export type Tag<Name extends string, Data = undefined> = Casework.Tag<Name, Data>;
export type { DataFrom, TagFrom } from "casework/briefcase";

export function Tag<const Name extends string>(
  tag: LiteralTag<Name>,
): Tag<Name, undefined>;
export function Tag<const Name extends string, const Data>(
  tag: LiteralTag<Name>,
  data: Data,
): Tag<Name, Data>;
export function Tag<const Name extends string, const Data>(
  tag: LiteralTag<Name>,
  data?: Data,
): Tag<Name, Data | undefined> {
  return { tag, data };
}

// Keep the public generic contracts shared with Casework. Its published entry
// points are TypeScript; the native and browser facades execute ordinary JS.
export const from: typeof Casework.from = () => ({
  MakeTag: (tag, ...payload) => ({ tag, data: payload[0] }),
}) as ReturnType<typeof Casework.from>;

type Handlers = Record<string, (...args: unknown[]) => unknown>;

function callHandler(handlers: Handlers, name: string, ...args: unknown[]) {
  const handler = typeof name === "string" && Object.hasOwn(handlers, name)
    ? handlers[name]
    : undefined;
  if (typeof handler !== "function") {
    throw new TypeError("Expected an own callable case handler");
  }
  return Reflect.apply(handler, handlers, args);
}

function callTagged(value: Tag<string, unknown>, handlers: Handlers) {
  return callHandler(handlers, value.tag, value.data);
}

export const match_into = (() => ({ from: callTagged })) as typeof Casework.match_into;

export const match = ((value: unknown, handlers: Handlers) => {
  if (typeof value === "string") return callHandler(handlers, value);
  if (value && typeof value === "object" && "tag" in value && "data" in value
      && typeof value.tag === "string" && Object.keys(value).length === 2) {
    return callTagged(value as Tag<string, unknown>, handlers);
  }
  return callHandler(handlers, "UNTAGGED", value);
}) as typeof Casework.match;
