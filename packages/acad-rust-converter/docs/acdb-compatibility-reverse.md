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


## Transaction failure-path hardening

The TypeScript façade now checks its transaction depth before issuing Rust commit/abort requests. This prevents a façade/Rust stack mismatch from committing or rolling back the resident document before discovering that no corresponding identity snapshot exists. Identity snapshots are retained until Rust confirms abort; if the worker rejects the rollback request, the caller can retry. The transaction() helper preserves the original action/commit error if rollback also fails, attaching the secondary failure as rollbackError. A read-only transactionDepth reports the façade's current nested depth.

Jest cases cover empty-stack commit/abort rejection and retry after a failed worker rollback. These tests have been added but are not executed in this environment.


## Lossless model-space entity-handle sidecar

The Rust/WASM API now exposes `dwg_entity_handle_page_json(handle, offset, limit)`, which returns a page-aligned array of uppercase hexadecimal entity handles directly from `CadDocument`. The worker and TypeScript client expose this as `entityHandlePage`. The native converter fetches this sidecar beside each model-space entity page, verifies equal lengths and valid hexadecimal values, and cross-checks the legacy numeric `common.handle` only when that number is a safe integer. Unsafe JS numbers are never converted back into claimed-exact handles.

This is an additive bridge: existing entity JSON remains backward compatible for the rendering mapper, while a lossless identity channel is now available for future ObjectId binding. It does not yet replace every numeric handle in section JSON or automatically bind every newly created MLightCAD ObjectId. Rust/WASM regeneration and build/runtime verification are still required.


## Extended implementation pass: identity, handles, blocks, INSERT and object metadata

### Automatic mapped-entity identity capture

The native converter accepts an optional `onObjectIdentity(objectId, dwgHandle)` callback. Every successfully mapped model-space or block-definition entity is appended first, then the database-generated MLightCAD ObjectId is paired with the exact hexadecimal DWG handle supplied by the Rust sidecar. Block batch and paged paths both validate exact-handle count/order before mapping. Unsupported entity variants are not assigned a fabricated identity.

The callback is opt-in and does not create a second object store. Consumers can feed these pairs into `ObjectIdentityMap.bindMany` or their own identity index.

### Section-wide exact-handle index

The Rust/WASM API adds `dwg_section_handle_index_json`. For each supported section it serializes in Rust, extracts handle-like fields while they are still exact u64 values, and returns JSON-path + uppercase-hex pairs. `AcadRustDocumentClient.sectionHandleIndex` exposes the sidecar. The native converter can optionally request it for every mapped section through `onSectionHandleIndex`; this is opt-in to avoid the additional serialization/request cost when unused.

This covers the supported section JSON trees, including OBJECTS and table records, but it is not a complete index of private non-Serde side maps or every possible reference field. The field-name matcher currently recognizes `handle`, `*_handle`, `owner`, `block_record`, `block_entity`, and `block_end`.

### Block definition lifecycle

The compatibility façade now supports block creation, member append, and guarded removal. Creation allocates a BLOCK-RECORD plus structural BLOCK and ENDBLK entities. Appending an ordinary entity sets its owner to the target block and delegates membership/index maintenance to `CadDocument::add_entity`. Removal is deliberately restricted to empty, unreferenced, non-layout user blocks; populated or INSERT-referenced blocks are rejected rather than leaving dangling references. Renaming and arbitrary member reordering are not yet exposed.

### INSERT and attributes

Typed INSERT placement patches support insertion point, XYZ scale, rotation, and extrusion normal without replacing block or ATTRIB handles. Single-line ATTRIB value changes preserve its handle, owner and ATTDEF link. ATTDEF default-value changes are separate and do not rewrite existing INSERT attribute instances. Multiline ATTRIB updates are refused until the embedded MTEXT payload can be updated atomically.

### XData, Extension Dictionary, XRecord, Reactor and dictionary references

The façade now exposes XDATA read/upsert/remove, Extension Dictionary creation, named XRecord creation/read/entry replacement, reactor-list replacement, and Extension Dictionary key upsert/removal. Handles cross these APIs as hexadecimal strings. XDATA upsert preserves other applications' records and registers a missing APPID. Reactor and dictionary target handles are validated against resident entities, tables, or OBJECTS; dictionary entries can declare hard-owner semantics. XRecord entry replacement is explicit and resets the stale serialized object-reference stream so references are rebuilt from typed entries.

### Remaining safety boundary

- There is no general Owner reassignment API: changing Owner requires moving the entity between block membership lists and updating related INSERT/attribute semantics.
- There is no generic Reactor reverse-link maintenance; this API replaces the owning entity's reactor list only.
- Block rename, moving entities between blocks, non-empty block deletion, dynamic-block graph editing, and multiline ATTRIB/MTEXT atomic editing remain pending.
- Section handle indexes do not yet replace the legacy numeric handle fields in the renderer JSON. They provide a separate lossless channel for consumers that need to preserve identities/references.
- Rust/WASM compilation, TypeScript build, Jest execution, native DWG save/reopen, AutoCAD open and AUDIT remain required. None is claimed as passed in this branch update.


The native converter also accepts an optional `identityMap: ObjectIdentityMap` config. When supplied, each successful graphical entity mapping automatically binds the newly assigned MLightCAD ObjectId to the exact DWG handle; `onObjectIdentity` remains available as an observer callback. This is currently graphical entity coverage, not a blanket identity map for symbol-table records and non-graphical OBJECTS items.

XDATA and XRecord `Handle` enum values in mutation payloads must be supplied as hexadecimal strings. Rust converts them to the pinned acadrust u64 Handle representation internally; numeric JS-originated handle payloads are rejected to avoid precision loss. XRecord handle references are validated against the resident document before replacement.


The optional converter `identityMap` now also binds the mapped LAYER, LTYPE, STYLE, DIMSTYLE, VPORT and BLOCK-RECORD table records by matching their database-generated ObjectIds to the exact `entries/<name>/handle` paths in the section sidecar. This still does not automatically cover every OBJECTS variant or all non-graphical records.


### Owner reassignment / moving entities between blocks

The Rust bridge now exposes `dwg_move_entity_to_block`. It refuses sub-entities and structural BLOCK/ENDBLK/SEQEND markers, validates the current owner block membership before mutation, removes stale duplicate memberships from other blocks, inserts the entity handle into the target block's membership list, and updates `common.owner_handle` plus model/paper `entity_mode`. The original entity handle, XDATA, reactors, Extension Dictionary and entity-specific references remain untouched. The TypeScript proxy exposes `AcDbEntity.moveToBlock` and `AcDbBlockTableRecord.moveEntityHere`.
