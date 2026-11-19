import {
  ACADRUST_SECTION_KINDS,
  type AcadRustEntity,
  type AcadRustMeta,
  type AcadRustSectionKind,
  type AcadRustPoint3d
} from './model/acadrust-model'

/**
 * Low-level client for the Architecture-B sectioned WASM protocol.
 *
 * It abstracts the `dwg_open` -> `dwg_meta` -> `dwg_section_json` ->
 * `dwg_entity_page` -> `dwg_close` round-trips behind one interface so that:
 *   * the production {@link AcadRustWorkerDocumentClient} talks to the real
 *     parser Web Worker, and
 *   * a jest test can inject a fake that replays the committed
 *     `__tests__/fixtures/sample-drawing/*.json` (node cannot load the
 *     `--target web` wasm glue, so the worker path itself is untestable there).
 *
 * Everything above the handle boundary is JSON text; this client parses it once
 * so the converter only ever sees typed model objects.
 */
export interface AcadRustDocumentClient {
  /** Parse the DWG once into a resident `CadDocument`; returns its handle. */
  open(data: ArrayBuffer): Promise<number>
  /** Bounded summary for progress + pagination upper bounds. */
  meta(handle: number): Promise<AcadRustMeta>
  /** One document section, parsed from serde JSON into `T`. */
  section<T>(handle: number, kind: AcadRustSectionKind): Promise<T>
  /** Exact u64 handle fields and their JSON paths for a section. */
  sectionHandleIndex(handle: number, kind: AcadRustSectionKind): Promise<AcadRustExactHandlePath[]>
  /** Exact dynamic-block graph and visibility metadata (read-only). */
  dynamicBlockIndex(handle: number): Promise<AcadRustDynamicBlockIndex>
  /** Inspect the ATTRIB/SEQEND chain for an INSERT. */
  insertSequence(handle: number, insertHandle: string): Promise<AcadRustInsertSequence>
  /** Save and reopen in Rust, comparing entity/object/relationship fingerprints. */
  roundtripAudit(handle: number): Promise<AcadRustRoundtripAudit>
  /** List opaque/custom OBJECTS while retaining their raw bytes inside Rust. */
  opaqueObjectIndex(handle: number): Promise<AcadRustOpaqueObjectInfo[]>
  /** One bounded page of model-space entities. */
  entityPage(handle: number, offset: number, limit: number): Promise<AcadRustEntity[]>
  /** Exact hexadecimal handles aligned by index with entityPage(). */
  entityHandlePage(handle: number, offset: number, limit: number): Promise<string[]>
  /** Lossless hexadecimal handles for all entities in a block definition. */
  blockEntityHandles(handle: number, blockName: string): Promise<string[]>
  /** One bounded page of a named block definition's entities. */
  blockEntityPage(
    handle: number,
    blockName: string,
    offset: number,
    limit: number
  ): Promise<AcadRustEntity[]>
  /**
   * Complete entity lists for several block definitions in one round-trip.
   * Callers batch only definitions small enough to fit one page budget and
   * keep {@link blockEntityPage} for oversized ones.
   */
  blockEntitiesBatch(
    handle: number,
    blockNames: string[]
  ): Promise<AcadRustBlockEntityGroup[]>
  /** Add one acadrust EntityType to model space; returns its lossless hex handle. */
  addEntity(handle: number, entityJson: string): Promise<string>
  /** Create a BLOCK-RECORD with matching BLOCK and ENDBLK structural markers. */
  addBlockRecord(handle: number, name: string, basePoint: AcadRustPoint3d, description?: string): Promise<AcadRustCreatedBlockRecord>
  /** Append an ordinary entity to an existing block and update owner membership. */
  addEntityToBlock(handle: number, blockName: string, entityJson: string): Promise<string>
  /** Remove only an empty, unreferenced user block definition. */
  removeEmptyBlockRecord(handle: number, blockName: string): Promise<boolean>
  /** Move an existing entity and atomically update Owner + block membership. */
  moveEntityToBlock(handle: number, entityHandle: string, blockName: string): Promise<void>
  objectXdata<T = unknown>(handle: number, objectHandle: string): Promise<T | null>
  setObjectXdataRecord(handle: number, objectHandle: string, record: unknown): Promise<void>
  removeObjectXdataRecord(handle: number, objectHandle: string, application: string): Promise<boolean>
  ensureExtensionDictionary(handle: number, ownerHandle: string): Promise<string>
  ensureXrecord(handle: number, ownerHandle: string, key: string): Promise<string>
  xrecord<T = unknown>(handle: number, ownerHandle: string, key: string): Promise<T | null>
  setXrecordEntries(handle: number, ownerHandle: string, key: string, entries: unknown[]): Promise<void>
  setEntityReactors(handle: number, entityHandle: string, reactorHandles: string[]): Promise<void>
  setExtensionDictionaryEntry(handle: number, ownerHandle: string, key: string, targetHandle: string, hardOwner?: boolean): Promise<string>
  removeExtensionDictionaryEntry(handle: number, ownerHandle: string, key: string): Promise<boolean>
  /** Read one entity by its lossless hexadecimal DWG handle. */
  getEntity(handle: number, entityHandle: string): Promise<AcadRustEntity>
  /** Typed LWPOLYLINE geometry update; preserves common/reference fields. */
  updateLwPolylineGeometry(handle: number, entityHandle: string, geometry: AcadRustLwPolylineGeometryPatch): Promise<void>
  /** Typed geometry update for LINE endpoints; leaves all reference fields untouched. */
  updateLineGeometry(handle: number, entityHandle: string, geometry: { start?: AcadRustPoint3d; end?: AcadRustPoint3d }): Promise<void>
  /** Typed CIRCLE/ARC geometry update; preserves all handle-bearing fields. */
  updateCircleArcGeometry(handle: number, entityHandle: string, geometry: AcadRustCircleArcGeometryPatch): Promise<void>
  /** Typed ELLIPSE geometry update; preserves common/reference fields. */
  updateEllipseGeometry(handle: number, entityHandle: string, geometry: AcadRustEllipseGeometryPatch): Promise<void>
  updateDimensionBase(handle: number, entityHandle: string, patch: AcadRustDimensionBasePatch): Promise<void>
  getDimensionMeasurement(handle: number, entityHandle: string): Promise<{ subtype: string; measurement: number; cachedMeasurement: number; blockName: string }>
  updateTextGeometry(handle: number, entityHandle: string, geometry: AcadRustTextGeometryPatch): Promise<void>
  updateMTextGeometry(handle: number, entityHandle: string, geometry: AcadRustMTextGeometryPatch): Promise<void>
  /** Typed NURBS SPLINE update; preserves common/reference fields and DWG-only state. */
  updateSplineGeometry(handle: number, entityHandle: string, geometry: AcadRustSplineGeometryPatch): Promise<void>
  /** Typed INSERT placement update; leaves block and ATTRIB references untouched. */
  updateInsertPlacement(handle: number, entityHandle: string, patch: AcadRustInsertPlacementPatch): Promise<void>
  updateInsertAttributeValue(handle: number, insertHandle: string, tag: string, value: string): Promise<void>
  updateAttdefDefaultValue(handle: number, attdefHandle: string, value: string): Promise<void>
  /** Patch reference-safe common properties without round-tripping embedded handles. */
  updateEntityCommon(handle: number, entityHandle: string, patch: AcadRustEntityCommonPatch): Promise<void>
  /** Remove an entity by its lossless hexadecimal DWG handle. */
  removeEntity(handle: number, entityHandle: string): Promise<boolean>
  /** Native DWG serialization of the current resident document. */
  save(handle: number): Promise<ArrayBuffer>
  beginTransaction(handle: number): Promise<void>
  commitTransaction(handle: number): Promise<void>
  abortTransaction(handle: number): Promise<void>
  close(handle: number): Promise<void>
  /** Tear down any transport (worker) resources; safe to call more than once. */
  dispose(): void
}

export interface AcadRustDynamicBlockIndex {
  dynamicObjects: Array<{
    handle: string
    owner: string
    dxfName: string
    cppClassName: string
    data: unknown
  }>
  visibilityParameters: Array<{
    handle: string
    owner: string
    name: string
    description: string
    allBlocks: string[]
    states: Array<{ name: string; visibleBlocks: string[]; visibleParams: string[] }>
  }>
  representations: Array<{ representationHandle: string; dynamicBlockHandle: string }>
}

export interface AcadRustInsertSequence {
  insertHandle: string
  blockName: string
  attributes: Array<{
    handle: string
    tag: string
    value: string
    attdefHandle: string
    ownerHandle: string
    attdefResolved: boolean
    ownerMatchesInsert: boolean
    uniqueHandle: boolean
  }>
  seqendHandle: string | null
  seqendResolved: boolean
  valid: boolean
}

export interface AcadRustRoundtripAudit {
  ok: boolean
  dwgByteLength: number
  entityCount: number
  blockRecordCount: number
  objectCount: number
  opaqueObjectCount: number
  entitiesEqual: boolean
  blockRecordsEqual: boolean
  objectsEqual: boolean
  opaqueObjectsEqual: boolean
  objectXdataEqual: boolean
  dynamicVisibilityEqual: boolean
  blockRepresentationsEqual: boolean
  tablesEqual: boolean
  extensionDictionaryLinksEqual: boolean
  reactorLinksEqual: boolean
  contextScalesEqual: boolean
  fieldDefinitionsEqual: boolean
  dgnLineStyleDefinitionsEqual: boolean
  dgnLineStyleComponentsEqual: boolean
  vxControlEntriesEqual: boolean
  eedRecordsEqual: boolean
  blockOwnedHandlesEqual: boolean
  dataStoreHandlesEqual: boolean
  acdsDataEqual: boolean
  metadataEqual: boolean
  beforeFingerprint: string
  afterFingerprint: string
}

export interface AcadRustOpaqueObjectInfo {
  handle: string
  intrinsicHandle: string
  owner: string
  typeName: string
  rawBytesLength: number
  rawBytesFingerprint: string | null
  handleStreamBits: number
}

export interface AcadRustCreatedBlockRecord {
  name: string
  handle: string
  blockEntityHandle: string
  blockEndHandle: string
}

export interface AcadRustExactHandlePath {
  /** JSON path segments; array indexes are decimal strings. */
  path: string[]
  /** Exact uppercase hexadecimal DWG handle. */
  handle: string
}

export interface AcadRustLwPolylineVertex {
  location: { x: number; y: number }
  bulge: number
  startWidth: number
  endWidth: number
  vertexId: number
}

export type AcadRustLwPolylineVertexOperation =
  | { op: 'insert'; index: number; vertex: AcadRustLwPolylineVertex }
  | { op: 'update'; index: number; vertex: AcadRustLwPolylineVertex }
  | { op: 'remove'; index: number }

export interface AcadRustLwPolylineGeometryPatch {
  vertices?: AcadRustLwPolylineVertex[]
  vertexOperations?: AcadRustLwPolylineVertexOperation[]
  isClosed?: boolean
  plinegen?: boolean
  constantWidth?: number
  elevation?: number
  thickness?: number
  normal?: AcadRustPoint3d
}

export interface AcadRustCircleArcGeometryPatch {
  center?: AcadRustPoint3d
  radius?: number
  startAngle?: number
  endAngle?: number
}

export interface AcadRustDimensionBasePatch {
  /** null clears the explicit text override and restores measured text. */
  textOverride?: string
  clearTextOverride?: boolean
  textMiddlePoint?: AcadRustPoint3d
  textRotation?: number
  horizontalDirection?: number
  styleName?: string
  lineSpacingFactor?: number
  lineSpacingStyle?: 1 | 2
  textUserPositioned?: boolean
  flipArrow1?: boolean
  flipArrow2?: boolean
}

export interface AcadRustTextGeometryPatch {
  value?: string
  insertionPoint?: AcadRustPoint3d
  alignmentPoint?: AcadRustPoint3d
  height?: number
  rotation?: number
  widthFactor?: number
  obliqueAngle?: number
  style?: string
  normal?: AcadRustPoint3d
  thickness?: number
  generationFlags?: number
}
export interface AcadRustMTextGeometryPatch {
  value?: string
  insertionPoint?: AcadRustPoint3d
  height?: number
  rectangleWidth?: number
  rectangleHeight?: number
  rotation?: number
  style?: string
  normal?: AcadRustPoint3d
  lineSpacingFactor?: number
  backgroundFillFlags?: number
  backgroundScale?: number
  backgroundTransparency?: number
}

export interface AcadRustSplineFlagsPatch {
  closed?: boolean
  periodic?: boolean
  rational?: boolean
  planar?: boolean
  linear?: boolean
}

export interface AcadRustSplineGeometryPatch {
  degree?: number
  flags?: AcadRustSplineFlagsPatch
  knots?: number[]
  controlPoints?: AcadRustPoint3d[]
  weights?: number[]
  fitPoints?: AcadRustPoint3d[]
  normal?: AcadRustPoint3d
  knotTolerance?: number
  controlTolerance?: number
  fitTolerance?: number
  beginTangent?: AcadRustPoint3d
  endTangent?: AcadRustPoint3d
  knotParameterization?: number
  cvFrameVisible?: boolean
  dwgFlags1?: number
}

export interface AcadRustEllipseGeometryPatch {
  center?: AcadRustPoint3d
  majorAxis?: AcadRustPoint3d
  minorAxisRatio?: number
  startParameter?: number
  endParameter?: number
  normal?: AcadRustPoint3d
}

export interface AcadRustInsertPlacementPatch {
  insertPoint?: AcadRustPoint3d
  scale?: [number, number, number]
  rotation?: number
  normal?: AcadRustPoint3d
}

export interface AcadRustEntityCommonPatch {
  layer?: string
  linetype?: string
  linetypeScale?: number
  invisible?: boolean
}

/** One block definition returned by {@link AcadRustDocumentClient.blockEntitiesBatch}. */
export interface AcadRustBlockEntityGroup {
  name: string
  entities: AcadRustEntity[]
}

/** Response payloads the worker can return for a given request id. */
interface AcadRustWorkerResult {
  type: 'result'
  id: number
  dxf?: ArrayBuffer
  dwg?: ArrayBuffer
  handle?: number
  entityHandle?: string
  removed?: boolean
  json?: string
  /** `open` only: wall time of the wasm `dwg_open` read, measured in-worker. */
  parseMs?: number
}

interface AcadRustWorkerError {
  type: 'error'
  id?: number
  message: string
}

type AcadRustWorkerMessage = AcadRustWorkerResult | AcadRustWorkerError

/** One in-flight request awaiting its id-matched worker response. */
interface PendingRequest {
  resolve: (result: AcadRustWorkerResult) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

/** Minimal worker-global shape (avoids depending on the WebWorker TS lib). */
interface WorkerLike {
  postMessage(message: unknown, transfer?: Transferable[]): void
  terminate(): void
  onmessage: ((event: MessageEvent) => void) | null
  onerror: ((event: ErrorEvent) => void) | null
}

/**
 * Options for the worker-backed client. Mirrors the fields
 * {@link AcDbAcadRustConverter} resolves from its config, so the factory can
 * hand the same asset URLs to either architecture.
 */
export interface AcadRustWorkerClientOptions {
  /** Canonical worker script URL, or `assetBaseUrl`-derived by the caller. */
  workerUrl: string | URL
  /** Per-request timeout in ms (open is the slow one; sized accordingly). */
  timeout: number
  /**
   * Input byte format for `open`: `'dwg'` (default) posts the `open` kind
   * (WASM `dwg_open`), `'dxf'` posts `dxf-open` (WASM `dxf_open`). Only the
   * open call is format-sensitive; every later sectioned call is handle-scoped
   * and identical for both.
   */
  inputFormat?: 'dwg' | 'dxf'
}

/**
 * {@link AcadRustDocumentClient} backed by the shared acad-rust parser Web
 * Worker. A single worker serves every sectioned call for one document, so the
 * handle opened by `open` stays valid in that worker's wasm memory until
 * `close`. Created lazily; reused for subsequent files; terminated only on a
 * fatal/unattributable failure so the next read transparently rebuilds it.
 */
export class AcadRustWorkerDocumentClient implements AcadRustDocumentClient {
  private worker: WorkerLike | null = null
  private nextRequestId = 1
  private readonly pending = new Map<number, PendingRequest>()
  private readonly options: AcadRustWorkerClientOptions
  /**
   * Per-request timeout, refreshed by {@link setRequestTimeout} because a
   * reused client serves files of different sizes (the converter scales the
   * budget with the input length).
   */
  private requestTimeout: number

  constructor(options: AcadRustWorkerClientOptions) {
    this.options = options
    this.requestTimeout = options.timeout
  }

  /**
   * Update the per-request timeout. A cached client outlives one conversion, so
   * the next `read()` can hand it a budget sized for the new file without
   * rebuilding the worker (and re-instantiating the wasm module).
   */
  setRequestTimeout(timeout: number): void {
    this.requestTimeout = Math.max(1, timeout)
  }

  async open(data: ArrayBuffer): Promise<number> {
    // Copy before transfer so the caller's ArrayBuffer is never detached.
    const payload = data.slice(0)
    const kind = this.options.inputFormat === 'dxf' ? 'dxf-open' : 'open'
    const result = await this.request(
      { id: 0, kind, data: payload },
      [payload]
    )
    if (result.handle == null) {
      throw new Error('acad-rust worker returned an open result without a handle.')
    }
    return result.handle
  }

  async meta(handle: number): Promise<AcadRustMeta> {
    const result = await this.request({ id: 0, kind: 'meta', handle })
    return this.parseJson<AcadRustMeta>(result, 'meta')
  }

  async section<T>(handle: number, kind: AcadRustSectionKind): Promise<T> {
    if (!ACADRUST_SECTION_KINDS.includes(kind)) {
      throw new Error(`Unknown acad-rust section kind: ${String(kind)}`)
    }
    const result = await this.request({ id: 0, kind: 'section', handle, section: kind })
    return this.parseJson<T>(result, `section:${kind}`)
  }

  async sectionHandleIndex(
    handle: number,
    kind: AcadRustSectionKind
  ): Promise<AcadRustExactHandlePath[]> {
    const result = await this.request({
      id: 0,
      kind: 'section-handle-index',
      handle,
      section: kind
    })
    return this.parseJson<AcadRustExactHandlePath[]>(result, `section-handle-index:${kind}`)
  }

  async opaqueObjectIndex(handle: number): Promise<AcadRustOpaqueObjectInfo[]> {
    const result = await this.request({ id: 0, kind: 'opaque-object-index', handle })
    return this.parseJson<AcadRustOpaqueObjectInfo[]>(result, 'opaque-object-index')
  }

  async roundtripAudit(handle: number): Promise<AcadRustRoundtripAudit> {
    const result = await this.request({ id: 0, kind: 'roundtrip-audit', handle })
    return this.parseJson<AcadRustRoundtripAudit>(result, 'roundtrip-audit')
  }

  async dynamicBlockIndex(handle: number): Promise<AcadRustDynamicBlockIndex> {
    const result = await this.request({ id: 0, kind: 'dynamic-block-index', handle })
    return this.parseJson<AcadRustDynamicBlockIndex>(result, 'dynamic-block-index')
  }

  async insertSequence(
    handle: number,
    insertHandle: string
  ): Promise<AcadRustInsertSequence> {
    const result = await this.request({
      id: 0,
      kind: 'insert-sequence',
      handle,
      entityHandle: insertHandle
    })
    return this.parseJson<AcadRustInsertSequence>(result, 'insert-sequence')
  }

  async entityPage(
    handle: number,
    offset: number,
    limit: number
  ): Promise<AcadRustEntity[]> {
    const result = await this.request({ id: 0, kind: 'page', handle, offset, limit })
    return this.parseJson<AcadRustEntity[]>(result, 'entity-page')
  }

  async entityHandlePage(
    handle: number,
    offset: number,
    limit: number
  ): Promise<string[]> {
    const result = await this.request({
      id: 0,
      kind: 'entity-handles-page',
      handle,
      offset,
      limit
    })
    return this.parseJson<string[]>(result, 'entity-handle-page')
  }

  async blockEntityHandles(handle: number, blockName: string): Promise<string[]> {
    const result = await this.request({ id: 0, kind: 'block-handles', handle, block: blockName })
    return this.parseJson<string[]>(result, 'block-entity-handles')
  }

  async blockEntityPage(
    handle: number,
    blockName: string,
    offset: number,
    limit: number
  ): Promise<AcadRustEntity[]> {
    const result = await this.request({
      id: 0,
      kind: 'block-page',
      handle,
      block: blockName,
      offset,
      limit
    })
    return this.parseJson<AcadRustEntity[]>(result, 'block-entity-page')
  }

  async blockEntitiesBatch(
    handle: number,
    blockNames: string[]
  ): Promise<AcadRustBlockEntityGroup[]> {
    if (blockNames.length === 0) {
      return []
    }
    // Names travel as one JSON string: a 500-entry array is a few KB, far
    // cheaper than a nested structured clone, and the worker hands the same
    // string straight to wasm without re-serializing it.
    const result = await this.request({
      id: 0,
      kind: 'block-batch',
      handle,
      names: JSON.stringify(blockNames)
    })
    return this.parseJson<AcadRustBlockEntityGroup[]>(
      result,
      'block-entities-batch'
    )
  }

  async setEntityReactors(
    handle: number,
    entityHandle: string,
    reactorHandles: string[]
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'set-entity-reactors',
      handle,
      entityHandle,
      reactors: JSON.stringify(reactorHandles)
    })
  }

  async setExtensionDictionaryEntry(
    handle: number,
    ownerHandle: string,
    key: string,
    targetHandle: string,
    hardOwner = false
  ): Promise<string> {
    const result = await this.request({
      id: 0,
      kind: 'set-dictionary-entry',
      handle,
      ownerHandle,
      key,
      targetHandle,
      hardOwner
    })
    if (result.entityHandle == null) throw new Error('acad-rust worker returned no dictionary handle.')
    return result.entityHandle
  }

  async removeExtensionDictionaryEntry(
    handle: number,
    ownerHandle: string,
    key: string
  ): Promise<boolean> {
    const result = await this.request({
      id: 0,
      kind: 'remove-dictionary-entry',
      handle,
      ownerHandle,
      key
    })
    return result.removed === true
  }

  async objectXdata<T = unknown>(handle: number, objectHandle: string): Promise<T | null> {
    const result = await this.request({ id: 0, kind: 'object-xdata', handle, objectHandle })
    return this.parseJson<T | null>(result, 'object-xdata')
  }

  async setObjectXdataRecord(
    handle: number,
    objectHandle: string,
    record: unknown
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'set-object-xdata',
      handle,
      objectHandle,
      record: JSON.stringify(record)
    })
  }

  async removeObjectXdataRecord(
    handle: number,
    objectHandle: string,
    application: string
  ): Promise<boolean> {
    const result = await this.request({
      id: 0,
      kind: 'remove-object-xdata',
      handle,
      objectHandle,
      application
    })
    return result.removed === true
  }

  async ensureExtensionDictionary(handle: number, ownerHandle: string): Promise<string> {
    const result = await this.request({
      id: 0,
      kind: 'ensure-extension-dictionary',
      handle,
      ownerHandle
    })
    if (result.entityHandle == null) throw new Error('acad-rust worker returned no extension dictionary handle.')
    return result.entityHandle
  }

  async ensureXrecord(handle: number, ownerHandle: string, key: string): Promise<string> {
    const result = await this.request({
      id: 0,
      kind: 'ensure-xrecord',
      handle,
      ownerHandle,
      key
    })
    if (result.entityHandle == null) throw new Error('acad-rust worker returned no XRecord handle.')
    return result.entityHandle
  }

  async xrecord<T = unknown>(handle: number, ownerHandle: string, key: string): Promise<T | null> {
    const result = await this.request({ id: 0, kind: 'read-xrecord', handle, ownerHandle, key })
    return this.parseJson<T | null>(result, 'xrecord')
  }

  async setXrecordEntries(
    handle: number,
    ownerHandle: string,
    key: string,
    entries: unknown[]
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'set-xrecord-entries',
      handle,
      ownerHandle,
      key,
      entries: JSON.stringify(entries)
    })
  }

  async addBlockRecord(
    handle: number,
    name: string,
    basePoint: AcadRustPoint3d,
    description = ''
  ): Promise<AcadRustCreatedBlockRecord> {
    const result = await this.request({
      id: 0,
      kind: 'add-block-record',
      handle,
      name,
      basePoint: JSON.stringify(basePoint),
      description
    })
    return this.parseJson<AcadRustCreatedBlockRecord>(result, 'created-block-record')
  }

  async addEntityToBlock(
    handle: number,
    blockName: string,
    entityJson: string
  ): Promise<string> {
    const result = await this.request({
      id: 0,
      kind: 'add-block-entity',
      handle,
      block: blockName,
      entity: entityJson
    })
    if (result.entityHandle == null) throw new Error('acad-rust worker returned no block entity handle.')
    return result.entityHandle
  }

  async moveEntityToBlock(
    handle: number,
    entityHandle: string,
    blockName: string
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'move-entity-to-block',
      handle,
      entityHandle,
      block: blockName
    })
  }

  async removeEmptyBlockRecord(handle: number, blockName: string): Promise<boolean> {
    const result = await this.request({
      id: 0,
      kind: 'remove-empty-block',
      handle,
      block: blockName
    })
    return result.removed === true
  }

  async addEntity(handle: number, entityJson: string): Promise<string> {
    const result = await this.request({ id: 0, kind: 'add-entity', handle, entityJson })
    if (result.entityHandle == null) throw new Error('acad-rust worker returned no entity handle.')
    return result.entityHandle
  }

  async getEntity(handle: number, entityHandle: string): Promise<AcadRustEntity> {
    const result = await this.request({ id: 0, kind: 'get-entity', handle, entityHandle })
    return this.parseJson<AcadRustEntity>(result, 'entity')
  }

  async updateLwPolylineGeometry(handle: number, entityHandle: string, geometry: AcadRustLwPolylineGeometryPatch): Promise<void> {
    await this.request({ id: 0, kind: 'update-lwpolyline', handle, entityHandle, geometryJson: JSON.stringify({
      vertices: geometry.vertices?.map(v => ({ location: v.location, bulge: v.bulge, start_width: v.startWidth, end_width: v.endWidth, vertex_id: v.vertexId })),
      vertex_operations: geometry.vertexOperations?.map(op => op.op === 'remove'
        ? { op: op.op, index: op.index }
        : { op: op.op, index: op.index, vertex: { location: op.vertex.location, bulge: op.vertex.bulge, start_width: op.vertex.startWidth, end_width: op.vertex.endWidth, vertex_id: op.vertex.vertexId } }),
      is_closed: geometry.isClosed, plinegen: geometry.plinegen, constant_width: geometry.constantWidth,
      elevation: geometry.elevation, thickness: geometry.thickness, normal: geometry.normal
    }) })
  }

  async updateLineGeometry(
    handle: number,
    entityHandle: string,
    geometry: { start?: AcadRustPoint3d; end?: AcadRustPoint3d }
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-line',
      handle,
      entityHandle,
      geometryJson: JSON.stringify(geometry)
    })
  }

  async updateCircleArcGeometry(
    handle: number,
    entityHandle: string,
    geometry: AcadRustCircleArcGeometryPatch
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-circle-arc',
      handle,
      entityHandle,
      geometryJson: JSON.stringify({
        center: geometry.center,
        radius: geometry.radius,
        start_angle: geometry.startAngle,
        end_angle: geometry.endAngle
      })
    })
  }

  async updateEllipseGeometry(
    handle: number,
    entityHandle: string,
    geometry: AcadRustEllipseGeometryPatch
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-ellipse',
      handle,
      entityHandle,
      geometryJson: JSON.stringify({
        center: geometry.center,
        major_axis: geometry.majorAxis,
        minor_axis_ratio: geometry.minorAxisRatio,
        start_parameter: geometry.startParameter,
        end_parameter: geometry.endParameter,
        normal: geometry.normal
      })
    })
  }

  async updateDimensionBase(handle: number, entityHandle: string, patch: AcadRustDimensionBasePatch): Promise<void> {
    await this.request({ id: 0, kind: 'update-dimension', handle, entityHandle, patchJson: JSON.stringify({
      text_override: patch.textOverride, clear_text_override: patch.clearTextOverride, text_middle_point: patch.textMiddlePoint,
      text_rotation: patch.textRotation, horizontal_direction: patch.horizontalDirection,
      style_name: patch.styleName, line_spacing_factor: patch.lineSpacingFactor,
      line_spacing_style: patch.lineSpacingStyle, text_user_positioned: patch.textUserPositioned,
      flip_arrow1: patch.flipArrow1, flip_arrow2: patch.flipArrow2
    }) })
  }

  async getDimensionMeasurement(
    handle: number,
    entityHandle: string
  ): Promise<{ subtype: string; measurement: number; cachedMeasurement: number; blockName: string }> {
    const result = await this.request({ id: 0, kind: 'dimension-measurement', handle, entityHandle })
    return this.parseJson(result, 'dimension measurement')
  }

  async updateTextGeometry(handle: number, entityHandle: string, geometry: AcadRustTextGeometryPatch): Promise<void> {
    await this.request({ id: 0, kind: 'update-text', handle, entityHandle, geometryJson: JSON.stringify({
      value: geometry.value, insertion_point: geometry.insertionPoint,
      alignment_point: geometry.alignmentPoint, height: geometry.height, rotation: geometry.rotation,
      width_factor: geometry.widthFactor, oblique_angle: geometry.obliqueAngle, style: geometry.style,
      normal: geometry.normal, thickness: geometry.thickness, generation_flags: geometry.generationFlags
    }) })
  }

  async updateMTextGeometry(handle: number, entityHandle: string, geometry: AcadRustMTextGeometryPatch): Promise<void> {
    await this.request({ id: 0, kind: 'update-mtext', handle, entityHandle, geometryJson: JSON.stringify({
      value: geometry.value, insertion_point: geometry.insertionPoint, height: geometry.height,
      rectangle_width: geometry.rectangleWidth, rectangle_height: geometry.rectangleHeight,
      rotation: geometry.rotation, style: geometry.style, normal: geometry.normal,
      line_spacing_factor: geometry.lineSpacingFactor, background_fill_flags: geometry.backgroundFillFlags,
      background_scale: geometry.backgroundScale, background_transparency: geometry.backgroundTransparency
    }) })
  }

  async updateSplineGeometry(
    handle: number,
    entityHandle: string,
    geometry: AcadRustSplineGeometryPatch
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-spline',
      handle,
      entityHandle,
      geometryJson: JSON.stringify({
        degree: geometry.degree,
        flags: geometry.flags,
        knots: geometry.knots,
        control_points: geometry.controlPoints,
        weights: geometry.weights,
        fit_points: geometry.fitPoints,
        normal: geometry.normal,
        knot_tolerance: geometry.knotTolerance,
        control_tolerance: geometry.controlTolerance,
        fit_tolerance: geometry.fitTolerance,
        begin_tangent: geometry.beginTangent,
        end_tangent: geometry.endTangent,
        knot_parameterization: geometry.knotParameterization,
        cv_frame_visible: geometry.cvFrameVisible,
        dwg_flags1: geometry.dwgFlags1
      })
    })
  }

  async updateInsertAttributeValue(
    handle: number,
    insertHandle: string,
    tag: string,
    value: string
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-insert-attribute',
      handle,
      entityHandle: insertHandle,
      tag,
      value
    })
  }

  async updateAttdefDefaultValue(
    handle: number,
    attdefHandle: string,
    value: string
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-attdef-default',
      handle,
      entityHandle: attdefHandle,
      value
    })
  }

  async updateInsertPlacement(
    handle: number,
    entityHandle: string,
    patch: AcadRustInsertPlacementPatch
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-insert-placement',
      handle,
      entityHandle,
      patchJson: JSON.stringify(patch)
    })
  }

  async updateEntityCommon(
    handle: number,
    entityHandle: string,
    patch: AcadRustEntityCommonPatch
  ): Promise<void> {
    await this.request({
      id: 0,
      kind: 'update-entity-common',
      handle,
      entityHandle,
      patchJson: JSON.stringify(patch)
    })
  }

  async removeEntity(handle: number, entityHandle: string): Promise<boolean> {
    const result = await this.request({ id: 0, kind: 'remove-entity', handle, entityHandle })
    return result.removed === true
  }

  async save(handle: number): Promise<ArrayBuffer> {
    const result = await this.request({ id: 0, kind: 'save', handle })
    if (result.dwg == null) throw new Error('acad-rust worker returned no DWG bytes.')
    return result.dwg
  }

  async beginTransaction(handle: number): Promise<void> {
    await this.request({ id: 0, kind: 'begin-transaction', handle })
  }

  async commitTransaction(handle: number): Promise<void> {
    await this.request({ id: 0, kind: 'commit-transaction', handle })
  }

  async abortTransaction(handle: number): Promise<void> {
    await this.request({ id: 0, kind: 'abort-transaction', handle })
  }

  async close(handle: number): Promise<void> {
    await this.request({ id: 0, kind: 'close', handle })
  }

  dispose(): void {
    this.rejectAllPending(new Error('acad-rust section client disposed.'))
    this.resetWorker()
  }

  /** Parse a `{json}` worker response, tagging failures with the request label. */
  private parseJson<T>(result: AcadRustWorkerResult, label: string): T {
    if (result.json == null) {
      throw new Error(`acad-rust worker returned no JSON for ${label}.`)
    }
    return JSON.parse(result.json) as T
  }

  /** Post one request, await its id-matched response (or reject on timeout). */
  private request(
    partial: Record<string, unknown>,
    transfer?: Transferable[]
  ): Promise<AcadRustWorkerResult> {
    const worker = this.ensureWorker()
    const id = this.nextRequestId++
    const message = { ...partial, id }
    return new Promise<AcadRustWorkerResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        // Drop routing so a late reply is ignored; keep the worker (a slow
        // section is not fatal) — the next read reuses it.
        this.pending.delete(id)
        reject(
          new Error(
            `acad-rust section request '${String(partial.kind)}' timed out after ` +
              `${this.requestTimeout}ms.`
          )
        )
      }, this.requestTimeout)
      this.pending.set(id, { resolve, reject, timer })
      worker.postMessage(message, transfer)
    })
  }

  private ensureWorker(): WorkerLike {
    if (this.worker) {
      return this.worker
    }
    const WorkerCtor = (
      globalThis as unknown as { Worker?: new (url: string | URL, opts?: { name?: string }) => WorkerLike }
    ).Worker
    if (!WorkerCtor) {
      throw new Error(
        'AcadRustWorkerDocumentClient requires a Web Worker global; run it in a ' +
          'browser, or inject a fake AcadRustDocumentClient (see the parity tests).'
      )
    }
    const worker = new WorkerCtor(this.options.workerUrl, {
      name: 'acad-rust-parser'
    })
    worker.onmessage = (event: MessageEvent) =>
      this.handleMessage(event.data as AcadRustWorkerMessage)
    worker.onerror = (event: ErrorEvent) => this.handleError(event.message)
    this.worker = worker
    return worker
  }

  private handleMessage(message: AcadRustWorkerMessage): void {
    if (!message || typeof message !== 'object') {
      return
    }
    const entry = this.takePending(message.id)
    if (message.type === 'result') {
      if (!entry) {
        return // late reply for an already-timed-out request — ignore
      }
      entry.resolve(message)
    } else if (message.type === 'error') {
      const error = new Error(
        message.message || 'acad-rust section request failed.'
      )
      if (entry) {
        entry.reject(error)
      } else {
        // Unattributable error (missing/invalid id): reset so the next read
        // builds a fresh worker instead of trusting a possibly-broken one.
        this.rejectAllPending(error)
        this.resetWorker()
      }
    }
  }

  private handleError(message: string): void {
    this.rejectAllPending(
      new Error(message || 'acad-rust parser worker failed to start.')
    )
    this.resetWorker()
  }

  private takePending(id: number | undefined): PendingRequest | undefined {
    if (id == null) {
      return undefined
    }
    const entry = this.pending.get(id)
    if (entry) {
      clearTimeout(entry.timer)
      this.pending.delete(id)
    }
    return entry
  }

  private rejectAllPending(error: Error): void {
    for (const entry of this.pending.values()) {
      clearTimeout(entry.timer)
      entry.reject(error)
    }
    this.pending.clear()
  }

  private resetWorker(): void {
    const worker = this.worker
    this.worker = null
    if (worker) {
      worker.onmessage = null
      worker.onerror = null
      worker.terminate()
    }
  }
}
