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
                    || key == "owner"
                    || key == "block_record"
                    || key == "block_entity"
                    || key == "block_end";
                if is_handle_field {
                    if let Some(raw) = child.as_u64() {
                        output.push(ExactHandlePath {
                            path: path.clone(),
                            handle: format!("{:X}", raw),
                        });
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
        if !record.entity_handles.is_empty() || !record.insert_handles.is_empty()
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
        let record: acadrust::xdata::ExtendedDataRecord = serde_json::from_str(record_json)
            .map_err(to_js("XDATA record parse failed"))?;
        if record.application_name.trim().is_empty() {
            return Err(JsValue::from_str("XDATA application name cannot be empty."));
        }
        with_document_mut(handle, |doc| {
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
        let entries: Vec<acadrust::objects::XRecordEntry> = serde_json::from_str(entries_json)
            .map_err(to_js("XRecord entries parse failed"))?;
        with_document_mut(handle, |doc| {
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

#[wasm_bindgen]
pub fn dwg_save(handle: u32) -> Result<Vec<u8>, JsValue> {
    guard(|| with_document(handle, |doc| {
        DwgWriter::write_to_vec(doc).map_err(to_js("acadrust DWG write failed"))
    }))
}
