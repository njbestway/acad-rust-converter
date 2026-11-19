# AcDb compatibility and reverse mapping

## Current implementation slice

`compatibility/ObjectIdentityMap.ts` is the first shared primitive for the bidirectional bridge. It maintains a one-to-one mapping between a MLightCAD `ObjectId` and a canonical DWG handle without owning either object's data.

Important invariants:
- ObjectId-to-handle and handle-to-ObjectId mappings are one-to-one.
- Rebinding a conflicting identity throws; callers must unbind explicitly.
- Handles are normalized as uppercase hexadecimal strings.
- `bigint` and hexadecimal strings are accepted losslessly.
- JavaScript `number` handles are accepted only when they are safe integers. Unsafe numbers are rejected instead of silently rounded.
- `clone()` provides an isolated identity-map snapshot suitable for transaction bookkeeping.

## Existing converter boundary (verified in source)

The current native path is:

`DWG bytes -> Rust CadDocument -> section/entity JSON -> AcDbDatabase`

The TypeScript mapper in `AcDbAcadRustNativeConverter.ts` creates MLightCAD records and currently re-derives native ObjectIds in several paths. The Rust bridge exposes model-space entity creation, entity lookup by hexadecimal handle, entity erase, nested transaction begin/commit/abort, and native DWG serialization through `DwgWriter::write_to_vec` (checked against the pinned acadrust 0.6.6 source). Existing entities can be patched in-place for a limited set of common properties (`layer`, `linetype`, `linetype_scale`, `invisible`) plus a typed LINE geometry endpoint update (`start` / `end`). Both patch paths avoid round-tripping handle-bearing fields. This patch deliberately excludes geometry and handle-bearing fields to avoid corrupting u64 references after JavaScript JSON parsing. The worker/client transport carries these operations. `AcDbDatabase`, handle-stable `AcDbEntity`, read-only `AcDbBlockTable`, and `AcDbBlockTableRecord` TypeScript proxies now delegate to the resident Rust document; they do not own a second CAD database. Block record membership is returned as exact hexadecimal entity handles rather than JS numbers.

Consequently, a complete reverse writer cannot safely be implemented only by reversing the existing TypeScript field assignments. It needs a Rust-side mutation/write contract as well.

## Required next slices

1. **Identity propagation:** route every mapped object through one shared `ObjectIdentityMap`; stop generating unrelated IDs when the source handle is known. Preserve Owner, block-record, dictionary, reactor, XData and extension-dictionary references.
2. **Compatibility façade (first slice added):** `AcDbDatabase` and `AcDbEntity` proxies now delegate open/read/common-property patch/append/erase/transaction/save to the resident `CadDocument`. Block-table and block-record lookup/enumeration are present as read-only proxies. Creating/renaming/erasing block definitions and editing other symbol tables and non-entity objects remain outstanding.
3. **Mutation protocol (in progress):** extend the initial create/erase/common-property patch/transaction operations to typed geometry updates for the remaining entity families, symbol-table edits, block/table/object/dictionary mutations, and validated reference changes. Each operation must run against the resident `CadDocument` and return authoritative identities.
4. **Reverse serialization (writer wired, validation pending):** `dwg_save` now calls the pinned acadrust 0.6.6 `DwgWriter::write_to_vec`. Add native DWG round-trip tests and AutoCAD open/AUDIT validation before treating output as production-safe.
5. **Round-trip tests:** compare handles, owners, block references, nested inserts, attributes, XData, dictionaries, tables, hatches, dimensions and unknown/proxy objects. A successful parse alone is not proof that AutoCAD can open the output. The new WASM mutation path and TypeScript façade have not yet been compiled or runtime-tested in this environment. A Jest compatibility façade spec has been added but not executed.

## Known blocker: u64 handles in JSON

The existing `AcadRustEntityCommon` and table model types describe handles as TypeScript `number`, while DWG handles are u64. JSON numbers above `Number.MAX_SAFE_INTEGER` cannot be represented exactly by JavaScript. `ObjectIdentityMap` refuses unsafe numeric inputs, but this alone cannot recover precision already lost by `JSON.parse`. The Rust worker protocol must eventually emit handle fields as hexadecimal strings (or use a lossless JSON parser) before claiming full u64 identity preservation.

## Scope discipline

Do not drop unknown/proxy objects just because the viewer cannot render them. Rendering support and serialization preservation are separate capabilities. Do not treat `@mlightcad/data-model` as a second source of truth: the acadrust document remains authoritative for the reverse-write path.


## Follow-up implementation: identity snapshots and typed geometry

The compatibility façade now exposes its `ObjectIdentityMap` through `AcDbDatabase.identities` and provides `bindObjectIdentity`, `bindObjectIdentities`, `getHandleForObjectId`, and `getObjectIdForHandle`. Batch binding is atomic: a conflicting pair leaves the existing map unchanged. The façade snapshots the identity map alongside each nested Rust transaction; abort restores the corresponding snapshot, and erasing an entity removes its mapped identity.

This is an explicit bridge API, not yet automatic identity capture from every `AcDbAcadRustNativeConverter` mapper. That integration still requires the converter to bind each native record at the moment its DWG handle becomes associated with the resulting MLightCAD ObjectId.

The typed geometry mutation set now includes LINE endpoints and CIRCLE/ARC center/radius/angle fields. These are Rust-side in-place mutations addressed by the exact hexadecimal entity handle; they do not replace the entity or round-trip its common/reference fields through JavaScript. CIRCLE rejects arc angle fields, coordinates and angles must be finite, and radii must be finite and positive. The façade and worker protocol expose these operations.

## Remaining work across the five tracks

- Lossless u64 transport is not complete for every section/object/entity reference. Existing broad section JSON still contains numeric handle fields. Exact handle strings are currently used for the façade's entity addressing, block membership enumeration and the new mutation routes.
- The reverse mapping registry is not yet wired into every converter mapper.
- BlockTable/BlockTableRecord remain read-only; block creation, edit, erase and INSERT reference maintenance are not implemented.
- More typed geometry/property patch families remain to be added (including LWPOLYLINE/POLYLINE, ELLIPSE, SPLINE, TEXT/MTEXT, INSERT and dimensions).
- Dictionary, reactor, XData and extension-dictionary mutation/reference maintenance remain outstanding. Existing resident objects should be preserved in-place until typed operations exist; do not deserialize and replace arbitrary handle-bearing object JSON.
- Rust/TypeScript build, Jest, native DWG round-trip and AutoCAD open/AUDIT validation are still required. No passing result is implied by the presence of tests.
