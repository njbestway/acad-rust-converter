// WASM bridge for the @mlightcad/acad-rust-converter npm package.
//
// Architecture A (`dwg_to_dxf`, MVP): read DWG bytes, emit ASCII DXF bytes. The
// TypeScript converter then hands those bytes to the MIT `AcDbNativeDxfConverter`,
// so no GPL code is involved. This stays the default and the fallback path.
//
// Architecture B (Phase 4, in progress): read a DWG once into a `CadDocument`
// that lives inside wasm linear memory for the duration of one conversion
// (`dwg_open` -> ... -> `dwg_close`), and expose it as *sections* / *entity
// pages* of serde JSON instead of one giant document dump. This is what avoids
// the whole-document OOM footgun that `dwg_to_json` (kept below for offline
// fixture generation only, never wired into the production worker protocol)
// would otherwise hit on large drawings.
//
// Both A and B borrow the caller's in-memory byte slice directly
// (`DwgReader::from_stream` over a `Cursor<&[u8]>`) because browsers only hand
// us an `ArrayBuffer`; there is no filesystem in wasm32-unknown-unknown.
// Wrapping the slice in a `Cursor` without `to_vec` avoids copying the whole
// DWG once more inside wasm memory on top of the wasm-bindgen array handoff.
//
// Slice 0 spike findings (checked against the vendored acadrust source - 0.6.3
// at spike time, still valid on the pinned 0.6.5,
// not guessed from docs) that made the sectioned approach viable without any
// wrapper crate or fork:
//   * `CadDocument` derives `Serialize` under the (already enabled) `serde`
//     feature, so every non-`#[serde(skip)]` field type implements `Serialize`
//     individually - including `HeaderVariables`, every `Table<T>`, and
//     `DxfClassCollection`.
//   * Those section fields (`header`, `layers`, `line_types`, `text_styles`,
//     `block_records`, `dim_styles`, `app_ids`, `views`, `vports`, `ucss`,
//     `vx_table`, `classes`, `objects`, `summary_info`) are `pub` and reachable
//     directly from this crate.
//   * The flat entity store itself (`entities: Vec<Arc<EntityType>>`) is
//     `pub(crate)` - NOT field-accessible here - but the public iterator
//     methods `entities()`, `model_space_entities()` and
//     `entities_in_block(name)` expose the same data as `&EntityType`, which is
//     what gets paged below.
//   * `Handle` serializes as a plain `u64` number; `DxfVersion` as its variant
//     name string (e.g. "AC1032"); `EntityType` is an externally-tagged serde
//     enum, i.e. one JSON object whose single key is the variant name
//     (e.g. `{"Line": {...}}`) - the TypeScript model mirrors exactly that.
use acadrust::{CadDocument, DwgReader, DxfReader, DwgWriter, DxfWriter};
use std::any::Any;
use std::cell::RefCell;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use wasm_bindgen::prelude::*;

pub mod tianzheng;

/// Convert a Rust error into a `JsValue` string so JS callers get a usable
/// diagnostic instead of an opaque abort.
fn to_js<E: std::fmt::Display>(context: &str) -> impl Fn(E) -> JsValue + '_ {
    move |err| JsValue::from_str(&format!("{context}: {err}"))
}

/// Serialize any (sub)section value to a JSON string, tagged with a
/// human-readable context prefix for the same diagnostic quality as `to_js`.
fn to_json<T: serde::Serialize>(context: &str, value: &T) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(to_js(context))
}

/// Best-effort readable text from a caught panic payload (`&str` / `String` are
/// the shapes emitted by `panic!` and by `unwrap`/`expect` on a message).
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic (no message)".to_string());
    format!("acad-rust panicked while processing the drawing: {detail}")
}

/// Run `f`, converting any panic raised inside acadrust (e.g. a malformed DWG
/// that trips an internal `unwrap`) into a `JsValue` error the worker can turn
/// into a rejected `read()` — instead of letting it abort the whole wasm instance
/// and take the host tab down with it.
///
/// This relies on the `unwind` panic strategy (`[profile.release] panic =
/// "unwind"` in Cargo.toml): under `abort` a panic terminates the instance and
/// `catch_unwind` never gets to run. Entry points that parse caller-supplied
/// bytes or serialize the resident document are wrapped in this.
fn guard<T, F: FnOnce() -> Result<T, JsValue>>(f: F) -> Result<T, JsValue> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|payload| Err(JsValue::from_str(&panic_message(payload))))
}

// ---------------------------------------------------------------------------
// Architecture A: single-shot DWG -> DXF.
// ---------------------------------------------------------------------------

/// Architecture A: read DWG bytes and return ASCII DXF bytes.
///
/// Tianzheng `TCH_TEXT` records are promoted first (see [`tianzheng`]) so this
/// path emits the same text geometry as the sectioned `dwg_open` path.
#[wasm_bindgen]
pub fn dwg_to_dxf(data: &[u8]) -> Result<Vec<u8>, JsValue> {
    guard(|| {
        let mut reader = DwgReader::from_stream(Cursor::new(data));
        let mut doc = reader.read().map_err(to_js("acadrust DWG read failed"))?;
        tianzheng::promote_tch_text(&mut doc);
        DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(to_js("acadrust DXF write failed"))
    })
}

/// Architecture A, DXF-input variant (benchmark only): read DXF bytes into a
/// `CadDocument` and re-emit ASCII DXF bytes. Analogous to [`dwg_to_dxf`] but
/// with [`DxfReader`] as the front end, so a `.dxf` can be pushed through the
/// acadrust pipeline (DXF -> CadDocument -> DXF -> the MIT TS DXF converter)
/// for A/B timing against the built-in DXF reader. `DxfReader::from_reader`
/// needs an owned `Read + Seek + 'static` source, hence the `to_vec` copy of
/// the caller's slice (browsers give us an `ArrayBuffer` view, not a file).
#[wasm_bindgen]
pub fn dxf_to_dxf(data: &[u8]) -> Result<Vec<u8>, JsValue> {
    guard(|| {
        let reader = DxfReader::from_reader(Cursor::new(data.to_vec()))
            .map_err(to_js("acadrust DXF read failed"))?;
        let doc = reader.read().map_err(to_js("acadrust DXF read failed"))?;
        DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(to_js("acadrust DXF write failed"))
    })
}

/// Architecture B (whole-document, unbounded): read DWG bytes and return the
/// document as one JSON string. NOT used by the production worker protocol -
/// serializing a large drawing in one shot is an OOM footgun (see the module
/// comment above). Kept only so the offline native dumper (`src/bin/
/// dump_fixture.rs`) and local fixtures can reuse the exact same serde schema
/// as the sectioned entry points below.
#[wasm_bindgen]
pub fn dwg_to_json(data: &[u8]) -> Result<String, JsValue> {
    guard(|| {
        let mut reader = DwgReader::from_stream(Cursor::new(data));
        let doc = reader.read().map_err(to_js("acadrust DWG read failed"))?;
        to_json("acadrust JSON serialize failed", &doc)
    })
}

// ---------------------------------------------------------------------------
// Architecture B (Phase 4): handle-based sectioned access.
//
// The parsed `CadDocument` is held in wasm linear memory for the lifetime of a
// single conversion, addressed by an opaque `u32` handle, so the TypeScript
// side can pull it apart section by section / page by page instead of ever
// materializing the whole document as one JSON string.
// ---------------------------------------------------------------------------

thread_local! {
    /// Open documents, indexed by handle. `None` marks a slot released by
    /// `dwg_close`; `dwg_open` reuses a free slot before growing the vec, so a
    /// long-lived page (opening and closing many drawings) doesn't leak slots.
    static OPEN_DOCUMENTS: RefCell<Vec<Option<CadDocument>>> = const { RefCell::new(Vec::new()) };
    static OPEN_TRANSACTIONS: RefCell<std::collections::HashMap<u32, Vec<CadDocument>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Run `f` against the document behind `handle`, or a `JsValue` error if the
/// handle was never opened or has already been closed.
fn with_document<T>(
    handle: u32,
    f: impl FnOnce(&CadDocument) -> Result<T, JsValue>,
) -> Result<T, JsValue> {
    OPEN_DOCUMENTS.with(|slot| {
        let docs = slot.borrow();
        match docs.get(handle as usize) {
            Some(Some(doc)) => f(doc),
            Some(None) => Err(JsValue::from_str(&format!(
                "acad-rust handle {handle} was already closed."
            ))),
            None => Err(JsValue::from_str(&format!(
                "acad-rust handle {handle} does not exist (never opened)."
            ))),
        }
    })
}

/// Store a freshly parsed document in [`OPEN_DOCUMENTS`] (reusing a freed slot
/// when possible) and return its handle. Shared by [`dwg_open`] and
/// [`dxf_open`] so the sectioned read path is entirely format-agnostic once the
/// `CadDocument` is resident.
fn with_document_mut<T>(
    handle: u32,
    f: impl FnOnce(&mut CadDocument) -> Result<T, JsValue>,
) -> Result<T, JsValue> {
    OPEN_DOCUMENTS.with(|slot| {
        let mut docs = slot.borrow_mut();
        match docs.get_mut(handle as usize) {
            Some(Some(doc)) => f(doc),
            Some(None) => Err(JsValue::from_str("acad-rust document is closed.")),
            None => Err(JsValue::from_str("Unknown acad-rust document handle.")),
        }
    })
}

fn parse_handle_hex(value: &str) -> Result<acadrust::types::Handle, JsValue> {
    let trimmed = value.trim();
    let digits = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);
    let raw = u64::from_str_radix(digits, 16).map_err(to_js("invalid hexadecimal DWG handle"))?;
    if raw == 0 { return Err(JsValue::from_str("DWG handle 0 is reserved.")); }
    Ok(acadrust::types::Handle::new(raw))
}

/// Convert external Handle enum payloads from exact hex strings to the
/// u64 representation expected by acadrust serde. Numeric JS-originated
/// handles are rejected because they may already have lost precision.
fn normalize_exact_handle_values(value: &mut serde_json::Value) -> Result<(), JsValue> {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(handle_value) = fields.get_mut("Handle") {
                let hex = handle_value.as_str()
                    .ok_or_else(|| JsValue::from_str("Handle values must be supplied as hexadecimal strings."))?;
                let handle = parse_handle_hex(hex)?;
                *handle_value = serde_json::Value::Number(serde_json::Number::from(handle.value()));
            }
            for child in fields.values_mut() {
                normalize_exact_handle_values(child)?;
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                normalize_exact_handle_values(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn register_document(doc: CadDocument) -> u32 {
    OPEN_DOCUMENTS.with(|slot| {
        let mut docs = slot.borrow_mut();
        let free = docs.iter().position(Option::is_none);
        match free {
            Some(index) => {
                docs[index] = Some(doc);
                index as u32
            }
            None => {
                docs.push(Some(doc));
                (docs.len() - 1) as u32
            }
        }
    })
}

/// Parse DWG bytes, keep the resulting `CadDocument` alive in wasm memory, and
/// return its handle for later `dwg_meta` / `dwg_section_json` /
/// `dwg_entity_page` / `dwg_block_entity_page` / `dwg_close` calls.
///
/// Tianzheng `TCH_TEXT` records are promoted to ordinary `Text` entities here,
/// before the document is registered, because their private `TDbText` payload
/// is otherwise untyped and never reaches the view (see [`tianzheng`]).
#[wasm_bindgen]
pub fn dwg_open(data: &[u8]) -> Result<u32, JsValue> {
    guard(|| {
        let mut reader = DwgReader::from_stream(Cursor::new(data));
        let mut doc = reader.read().map_err(to_js("acadrust DWG read failed"))?;
        tianzheng::promote_tch_text(&mut doc);
        Ok(register_document(doc))
    })
}

/// DXF-input companion to [`dwg_open`] (benchmark only): parse DXF bytes into a
/// `CadDocument` and register it under a handle. Every downstream sectioned
/// call (`dwg_meta` / `dwg_section_json` / `dwg_entity_page` /
/// `dwg_block_entity_page` / `dwg_close`) works unchanged, since it only ever
/// sees the resident document, never its origin format. This is what lets
/// Architecture-B map a `.dxf` straight from the acadrust `CadDocument` to the
/// database (DXF -> CadDocument -> JSON -> DB) for A/B timing.
#[wasm_bindgen]
pub fn dxf_open(data: &[u8]) -> Result<u32, JsValue> {
    guard(|| {
        let reader = DxfReader::from_reader(Cursor::new(data.to_vec()))
            .map_err(to_js("acadrust DXF open failed"))?;
        let doc = reader.read().map_err(to_js("acadrust DXF read failed"))?;
        Ok(register_document(doc))
    })
}

/// Release the document behind `handle`. The slot stays reserved (as `None`)
/// for a later `dwg_open` to reuse, so handles never get silently renumbered.
#[wasm_bindgen]
pub fn dwg_close(handle: u32) {
    OPEN_DOCUMENTS.with(|slot| {
        let mut docs = slot.borrow_mut();
        if let Some(Some(_)) = docs.get(handle as usize) {
            docs[handle as usize] = None;
        }
    });
    OPEN_TRANSACTIONS.with(|transactions| {
        transactions.borrow_mut().remove(&handle);
    });
}

/// Bounded summary for progress reporting and pagination math: version, key
/// table/entity counts. Built with `serde_json::json!` (a `Value`, not a
/// derived struct) so this crate needs no direct `serde` dependency beyond
/// what `acadrust`'s `serde` feature and `serde_json` already pull in.
#[wasm_bindgen]
pub fn dwg_meta(handle: u32) -> Result<String, JsValue> {
    guard(|| {
        with_document(handle, |doc| {
            let value = serde_json::json!({
                "version": doc.version,
                "dwgSourceVersion": doc.dwg_source_version,
                "entityCount": doc.entity_count(),
                "modelSpaceEntityCount": doc.model_space_entities().count(),
                "blockRecordCount": doc.block_records.len(),
                "layerCount": doc.layers.len(),
                "lineTypeCount": doc.line_types.len(),
                "textStyleCount": doc.text_styles.len(),
                "dimStyleCount": doc.dim_styles.len(),
                "vportCount": doc.vports.len(),
                "objectCount": doc.objects.len(),
                "classCount": doc.classes.len(),
            });
            to_json("acadrust meta serialize failed", &value)
        })
    })
}

/// Serialize one document section as JSON. `kind` selects which field to
/// serde; see `SECTION_KINDS` for the full list, kept in sync with the
/// TypeScript worker's validation of the same string.
///
/// Deliberately no "block-content" kind: block definitions are unbounded per
/// block name, so they are paged separately via `dwg_block_entity_page`
/// instead of ever being dumped whole.
#[wasm_bindgen]
pub fn dwg_section_json(handle: u32, kind: &str) -> Result<String, JsValue> {
    guard(|| {
        with_document(handle, |doc| match kind {
            "header" => to_json("acadrust header serialize failed", &doc.header),
            "summary-info" => to_json("acadrust summary info serialize failed", &doc.summary_info),
            "classes" => to_json("acadrust classes serialize failed", &doc.classes),
            "tables-layer" => to_json("acadrust layer table serialize failed", &doc.layers),
            "tables-ltype" => to_json("acadrust linetype table serialize failed", &doc.line_types),
            "tables-style" => to_json("acadrust text style table serialize failed", &doc.text_styles),
            "tables-blockrecord" => {
                to_json("acadrust block record table serialize failed", &doc.block_records)
            }
            "tables-dimstyle" => to_json("acadrust dim style table serialize failed", &doc.dim_styles),
            "tables-appid" => to_json("acadrust app id table serialize failed", &doc.app_ids),
            "tables-view" => to_json("acadrust view table serialize failed", &doc.views),
            "tables-vport" => to_json("acadrust vport table serialize failed", &doc.vports),
            "tables-ucs" => to_json("acadrust ucs table serialize failed", &doc.ucss),
            "tables-vx" => to_json("acadrust vx table serialize failed", &doc.vx_table),
            "objects" => to_json("acadrust objects serialize failed", &doc.objects),
            other => Err(JsValue::from_str(&format!(
                "Unknown acad-rust section kind: {other}"
            ))),
        })
    })
}

/// One exact u64 handle discovered inside a serialized document section.
#[derive(serde::Serialize)]
struct ExactHandlePath {
    /// JSON path segments from the section root (array indexes are decimal).
    path: Vec<String>,
    /// Canonical uppercase hexadecimal, without a 0x prefix.
    handle: String,
}

fn collect_exact_handles(
    value: &serde_json::Value,
    path: &mut Vec<String>,
    output: &mut Vec<ExactHandlePath>,
) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, child) in fields {
                path.push(key.clone());
                let is_handle_field = key == "handle"
                    || key.ends_with("_handle")
                    || matches!(
                        key.as_str(),
                        "owner" | "block_record" | "block_entity" | "block_end"
                            | "block" | "dependency" | "expression"
                            | "associated_variable" | "material"
                    );
                if is_handle_field {
                    if let Some(raw) = child.as_u64() {
                        output.push(ExactHandlePath {
                            path: path.clone(),
                            handle: format!("{:X}", raw),
                        });
                    }
                }

                // Named arrays below contain DWG handles, not arbitrary numeric
                // payloads. Preserve each exact u64 with its JSON path.
                let is_handle_array = matches!(
                    key.as_str(),
                    "reactors" | "viewports" | "entities" | "object_references"
                        | "entity_handles" | "insert_handles" | "hard_owner_entries"
                        | "handles" | "attribute_handles" | "dependencies"
                        | "all_blocks" | "visible_blocks" | "visible_params"
                );
                if is_handle_array {
                    if let Some(items) = child.as_array() {
                        for (index, item) in items.iter().enumerate() {
                            if let Some(raw) = item.as_u64() {
                                path.push(index.to_string());
                                output.push(ExactHandlePath {
                                    path: path.clone(),
                                    handle: format!("{:X}", raw),
                                });
                                path.pop();
                            }
                        }
                    }
                }

                // DICTIONARY.entries is a sequence of [key, Handle] tuples.
                // XRecord entries are structs and therefore do not match.
                if key == "entries" {
                    if let Some(items) = child.as_array() {
                        for (index, item) in items.iter().enumerate() {
                            if let Some(pair) = item.as_array() {
                                if pair.len() == 2 && pair[0].is_string() {
                                    if let Some(raw) = pair[1].as_u64() {
                                        path.push(index.to_string());
                                        path.push("1".to_string());
                                        output.push(ExactHandlePath {
                                            path: path.clone(),
                                            handle: format!("{:X}", raw),
                                        });
                                        path.pop();
                                        path.pop();
                                    }
                                }
                            }
                        }
                    }
                }

                collect_exact_handles(child, path, output);
                path.pop();
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_exact_handles(child, path, output);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Lossless sidecar for handle-like fields in any supported document section.
/// The regular section JSON remains unchanged for existing consumers; this
/// index preserves exact u64 values and their JSON paths for identity/reference
/// reconciliation without converting them through JavaScript Number.
#[wasm_bindgen]
pub fn dwg_section_handle_index_json(handle: u32, kind: &str) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let json = match kind {
            "header" => serde_json::to_value(&doc.header),
            "summary-info" => serde_json::to_value(&doc.summary_info),
            "classes" => serde_json::to_value(&doc.classes),
            "tables-layer" => serde_json::to_value(&doc.layers),
            "tables-ltype" => serde_json::to_value(&doc.line_types),
            "tables-style" => serde_json::to_value(&doc.text_styles),
            "tables-blockrecord" => serde_json::to_value(&doc.block_records),
            "tables-dimstyle" => serde_json::to_value(&doc.dim_styles),
            "tables-appid" => serde_json::to_value(&doc.app_ids),
            "tables-view" => serde_json::to_value(&doc.views),
            "tables-vport" => serde_json::to_value(&doc.vports),
            "tables-ucs" => serde_json::to_value(&doc.ucss),
            "tables-vx" => serde_json::to_value(&doc.vx_table),
            "objects" => serde_json::to_value(&doc.objects),
            other => return Err(JsValue::from_str(&format!(
                "Unknown acad-rust section kind for handle index: {other}"
            ))),
        }.map_err(to_js("acadrust section handle index source serialize failed"))?;
        let mut handles = Vec::new();
        collect_exact_handles(&json, &mut Vec::new(), &mut handles);
        to_json("acadrust section handle index serialize failed", &handles)
    }))
}

/// One page of model-space entities (the primary drawable set), as a JSON
/// array of externally-tagged `EntityType` values. `offset`/`limit` bound the
/// wasm->JS payload so a huge drawing never materializes all its entities in
/// one string; the same-handle document stays resident between calls.
#[wasm_bindgen]
pub fn dwg_entity_page(handle: u32, offset: u32, limit: u32) -> Result<String, JsValue> {
    guard(|| {
        with_document(handle, |doc| {
            let page: Vec<_> = doc
                .model_space_entities()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            to_json("acadrust entity page serialize failed", &page)
        })
    })
}

/// Return exact hexadecimal handles for one page of model-space entities.
/// This sidecar is intentionally separate from entity JSON: common.handle is
/// currently serialized as a JSON number and may lose u64 precision in JS.
#[wasm_bindgen]
pub fn dwg_entity_handle_page_json(
    handle: u32,
    offset: u32,
    limit: u32,
) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let handles: Vec<String> = doc.model_space_entities()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|entity| format!("{:X}", entity.common().handle.value()))
            .collect();
        to_json("acadrust entity handle page serialize failed", &handles)
    }))
}

/// One page of a named block definition's entities (see
/// `CadDocument::entities_in_block`), for the same bounded-payload reason as
/// `dwg_entity_page` but scoped to block geometry instead of model space.
#[wasm_bindgen]
pub fn dwg_block_entity_page(
    handle: u32,
    block_name: &str,
    offset: u32,
    limit: u32,
) -> Result<String, JsValue> {
    guard(|| {
        with_document(handle, |doc| {
            let page: Vec<_> = doc
                .entities_in_block(block_name)
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            to_json("acadrust block entity page serialize failed", &page)
        })
    })
}

/// Return exact hexadecimal handles for the entities owned by one block record.
#[wasm_bindgen]
pub fn dwg_block_entity_handles_json(
    handle: u32,
    block_name: &str,
) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let handles: Vec<String> = doc.entities_in_block(block_name)
            .map(|entity| format!("{:X}", entity.common().handle.value()))
            .collect();
        to_json("acadrust block entity handles serialize failed", &handles)
    }))
}

/// One block's complete entity list inside a [`dwg_block_entities_batch`]
/// reply. Serialized directly (no `serde_json::Value` intermediate) so the
/// batched path keeps the same per-entity encode cost as the paged one.
#[derive(serde::Serialize)]
struct BlockEntityGroup<'a> {
    name: &'a str,
    entities: Vec<&'a acadrust::EntityType>,
}

/// Architecture B: full entity lists for several block definitions in one
/// worker round-trip.
///
/// [`dwg_block_entity_page`] costs one request per block definition, and on
/// block-heavy drawings (537 definitions, ~452 entities each on the 15 MB
/// reference file) those round-trips dominate the main-thread consume phase
/// even though each definition is far smaller than one page. The caller
/// batches small definitions here (keeping the combined entity count within
/// its page budget) and falls back to paging only for oversized definitions.
/// `names_json` is a JSON string array; the reply is
/// `[{ "name": ..., "entities": [...] }, ...]` in request order. Unknown
/// names yield an empty `entities` array, matching `entities_in_block`'s empty
/// iterator instead of turning a data mismatch into a hard error.
#[wasm_bindgen]
pub fn dwg_block_entities_batch(
    handle: u32,
    names_json: &str,
) -> Result<String, JsValue> {
    guard(|| {
        let names: Vec<String> = serde_json::from_str(names_json)
            .map_err(to_js("acadrust block batch names parse failed"))?;
        with_document(handle, |doc| {
            let groups: Vec<BlockEntityGroup> = names
                .iter()
                .map(|name| BlockEntityGroup {
                    name: name.as_str(),
                    entities: doc.entities_in_block(name).collect(),
                })
                .collect();
            to_json("acadrust block batch serialize failed", &groups)
        })
    })
}

/// Exported for the TypeScript side to keep its worker-side validation of
/// `dwg_section_json`'s `kind` argument in sync with what Rust actually
/// accepts (checked by `packages/acad-rust-converter/__tests__` against the
/// worker's own list).
///
/// These entry points are only exercised through a JS/wasm runtime, so they
/// have no native `#[cfg(test)]` unit tests here: `wasm_bindgen::JsValue`
/// conversions (used for every error path above) abort under a native test
/// harness, since they call into the JS host glue directly.
pub const SECTION_KINDS: &[&str] = &[
    "header",
    "summary-info",
    "classes",
    "tables-layer",
    "tables-ltype",
    "tables-style",
    "tables-blockrecord",
    "tables-dimstyle",
    "tables-appid",
    "tables-view",
    "tables-vport",
    "tables-ucs",
    "tables-vx",
    "objects",
];


// Mutable CadDocument API used by the TypeScript AcDb compatibility layer.
#[wasm_bindgen]
pub fn dwg_begin_transaction(handle: u32) -> Result<(), JsValue> {
    guard(|| {
        let snapshot = OPEN_DOCUMENTS.with(|slot| {
            let docs = slot.borrow();
            match docs.get(handle as usize) {
                Some(Some(doc)) => Ok(doc.clone()),
                Some(None) => Err(JsValue::from_str("Cannot transact on a closed document.")),
                None => Err(JsValue::from_str("Unknown acad-rust document handle.")),
            }
        })?;
        OPEN_TRANSACTIONS.with(|transactions| transactions.borrow_mut().entry(handle).or_default().push(snapshot));
        Ok(())
    })
}

#[wasm_bindgen]
pub fn dwg_commit_transaction(handle: u32) -> Result<(), JsValue> {
    guard(|| OPEN_TRANSACTIONS.with(|transactions| {
        let mut transactions = transactions.borrow_mut();
        let stack = transactions.get_mut(&handle)
            .ok_or_else(|| JsValue::from_str("No active transaction for this document."))?;
        stack.pop().ok_or_else(|| JsValue::from_str("No active transaction for this document."))?;
        if stack.is_empty() { transactions.remove(&handle); }
        Ok(())
    }))
}

#[wasm_bindgen]
pub fn dwg_abort_transaction(handle: u32) -> Result<(), JsValue> {
    guard(|| {
        let snapshot = OPEN_TRANSACTIONS.with(|transactions| {
            let mut transactions = transactions.borrow_mut();
            let stack = transactions.get_mut(&handle)
                .ok_or_else(|| JsValue::from_str("No active transaction for this document."))?;
            let snapshot = stack.pop().ok_or_else(|| JsValue::from_str("No active transaction for this document."))?;
            if stack.is_empty() { transactions.remove(&handle); }
            Ok(snapshot)
        })?;
        OPEN_DOCUMENTS.with(|slot| {
            let mut docs = slot.borrow_mut();
            match docs.get_mut(handle as usize) {
                Some(document @ Some(_)) => { *document = Some(snapshot); Ok(()) }
                Some(None) => Err(JsValue::from_str("Cannot roll back a closed document.")),
                None => Err(JsValue::from_str("Unknown acad-rust document handle.")),
            }
        })
    })
}

/// Add one serde-encoded EntityType to model space; the allocated handle is
/// returned as uppercase hex so JavaScript never rounds a DWG u64.
#[wasm_bindgen]
pub fn dwg_add_entity_json(handle: u32, entity_json: &str) -> Result<String, JsValue> {
    guard(|| {
        let mut entity: acadrust::EntityType = serde_json::from_str(entity_json)
            .map_err(to_js("acadrust entity JSON parse failed"))?;
        if matches!(&entity, acadrust::EntityType::Unknown(_)) {
            return Err(JsValue::from_str(
                "Adding raw Unknown entities is not supported; preserve existing unknown records in-place.",
            ));
        }
        entity.common_mut().handle = acadrust::types::Handle::NULL;
        entity.common_mut().owner_handle = acadrust::types::Handle::NULL;
        with_document_mut(handle, |doc| {
            let entity_handle = doc.add_entity(entity).map_err(to_js("acadrust add_entity failed"))?;
            Ok(format!("{:X}", entity_handle.value()))
        })
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntityCommonPatch {
    layer: Option<String>,
    linetype: Option<String>,
    linetype_scale: Option<f64>,
    invisible: Option<bool>,
}

#[derive(serde::Deserialize)]
struct Point3Patch {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(serde::Deserialize)]
struct Point2Patch { x: f64, y: f64 }
#[derive(serde::Deserialize)]
struct LwPolylineVertexPatch { location: Point2Patch, bulge: f64, start_width: f64, end_width: f64, vertex_id: i32 }
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum LwPolylineVertexOperation {
    Insert { index: usize, vertex: LwPolylineVertexPatch },
    Update { index: usize, vertex: LwPolylineVertexPatch },
    Remove { index: usize },
}
#[derive(serde::Deserialize)]
struct LwPolylineGeometryPatch {
    vertices: Option<Vec<LwPolylineVertexPatch>>,
    vertex_operations: Option<Vec<LwPolylineVertexOperation>>,
    is_closed: Option<bool>, plinegen: Option<bool>,
    constant_width: Option<f64>, elevation: Option<f64>, thickness: Option<f64>, normal: Option<Point3Patch>,
}

fn apply_lwpolyline_vertex_operations(
    mut vertices: Vec<acadrust::entities::LwVertex>,
    operations: Vec<LwPolylineVertexOperation>,
) -> Result<Vec<acadrust::entities::LwVertex>, &'static str> {
    for operation in operations {
        match operation {
            LwPolylineVertexOperation::Insert { index, vertex } => {
                if index > vertices.len() { return Err("LWPOLYLINE insert index is out of range."); }
                vertices.insert(index, acadrust::entities::LwVertex {
                    location: acadrust::types::Vector2::new(vertex.location.x, vertex.location.y),
                    bulge: vertex.bulge, start_width: vertex.start_width, end_width: vertex.end_width,
                    vertex_id: vertex.vertex_id,
                });
            }
            LwPolylineVertexOperation::Update { index, vertex } => {
                let target = vertices.get_mut(index).ok_or("LWPOLYLINE update index is out of range.")?;
                target.location = acadrust::types::Vector2::new(vertex.location.x, vertex.location.y);
                target.bulge = vertex.bulge; target.start_width = vertex.start_width;
                target.end_width = vertex.end_width; target.vertex_id = vertex.vertex_id;
            }
            LwPolylineVertexOperation::Remove { index } => {
                if index >= vertices.len() { return Err("LWPOLYLINE remove index is out of range."); }
                vertices.remove(index);
            }
        }
    }
    Ok(vertices)
}

/// In-place LWPOLYLINE patch: never replaces common data or handle relationships.
#[wasm_bindgen]
pub fn dwg_update_lwpolyline_geometry_json(handle: u32, entity_handle_hex: &str, geometry_json: &str) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: LwPolylineGeometryPatch = serde_json::from_str(geometry_json)
            .map_err(to_js("LWPOLYLINE geometry patch JSON parse failed"))?;
        let validate_vertex = |v: &LwPolylineVertexPatch| -> Result<(), JsValue> {
            if ![v.location.x, v.location.y, v.bulge, v.start_width, v.end_width].iter().all(|x| x.is_finite()) {
                return Err(JsValue::from_str("LWPOLYLINE vertex values must be finite."));
            }
            if v.start_width < 0.0 || v.end_width < 0.0 { return Err(JsValue::from_str("LWPOLYLINE vertex widths cannot be negative.")); }
            Ok(())
        };
        if let Some(vertices) = patch.vertices.as_ref() {
            for v in vertices {
                validate_vertex(v)?;
            }
        }
        if let Some(operations) = patch.vertex_operations.as_ref() {
            for operation in operations {
                match operation {
                    LwPolylineVertexOperation::Insert { vertex, .. } | LwPolylineVertexOperation::Update { vertex, .. } => validate_vertex(vertex)?,
                    LwPolylineVertexOperation::Remove { .. } => {}
                }
            }
        }
        for value in [patch.constant_width, patch.elevation, patch.thickness].into_iter().flatten() {
            if !value.is_finite() { return Err(JsValue::from_str("LWPOLYLINE scalar geometry values must be finite.")); }
        }
        if let Some(n) = patch.normal.as_ref() { if ![n.x,n.y,n.z].iter().all(|x| x.is_finite()) { return Err(JsValue::from_str("LWPOLYLINE normal must be finite.")); } }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle).ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            match entity {
                acadrust::EntityType::LwPolyline(polyline) => {
                    let has_vertex_patch = patch.vertices.is_some() || patch.vertex_operations.is_some();
                    let mut vertices = patch.vertices.map(|values| values.into_iter().map(|v| acadrust::entities::LwVertex {
                        location: acadrust::types::Vector2::new(v.location.x, v.location.y), bulge: v.bulge,
                        start_width: v.start_width, end_width: v.end_width, vertex_id: v.vertex_id,
                    }).collect()).unwrap_or_else(|| polyline.vertices.clone());
                    let vertices = if let Some(operations) = patch.vertex_operations {
                        apply_lwpolyline_vertex_operations(vertices, operations).map_err(JsValue::from_str)?
                    } else { vertices };
                    if has_vertex_patch { polyline.vertices = vertices; }
                    if let Some(v) = patch.is_closed { polyline.is_closed = v; }
                    if let Some(v) = patch.plinegen { polyline.plinegen = v; }
                    if let Some(v) = patch.constant_width { polyline.constant_width = v; }
                    if let Some(v) = patch.elevation { polyline.elevation = v; }
                    if let Some(v) = patch.thickness { polyline.thickness = v; }
                    if let Some(v) = patch.normal { polyline.normal = acadrust::types::Vector3::new(v.x, v.y, v.z); }
                    Ok(())
                }
                _ => Err(JsValue::from_str("The target entity is not an LWPOLYLINE.")),
            }
        })
    })
}

#[derive(serde::Deserialize)]
struct LineGeometryPatch {
    start: Option<Point3Patch>,
    end: Option<Point3Patch>,
}

/// Update LINE endpoints without serializing or replacing its handle-bearing
/// common/reference fields. This is the first typed geometry mutation.
#[wasm_bindgen]
pub fn dwg_update_line_geometry_json(
    handle: u32,
    entity_handle_hex: &str,
    geometry_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: LineGeometryPatch = serde_json::from_str(geometry_json)
            .map_err(to_js("line geometry patch JSON parse failed"))?;
        for point in [&patch.start, &patch.end].into_iter().flatten() {
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return Err(JsValue::from_str("LINE coordinates must all be finite."));
            }
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            match entity {
                acadrust::EntityType::Line(line) => {
                    if let Some(point) = patch.start {
                        line.start = acadrust::types::Vector3::new(point.x, point.y, point.z);
                    }
                    if let Some(point) = patch.end {
                        line.end = acadrust::types::Vector3::new(point.x, point.y, point.z);
                    }
                    Ok(())
                }
                _ => Err(JsValue::from_str("The target entity is not a LINE.")),
            }
        })
    })
}

#[derive(serde::Deserialize)]
struct CircleArcGeometryPatch {
    center: Option<Point3Patch>,
    radius: Option<f64>,
    start_angle: Option<f64>,
    end_angle: Option<f64>,
}

/// Typed CIRCLE/ARC geometry patch. Mutates geometry in-place so handle,
/// owner, reactors, XData and extension-dictionary links are never round-tripped
/// through JavaScript JSON numbers.
#[wasm_bindgen]
pub fn dwg_update_circle_arc_geometry_json(
    handle: u32,
    entity_handle_hex: &str,
    geometry_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: CircleArcGeometryPatch = serde_json::from_str(geometry_json)
            .map_err(to_js("circle/arc geometry patch JSON parse failed"))?;
        if let Some(center) = patch.center.as_ref() {
            if !center.x.is_finite() || !center.y.is_finite() || !center.z.is_finite() {
                return Err(JsValue::from_str("CIRCLE/ARC center coordinates must be finite."));
            }
        }
        if let Some(radius) = patch.radius {
            if !radius.is_finite() || radius <= 0.0 {
                return Err(JsValue::from_str("CIRCLE/ARC radius must be finite and greater than zero."));
            }
        }
        for angle in [patch.start_angle, patch.end_angle].into_iter().flatten() {
            if !angle.is_finite() {
                return Err(JsValue::from_str("ARC angles must be finite."));
            }
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            match entity {
                acadrust::EntityType::Circle(circle) => {
                    if patch.start_angle.is_some() || patch.end_angle.is_some() {
                        return Err(JsValue::from_str("CIRCLE does not have start/end angles."));
                    }
                    if let Some(center) = patch.center {
                        circle.center = acadrust::types::Vector3::new(center.x, center.y, center.z);
                    }
                    if let Some(radius) = patch.radius { circle.radius = radius; }
                    Ok(())
                }
                acadrust::EntityType::Arc(arc) => {
                    if let Some(center) = patch.center {
                        arc.center = acadrust::types::Vector3::new(center.x, center.y, center.z);
                    }
                    if let Some(radius) = patch.radius { arc.radius = radius; }
                    if let Some(start_angle) = patch.start_angle { arc.start_angle = start_angle; }
                    if let Some(end_angle) = patch.end_angle { arc.end_angle = end_angle; }
                    Ok(())
                }
                _ => Err(JsValue::from_str("Entity is neither CIRCLE nor ARC.")),
            }
        })
    })
}

#[derive(serde::Deserialize)]
struct EllipseGeometryPatch {
    center: Option<Point3Patch>,
    major_axis: Option<Point3Patch>,
    minor_axis_ratio: Option<f64>,
    start_parameter: Option<f64>,
    end_parameter: Option<f64>,
    normal: Option<Point3Patch>,
}

/// Reference-safe ELLIPSE geometry update. The entity's common data and all
/// handle-bearing relationships remain resident in CadDocument.
#[wasm_bindgen]
pub fn dwg_update_ellipse_geometry_json(
    handle: u32,
    entity_handle_hex: &str,
    geometry_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: EllipseGeometryPatch = serde_json::from_str(geometry_json)
            .map_err(to_js("ELLIPSE geometry patch JSON parse failed"))?;
        for point in [patch.center.as_ref(), patch.major_axis.as_ref(), patch.normal.as_ref()].into_iter().flatten() {
            if ![point.x, point.y, point.z].iter().all(|value| value.is_finite()) {
                return Err(JsValue::from_str("ELLIPSE point and vector values must be finite."));
            }
        }
        if let Some(ratio) = patch.minor_axis_ratio {
            if !ratio.is_finite() || ratio <= 0.0 {
                return Err(JsValue::from_str("ELLIPSE minor-axis ratio must be finite and greater than zero."));
            }
        }
        for value in [patch.start_parameter, patch.end_parameter].into_iter().flatten() {
            if !value.is_finite() {
                return Err(JsValue::from_str("ELLIPSE parameters must be finite."));
            }
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            match entity {
                acadrust::EntityType::Ellipse(ellipse) => {
                    if let Some(point) = patch.center {
                        ellipse.center = acadrust::types::Vector3::new(point.x, point.y, point.z);
                    }
                    if let Some(axis) = patch.major_axis {
                        ellipse.major_axis = acadrust::types::Vector3::new(axis.x, axis.y, axis.z);
                    }
                    if let Some(ratio) = patch.minor_axis_ratio { ellipse.minor_axis_ratio = ratio; }
                    if let Some(value) = patch.start_parameter { ellipse.start_parameter = value; }
                    if let Some(value) = patch.end_parameter { ellipse.end_parameter = value; }
                    if let Some(normal) = patch.normal {
                        ellipse.normal = acadrust::types::Vector3::new(normal.x, normal.y, normal.z);
                    }
                    Ok(())
                }
                _ => Err(JsValue::from_str("The target entity is not an ELLIPSE.")),
            }
        })
    })
}

#[derive(serde::Deserialize, Default)]
struct SplineFlagsPatch {
    closed: Option<bool>,
    periodic: Option<bool>,
    rational: Option<bool>,
    planar: Option<bool>,
    linear: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct SplineGeometryPatch {
    degree: Option<i32>,
    flags: Option<SplineFlagsPatch>,
    knots: Option<Vec<f64>>,
    control_points: Option<Vec<Point3Patch>>,
    weights: Option<Vec<f64>>,
    fit_points: Option<Vec<Point3Patch>>,
    normal: Option<Point3Patch>,
    knot_tolerance: Option<f64>,
    control_tolerance: Option<f64>,
    fit_tolerance: Option<f64>,
    begin_tangent: Option<Point3Patch>,
    end_tangent: Option<Point3Patch>,
    knot_parameterization: Option<i32>,
    cv_frame_visible: Option<bool>,
    dwg_flags1: Option<i32>,
}

/// Apply a complete typed NURBS patch to the resident Spline. Work on cloned
/// candidate fields first, so malformed arrays cannot leave a half-edited curve.
#[wasm_bindgen]
pub fn dwg_update_spline_geometry_json(
    handle: u32,
    entity_handle_hex: &str,
    geometry_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: SplineGeometryPatch = serde_json::from_str(geometry_json)
            .map_err(to_js("SPLINE geometry patch JSON parse failed"))?;
        let valid_point = |p: &Point3Patch| [p.x, p.y, p.z].iter().all(|v| v.is_finite());
        for points in [patch.control_points.as_ref(), patch.fit_points.as_ref()].into_iter().flatten() {
            if points.iter().any(|p| !valid_point(p)) {
                return Err(JsValue::from_str("SPLINE point coordinates must be finite."));
            }
        }
        for point in [patch.normal.as_ref(), patch.begin_tangent.as_ref(), patch.end_tangent.as_ref()].into_iter().flatten() {
            if !valid_point(point) {
                return Err(JsValue::from_str("SPLINE vector values must be finite."));
            }
        }
        for values in [patch.knots.as_ref(), patch.weights.as_ref()].into_iter().flatten() {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(JsValue::from_str("SPLINE knots and weights must be finite."));
            }
        }
        for value in [patch.knot_tolerance, patch.control_tolerance, patch.fit_tolerance].into_iter().flatten() {
            if !value.is_finite() || value < 0.0 {
                return Err(JsValue::from_str("SPLINE tolerances must be finite and non-negative."));
            }
        }
        if patch.degree.is_some_and(|degree| degree < 1) {
            return Err(JsValue::from_str("SPLINE degree must be at least 1."));
        }

        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            let spline = match entity {
                acadrust::EntityType::Spline(spline) => spline,
                _ => return Err(JsValue::from_str("The target entity is not a SPLINE.")),
            };

            let rational_flag_changed = patch.flags.as_ref().is_some_and(|f| f.rational.is_some());
            let geometry_arrays_changed = patch.degree.is_some()
                || patch.knots.is_some()
                || patch.control_points.is_some()
                || patch.weights.is_some()
                || rational_flag_changed;
            let mut degree = spline.degree;
            let mut flags = spline.flags;
            let mut knots = spline.knots.clone();
            let mut control_points = spline.control_points.clone();
            let mut weights = spline.weights.clone();
            let mut fit_points = spline.fit_points.clone();
            if let Some(value) = patch.degree { degree = value; }
            if let Some(update) = patch.flags {
                if let Some(value) = update.closed { flags.closed = value; }
                if let Some(value) = update.periodic { flags.periodic = value; }
                if let Some(value) = update.rational { flags.rational = value; }
                if let Some(value) = update.planar { flags.planar = value; }
                if let Some(value) = update.linear { flags.linear = value; }
            }
            if let Some(values) = patch.knots { knots = values; }
            if let Some(values) = patch.control_points {
                control_points = values.into_iter().map(|p| acadrust::types::Vector3::new(p.x, p.y, p.z)).collect();
            }
            if let Some(values) = patch.weights { weights = values; }
            if let Some(values) = patch.fit_points {
                fit_points = values.into_iter().map(|p| acadrust::types::Vector3::new(p.x, p.y, p.z)).collect();
            }

            if !knots.windows(2).all(|pair| pair[0] <= pair[1]) {
                return Err(JsValue::from_str("SPLINE knot values must be non-decreasing."));
            }
            if geometry_arrays_changed && !control_points.is_empty() {
                if !knots.is_empty() && knots.len() != control_points.len() + degree as usize + 1 {
                    return Err(JsValue::from_str("SPLINE knot count must equal control-point count + degree + 1."));
                }
                if !weights.is_empty() && weights.len() != control_points.len() {
                    return Err(JsValue::from_str("SPLINE weight count must equal control-point count."));
                }
                if flags.rational && weights.len() != control_points.len() {
                    return Err(JsValue::from_str("A rational SPLINE requires one weight per control point."));
                }
            }

            spline.degree = degree;
            spline.flags = flags;
            spline.knots = knots;
            spline.control_points = control_points;
            spline.weights = weights;
            spline.fit_points = fit_points;
            if let Some(p) = patch.normal { spline.normal = acadrust::types::Vector3::new(p.x, p.y, p.z); }
            if let Some(v) = patch.knot_tolerance { spline.knot_tolerance = v; }
            if let Some(v) = patch.control_tolerance { spline.control_tolerance = v; }
            if let Some(v) = patch.fit_tolerance { spline.fit_tolerance = v; }
            if let Some(p) = patch.begin_tangent { spline.begin_tangent = acadrust::types::Vector3::new(p.x, p.y, p.z); }
            if let Some(p) = patch.end_tangent { spline.end_tangent = acadrust::types::Vector3::new(p.x, p.y, p.z); }
            if let Some(v) = patch.knot_parameterization { spline.knot_parameterization = v; }
            if let Some(v) = patch.cv_frame_visible { spline.cv_frame_visible = v; }
            if let Some(v) = patch.dwg_flags1 { spline.dwg_flags1 = v; }
            Ok(())
        })
    })
}

//// Create a structurally complete BLOCK definition (BLOCK-RECORD, BLOCK,
/// ENDBLK) in the resident document. Entity membership remains empty until
/// dwg_add_entity_to_block_json is called.
#[wasm_bindgen]
pub fn dwg_add_block_record_json(
    handle: u32,
    name: &str,
    base_point_json: &str,
    description: &str,
) -> Result<String, JsValue> {
    guard(|| with_document_mut(handle, |doc| {
        let name = name.trim();
        if name.is_empty() || name.starts_with('*') {
            return Err(JsValue::from_str("Block name must be non-empty and cannot start with '*'."));
        }
        if doc.block_records.contains(name) {
            return Err(JsValue::from_str(&format!("Block '{name}' already exists.")));
        }
        let point: Point3Patch = serde_json::from_str(base_point_json)
            .map_err(to_js("block base point parse failed"))?;
        let base = Vector3::new(point.x, point.y, point.z);
        let mut record = acadrust::BlockRecord::new(name);
        record.handle = doc.allocate_handle();
        record.block_entity_handle = doc.allocate_handle();
        record.block_end_handle = doc.allocate_handle();
        record.base_point = base;
        record.description = description.to_string();
        let record_handle = record.handle;
        let block_handle = record.block_entity_handle;
        let end_handle = record.block_end_handle;
        doc.block_records.add(record).map_err(to_js("block record insert failed"))?;

        let mut block = acadrust::entities::Block::new(name, base).with_description(description);
        block.common.handle = block_handle;
        block.common.owner_handle = record_handle;
        doc.add_entity(acadrust::EntityType::Block(block))
            .map_err(to_js("BLOCK marker insert failed"))?;

        let mut end = acadrust::entities::BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = record_handle;
        doc.add_entity(acadrust::EntityType::BlockEnd(end))
            .map_err(to_js("ENDBLK marker insert failed"))?;

        let result = serde_json::json!({
            "name": name,
            "handle": format!("{:X}", record_handle.value()),
            "blockEntityHandle": format!("{:X}", block_handle.value()),
            "blockEndHandle": format!("{:X}", end_handle.value())
        });
        to_json("created block record serialize failed", &result)
    }))
}

/// Add an entity to an existing block definition. Setting owner_handle before
/// CadDocument::add_entity lets acadrust update both flat entity storage and
/// the BlockRecord.entity_handles membership index atomically.
#[wasm_bindgen]
pub fn dwg_add_entity_to_block_json(
    handle: u32,
    block_name: &str,
    entity_json: &str,
) -> Result<String, JsValue> {
    guard(|| with_document_mut(handle, |doc| {
        let record = doc.block_records.get(block_name)
            .ok_or_else(|| JsValue::from_str(&format!("Block '{block_name}' does not exist.")))?;
        let owner = record.handle;
        let mut entity: acadrust::EntityType = serde_json::from_str(entity_json)
            .map_err(to_js("block entity JSON parse failed"))?;
        if !entity.common().handle.is_null() {
            return Err(JsValue::from_str("New block entities must not supply a pre-existing DWG handle."));
        }
        if matches!(
            &entity,
            acadrust::EntityType::AttributeEntity(_)
                | acadrust::EntityType::Block(_)
                | acadrust::EntityType::BlockEnd(_)
        ) {
            return Err(JsValue::from_str("AttributeEntity, BLOCK and ENDBLK cannot be appended as ordinary block members."));
        }
        entity.common_mut().owner_handle = owner;
        let added = doc.add_entity(entity).map_err(to_js("block entity append failed"))?;
        Ok(format!("{:X}", added.value()))
    }))
}

/// Move an existing ordinary entity to another block definition while
/// preserving its DWG handle, XDATA, reactors, Extension Dictionary and all
/// entity-specific references. Both block membership lists and common owner
/// are updated together in the resident CadDocument.
#[wasm_bindgen]
pub fn dwg_move_entity_to_block(
    handle: u32,
    entity_handle_hex: &str,
    target_block_name: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        with_document_mut(handle, |doc| {
            let target = doc.block_records.get(target_block_name)
                .ok_or_else(|| JsValue::from_str("Target block does not exist."))?;
            let target_handle = target.handle;
            let target_is_model = target.is_model_space();
            let target_is_paper = target.is_paper_space();
            let target_already_contains = target.entity_handles.contains(&entity_handle);
            let current = doc.get_entity(entity_handle)
                .ok_or_else(|| JsValue::from_str("Entity handle was not found."))?;
            if matches!(
                current,
                acadrust::EntityType::AttributeEntity(_)
                    | acadrust::EntityType::Block(_)
                    | acadrust::EntityType::BlockEnd(_)
                    | acadrust::EntityType::Seqend(_)
            ) {
                return Err(JsValue::from_str("Sub-entities and structural block markers cannot be moved independently."));
            }
            let old_owner = current.common().owner_handle;
            if old_owner == target_handle {
                if !target_already_contains {
                    if let Some(record) = doc.block_records.iter_mut().find(|record| record.handle == target_handle) {
                        record.entity_handles.push(entity_handle);
                    }
                }
                return Ok(());
            }
            let old_record_contains = doc.block_records.iter()
                .find(|record| record.handle == old_owner)
                .is_some_and(|record| record.entity_handles.contains(&entity_handle));
            if !old_owner.is_null() && !old_record_contains {
                return Err(JsValue::from_str("Current owner block membership does not contain the entity handle."));
            }
            for record in doc.block_records.iter_mut() {
                if record.handle != target_handle {
                    record.entity_handles.retain(|member| *member != entity_handle);
                }
            }
            if let Some(new_record) = doc.block_records.iter_mut().find(|record| record.handle == target_handle) {
                if !new_record.entity_handles.contains(&entity_handle) {
                    new_record.entity_handles.push(entity_handle);
                }
            }
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("Entity disappeared during owner reassignment."))?;
            entity.common_mut().owner_handle = target_handle;
            entity.common_mut().entity_mode = if target_is_model {
                Some(2)
            } else if target_is_paper {
                Some(1)
            } else {
                None
            };
            Ok(())
        })
    })
}

/// Delete only an empty, unreferenced user block. Refuse destructive deletion
/// when child entities or INSERT references remain; callers must explicitly
/// remove/repoint those objects first.
#[wasm_bindgen]
pub fn dwg_remove_empty_block_record(handle: u32, block_name: &str) -> Result<bool, JsValue> {
    guard(|| with_document_mut(handle, |doc| {
        let Some(record) = doc.block_records.get(block_name).cloned() else {
            return Ok(false);
        };
        if record.is_standard() || record.is_layout() {
            return Err(JsValue::from_str("Cannot remove model/paper space or layout block records."));
        }
        let has_dictionary_refs = doc.objects.values().any(|object| match object {
            acadrust::objects::ObjectType::Dictionary(dictionary) => dictionary.entries.iter()
                .any(|(_, target)| *target == record.handle),
            _ => false,
        });
        if !record.entity_handles.is_empty()
            || !record.insert_handles.is_empty()
            || doc.extension_dictionary_handle(record.handle).is_some()
            || doc.object_xdata(record.handle).is_some()
            || has_dictionary_refs
            || doc.entities().any(|entity| matches!(
                entity,
                acadrust::EntityType::Insert(insert)
                    if insert.block_name.eq_ignore_ascii_case(block_name)
            ))
        {
            return Err(JsValue::from_str("Cannot remove a block that still has entities or INSERT references."));
        }
        let _ = doc.remove_entity(record.block_entity_handle);
        let _ = doc.remove_entity(record.block_end_handle);
        doc.block_records.remove(block_name);
        Ok(true)
    }))
}

/// Read the XDATA collection attached to any resident entity/table/object handle.
#[wasm_bindgen]
pub fn dwg_object_xdata_json(handle: u32, object_handle_hex: &str) -> Result<String, JsValue> {
    guard(|| {
        let object_handle = parse_handle_hex(object_handle_hex)?;
        with_document(handle, |doc| {
            to_json("acadrust object XDATA serialize failed", &doc.object_xdata(object_handle))
        })
    })
}

/// Upsert one application-scoped XDATA record without replacing other apps.
#[wasm_bindgen]
pub fn dwg_set_object_xdata_record_json(
    handle: u32,
    object_handle_hex: &str,
    record_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let object_handle = parse_handle_hex(object_handle_hex)?;
        let mut record_value: serde_json::Value = serde_json::from_str(record_json)
            .map_err(to_js("XDATA record JSON parse failed"))?;
        normalize_exact_handle_values(&mut record_value)?;
        let record: acadrust::xdata::ExtendedDataRecord = serde_json::from_value(record_value)
            .map_err(to_js("XDATA record parse failed"))?;
        if record.application_name.trim().is_empty() {
            return Err(JsValue::from_str("XDATA application name cannot be empty."));
        }
        with_document_mut(handle, |doc| {
            if !document_has_handle(doc, object_handle) {
                return Err(JsValue::from_str("XDATA owner handle does not resolve."));
            }
            if !doc.app_ids.contains(&record.application_name) {
                let mut app = acadrust::AppId::new(&record.application_name);
                app.set_handle(doc.allocate_handle());
                doc.app_ids.add(app).map_err(to_js("XDATA APPID creation failed"))?;
            }
            doc.set_object_xdata_record(object_handle, record);
            Ok(())
        })
    })
}

/// Remove one application's XDATA, including its verbatim DWG EED payload.
#[wasm_bindgen]
pub fn dwg_remove_object_xdata_record(
    handle: u32,
    object_handle_hex: &str,
    application_name: &str,
) -> Result<bool, JsValue> {
    guard(|| {
        let object_handle = parse_handle_hex(object_handle_hex)?;
        with_document_mut(handle, |doc| {
            Ok(doc.remove_object_xdata_record(object_handle, application_name))
        })
    })
}

/// Ensure an Extension Dictionary is attached to the specified owner.
#[wasm_bindgen]
pub fn dwg_ensure_extension_dictionary(
    handle: u32,
    owner_handle_hex: &str,
) -> Result<String, JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        with_document_mut(handle, |doc| {
            if !document_has_handle(doc, owner) {
                return Err(JsValue::from_str("Extension Dictionary owner handle does not resolve."));
            }
            let dictionary = doc.ensure_extension_dictionary(owner);
            Ok(format!("{:X}", dictionary.value()))
        })
    })
}

/// Ensure a named XRecord in an owner's Extension Dictionary.
#[wasm_bindgen]
pub fn dwg_ensure_xrecord(
    handle: u32,
    owner_handle_hex: &str,
    key: &str,
) -> Result<String, JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        if key.trim().is_empty() {
            return Err(JsValue::from_str("XRecord key cannot be empty."));
        }
        with_document_mut(handle, |doc| {
            if !document_has_handle(doc, owner) {
                return Err(JsValue::from_str("XRecord owner handle does not resolve."));
            }
            let record = doc.ensure_xrecord(owner, key);
            Ok(format!("{:X}", record.value()))
        })
    })
}

/// Read a named XRecord from an owner's Extension Dictionary.
#[wasm_bindgen]
pub fn dwg_xrecord_json(
    handle: u32,
    owner_handle_hex: &str,
    key: &str,
) -> Result<String, JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        with_document(handle, |doc| {
            let record = doc.xrecord(owner, key);
            to_json("acadrust XRecord serialize failed", &record)
        })
    })
}

/// Replace the entries of one named XRecord. Handle references are rebuilt
/// from the typed entry list rather than reusing a stale DWG object-reference
/// stream.
#[wasm_bindgen]
pub fn dwg_set_xrecord_entries_json(
    handle: u32,
    owner_handle_hex: &str,
    key: &str,
    entries_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        let mut entries_value: serde_json::Value = serde_json::from_str(entries_json)
            .map_err(to_js("XRecord entries JSON parse failed"))?;
        normalize_exact_handle_values(&mut entries_value)?;
        let entries: Vec<acadrust::objects::XRecordEntry> = serde_json::from_value(entries_value)
            .map_err(to_js("XRecord entries parse failed"))?;
        with_document_mut(handle, |doc| {
            for entry in &entries {
                if let Some(reference) = entry.value.as_handle() {
                    if !document_has_handle(doc, reference) {
                        return Err(JsValue::from_str("XRecord contains a handle reference that does not resolve."));
                    }
                }
            }
            let record = doc.xrecord_mut(owner, key)
                .ok_or_else(|| JsValue::from_str("Named XRecord does not exist."))?;
            record.entries = entries;
            record.object_references.clear();
            record.preserve_object_reference_stream = false;
            Ok(())
        })
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InsertPlacementPatch {
    insert_point: Option<Point3Patch>,
    scale: Option<[f64; 3]>,
    rotation: Option<f64>,
    normal: Option<Point3Patch>,
}

/// Update INSERT placement without replacing its block or ATTRIB references.
#[wasm_bindgen]
pub fn dwg_update_insert_placement_json(
    handle: u32,
    entity_handle_hex: &str,
    patch_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: InsertPlacementPatch = serde_json::from_str(patch_json)
            .map_err(to_js("INSERT placement patch JSON parse failed"))?;
        if let Some(point) = patch.insert_point.as_ref() {
            if ![point.x, point.y, point.z].iter().all(|v| v.is_finite()) {
                return Err(JsValue::from_str("INSERT insertion point must be finite."));
            }
        }
        if let Some(scale) = patch.scale {
            if !scale.iter().all(|v| v.is_finite() && v.abs() > 1e-12) {
                return Err(JsValue::from_str("INSERT scale factors must be finite and non-zero."));
            }
        }
        if let Some(rotation) = patch.rotation {
            if !rotation.is_finite() {
                return Err(JsValue::from_str("INSERT rotation must be finite."));
            }
        }
        if let Some(normal) = patch.normal.as_ref() {
            if ![normal.x, normal.y, normal.z].iter().all(|v| v.is_finite())
                || (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z) <= 1e-24 {
                return Err(JsValue::from_str("INSERT normal must be finite and non-zero."));
            }
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            let insert = match entity {
                acadrust::EntityType::Insert(insert) => insert,
                _ => return Err(JsValue::from_str("Entity is not an INSERT.")),
            };
            if let Some(point) = patch.insert_point {
                insert.insert_point = acadrust::Vector3::new(point.x, point.y, point.z);
            }
            if let Some([x, y, z]) = patch.scale {
                insert.set_x_scale(x);
                insert.set_y_scale(y);
                insert.set_z_scale(z);
            }
            if let Some(rotation) = patch.rotation {
                insert.rotation = rotation;
            }
            if let Some(normal) = patch.normal {
                insert.normal = acadrust::Vector3::new(normal.x, normal.y, normal.z);
            }
            Ok(())
        })
    })
}

/// Change one single-line ATTRIB value while retaining its handle, owner,
/// ATTDEF link, and INSERT sequence references. Multiline attributes are
/// refused until the embedded MTEXT payload can be edited atomically too.
#[wasm_bindgen]
pub fn dwg_update_insert_attribute_value(
    handle: u32,
    insert_handle_hex: &str,
    tag: &str,
    value: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let insert_handle = parse_handle_hex(insert_handle_hex)?;
        if tag.trim().is_empty() {
            return Err(JsValue::from_str("Attribute tag cannot be empty."));
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(insert_handle)
                .ok_or_else(|| JsValue::from_str("INSERT handle was not found."))?;
            let insert = match entity {
                acadrust::EntityType::Insert(insert) => insert,
                _ => return Err(JsValue::from_str("Entity is not an INSERT.")),
            };
            let attribute = insert.attributes.iter_mut()
                .find(|attribute| attribute.tag.eq_ignore_ascii_case(tag))
                .ok_or_else(|| JsValue::from_str("INSERT attribute tag was not found."))?;
            if attribute.is_multiline || attribute.embedded_mtext.is_some() {
                return Err(JsValue::from_str("Multiline ATTRIB edits require an atomic embedded MTEXT patch."));
            }
            attribute.value = value.to_string();
            Ok(())
        })
    })
}

/// Change an ATTDEF default value without replacing its handle or definition
/// identity. Existing INSERT attribute instances are intentionally unchanged.
#[wasm_bindgen]
pub fn dwg_update_attdef_default_value(
    handle: u32,
    attdef_handle_hex: &str,
    value: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let attdef_handle = parse_handle_hex(attdef_handle_hex)?;
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(attdef_handle)
                .ok_or_else(|| JsValue::from_str("ATTDEF handle was not found."))?;
            match entity {
                acadrust::EntityType::AttributeDefinition(attdef) => {
                    attdef.default_value = value.to_string();
                    Ok(())
                }
                _ => Err(JsValue::from_str("Entity is not an ATTDEF.")),
            }
        })
    })
}

fn document_has_handle(doc: &CadDocument, handle: acadrust::types::Handle) -> bool {
    doc.get_entity(handle).is_some()
        || doc.objects.contains_key(&handle)
        || doc.layers.iter().any(|entry| entry.handle == handle)
        || doc.line_types.iter().any(|entry| entry.handle == handle)
        || doc.text_styles.iter().any(|entry| entry.handle == handle)
        || doc.dim_styles.iter().any(|entry| entry.handle == handle)
        || doc.app_ids.iter().any(|entry| entry.handle == handle)
        || doc.views.iter().any(|entry| entry.handle == handle)
        || doc.vports.iter().any(|entry| entry.handle == handle)
        || doc.ucss.iter().any(|entry| entry.handle == handle)
        || doc.vx_table.iter().any(|entry| entry.handle == handle)
        || doc.block_records.iter().any(|entry| {
            entry.handle == handle
                || entry.block_entity_handle == handle
                || entry.block_end_handle == handle
        })
}

/// Replace an entity's reactor list from exact hexadecimal handles. All target
/// handles must resolve to a resident entity, table record, or OBJECTS entry.
#[wasm_bindgen]
pub fn dwg_set_entity_reactors_json(
    handle: u32,
    entity_handle_hex: &str,
    reactors_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let reactor_hexes: Vec<String> = serde_json::from_str(reactors_json)
            .map_err(to_js("reactor handle list parse failed"))?;
        let mut reactors = Vec::with_capacity(reactor_hexes.len());
        let mut seen = std::collections::HashSet::new();
        for hex in reactor_hexes {
            let reactor = parse_handle_hex(&hex)?;
            if reactor == entity_handle {
                return Err(JsValue::from_str("An entity cannot be its own reactor."));
            }
            if !seen.insert(reactor.value()) {
                return Err(JsValue::from_str("Duplicate reactor handles are not allowed."));
            }
            reactors.push(reactor);
        }
        with_document_mut(handle, |doc| {
            for reactor in &reactors {
                if !document_has_handle(doc, *reactor) {
                    return Err(JsValue::from_str(&format!(
                        "Reactor handle {:X} does not resolve in the resident document.",
                        reactor.value()
                    )));
                }
            }
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("Entity handle was not found."))?;
            entity.common_mut().reactors = reactors;
            Ok(())
        })
    })
}

/// Upsert one key in the owner's Extension Dictionary. The target must already
/// exist; the method never creates dangling dictionary references.
#[wasm_bindgen]
pub fn dwg_set_extension_dictionary_entry(
    handle: u32,
    owner_handle_hex: &str,
    key: &str,
    target_handle_hex: &str,
    hard_owner: bool,
) -> Result<String, JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        let target = parse_handle_hex(target_handle_hex)?;
        if key.trim().is_empty() {
            return Err(JsValue::from_str("Dictionary key cannot be empty."));
        }
        with_document_mut(handle, |doc| {
            if !document_has_handle(doc, owner) {
                return Err(JsValue::from_str("Dictionary owner handle does not resolve."));
            }
            if owner == target {
                return Err(JsValue::from_str("An Extension Dictionary entry cannot target its own owner."));
            }
            if !document_has_handle(doc, target) {
                return Err(JsValue::from_str("Dictionary target handle does not resolve."));
            }
            let dictionary_handle = doc.ensure_extension_dictionary(owner);
            match doc.objects.get_mut(&dictionary_handle) {
                Some(acadrust::objects::ObjectType::Dictionary(dictionary)) => {
                    if let Some((_, value)) = dictionary.entries.iter_mut()
                        .find(|(existing, _)| existing.eq_ignore_ascii_case(key)) {
                        *value = target;
                    } else {
                        dictionary.add_entry(key, target);
                    }
                    dictionary.set_entry_hard_owner(key, hard_owner);
                    Ok(format!("{:X}", dictionary_handle.value()))
                }
                _ => Err(JsValue::from_str("Owner Extension Dictionary is missing or has an invalid type.")),
            }
        })
    })
}

/// Remove a key from the owner's Extension Dictionary without deleting its
/// target object. Empty dictionaries remain attached for safe later reuse.
#[wasm_bindgen]
pub fn dwg_remove_extension_dictionary_entry(
    handle: u32,
    owner_handle_hex: &str,
    key: &str,
) -> Result<bool, JsValue> {
    guard(|| {
        let owner = parse_handle_hex(owner_handle_hex)?;
        with_document_mut(handle, |doc| {
            let Some(dictionary_handle) = doc.extension_dictionary_handle(owner) else {
                return Ok(false);
            };
            match doc.objects.get_mut(&dictionary_handle) {
                Some(acadrust::objects::ObjectType::Dictionary(dictionary)) => {
                    let before = dictionary.entries.len();
                    dictionary.entries.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
                    dictionary.hard_owner_entries.retain(|existing| !existing.eq_ignore_ascii_case(key));
                    Ok(dictionary.entries.len() != before)
                }
                _ => Err(JsValue::from_str("Owner Extension Dictionary has an invalid type.")),
            }
        })
    })
}

// Update only reference-safe common properties. Geometry and handle-bearing
/// fields are deliberately excluded from this patch protocol so JS number
/// precision cannot corrupt embedded u64 references.
#[wasm_bindgen]
pub fn dwg_update_entity_common_json(
    handle: u32,
    entity_handle_hex: &str,
    patch_json: &str,
) -> Result<(), JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        let patch: EntityCommonPatch = serde_json::from_str(patch_json)
            .map_err(to_js("entity common patch JSON parse failed"))?;
        if let Some(layer) = patch.layer.as_ref() {
            if layer.trim().is_empty() {
                return Err(JsValue::from_str("Entity layer must not be empty."));
            }
        }
        if let Some(scale) = patch.linetype_scale {
            if !scale.is_finite() || scale <= 0.0 {
                return Err(JsValue::from_str("Linetype scale must be finite and greater than zero."));
            }
        }
        with_document_mut(handle, |doc| {
            let entity = doc.get_entity_mut(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            let common = entity.common_mut();
            if let Some(layer) = patch.layer { common.layer = layer; }
            if let Some(linetype) = patch.linetype { common.linetype = linetype; }
            if let Some(scale) = patch.linetype_scale { common.linetype_scale = scale; }
            if let Some(invisible) = patch.invisible { common.invisible = invisible; }
            Ok(())
        })
    })
}

/// Read one entity by its exact hexadecimal DWG handle.
#[wasm_bindgen]
pub fn dwg_entity_json(handle: u32, entity_handle_hex: &str) -> Result<String, JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        with_document(handle, |doc| {
            let entity = doc.get_entity(entity_handle)
                .ok_or_else(|| JsValue::from_str("DWG entity handle was not found."))?;
            to_json("acadrust entity serialize failed", entity)
        })
    })
}

#[wasm_bindgen]
pub fn dwg_remove_entity(handle: u32, entity_handle_hex: &str) -> Result<bool, JsValue> {
    guard(|| {
        let entity_handle = parse_handle_hex(entity_handle_hex)?;
        with_document_mut(handle, |doc| Ok(doc.remove_entity(entity_handle).is_some()))
    })
}

/// Lossless, handle-string view of dynamic-block evaluation metadata.
#[wasm_bindgen]
pub fn dwg_dynamic_block_index_json(handle: u32) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let mut dynamic_objects = Vec::new();
        for (object_handle, object) in &doc.objects {
            if let acadrust::objects::ObjectType::DynamicBlock(dynamic) = object {
                dynamic_objects.push(serde_json::json!({
                    "handle": format!("{:X}", object_handle.value()),
                    "owner": format!("{:X}", dynamic.owner.value()),
                    "dxfName": dynamic.dxf_name,
                    "cppClassName": dynamic.cpp_class_name,
                    "data": dynamic.data
                }));
            }
        }
        dynamic_objects.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

        let mut visibility_parameters = Vec::new();
        for (parameter_handle, parameter) in &doc.block_visibility_params {
            let states: Vec<_> = parameter.states.iter().map(|state| {
                serde_json::json!({
                    "name": state.name,
                    "visibleBlocks": state.visible_blocks.iter()
                        .map(|h| format!("{:X}", h.value())).collect::<Vec<_>>(),
                    "visibleParams": state.visible_params.iter()
                        .map(|h| format!("{:X}", h.value())).collect::<Vec<_>>()
                })
            }).collect();
            visibility_parameters.push(serde_json::json!({
                "handle": format!("{:X}", parameter_handle.value()),
                "owner": format!("{:X}", parameter.owner.value()),
                "name": parameter.name,
                "description": parameter.description,
                "allBlocks": parameter.all_blocks.iter()
                    .map(|h| format!("{:X}", h.value())).collect::<Vec<_>>(),
                "states": states
            }));
        }
        visibility_parameters.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

        let mut representations = Vec::new();
        for (representation, definition) in &doc.block_representations {
            representations.push(serde_json::json!({
                "representationHandle": format!("{:X}", representation.value()),
                "dynamicBlockHandle": format!("{:X}", definition.value())
            }));
        }
        representations.sort_by(|a, b| {
            a["representationHandle"].as_str().cmp(&b["representationHandle"].as_str())
        });
        let result = serde_json::json!({
            "dynamicObjects": dynamic_objects,
            "visibilityParameters": visibility_parameters,
            "representations": representations
        });
        to_json("dynamic block index serialize failed", &result)
    }))
}

/// Read an INSERT's ATTRIB/SEQEND chain using exact hexadecimal handles.
/// Broken links are reported; source data is never silently repaired.
#[wasm_bindgen]
pub fn dwg_insert_sequence_json(
    handle: u32,
    insert_handle_hex: &str,
) -> Result<String, JsValue> {
    guard(|| {
        let insert_handle = parse_handle_hex(insert_handle_hex)?;
        with_document(handle, |doc| {
            let insert = match doc.get_entity(insert_handle) {
                Some(acadrust::EntityType::Insert(insert)) => insert,
                Some(_) => return Err(JsValue::from_str("Target entity is not an INSERT.")),
                None => return Err(JsValue::from_str("INSERT handle was not found.")),
            };
            let mut attributes = Vec::with_capacity(insert.attributes.len());
            let mut seen = std::collections::HashSet::new();
            let mut valid = true;
            for attribute in &insert.attributes {
                let attribute_handle = attribute.common.handle;
                let attdef = !attribute.attdef_handle.is_null()
                    && matches!(
                        doc.get_entity(attribute.attdef_handle),
                        Some(acadrust::EntityType::AttributeDefinition(_))
                    );
                let owner_matches = attribute.common.owner_handle == insert_handle;
                let unique = seen.insert(attribute_handle.value());
                valid &= !attribute_handle.is_null() && attdef && owner_matches && unique;
                attributes.push(serde_json::json!({
                    "handle": format!("{:X}", attribute_handle.value()),
                    "tag": attribute.tag,
                    "value": attribute.value,
                    "attdefHandle": if attribute.attdef_handle.is_null() {
                        String::new()
                    } else {
                        format!("{:X}", attribute.attdef_handle.value())
                    },
                    "ownerHandle": if attribute.common.owner_handle.is_null() {
                        String::new()
                    } else {
                        format!("{:X}", attribute.common.owner_handle.value())
                    },
                    "attdefResolved": attdef,
                    "ownerMatchesInsert": owner_matches,
                    "uniqueHandle": unique
                }));
            }
            let seqend_handle = insert.seqend_handle;
            let seqend_resolved = seqend_handle.is_some_and(|seqend| {
                matches!(doc.get_entity(seqend), Some(acadrust::EntityType::Seqend(_)))
            });
            if !insert.attributes.is_empty() {
                valid &= seqend_resolved;
            }
            let result = serde_json::json!({
                "insertHandle": format!("{:X}", insert_handle.value()),
                "blockName": insert.block_name,
                "attributes": attributes,
                "seqendHandle": seqend_handle
                    .filter(|h| !h.is_null())
                    .map(|h| format!("{:X}", h.value())),
                "seqendResolved": seqend_resolved,
                "valid": valid
            });
            to_json("INSERT attribute sequence serialize failed", &result)
        })
    })
}

fn stable_bytes_fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016X}", hash)
}

/// Build a deterministic relationship-preservation snapshot. Every collection
/// is sorted by exact handle before serialization, avoiding HashMap iteration
/// order as a source of false round-trip differences.
fn document_roundtrip_fingerprint(doc: &CadDocument) -> Result<serde_json::Value, JsValue> {
    let mut entities = Vec::new();
    for entity in doc.entities() {
        let handle = entity.common().handle.value();
        let payload = serde_json::to_value(entity)
            .map_err(to_js("round-trip entity snapshot serialize failed"))?;
        entities.push((handle, payload));
    }
    entities.sort_by_key(|(handle, _)| *handle);

    let mut block_records = Vec::new();
    for record in doc.block_records.iter() {
        let handle = record.handle.value();
        let members: Vec<String> = record.entity_handles.iter()
            .map(|value| format!("{:X}", value.value())).collect();
        let inserts: Vec<String> = record.insert_handles.iter()
            .map(|value| format!("{:X}", value.value())).collect();
        block_records.push(serde_json::json!({
            "handle": format!("{:X}", handle),
            "name": record.name,
            "blockEntity": format!("{:X}", record.block_entity_handle.value()),
            "blockEnd": format!("{:X}", record.block_end_handle.value()),
            "members": members,
            "inserts": inserts
        }));
    }
    block_records.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut objects = Vec::new();
    let mut opaque_objects = Vec::new();
    for (handle, object) in &doc.objects {
        let payload = serde_json::to_value(object)
            .map_err(to_js("round-trip object snapshot serialize failed"))?;
        objects.push((handle.value(), payload));
        if let acadrust::objects::ObjectType::Unknown {
            type_name,
            handle: object_handle,
            owner,
            raw_dwg_data,
            ..
        } = object {
            opaque_objects.push(serde_json::json!({
                "handle": format!("{:X}", object_handle.value()),
                "owner": format!("{:X}", owner.value()),
                "typeName": type_name,
                "rawBytesLength": raw_dwg_data.as_ref().map_or(0, |bytes| bytes.len()),
                "rawBytesFingerprint": raw_dwg_data.as_ref()
                    .map(|bytes| stable_bytes_fingerprint(bytes))
            }));
        }
    }
    objects.sort_by_key(|(handle, _)| *handle);
    opaque_objects.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut object_xdata = Vec::new();
    for (handle, data) in &doc.object_xdata {
        object_xdata.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "data": serde_json::to_value(data)
                .map_err(to_js("round-trip XDATA snapshot serialize failed"))?
        }));
    }
    object_xdata.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut visibility = Vec::new();
    for (handle, value) in &doc.block_visibility_params {
        visibility.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "value": serde_json::to_value(value)
                .map_err(to_js("round-trip dynamic visibility snapshot serialize failed"))?
        }));
    }
    visibility.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut representations = Vec::new();
    for (representation, definition) in &doc.block_representations {
        representations.push(serde_json::json!([
            format!("{:X}", representation.value()),
            format!("{:X}", definition.value())
        ]));
    }
    representations.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));

    // Table records and document-level relationship side maps are part of
    // the DWG object graph too. Comparing only drawable entities can miss a
    // broken layer/style table, extension-dictionary pointer or reactor edge.
    let tables = serde_json::json!({
        "layers": serde_json::to_value(&doc.layers).map_err(to_js("round-trip layer table snapshot failed"))?,
        "lineTypes": serde_json::to_value(&doc.line_types).map_err(to_js("round-trip linetype table snapshot failed"))?,
        "textStyles": serde_json::to_value(&doc.text_styles).map_err(to_js("round-trip text style table snapshot failed"))?,
        "dimStyles": serde_json::to_value(&doc.dim_styles).map_err(to_js("round-trip dimstyle table snapshot failed"))?,
        "appIds": serde_json::to_value(&doc.app_ids).map_err(to_js("round-trip APPID table snapshot failed"))?,
        "views": serde_json::to_value(&doc.views).map_err(to_js("round-trip VIEW table snapshot failed"))?,
        "vports": serde_json::to_value(&doc.vports).map_err(to_js("round-trip VPORT table snapshot failed"))?,
        "ucss": serde_json::to_value(&doc.ucss).map_err(to_js("round-trip UCS table snapshot failed"))?,
        "vxTable": serde_json::to_value(&doc.vx_table).map_err(to_js("round-trip VX table snapshot failed"))?
    });

    let mut extension_dictionary_links = Vec::new();
    for (owner, dictionary) in &doc.xdic_by_handle {
        extension_dictionary_links.push(serde_json::json!([
            format!("{:X}", owner.value()),
            format!("{:X}", dictionary.value())
        ]));
    }
    extension_dictionary_links.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));

    let mut reactor_links = Vec::new();
    for (owner, reactors) in &doc.reactors_by_handle {
        reactor_links.push(serde_json::json!({
            "owner": format!("{:X}", owner.value()),
            "reactors": reactors.iter().map(|h| format!("{:X}", h.value())).collect::<Vec<_>>()
        }));
    }
    reactor_links.sort_by(|a, b| a["owner"].as_str().cmp(&b["owner"].as_str()));

    let mut context_scales = Vec::new();
    for (context, scale) in &doc.context_scales {
        context_scales.push(serde_json::json!([
            format!("{:X}", context.value()),
            format!("{:X}", scale.value())
        ]));
    }
    context_scales.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));

    let mut field_definitions = Vec::new();
    for (handle, field) in &doc.fields {
        field_definitions.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "value": serde_json::to_value(field)
                .map_err(to_js("round-trip FIELD snapshot serialize failed"))?
        }));
    }
    field_definitions.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut dgn_definitions = Vec::new();
    for (handle, definition) in &doc.dgn_ls_definitions {
        dgn_definitions.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "value": serde_json::to_value(definition)
                .map_err(to_js("round-trip DGN linetype snapshot serialize failed"))?
        }));
    }
    dgn_definitions.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut dgn_components = Vec::new();
    for (handle, component) in &doc.dgn_ls_components {
        dgn_components.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "value": serde_json::to_value(component)
                .map_err(to_js("round-trip DGN component snapshot serialize failed"))?
        }));
    }
    dgn_components.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let vx_control_entries: Vec<String> = doc.vx_control_entries.iter()
        .map(|handle| format!("{:X}", handle.value())).collect();

    let mut eed_records = Vec::new();
    for (handle, records) in &doc.eed_by_handle {
        let values: Vec<_> = records.iter().map(|(application, bytes)| {
            serde_json::json!({
                "application": format!("{:X}", application),
                "byteLength": bytes.len(),
                "fingerprint": stable_bytes_fingerprint(bytes)
            })
        }).collect();
        eed_records.push(serde_json::json!({
            "handle": format!("{:X}", handle.value()),
            "records": values
        }));
    }
    eed_records.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));

    let mut block_owned_handles = Vec::new();
    for (block, handles) in &doc.block_entity_handles {
        block_owned_handles.push(serde_json::json!({
            "block": format!("{:X}", block.value()),
            "handles": handles.iter().map(|h| format!("{:X}", h.value())).collect::<Vec<_>>()
        }));
    }
    block_owned_handles.sort_by(|a, b| a["block"].as_str().cmp(&b["block"].as_str()));

    let mut data_store_handles: Vec<String> = doc.dwg_data_store_handles.iter()
        .map(|handle| format!("{:X}", handle.value())).collect();
    data_store_handles.sort();

    let acds_data = doc.raw_acds_data.as_ref().map(|bytes| {
        serde_json::json!({
            "byteLength": bytes.len(),
            "fingerprint": stable_bytes_fingerprint(bytes.as_slice())
        })
    });

    let metadata = serde_json::json!({
        "version": serde_json::to_value(&doc.version).map_err(to_js("round-trip version snapshot failed"))?,
        "maintenanceVersion": doc.maintenance_version,
        "dwgSourceVersion": serde_json::to_value(&doc.dwg_source_version).map_err(to_js("round-trip DWG source version snapshot failed"))?,
        "header": serde_json::to_value(&doc.header).map_err(to_js("round-trip header snapshot failed"))?,
        "summaryInfo": serde_json::to_value(&doc.summary_info).map_err(to_js("round-trip summary info snapshot failed"))?,
        "classes": serde_json::to_value(&doc.classes).map_err(to_js("round-trip classes snapshot failed"))?
    });

    Ok(serde_json::json!({
        "entities": entities.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
        "blockRecords": block_records,
        "objects": objects.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
        "opaqueObjects": opaque_objects,
        "objectXdata": object_xdata,
        "dynamicVisibility": visibility,
        "blockRepresentations": representations,
        "tables": tables,
        "extensionDictionaryLinks": extension_dictionary_links,
        "reactorLinks": reactor_links,
        "contextScales": context_scales,
        "fieldDefinitions": field_definitions,
        "dgnLineStyleDefinitions": dgn_definitions,
        "dgnLineStyleComponents": dgn_components,
        "vxControlEntries": vx_control_entries,
        "eedRecords": eed_records,
        "blockOwnedHandles": block_owned_handles,
        "dataStoreHandles": data_store_handles,
        "acdsData": acds_data,
        "metadata": metadata
    }))
}

/// Enumerate opaque DWG OBJECTS records without exposing or mutating their
/// raw byte payloads. This includes custom application objects (for example
/// Mechanical/vertical-product objects) that acadrust cannot semantically edit.
#[wasm_bindgen]
pub fn dwg_opaque_object_index_json(handle: u32) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let mut items = Vec::new();
        for (object_handle, object) in &doc.objects {
            if let acadrust::objects::ObjectType::Unknown {
                type_name,
                handle: intrinsic_handle,
                owner,
                raw_dwg_data,
                raw_dwg_handle_bits,
                ..
            } = object {
                items.push(serde_json::json!({
                    "handle": format!("{:X}", object_handle.value()),
                    "intrinsicHandle": format!("{:X}", intrinsic_handle.value()),
                    "owner": format!("{:X}", owner.value()),
                    "typeName": type_name,
                    "rawBytesLength": raw_dwg_data.as_ref().map_or(0, |bytes| bytes.len()),
                    "rawBytesFingerprint": raw_dwg_data.as_ref()
                        .map(|bytes| stable_bytes_fingerprint(bytes)),
                    "handleStreamBits": raw_dwg_handle_bits
                }));
            }
        }
        items.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));
        to_json("opaque OBJECTS index serialize failed", &items)
    }))
}

/// Save and reopen the resident document in memory, then compare a stable
/// snapshot of entity geometry/references, block membership, OBJECTS, XDATA,
/// dynamic visibility and opaque-object byte fingerprints. This validates the
/// acadrust writer/reader pair only; AutoCAD compatibility still needs a
/// separate external open + AUDIT test.
#[wasm_bindgen]
pub fn dwg_roundtrip_audit_json(handle: u32) -> Result<String, JsValue> {
    guard(|| with_document(handle, |doc| {
        let before = document_roundtrip_fingerprint(doc)?;
        let bytes = DwgWriter::write_to_vec(doc)
            .map_err(to_js("round-trip audit DWG write failed"))?;
        let byte_length = bytes.len();
        let mut reader = DwgReader::from_stream(Cursor::new(bytes));
        let reopened = reader.read().map_err(to_js("round-trip audit DWG reopen failed"))?;
        let after = document_roundtrip_fingerprint(&reopened)?;
        let equal = |key: &str| before.get(key) == after.get(key);
        let result = serde_json::json!({
            "ok": before == after,
            "dwgByteLength": byte_length,
            "entityCount": before["entities"].as_array().map_or(0, |v| v.len()),
            "blockRecordCount": before["blockRecords"].as_array().map_or(0, |v| v.len()),
            "objectCount": before["objects"].as_array().map_or(0, |v| v.len()),
            "opaqueObjectCount": before["opaqueObjects"].as_array().map_or(0, |v| v.len()),
            "entitiesEqual": equal("entities"),
            "blockRecordsEqual": equal("blockRecords"),
            "objectsEqual": equal("objects"),
            "opaqueObjectsEqual": equal("opaqueObjects"),
            "objectXdataEqual": equal("objectXdata"),
            "dynamicVisibilityEqual": equal("dynamicVisibility"),
            "blockRepresentationsEqual": equal("blockRepresentations"),
            "tablesEqual": equal("tables"),
            "extensionDictionaryLinksEqual": equal("extensionDictionaryLinks"),
            "reactorLinksEqual": equal("reactorLinks"),
            "contextScalesEqual": equal("contextScales"),
            "fieldDefinitionsEqual": equal("fieldDefinitions"),
            "dgnLineStyleDefinitionsEqual": equal("dgnLineStyleDefinitions"),
            "dgnLineStyleComponentsEqual": equal("dgnLineStyleComponents"),
            "vxControlEntriesEqual": equal("vxControlEntries"),
            "eedRecordsEqual": equal("eedRecords"),
            "blockOwnedHandlesEqual": equal("blockOwnedHandles"),
            "dataStoreHandlesEqual": equal("dataStoreHandles"),
            "acdsDataEqual": equal("acdsData"),
            "metadataEqual": equal("metadata"),
            "beforeFingerprint": stable_bytes_fingerprint(
                &serde_json::to_vec(&before).map_err(to_js("before fingerprint serialize failed"))?
            ),
            "afterFingerprint": stable_bytes_fingerprint(
                &serde_json::to_vec(&after).map_err(to_js("after fingerprint serialize failed"))?
            )
        });
        to_json("round-trip audit result serialize failed", &result)
    }))
}

#[wasm_bindgen]
pub fn dwg_save(handle: u32) -> Result<Vec<u8>, JsValue> {
    guard(|| with_document(handle, |doc| {
        DwgWriter::write_to_vec(doc).map_err(to_js("acadrust DWG write failed"))
    }))
}


#[cfg(test)]
mod exact_handle_index_tests {
    use super::{collect_exact_handles, ExactHandlePath};

    #[test]
    fn indexes_scalar_array_and_dictionary_handle_references_without_touching_geometry() {
        let value = serde_json::json!({
            "handle": 18446744073709551614u64,
            "owner": 7u64,
            "reactors": [8u64, 9u64],
            "entities": [10u64, 11u64],
            "object_references": [12u64],
            "entries": [["ACAD_LAYOUT", 13u64], ["ACAD_GROUP", 14u64]],
            "points": [1.0, 2.0, 3.0],
            "xdata": [{"code": 1000, "value": "not a handle"}]
        });
        let mut paths = Vec::new();
        let mut output: Vec<ExactHandlePath> = Vec::new();
        collect_exact_handles(&value, &mut paths, &mut output);

        let indexed: std::collections::HashSet<(String, String)> = output
            .into_iter()
            .map(|item| (item.path.join("/"), item.handle))
            .collect();
        assert!(indexed.contains(&("handle".into(), "FFFFFFFFFFFFFFFE".into())));
        assert!(indexed.contains(&("owner".into(), "7".into())));
        assert!(indexed.contains(&("reactors/0".into(), "8".into())));
        assert!(indexed.contains(&("reactors/1".into(), "9".into())));
        assert!(indexed.contains(&("entities/0".into(), "A".into())));
        assert!(indexed.contains(&("object_references/0".into(), "C".into())));
        assert!(indexed.contains(&("entries/0/1".into(), "D".into())));
        assert!(indexed.contains(&("entries/1/1".into(), "E".into())));
        assert!(!indexed.iter().any(|(path, _)| path.starts_with("points/")));
    }
}


#[cfg(test)]
mod dwg_roundtrip_audit_tests {
    use super::document_roundtrip_fingerprint;
    use acadrust::{CadDocument, DwgReader, DwgWriter};
    use std::io::Cursor;

    #[test]
    fn simple_entity_survives_native_dwg_write_and_reopen() {
        let mut document = CadDocument::new();
        let line = acadrust::entities::Line::from_coords(
            1.0, 2.0, 0.0,
            30.0, 40.0, 0.0,
        );
        document.add_entity(acadrust::EntityType::Line(line)).unwrap();

        let before = document_roundtrip_fingerprint(&document).unwrap();
        let bytes = DwgWriter::write_to_vec(&document).unwrap();
        let mut reader = DwgReader::from_stream(Cursor::new(bytes));
        let reopened = reader.read().unwrap();
        let after = document_roundtrip_fingerprint(&reopened).unwrap();

        assert_eq!(before, after);
    }
}


#[cfg(test)]
mod lwpolyline_vertex_operation_tests {
    use super::*;

    fn vertex(x: f64, id: i32) -> LwPolylineVertexPatch {
        LwPolylineVertexPatch {
            location: Point2Patch { x, y: x + 1.0 },
            bulge: 0.25,
            start_width: 1.0,
            end_width: 2.0,
            vertex_id: id,
        }
    }

    fn existing_vertex(x: f64, id: i32) -> acadrust::entities::LwVertex {
        acadrust::entities::LwVertex {
            location: acadrust::types::Vector2::new(x, x + 1.0),
            bulge: 0.0,
            start_width: 0.0,
            end_width: 0.0,
            vertex_id: id,
        }
    }

    #[test]
    fn indexed_insert_update_remove_preserve_order_and_vertex_metadata() {
        let result = apply_lwpolyline_vertex_operations(
            vec![existing_vertex(0.0, 10), existing_vertex(20.0, 30)],
            vec![
                LwPolylineVertexOperation::Insert { index: 1, vertex: vertex(10.0, 20) },
                LwPolylineVertexOperation::Update { index: 2, vertex: vertex(25.0, 31) },
                LwPolylineVertexOperation::Remove { index: 0 },
            ],
        ).expect("valid operations");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].vertex_id, 20);
        assert_eq!(result[0].location.x, 10.0);
        assert_eq!(result[0].bulge, 0.25);
        assert_eq!(result[1].vertex_id, 31);
        assert_eq!(result[1].start_width, 1.0);
        assert_eq!(result[1].end_width, 2.0);
    }

    #[test]
    fn invalid_index_returns_error_without_mutating_original_vector() {
        let original = vec![existing_vertex(0.0, 10)];
        let result = apply_lwpolyline_vertex_operations(
            original.clone(),
            vec![
                LwPolylineVertexOperation::Insert { index: 1, vertex: vertex(10.0, 20) },
                LwPolylineVertexOperation::Remove { index: 99 },
            ],
        );
        assert!(result.is_err());
        assert_eq!(original.len(), 1);
        assert_eq!(original[0].vertex_id, 10);
    }
}
