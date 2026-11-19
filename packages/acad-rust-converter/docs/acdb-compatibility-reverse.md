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

The TypeScript mapper in `AcDbAcadRustNativeConverter.ts` creates MLightCAD records and currently re-derives native ObjectIds in several paths. The Rust bridge now exposes a first mutation slice for the resident `CadDocument`: model-space entity creation from acadrust serde JSON, entity erase by hexadecimal handle, nested transaction begin/commit/abort, and native DWG serialization through `DwgWriter::write_to_vec` (verified against the pinned acadrust 0.6.6 source). The worker/client transport carries these operations. Existing-entity updates, table/object/dictionary mutations, and a complete AcDb façade are not implemented yet.

Consequently, a complete reverse writer cannot safely be implemented only by reversing the existing TypeScript field assignments. It needs a Rust-side mutation/write contract as well.

## Required next slices

1. **Identity propagation:** route every mapped object through one shared `ObjectIdentityMap`; stop generating unrelated IDs when the source handle is known. Preserve Owner, block-record, dictionary, reactor, XData and extension-dictionary references.
2. **Compatibility façade:** add AcDb-style database/table/entity/object proxies over the current data-model objects. Proxies must not create a second authoritative CAD database.
3. **Mutation protocol (in progress):** extend the initial create/erase/transaction operations to typed updates, symbol-table edits, block/table/object/dictionary mutations, and validated reference changes. Each operation must run against the resident `CadDocument` and return authoritative identities.
4. **Reverse serialization (writer wired, validation pending):** `dwg_save` now calls the pinned acadrust 0.6.6 `DwgWriter::write_to_vec`. Add native DWG round-trip tests and AutoCAD open/AUDIT validation before treating output as production-safe.
5. **Round-trip tests:** compare handles, owners, block references, nested inserts, attributes, XData, dictionaries, tables, hatches, dimensions and unknown/proxy objects. A successful parse alone is not proof that AutoCAD can open the output. The new WASM mutation path has not yet been compiled or runtime-tested in this environment.

## Known blocker: u64 handles in JSON

The existing `AcadRustEntityCommon` and table model types describe handles as TypeScript `number`, while DWG handles are u64. JSON numbers above `Number.MAX_SAFE_INTEGER` cannot be represented exactly by JavaScript. `ObjectIdentityMap` refuses unsafe numeric inputs, but this alone cannot recover precision already lost by `JSON.parse`. The Rust worker protocol must eventually emit handle fields as hexadecimal strings (or use a lossless JSON parser) before claiming full u64 identity preservation.

## Scope discipline

Do not drop unknown/proxy objects just because the viewer cannot render them. Rendering support and serialization preservation are separate capabilities. Do not treat `@mlightcad/data-model` as a second source of truth: the acadrust document remains authoritative for the reverse-write path.
