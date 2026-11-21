import type { JsonValue } from "./runtime.js";
import DecimalJs from "decimal.js";

// cratestack#498: every `Decimal`-typed schema field is carried as a real
// arbitrary-precision value, not a wire-format-dependent opaque `string`.
// #495/#496 made `decimal-bigdecimal` a real server-side backend for
// values beyond `rust_decimal`'s ~28-29 significant-digit cap, but
// `rust_decimal`'s `Display` never emits scientific notation while
// `bigdecimal`'s does past a magnitude threshold (`"0.0000001"` vs.
// `"1E-7"` for the identical value) — a client that just typed the field
// `string` saw a backend-dependent format and had no way to do
// arithmetic/comparison on it at all. `decimal.js` parses both notations
// into the identical value (`new Decimal("1E-7").equals(new
// Decimal("0.0000001"))` is `true`), closing that gap.
//
// Cloned with an effectively-unbounded exponential-notation threshold
// (`toExpNeg`/`toExpPos`) rather than used directly: decimal.js's own
// default `.toString()`/`.toJSON()` switches to scientific notation past
// ±7/21 orders of magnitude, which would make even an *ordinary* value
// like `0.00000001` re-encode as `"1e-8"`. Every outbound encode path
// goes through one of these two: `JSON.stringify` (`jsonRpcCodec`, REST
// bodies) or `encodeWireFields`'s own `.toString()` call (the native
// `@cratestack/cbor` codec, see that function's doc comment below) — so
// this one unbounded threshold governs all of them uniformly. Forcing
// plain positional notation matches `rust_decimal`'s own `Display`
// exactly, so a re-encoded value is always accepted back by a server on
// *either* backend without needing to know whether it parses scientific
// notation.
export const Decimal = DecimalJs.clone({ toExpNeg: -1e9, toExpPos: 1e9 });
export type Decimal = DecimalJs;

/** One model/`type`'s own decode-time shape: `decimalKeys` /
 *  `bigintKeys` / `uuidKeys` / `bytesKeys` / `bytesListKeys` are its *direct* field wire
 *  names whose
 *  decoded values need converting out of their wire form, and `nested`
 *  maps a field name to the shape name (a {@link wireShapes} key) that
 *  field's own value should be revived against, for any field whose type
 *  is itself another declared model/`type`. See {@link wireShapes}'s doc
 *  comment for why this is keyed by structural path (via `nested`) rather
 *  than a single flat, schema-wide field-name set.
 *
 *  `Bytes` keys are split by arity because their wire forms are not
 *  self-identifying at every value: a populated `Bytes` is `number[]` and
 *  a populated `Bytes[]` is `number[][]`, but `[]` is both — an empty
 *  `Uint8Array` and an empty list of them are the same three characters
 *  on the wire. The schema knows which; the runtime cannot.
 *
 *  `bigintKeys` is one list for every arity: a `BigInt` leaf is a string
 *  and a `BigInt[]` is an array of strings, which {@link toBigInt} tells
 *  apart structurally. An empty `[]` needs no conversion either way.
 *
 *  `uuidKeys` is one list for every arity too (cratestack#1359): a `Uuid`
 *  leaf is a 16-byte `Uint8Array` (or a string on JSON) and a `Uuid[]` is an
 *  array of them, which {@link toUuid} tells apart structurally.
 *
 *  `decimalKeys` likewise covers every arity, scalar and list: a `Decimal`
 *  leaf is a string and a `Decimal[]` is an array of strings, which
 *  {@link toDecimal} maps element by element. */
export interface WireShape {
  readonly decimalKeys: readonly string[];
  readonly bigintKeys: readonly string[];
  readonly uuidKeys: readonly string[];
  readonly bytesKeys: readonly string[];
  readonly bytesListKeys: readonly string[];
  readonly nested: Readonly<Record<string, string>>;
}

/** One entry per model/`type` this schema declares (`crate::wire_shapes::
 *  build_wire_shapes`) — `reviveWireFields`'s registry.
 *
 *  cratestack#499 review: an earlier version of this revival scheme kept a
 *  single flat `Set<string>` of every `Decimal` field name reachable from
 *  a response's root type (its own fields *and* every relation's/`type`'s
 *  fields, unioned together) and matched it against a decoded response's
 *  keys at *any* nesting depth. That's provably unsound the moment two
 *  *different* reachable types can each contribute a field name to the
 *  same flat set: a non-`Decimal` field in one type that happens to share
 *  a name with a `Decimal` field in another reachable type gets wrongly
 *  converted. Confirmed empirically (not just theorized) with an
 *  `Order.total: Decimal` + related `Account.total: String` schema,
 *  `include`-ing the relation: a real (non-numeric) account reference
 *  threw `[DecimalError] Invalid argument]` decoding a perfectly valid
 *  response, and a numeric-looking one (`"00123"`) was silently corrupted
 *  into `Decimal("123")`, losing its leading zeros.
 *
 *  This registry fixes that by keeping every type's `Decimal` field names
 *  in *that type's own* `WireShape` only, never merged with another
 *  type's. `reviveShaped` (below) looks up a nested field's *own* shape
 *  via `nested` rather than testing the parent's key set against it, so
 *  `Account.total` is only ever checked against *Account's* shape (which,
 *  correctly, has no `total` key) — not `Order`'s. */
export const wireShapes: Readonly<Record<string, WireShape>> = {
  Principal: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'ownerId'], bytesKeys: [], bytesListKeys: [], nested: { 'memberships': 'TeamMember' } },
  Team: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id'], bytesKeys: [], bytesListKeys: [], nested: { 'members': 'TeamMember', 'projects': 'Project' } },
  TeamMember: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'teamId', 'principalId'], bytesKeys: [], bytesListKeys: [], nested: { 'team': 'Team', 'principal': 'Principal' } },
  Project: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'teamId'], bytesKeys: [], bytesListKeys: [], nested: { 'team': 'Team', 'tasks': 'Task' } },
  Task: { decimalKeys: [], bigintKeys: ['leaseEpoch'], uuidKeys: ['id', 'projectId', 'parentId', 'claimedBy', 'createdBy'], bytesKeys: [], bytesListKeys: [], nested: { 'project': 'Project', 'items': 'Item', 'comments': 'Comment' } },
  Item: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'taskId', 'doneBy'], bytesKeys: [], bytesListKeys: [], nested: { 'task': 'Task' } },
  Comment: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'taskId', 'authorId'], bytesKeys: [], bytesListKeys: [], nested: { 'task': 'Task' } },
  TaskDep: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'taskId', 'dependsOn'], bytesKeys: [], bytesListKeys: [], nested: { 'task': 'Task' } },
  WakeTarget: { decimalKeys: [], bigintKeys: [], uuidKeys: ['id', 'agentId'], bytesKeys: [], bytesListKeys: [], nested: { 'agent': 'Principal' } },
  Event: { decimalKeys: [], bigintKeys: ['seq'], uuidKeys: ['projectId', 'taskId', 'actorId'], bytesKeys: [], bytesListKeys: [], nested: { 'project': 'Project' } },
  TaskItemView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  TaskCommentView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  TaskLinkView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  TaskView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  TaskDetail: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'items': 'TaskItemView', 'comments': 'TaskCommentView', 'dependencies': 'TaskLinkView', 'subtasks': 'TaskLinkView' } },
  LeaseView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  ClaimResult: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'task': 'TaskDetail', 'lease': 'LeaseView' } },
  LeaseRenewal: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  MarkdownDoc: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  OkResult: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  ProjectSummary: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  TeamRole: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  WhoamiView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'teams': 'TeamRole', 'current_leases': 'TaskView' } },
  ChangePayload: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  ChangeView: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payload': 'ChangePayload' } },
  ChangeFeed: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'events': 'ChangeView' } },
  WaitResult: { decimalKeys: [], bigintKeys: [], uuidKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'ready': 'TaskView' } },
};

/** Decodes `value` against the named entry in {@link wireShapes},
 *  replacing every string (or, for a `Decimal[]` field, every element of
 *  an array of strings) at a key that shape's own `decimalKeys` names
 *  with a real {@link Decimal}, every canonical decimal string (or array
 *  of them) at a `bigintKeys` name with a `bigint`, every 16-byte
 *  `Uint8Array` (or array of them) at a `uuidKeys` name with the canonical
 *  UUID string, and every wire integer
 *  array at a `bytesKeys`/`bytesListKeys` name with a `Uint8Array`, and
 *  recursing into any key `nested` names using *that* field's own shape.
 *
 *  A `bigintKeys` key holding anything but a canonical decimal string (or
 *  an array of them, or `null`) **throws** a `TypeError` naming the type
 *  and field. See {@link toBigInt}.
 *
 *  A `shapeName` not found in the registry (a plain scalar or enum
 *  return, e.g. `echoName(): string`) is a documented no-op fast path, so
 *  every model/procedure call site calls this unconditionally rather than
 *  the generator branching per call site. */
export function reviveWireFields(value: unknown, shapeName: string): unknown {
  const shape = wireShapes[shapeName];
  if (!shape) {
    return value;
  }
  return reviveShaped(value, shape, shapeName);
}

/** {@link reviveWireFields} for a `Page<T>` envelope: applies `T`'s own
 *  shape to `.items` (a `Page` envelope's own keys — `items`/`totalCount`/
 *  `pageInfo` — are never themselves `T`'s fields, so `T`'s shape can't be
 *  applied to the envelope directly the way a plain `T`/`T[]` response
 *  can), leaving `totalCount`/`pageInfo` untouched. */
export function revivePagedWireFields(value: unknown, shapeName: string): unknown {
  const shape = wireShapes[shapeName];
  if (!shape || value === null || typeof value !== "object" || !("items" in value)) {
    return value;
  }
  const page = value as { items: unknown };
  return { ...page, items: reviveShaped(page.items, shape, shapeName) };
}

function reviveShaped(value: unknown, shape: WireShape, shapeName: string): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => reviveShaped(item, shape, shapeName));
  }
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      const nestedShapeName = shape.nested[key];
      if (nestedShapeName !== undefined) {
        const nestedShape = wireShapes[nestedShapeName];
        result[key] = nestedShape ? reviveShaped(entry, nestedShape, nestedShapeName) : entry;
      } else if (shape.decimalKeys.includes(key)) {
        result[key] = toDecimal(entry);
      } else if (shape.bigintKeys.includes(key)) {
        result[key] = toBigInt(entry, `${shapeName}.${key}`);
      } else if (shape.uuidKeys.includes(key)) {
        result[key] = toUuid(entry, `${shapeName}.${key}`);
      } else if (shape.bytesKeys.includes(key)) {
        result[key] = toBytes(entry);
      } else if (shape.bytesListKeys.includes(key)) {
        result[key] = Array.isArray(entry) ? entry.map(toBytes) : entry;
      } else {
        result[key] = entry;
      }
    }
    return result;
  }
  return value;
}

/** One `Decimal` leaf (or an array of them), wire form -> {@link Decimal}.
 *
 *  The same conversion as {@link reviveWireScalar}'s `"decimal"` arm, for a
 *  field rather than a bare procedure return (cratestack#1155). A `Decimal[]`
 *  field shares the one `decimalKeys` list with a scalar `Decimal`, so an
 *  array is revived item by item here, as `reviveWireScalar` already does
 *  for a bare `Decimal[]` return. A `null` element, a `null` array and a
 *  non-string value pass through untouched, as they did before. */
function toDecimal(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(toDecimal);
  }
  return typeof value === "string" ? new Decimal(value) : value;
}

/** The canonical `BigInt` wire grammar (ADR 0019 D2): `0`, or an optional
 *  `-`, a non-zero digit and at most 18 more digits. No `+`, no leading
 *  zeros, no `-0`, no whitespace. The server writes only this form, so a
 *  string outside it is not something to normalise but something to refuse. */
const CANONICAL_BIGINT = /^(0|-?[1-9][0-9]{0,18})$/;
const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;

/** One `BigInt` leaf (or an array of them), wire form -> `bigint`.
 *
 *  `null` passes through: a nullable `BigInt` column is `null` on the wire
 *  and `null` in the generated `bigint | null` type. An array is revived
 *  item by item, which is how a `BigInt[]` field decodes.
 *
 *  **Anything else throws**, unlike {@link toBytes}, which hands back what
 *  it was given. A `number` (or a `bigint`, which `@cratestack/cbor-node`
 *  returns for a large CBOR integer) at a `BigInt` key means the server
 *  predates the cutover, and a JSON number above 2^53 may already have been
 *  rounded on its way here. Reading it as the integer it appears to be would
 *  be a silent wrong answer, and passing it through would put a `number`
 *  under a `bigint` type. A string that is not the canonical form, or is
 *  outside `i64`, is refused for the same reason.
 *
 *  `where` is the `Type.field` the value was found at, for the message. */
function toBigInt(value: unknown, where: string): unknown {
  if (value === null || value === undefined) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toBigInt(item, where));
  }
  if (typeof value === "string" && CANONICAL_BIGINT.test(value)) {
    const parsed = BigInt(value);
    if (parsed >= I64_MIN && parsed <= I64_MAX) {
      return parsed;
    }
  }
  const found = typeof value === "object" ? "an object" : `${typeof value} ${String(value)}`;
  throw new TypeError(
    `BigInt field ${where}: expected a canonical decimal string, got ${found}`,
  );
}

/** One `Uuid` leaf (or an array of them), wire form -> canonical string
 *  (cratestack#1359).
 *
 *  Over CBOR the server writes a `Uuid` as a 16-byte byte string, which
 *  `@cratestack/cbor` decodes to a `Uint8Array` — a lie under the generated
 *  `string` type. This formats those 16 bytes as lowercase hyphenated
 *  `8-4-4-4-12` hex. A string (the JSON wire, and any server that predates
 *  the byte-string form) and `null` pass through untouched; an array is
 *  revived item by item, which is how a `Uuid[]` decodes.
 *
 *  **Anything else throws**, like {@link toBigInt}: a `Uint8Array` of the
 *  wrong length, or a number or object, under a `string` type is a silent
 *  wrong answer if passed through.
 *
 *  Requests are not revived or re-encoded: a `Uuid` is sent as text, and the
 *  server accepts text or bytes (`cratestack_core::lenient_uuid`).
 *
 *  `where` is the `Type.field` the value was found at, for the message. */
function toUuid(value: unknown, where: string): unknown {
  if (value === null || value === undefined || typeof value === "string") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toUuid(item, where));
  }
  if (value instanceof Uint8Array && value.length === 16) {
    const hex = Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  }
  const found =
    value instanceof Uint8Array
      ? `${value.length} bytes`
      : typeof value === "object"
        ? "an object"
        : `${typeof value} ${String(value)}`;
  throw new TypeError(
    `Uuid field ${where}: expected a UUID string or 16 bytes, got ${found}`,
  );
}

/** One `Bytes` leaf, wire form -> `Uint8Array`.
 *
 *  `null` passes through untouched: a nullable `Bytes` column is `null`
 *  on the wire, and turning that into an empty `Uint8Array` would erase
 *  the difference between "no value" and "zero bytes" — the same
 *  distinction the generated `Uint8Array | null` field type keeps.
 *
 *  Anything that isn't an array is also passed through rather than
 *  coerced. The shape registry says this key *should* be `Bytes`, but a
 *  response that disagrees (a hand-rolled server, a proxy rewriting
 *  bodies, a schema/deployment skew) is better surfaced to the caller as
 *  the value it actually was than silently turned into empty bytes. */
function toBytes(value: unknown): unknown {
  return Array.isArray(value) ? Uint8Array.from(value as number[]) : value;
}

/** Counterpart to {@link reviveWireFields} for a procedure whose return
 *  type is a bare revivable scalar — `Decimal`, `BigInt`, `Uuid` or `Bytes`, at any
 *  arity —
 *  rather than an object with such fields. {@link reviveWireFields} only
 *  walks object/array *containers* looking for shaped properties, so a
 *  raw top-level decoded value needs this simpler counterpart instead
 *  (cratestack#498 F2: a procedure like `quote(): Decimal` was previously
 *  declared `Decimal` but decoded as an untouched `string`).
 *
 *  `kind` comes from `crate::wire_shapes::ScalarRevival`, rendered into
 *  the call site by the generator, because the wire form alone cannot say
 *  which conversion is wanted: `Bytes` and `Bytes[]` are `number[]` and
 *  `number[][]` when populated but both `[]` when empty. An unrecognised
 *  `kind` is a no-op rather than a throw — a generated client should never
 *  produce one, and degrading to the raw decoded value beats failing a
 *  response that is otherwise perfectly readable.
 *
 *  See `procedure_views.rs::ProcedureView::revival_kind` for which
 *  generated call sites use this vs. {@link reviveWireFields}. */
export function reviveWireScalar(value: unknown, kind: string): unknown {
  if (kind === "bytes") {
    // A bare `Bytes`/`Bytes?` return: one wire integer array (or `null`).
    return toBytes(value);
  }
  if (kind === "bytesList") {
    // A bare `Bytes[]` return: an array *of* wire integer arrays. Handled
    // here rather than by recursing through the `"bytes"` branch, which
    // would see the outer array and convert it whole.
    return Array.isArray(value) ? value.map(toBytes) : value;
  }
  if (kind === "bigint") {
    // A bare `BigInt`, `BigInt?` or `BigInt[]` return. Strict, like the
    // field-level key: see {@link toBigInt}.
    return toBigInt(value, "procedure return");
  }
  if (kind === "uuid") {
    // A bare `Uuid`, `Uuid?` or `Uuid[]` return: 16 bytes -> string
    // (cratestack#1359). Strict, see {@link toUuid}.
    return toUuid(value, "procedure return");
  }
  if (kind !== "decimal") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => reviveWireScalar(item, kind));
  }
  return typeof value === "string" ? new Decimal(value) : value;
}

/** Encode-side counterpart to {@link reviveWireFields}/
 *  {@link reviveWireScalar} (cratestack#746 follow-up): every RPC
 *  request body — `create`/`update` inputs, procedure arguments (plain,
 *  nested, or array-valued), a `FindMany<Model>` argument's
 *  `<Model>Where`/`DecimalFilter` operators, and each frame's own `input`
 *  inside a `batch()` payload — can carry a real {@link Decimal} instance
 *  wherever the schema declares a `Decimal`-typed field, and every one of
 *  those needs to reach the wire as a plain string before it reaches a
 *  codec's `encode()`.
 *
 *  `jsonRpcCodec`'s `JSON.stringify` already handles this for free via
 *  `Decimal.prototype.toJSON` (an alias for `.toString()`, see the
 *  `Decimal` export's own doc comment above) — but `@cratestack/cbor`
 *  (the default codec since #746) walks a value's *own enumerable
 *  properties* on its way to a `serde_json::Value` and never calls
 *  `toJSON`. `decimal.js`'s `clone()` (used to build this file's
 *  `Decimal` export) assigns `constructor` as an own enumerable property
 *  pointing at a function, and a function has no `serde_json::Value`
 *  representation — so encoding any request body containing a real
 *  `Decimal` instance under the native codec throws `JS functions cannot
 *  be represented as a serde_json::Value` before a request is ever sent.
 *  Confirmed against the actually-published `@cratestack/cbor`, not
 *  theorized.
 *
 *  A `bigint` (a `BigInt` field, ADR 0019) is converted here too, to its
 *  canonical decimal string, for BOTH codecs: that string is the one wire
 *  form on JSON and CBOR alike. Unlike a `Decimal`, a `bigint` is not merely
 *  awkward for the native codec but silently wrong: it would be encoded as a
 *  CBOR integer, which the server refuses for a `BigInt` field. Also unlike
 *  a `Decimal`, `JSON.stringify` has no `toJSON` to fall back on and throws,
 *  so the JSON codec needs {@link encodeBinaryAsJson}'s own `bigint` arm for
 *  the callers that never reach this function.
 *
 *  Deliberately NOT built on {@link wireShapes}/{@link reviveShaped}'s
 *  by-field-name registry, unlike the decode-side functions above: this
 *  direction already has a real, unambiguous {@link Decimal} instance in
 *  hand at every leaf, so `value instanceof Decimal` identifies exactly
 *  what needs converting with no risk of the field-name collision hazard
 *  {@link wireShapes}'s own doc comment (`crate::decimal`, cratestack#499)
 *  exists to avoid — and it needs no per-shape registry entry for
 *  `<Model>Where`/procedure-argument wrapper interfaces, neither of which
 *  {@link wireShapes} describes at all (it is built only for
 *  models/`type`s, which is all the *decode* side ever needs — see that
 *  module's doc comment). A plain recursive walk that only special-cases
 *  the one JS runtime type that can't survive the native codec covers
 *  every shape above uniformly.
 *
 *  Called unconditionally from `rpc-runtime.ts.j2`'s `terminalLink`/
 *  `rpc-stream-terminal.ts.j2`'s `terminalStreamLink` — the two places
 *  every unary/batch/stream request body reaches a codec's `encode()` —
 *  rather than gated on which codec is in use: converting a `Decimal` to
 *  its `.toString()` form before `JSON.stringify` produces the
 *  byte-identical request body `jsonRpcCodec` already sends (the same
 *  `.toString()` `Decimal.prototype.toJSON` calls), so applying it
 *  unconditionally is a true no-op for a caller who passes
 *  `codec: jsonRpcCodec`. */
/** Rewrites every `Uint8Array` in `value` into the plain integer array a
 *  `Bytes` field travels as on the wire, every `bigint` into its canonical
 *  decimal string, and every {@link Decimal} into its string, leaving
 *  everything else alone.
 *
 *  **JSON encode paths only** — {@link jsonRpcCodec}, the REST runtime's
 *  request body, and the REST runtime's JSON-encoded query values
 *  (`computedParams`). The native `@cratestack/cbor` codec must keep
 *  receiving the real `Uint8Array`, because it encodes one as a CBOR byte
 *  string (RFC 8949 major type 2, cratestack#783), which is both correct
 *  and about half the bytes of the integer array. Applying this
 *  unconditionally would throw that away.
 *
 *  The `bigint` and `Decimal` arms are the reason this walk, and not
 *  `JSON.stringify` alone, sits in front of every JSON encode.
 *  `JSON.stringify` throws on a `bigint` ("Do not know how to serialize a
 *  BigInt"), and a `Decimal` only survives because of its `toJSON`, which
 *  this walk would otherwise bypass: it rebuilds every object through
 *  `Object.entries`, turning a `Decimal` into its internal `{s, e, d}`
 *  fields. Converting both here means no JSON path can reach
 *  `JSON.stringify` holding either, whichever caller got there first
 *  (`@cratestack/link-batch` hands a codec raw inputs that
 *  {@link encodeWireFields} never saw).
 *
 *  `JSON.stringify` cannot do this itself: a `Uint8Array` has no `toJSON`,
 *  so it serializes as an index-keyed *object* (`{"0":1,"1":2}`) that no
 *  server-side `Vec<u8>` can decode — the same defect cratestack#783 fixed
 *  on the CBOR side, in a different disguise.
 *
 *  A `JSON.stringify` replacer would be cheaper than this pre-walk but is
 *  not sufficient: Node's `Buffer` is a `Uint8Array` subclass (so it is
 *  assignable to a generated `Bytes` field) *and* defines its own
 *  `toJSON`, which `JSON.stringify` applies **before** the replacer ever
 *  sees the value — yielding `{"type":"Buffer","data":[...]}`. Walking the
 *  value first sidesteps `toJSON` entirely, so a `Buffer` and a plain
 *  `Uint8Array` encode identically. Measured, not assumed. */
export function encodeBinaryAsJson(value: unknown): unknown {
  if (typeof value === "bigint" || value instanceof Decimal) {
    return value.toString();
  }
  if (value instanceof Uint8Array) {
    return Array.from(value);
  }
  if (Array.isArray(value)) {
    return value.map((item) => encodeBinaryAsJson(item));
  }
  // `instanceof Uint8Array` above already caught every typed array this
  // client's own types can produce; anything else object-shaped is walked
  // for nested binary. (`Decimal` and `bigint` were handled above, before
  // this walk can rebuild them.)
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      result[key] = encodeBinaryAsJson(entry);
    }
    return result;
  }
  return value;
}

export function encodeWireFields(value: unknown): unknown {
  if (value instanceof Decimal) {
    return value.toString();
  }
  // A `BigInt` field travels as its canonical decimal string on every codec
  // (ADR 0019 D2), so unlike `Uint8Array` below there is no codec that wants
  // the real value. Required, not cosmetic: `JSON.stringify` throws on a
  // `bigint`, and the native codec would silently encode one as a CBOR
  // *integer*, which a server refuses (a `BigInt` is a text string there).
  // `bigint.toString()` is plain decimal (`i64`'s `Display`), so the output
  // is always the canonical form.
  if (typeof value === "bigint") {
    return value.toString();
  }
  // Pass a `Bytes` value through UNTOUCHED (cratestack#820). Load-bearing
  // and easy to delete by accident, because it looks like a no-op.
  //
  // A `Uint8Array` is not `Array.isArray`, so without this it falls into
  // the generic object branch below and is rebuilt through
  // `Object.entries` into `{"0":1,"1":2,"2":3}`. That happens before
  // EITHER codec runs — `terminalLink` calls this function
  // unconditionally — so it broke both of them at once: the native codec
  // emitted a CBOR map where a byte string belongs, and on the JSON path
  // `encodeBinaryAsJson` never saw a real `Uint8Array` to convert, so its
  // `Array.from` never fired either. Measured on a real generated client,
  // not theorized.
  //
  // Returning it as-is is correct for both paths rather than a
  // native-only carve-out: the native codec wants the real typed array
  // (RFC 8949 major type 2, cratestack#783), and the JSON path wants
  // `encodeBinaryAsJson` to receive one so it can produce the integer
  // array — which is exactly what this restores.
  if (value instanceof Uint8Array) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => encodeWireFields(item));
  }
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      result[key] = encodeWireFields(entry);
    }
    return result;
  }
  return value;
}

// Mirrors cratestack-core::page::{Page, PageInfo} exactly — this is
// the literal wire shape every `@@paged` list route serializes with
// `#[serde(rename_all = "camelCase")]`, not an independently designed
// client-side type. Keep field names and optionality in lockstep with
// that struct; do not add/rename fields here without changing it
// there first.
export interface PageInfo {
  limit: number | null;
  offset: number | null;
  hasNextPage: boolean;
  hasPreviousPage: boolean;
  /** Keyset cursor for the next page; present only on cursor-paged list calls with more rows. */
  nextCursor?: string | null;
}

export interface Page<T> {
  items: T[];
  totalCount: number | null;
  pageInfo: PageInfo;
}

// Mirrors cratestack-core::page::PageInput exactly — the request-side
// counterpart to Page/PageInfo above, currently usable only as a
// procedure argument type. Keep field names and optionality in lockstep
// with that struct.
export interface PageInput {
  limit: number | null;
  offset: number | null;
}

// Mirrors cratestack-core::page::{CursorPageInput, CursorPage} exactly
// (`#[serde(rename_all = "camelCase")]`): keyset paging for procedures.
// `cursor` absent/empty is the first page; pass a previous page's
// `nextCursor` to continue. `nextCursor` is omitted on the last page.
export interface CursorPageInput {
  limit: number | null;
  cursor: string | null;
}

export interface CursorPage<T> {
  items: T[];
  nextCursor?: string | null;
}

// Shared building blocks for every `<Model>Where`/`<Model>FindMany`
// pair below (search-with-filters for procedures — mirrors
// cratestack-core::find_many::FieldFilterInput and
// cratestack-macros's per-model `<Model>Where`/`<Model>SortField`/
// `<Model>OrderByClause`/`<Model>FindManyInput` exactly). Usable only
// as a procedure argument type.
export interface EqualityFilter<V> {
  eq?: V;
  ne?: V;
  in?: V[];
  isNull?: boolean;
}

export interface ComparableFilter<V> extends EqualityFilter<V> {
  lt?: V;
  lte?: V;
  gt?: V;
  gte?: V;
}

export interface StringFilter extends ComparableFilter<string> {
  contains?: string;
  startsWith?: string;
}

export type NumberFilter = ComparableFilter<number>;
// ADR 0019: `bigint`, the exact `i64` counterpart to `NumberFilter`. Like
// `DecimalFilter` it only travels outbound, as part of a `<Model>Where`/
// `FindMany` procedure argument, and `encodeWireFields` turns each operand
// into the canonical decimal string the server reads.
export type BigIntFilter = ComparableFilter<bigint>;
export type BooleanFilter = EqualityFilter<boolean>;
// cratestack#1359: operands are sent as text, which the server accepts
// under CBOR as well as the 16-byte form it writes.
export type UuidFilter = ComparableFilter<string>;
export type DateTimeFilter = ComparableFilter<string>;
// cratestack#498: `Decimal`, not `string` — see this file's own
// `Decimal`/`reviveWireFields` doc comments above for why. Unlike a
// response's own `Decimal` fields, a `DecimalFilter` only ever travels
// *outbound* as part of a `<Model>Where`/`FindMany` procedure argument
// (`find_many_views.rs`'s own doc comment — "usable only as a procedure
// argument type"), so it needs no `reviveWireFields` counterpart on
// decode. It DOES need `encodeWireFields` on encode, same as every
// other `Decimal`-carrying request body (see that function's own doc
// comment for why `Decimal.prototype.toJSON`/`JSON.stringify` alone isn't
// enough once `@cratestack/cbor` is the codec, cratestack#746 follow-up)
// — `rpc-runtime.ts.j2`'s `terminalLink` applies it uniformly to every
// outbound RPC request body, so a `FindMany<Model>` argument's `where`
// filter needs no separate glue at this call site either.
export type DecimalFilter = ComparableFilter<Decimal>;

export type SortDirection = "asc" | "desc";

export type PrincipalSortField = 'id' | 'kind' | 'name' | 'ownerId' | 'oidcLogin' | 'agentKind' | 'runtime' | 'maxLeases' | 'isAdmin' | 'disabledAt' | 'lastSeenAt' | 'createdAt';
export const PrincipalSortFieldValues = [
  "id",
  "kind",
  "name",
  "ownerId",
  "oidcLogin",
  "agentKind",
  "runtime",
  "maxLeases",
  "isAdmin",
  "disabledAt",
  "lastSeenAt",
  "createdAt",
] as const satisfies readonly PrincipalSortField[];

export type TeamSortField = 'id' | 'name';
export const TeamSortFieldValues = [
  "id",
  "name",
] as const satisfies readonly TeamSortField[];

export type TeamMemberSortField = 'id' | 'teamId' | 'principalId' | 'role';
export const TeamMemberSortFieldValues = [
  "id",
  "teamId",
  "principalId",
  "role",
] as const satisfies readonly TeamMemberSortField[];

export type ProjectSortField = 'id' | 'teamId' | 'key' | 'name' | 'repoUrl' | 'nextNumber';
export const ProjectSortFieldValues = [
  "id",
  "teamId",
  "key",
  "name",
  "repoUrl",
  "nextNumber",
] as const satisfies readonly ProjectSortField[];

export type TaskSortField = 'id' | 'projectId' | 'number' | 'title' | 'body' | 'priority' | 'parentId' | 'status' | 'claimedBy' | 'leaseEpoch' | 'leaseExpiresAt' | 'result' | 'createdBy' | 'createdAt' | 'updatedAt';
export const TaskSortFieldValues = [
  "id",
  "projectId",
  "number",
  "title",
  "body",
  "priority",
  "parentId",
  "status",
  "claimedBy",
  "leaseEpoch",
  "leaseExpiresAt",
  "result",
  "createdBy",
  "createdAt",
  "updatedAt",
] as const satisfies readonly TaskSortField[];

export type ItemSortField = 'id' | 'taskId' | 'position' | 'text' | 'done' | 'doneBy';
export const ItemSortFieldValues = [
  "id",
  "taskId",
  "position",
  "text",
  "done",
  "doneBy",
] as const satisfies readonly ItemSortField[];

export type CommentSortField = 'id' | 'taskId' | 'authorId' | 'body' | 'createdAt';
export const CommentSortFieldValues = [
  "id",
  "taskId",
  "authorId",
  "body",
  "createdAt",
] as const satisfies readonly CommentSortField[];

export type TaskDepSortField = 'id' | 'taskId' | 'dependsOn';
export const TaskDepSortFieldValues = [
  "id",
  "taskId",
  "dependsOn",
] as const satisfies readonly TaskDepSortField[];

export type WakeTargetSortField = 'id' | 'agentId' | 'type';
export const WakeTargetSortFieldValues = [
  "id",
  "agentId",
  "type",
] as const satisfies readonly WakeTargetSortField[];

export type EventSortField = 'seq' | 'projectId' | 'taskId' | 'kind' | 'actorId' | 'payload' | 'at';
export const EventSortFieldValues = [
  "seq",
  "projectId",
  "taskId",
  "kind",
  "actorId",
  "payload",
  "at",
] as const satisfies readonly EventSortField[];

export interface TaskItemView {
  position: number;
  text: string;
  done: boolean;
  done_by?: string | null;
}

export interface TaskCommentView {
  author?: string | null;
  body: string;
  at: string;
}

export interface TaskLinkView {
  id: string;
  title: string;
  status: string;
}

export interface TaskView {
  id: string;
  project: string;
  number: number;
  title: string;
  body: string;
  priority: number;
  labels: string[];
  status: string;
  parent?: string | null;
  depends_on: string[];
  claimed_by?: string | null;
  lease_epoch: number;
  lease_expires_at?: string | null;
  lease_live?: boolean | null;
  ready?: boolean | null;
  open_items: number;
  total_items: number;
  result?: string | null;
  created_at: string;
  updated_at: string;
}

export interface TaskDetail {
  id: string;
  project: string;
  number: number;
  title: string;
  body: string;
  priority: number;
  labels: string[];
  status: string;
  parent?: string | null;
  depends_on: string[];
  claimed_by?: string | null;
  lease_epoch: number;
  lease_expires_at?: string | null;
  lease_live?: boolean | null;
  ready?: boolean | null;
  open_items: number;
  total_items: number;
  result?: string | null;
  created_at: string;
  updated_at: string;
  items: TaskItemView[];
  comments: TaskCommentView[];
  dependencies: TaskLinkView[];
  subtasks: TaskLinkView[];
}

export interface LeaseView {
  epoch: number;
  expires_at: string;
}

export interface ClaimResult {
  task?: TaskDetail | null;
  lease?: LeaseView | null;
  none?: boolean | null;
}

export interface LeaseRenewal {
  id: string;
  epoch: number;
  expires_at: string;
}

export interface MarkdownDoc {
  markdown: string;
}

export interface OkResult {
  ok: boolean;
}

export interface ProjectSummary {
  key: string;
  name: string;
  repo_url?: string | null;
  team?: string | null;
  lease_ttl_s: number;
  open: number;
  ready: number;
  claimed: number;
  blocked: number;
  done: number;
}

export interface TeamRole {
  name: string;
  role: string;
}

export interface WhoamiView {
  name: string;
  kind: string;
  agent_kind?: string | null;
  runtime?: string | null;
  is_admin: boolean;
  max_leases: number;
  teams: TeamRole[];
  projects: string[];
  current_leases: TaskView[];
}

export interface ChangePayload {
  title?: string | null;
  agent?: string | null;
  epoch?: number | null;
  position?: number | null;
  done?: boolean | null;
  result?: string | null;
  reason?: string | null;
}

export interface ChangeView {
  seq: number;
  kind: string;
  project: string;
  task?: string | null;
  actor?: string | null;
  at: string;
  payload: ChangePayload;
}

export interface ChangeFeed {
  events: ChangeView[];
  next_cursor: number;
}

export interface WaitResult {
  ready: TaskView[];
}

export interface Principal {
  id?: string;
  kind?: string;
  name?: string;
  ownerId?: string | null;
  oidcLogin?: string | null;
  agentKind?: string | null;
  runtime?: string | null;
  maxLeases?: number;
  isAdmin?: boolean;
  disabledAt?: string | null;
  lastSeenAt?: string | null;
  createdAt?: string;
  memberships?: TeamMember[];
}

export interface UpdatePrincipalInput {
  kind?: string;
  name?: string;
  ownerId?: string | null;
  oidcLogin?: string | null;
  agentKind?: string | null;
  runtime?: string | null;
  maxLeases?: number;
  isAdmin?: boolean;
  disabledAt?: string | null;
  lastSeenAt?: string | null;
  createdAt?: string;
}

export interface PrincipalWhere {
  id?: UuidFilter;
  kind?: StringFilter;
  name?: StringFilter;
  ownerId?: UuidFilter;
  oidcLogin?: StringFilter;
  agentKind?: StringFilter;
  runtime?: StringFilter;
  maxLeases?: NumberFilter;
  isAdmin?: BooleanFilter;
  disabledAt?: DateTimeFilter;
  lastSeenAt?: DateTimeFilter;
  createdAt?: DateTimeFilter;
}

export interface PrincipalOrderByClause {
  field: PrincipalSortField;
  direction: SortDirection;
}

export interface PrincipalFindMany {
  where?: PrincipalWhere;
  orderBy?: PrincipalOrderByClause[];
}

export interface Team {
  id?: string;
  name?: string;
  members?: TeamMember[];
  projects?: Project[];
}

export interface CreateTeamInput {
  name: string;
}

export interface UpdateTeamInput {
  name?: string;
}

export interface TeamWhere {
  id?: UuidFilter;
  name?: StringFilter;
}

export interface TeamOrderByClause {
  field: TeamSortField;
  direction: SortDirection;
}

export interface TeamFindMany {
  where?: TeamWhere;
  orderBy?: TeamOrderByClause[];
}

export interface TeamMember {
  id?: string;
  teamId?: string;
  principalId?: string;
  role?: string;
  team?: Team;
  principal?: Principal;
}

export interface CreateTeamMemberInput {
  teamId: string;
  principalId: string;
  role: string;
}

export interface UpdateTeamMemberInput {
  teamId?: string;
  principalId?: string;
  role?: string;
}

export interface TeamMemberWhere {
  id?: UuidFilter;
  teamId?: UuidFilter;
  principalId?: UuidFilter;
  role?: StringFilter;
}

export interface TeamMemberOrderByClause {
  field: TeamMemberSortField;
  direction: SortDirection;
}

export interface TeamMemberFindMany {
  where?: TeamMemberWhere;
  orderBy?: TeamMemberOrderByClause[];
}

export interface Project {
  id?: string;
  teamId?: string;
  key?: string;
  name?: string;
  repoUrl?: string | null;
  nextNumber?: number;
  team?: Team;
  tasks?: Task[];
}

export interface CreateProjectInput {
  teamId: string;
  key: string;
  name: string;
  repoUrl?: string | null;
  nextNumber: number;
}

export interface UpdateProjectInput {
  teamId?: string;
  key?: string;
  name?: string;
  repoUrl?: string | null;
  nextNumber?: number;
}

export interface ProjectWhere {
  id?: UuidFilter;
  teamId?: UuidFilter;
  key?: StringFilter;
  name?: StringFilter;
  repoUrl?: StringFilter;
  nextNumber?: NumberFilter;
}

export interface ProjectOrderByClause {
  field: ProjectSortField;
  direction: SortDirection;
}

export interface ProjectFindMany {
  where?: ProjectWhere;
  orderBy?: ProjectOrderByClause[];
}

export interface Task {
  id?: string;
  projectId?: string;
  number?: number;
  title?: string;
  body?: string;
  priority?: number;
  parentId?: string | null;
  status?: string;
  claimedBy?: string | null;
  leaseEpoch?: bigint;
  leaseExpiresAt?: string | null;
  result?: string | null;
  createdBy?: string | null;
  createdAt?: string;
  updatedAt?: string;
  project?: Project;
  items?: Item[];
  comments?: Comment[];
}

export interface UpdateTaskInput {
  projectId?: string;
  number?: number;
  title?: string;
  body?: string;
  priority?: number;
  parentId?: string | null;
  status?: string;
  claimedBy?: string | null;
  leaseEpoch?: bigint;
  leaseExpiresAt?: string | null;
  result?: string | null;
  createdBy?: string | null;
  createdAt?: string;
  updatedAt?: string;
}

export interface TaskWhere {
  id?: UuidFilter;
  projectId?: UuidFilter;
  number?: NumberFilter;
  title?: StringFilter;
  body?: StringFilter;
  priority?: NumberFilter;
  parentId?: UuidFilter;
  status?: StringFilter;
  claimedBy?: UuidFilter;
  leaseEpoch?: BigIntFilter;
  leaseExpiresAt?: DateTimeFilter;
  result?: StringFilter;
  createdBy?: UuidFilter;
  createdAt?: DateTimeFilter;
  updatedAt?: DateTimeFilter;
}

export interface TaskOrderByClause {
  field: TaskSortField;
  direction: SortDirection;
}

export interface TaskFindMany {
  where?: TaskWhere;
  orderBy?: TaskOrderByClause[];
}

export interface Item {
  id?: string;
  taskId?: string;
  position?: number;
  text?: string;
  done?: boolean;
  doneBy?: string | null;
  task?: Task;
}

export interface UpdateItemInput {
  taskId?: string;
  position?: number;
  text?: string;
  done?: boolean;
  doneBy?: string | null;
}

export interface ItemWhere {
  id?: UuidFilter;
  taskId?: UuidFilter;
  position?: NumberFilter;
  text?: StringFilter;
  done?: BooleanFilter;
  doneBy?: UuidFilter;
}

export interface ItemOrderByClause {
  field: ItemSortField;
  direction: SortDirection;
}

export interface ItemFindMany {
  where?: ItemWhere;
  orderBy?: ItemOrderByClause[];
}

export interface Comment {
  id?: string;
  taskId?: string;
  authorId?: string | null;
  body?: string;
  createdAt?: string;
  task?: Task;
}

export interface UpdateCommentInput {
  taskId?: string;
  authorId?: string | null;
  body?: string;
  createdAt?: string;
}

export interface CommentWhere {
  id?: UuidFilter;
  taskId?: UuidFilter;
  authorId?: UuidFilter;
  body?: StringFilter;
  createdAt?: DateTimeFilter;
}

export interface CommentOrderByClause {
  field: CommentSortField;
  direction: SortDirection;
}

export interface CommentFindMany {
  where?: CommentWhere;
  orderBy?: CommentOrderByClause[];
}

export interface TaskDep {
  id?: string;
  taskId?: string;
  dependsOn?: string;
  task?: Task;
}

export interface UpdateTaskDepInput {
  taskId?: string;
  dependsOn?: string;
}

export interface TaskDepWhere {
  id?: UuidFilter;
  taskId?: UuidFilter;
  dependsOn?: UuidFilter;
}

export interface TaskDepOrderByClause {
  field: TaskDepSortField;
  direction: SortDirection;
}

export interface TaskDepFindMany {
  where?: TaskDepWhere;
  orderBy?: TaskDepOrderByClause[];
}

export interface WakeTarget {
  id?: string;
  agentId?: string;
  type?: string;
  agent?: Principal;
}

export interface UpdateWakeTargetInput {
  agentId?: string;
  type?: string;
}

export interface WakeTargetWhere {
  id?: UuidFilter;
  agentId?: UuidFilter;
  type?: StringFilter;
}

export interface WakeTargetOrderByClause {
  field: WakeTargetSortField;
  direction: SortDirection;
}

export interface WakeTargetFindMany {
  where?: WakeTargetWhere;
  orderBy?: WakeTargetOrderByClause[];
}

export interface Event {
  seq?: bigint;
  projectId?: string | null;
  taskId?: string | null;
  kind?: string;
  actorId?: string | null;
  payload?: JsonValue;
  at?: string;
  project?: Project | null;
}

export interface UpdateEventInput {
  projectId?: string | null;
  taskId?: string | null;
  kind?: string;
  actorId?: string | null;
  payload?: JsonValue;
  at?: string;
}

export interface EventWhere {
  seq?: BigIntFilter;
  projectId?: UuidFilter;
  taskId?: UuidFilter;
  kind?: StringFilter;
  actorId?: UuidFilter;
  at?: DateTimeFilter;
}

export interface EventOrderByClause {
  field: EventSortField;
  direction: SortDirection;
}

export interface EventFindMany {
  where?: EventWhere;
  orderBy?: EventOrderByClause[];
}

export interface WhoamiArgs {
}

export interface ProjectOverviewArgs {
}

export interface ProjectTasksArgs {
  project: string;
  status?: string | null;
  label?: string | null;
  readyOnly?: boolean | null;
}

export interface TaskDetailArgs {
  id: string;
}

export interface GetBoardArgs {
  project: string;
}

export interface GetTaskMarkdownArgs {
  id: string;
}

export interface TaskCreateArgs {
  project: string;
  title: string;
  body?: string | null;
  items?: string | null;
  parent?: string | null;
  dependsOn?: string | null;
  labels?: string | null;
  priority?: number | null;
}

export interface TaskEditArgs {
  id: string;
  title?: string | null;
  body?: string | null;
  priority?: number | null;
  labels?: string | null;
}

export interface ClaimNextArgs {
  project: string;
  labels?: string | null;
}

export interface ClaimTaskArgs {
  id: string;
}

export interface HeartbeatArgs {
  id: string;
  epoch: number;
  ttlS?: number | null;
}

export interface CheckItemArgs {
  id: string;
  epoch: number;
  position: number;
  done: boolean;
}

export interface CommentArgs {
  id: string;
  body: string;
  epoch?: number | null;
}

export interface CompleteArgs {
  id: string;
  epoch: number;
  result: string;
}

export interface ReleaseArgs {
  id: string;
  epoch: number;
  reason: string;
}

export interface BlockArgs {
  id: string;
  epoch: number;
  reason: string;
}

export interface ChangesSinceArgs {
  cursor: number;
  project?: string | null;
  limit?: number | null;
}

export interface WaitForWorkArgs {
  project?: string | null;
  timeoutS?: number | null;
}

export interface UnblockTaskArgs {
  id: string;
}

export interface CancelTaskArgs {
  id: string;
}

