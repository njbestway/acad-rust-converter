import {
  acadRustEntityKind,
  type AcadRustEntity,
  type AcadRustBlockRecord,
  type AcadRustBlockRecordTable,
  type AcadRustPoint3d
} from '../model/acadrust-model'
import {
  type AcadRustDocumentClient,
  type AcadRustEntityCommonPatch,
  type AcadRustLwPolylineGeometryPatch,
  type AcadRustCircleArcGeometryPatch,
  type AcadRustEllipseGeometryPatch,
  type AcadRustSplineGeometryPatch,
  type AcadRustTextGeometryPatch,
  type AcadRustMTextGeometryPatch,
  type AcadRustDimensionBasePatch,
  type AcadRustLwPolylineVertex,
  type AcadRustInsertPlacementPatch,
  type AcadRustDynamicBlockIndex,
  type AcadRustInsertSequence,
  type AcadRustRoundtripAudit,
  type AcadRustOpaqueObjectInfo
} from '../AcadRustSectionClient'
import {
  normalizeDwgHandle,
  ObjectIdentityMap,
  type DwgHandleLike,
  type ObjectIdentityEntry
} from './ObjectIdentityMap'

export type AcDbHandle = string
export type AcDbEntityPatch = AcadRustLwPolylineGeometryPatch | AcadRustEntityCommonPatch | { start?: AcadRustPoint3d; end?: AcadRustPoint3d } | AcadRustCircleArcGeometryPatch | AcadRustEllipseGeometryPatch | AcadRustSplineGeometryPatch | AcadRustTextGeometryPatch | AcadRustMTextGeometryPatch | AcadRustDimensionBasePatch | AcadRustInsertPlacementPatch | { attributeTag: string; attributeValue: string } | { defaultValue: string } | { ownerBlockName: string }

export type AcDbDatabaseEvent =
  | { type: 'objectAppended'; handle: AcDbHandle; entity: AcDbEntity }
  | { type: 'objectModified'; handle: AcDbHandle; entity: AcDbEntity; patch: AcDbEntityPatch }
  | { type: 'objectErased'; handle: AcDbHandle }
  | { type: 'transactionStarted' }
  | { type: 'transactionCommitted' }
  | { type: 'transactionAborted' }
  | { type: 'databaseSaved' }

export type AcDbDatabaseListener = (event: AcDbDatabaseEvent) => void

/**
 * AcDbDatabase façade over one resident acadrust CadDocument.
 *
 * This class owns no CAD entity storage. Every read/write is delegated to the
 * worker-backed document client and the Rust CadDocument addressed by handle.
 */
export class AcDbDatabase {
  private readonly listeners = new Set<AcDbDatabaseListener>()
  // Observer failures must never make a completed Rust mutation look failed.
  // Keep a bounded diagnostic queue so callers can inspect callback failures.
  private readonly listenerErrors: unknown[] = []
  /** Cross-reference only; the resident Rust CadDocument remains authoritative. */
  readonly identities = new ObjectIdentityMap()
  private readonly identitySnapshots: ObjectIdentityMap[] = []
  private closed = false
  private transactionControlInFlight = false

  private constructor(
    private readonly client: AcadRustDocumentClient,
    readonly documentHandle: number
  ) {}

  static async open(
    client: AcadRustDocumentClient,
    data: ArrayBuffer
  ): Promise<AcDbDatabase> {
    const handle = await client.open(data)
    return new AcDbDatabase(client, handle)
  }

  on(listener: AcDbDatabaseListener): () => void {
    this.assertOpen()
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  off(listener: AcDbDatabaseListener): void {
    this.listeners.delete(listener)
  }

  get blockTable(): AcDbBlockTable {
    this.assertOpen()
    return new AcDbBlockTable(this)
  }

  getEntity(handle: DwgHandleLike): AcDbEntity {
    this.assertOpen()
    return new AcDbEntity(this, normalizeDwgHandle(handle))
  }

  /** Bind a MLightCAD ObjectId to the exact DWG handle represented by a proxy. */
  bindObjectIdentity(objectId: string, handle: DwgHandleLike): ObjectIdentityEntry {
    this.assertOpen()
    return this.identities.bind(objectId, handle)
  }

  /** Atomically import identity pairs produced by a converter mapping pass. */
  bindObjectIdentities(entries: readonly ObjectIdentityEntry[]): void {
    this.assertOpen()
    this.identities.bindMany(entries)
  }

  getHandleForObjectId(objectId: string): string | undefined {
    this.assertOpen()
    return this.identities.getHandle(objectId)
  }

  getObjectIdForHandle(handle: DwgHandleLike): string | undefined {
    this.assertOpen()
    return this.identities.getObjectId(handle)
  }

  async addEntity(entity: AcadRustEntity): Promise<AcDbEntity> {
    this.assertOpen()
    const handle = await this.client.addEntity(
      this.documentHandle,
      JSON.stringify(entity)
    )
    const proxy = new AcDbEntity(this, normalizeDwgHandle(handle))
    this.emit({ type: 'objectAppended', handle: proxy.handle, entity: proxy })
    return proxy
  }

  async eraseEntity(handle: DwgHandleLike): Promise<boolean> {
    this.assertOpen()
    const canonical = normalizeDwgHandle(handle)
    const removed = await this.client.removeEntity(this.documentHandle, canonical)
    if (removed) {
      this.identities.unbindByHandle(canonical)
      this.emit({ type: 'objectErased', handle: canonical })
    }
    return removed
  }

  async beginTransaction(): Promise<void> {
    this.assertOpen()
    this.enterTransactionControl()
    try {
      await this.client.beginTransaction(this.documentHandle)
      this.identitySnapshots.push(this.identities.clone())
      this.emit({ type: 'transactionStarted' })
    } finally {
      this.leaveTransactionControl()
    }
  }

  async commitTransaction(): Promise<void> {
    this.assertOpen()
    if (this.identitySnapshots.length === 0) {
      throw new Error('No active transaction for this database.')
    }
    this.enterTransactionControl()
    try {
      await this.client.commitTransaction(this.documentHandle)
      this.identitySnapshots.pop()
      this.emit({ type: 'transactionCommitted' })
    } finally {
      this.leaveTransactionControl()
    }
  }

  async abortTransaction(): Promise<void> {
    this.assertOpen()
    const snapshot = this.identitySnapshots[this.identitySnapshots.length - 1]
    if (snapshot == null) {
      throw new Error('No active transaction for this database.')
    }
    this.enterTransactionControl()
    try {
      await this.client.abortTransaction(this.documentHandle)
      this.identitySnapshots.pop()
      this.identities.restore(snapshot)
      this.emit({ type: 'transactionAborted' })
    } finally {
      this.leaveTransactionControl()
    }
  }

  async transaction<T>(action: () => Promise<T>): Promise<T> {
    await this.beginTransaction()
    try {
      const result = await action()
      await this.commitTransaction()
      return result
    } catch (error) {
      // A failed commit may leave the Rust transaction active; a failed action
      // definitely does. Attempt rollback, but never replace the original
      // failure with a secondary rollback error.
      try {
        if (this.identitySnapshots.length > 0) await this.abortTransaction()
      } catch (rollbackError) {
        if (error instanceof Error) {
          Object.defineProperty(error, 'rollbackError', {
            value: rollbackError,
            configurable: true
          })
        }
      }
      throw error
    }
  }

  get transactionDepth(): number {
    return this.identitySnapshots.length
  }

  async save(): Promise<ArrayBuffer> {
    this.assertOpen()
    const bytes = await this.client.save(this.documentHandle)
    this.emit({ type: 'databaseSaved' })
    return bytes
  }

  /** List unsupported/custom OBJECTS by exact handle and raw-payload fingerprint. */
  async getOpaqueObjectIndex(): Promise<AcadRustOpaqueObjectInfo[]> {
    this.assertOpen()
    return this.client.opaqueObjectIndex(this.documentHandle)
  }

  /** Save/reopen the resident DWG in Rust and compare key identity/relationship fingerprints. */
  async auditRoundtrip(): Promise<AcadRustRoundtripAudit> {
    this.assertOpen()
    return this.client.roundtripAudit(this.documentHandle)
  }

  /** Read dynamic-block parameters, visibility states and anonymous representation links. */
  async getDynamicBlockIndex(): Promise<AcadRustDynamicBlockIndex> {
    this.assertOpen()
    return this.client.dynamicBlockIndex(this.documentHandle)
  }

  /**
   * Resolve all dynamic-block metadata associated with one exact DWG handle.
   * The returned index is a detached read model; mutations still go through
   * the resident Rust document and are not implied by this lookup.
   */
  async getDynamicBlockInfo(handle: DwgHandleLike): Promise<{
    objects: AcadRustDynamicBlockIndex['dynamicObjects']
    visibilityParameters: AcadRustDynamicBlockIndex['visibilityParameters']
    representations: AcadRustDynamicBlockIndex['representations']
  }> {
    this.assertOpen()
    const canonical = normalizeDwgHandle(handle)
    const index = await this.client.dynamicBlockIndex(this.documentHandle)
    const objects = index.dynamicObjects.filter(item =>
      normalizeDwgHandle(item.handle) === canonical ||
      normalizeDwgHandle(item.owner) === canonical
    )
    const objectHandles = new Set(objects.map(item => normalizeDwgHandle(item.handle)))
    const visibilityParameters = index.visibilityParameters.filter(item =>
      normalizeDwgHandle(item.handle) === canonical ||
      normalizeDwgHandle(item.owner) === canonical ||
      objects.some(object => normalizeDwgHandle(object.owner) === normalizeDwgHandle(item.owner))
    )
    const representations = index.representations.filter(item =>
      normalizeDwgHandle(item.representationHandle) === canonical ||
      normalizeDwgHandle(item.dynamicBlockHandle) === canonical ||
      objectHandles.has(normalizeDwgHandle(item.dynamicBlockHandle))
    )
    return { objects, visibilityParameters, representations }
  }

  get isClosed(): boolean {
    return this.closed
  }

  /** Inspect an INSERT's embedded ATTRIB and SEQEND sequence integrity. */
  async getInsertSequence(insertHandle: DwgHandleLike): Promise<AcadRustInsertSequence> {
    this.assertOpen()
    return this.client.insertSequence(
      this.documentHandle,
      normalizeDwgHandle(insertHandle)
    )
  }

  async close(): Promise<void> {
    if (this.closed) return
    if (this.transactionControlInFlight) {
      throw new Error('Cannot close AcDbDatabase while a transaction operation is in flight.')
    }
    if (this.identitySnapshots.length > 0) {
      throw new Error('Cannot close AcDbDatabase with active transactions. Commit or abort them first.')
    }
    await this.client.close(this.documentHandle)
    this.closed = true
    this.identitySnapshots.length = 0
    this.identities.clear()
    this.listeners.clear()
    this.listenerErrors.length = 0
  }

  /** @internal LWPOLYLINE geometry proxy bridge. */
  async patchLwPolylineGeometry(handle: AcDbHandle, geometry: AcadRustLwPolylineGeometryPatch): Promise<void> {
    this.assertOpen()
    await this.client.updateLwPolylineGeometry(this.documentHandle, handle, geometry)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: geometry })
  }

  /** @internal LINE geometry proxy bridge. */
  async patchLineGeometry(
    handle: AcDbHandle,
    geometry: { start?: AcadRustPoint3d; end?: AcadRustPoint3d }
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateLineGeometry(this.documentHandle, handle, geometry)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: geometry })
  }

  async patchInsertAttributeValue(
    handle: AcDbHandle,
    tag: string,
    value: string
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateInsertAttributeValue(this.documentHandle, handle, tag, value)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: { attributeTag: tag, attributeValue: value } })
  }

  async patchAttdefDefaultValue(handle: AcDbHandle, value: string): Promise<void> {
    this.assertOpen()
    await this.client.updateAttdefDefaultValue(this.documentHandle, handle, value)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: { defaultValue: value } })
  }

  async patchInsertPlacement(
    handle: AcDbHandle,
    patch: AcadRustInsertPlacementPatch
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateInsertPlacement(this.documentHandle, handle, patch)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch })
  }

  /** @internal CIRCLE/ARC geometry proxy bridge. */
  async patchCircleArcGeometry(
    handle: AcDbHandle,
    geometry: AcadRustCircleArcGeometryPatch
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateCircleArcGeometry(this.documentHandle, handle, geometry)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: geometry })
  }

  /** @internal ELLIPSE geometry proxy bridge. */
  async patchEllipseGeometry(
    handle: AcDbHandle,
    geometry: AcadRustEllipseGeometryPatch
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateEllipseGeometry(this.documentHandle, handle, geometry)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: geometry })
  }

  /** @internal Read subtype-derived DIMENSION measurement from Rust geometry. */
  async readDimensionMeasurement(
    handle: AcDbHandle
  ): Promise<{ subtype: string; measurement: number; cachedMeasurement: number; blockName: string }> {
    this.assertOpen()
    return this.client.getDimensionMeasurement(this.documentHandle, handle)
  }

  /** @internal DIMENSION base-property proxy bridge. */
  async patchDimensionBase(handle: AcDbHandle, patch: AcadRustDimensionBasePatch): Promise<void> {
    this.assertOpen()
    await this.client.updateDimensionBase(this.documentHandle, handle, patch)
    this.emit({ type: 'objectModified', handle, entity: new AcDbEntity(this, handle), patch })
  }

  /** @internal TEXT and MTEXT proxy bridges. */
  async patchTextGeometry(handle: AcDbHandle, geometry: AcadRustTextGeometryPatch): Promise<void> {
    this.assertOpen()
    await this.client.updateTextGeometry(this.documentHandle, handle, geometry)
    this.emit({ type: 'objectModified', handle, entity: new AcDbEntity(this, handle), patch: geometry })
  }

  async patchMTextGeometry(handle: AcDbHandle, geometry: AcadRustMTextGeometryPatch): Promise<void> {
    this.assertOpen()
    await this.client.updateMTextGeometry(this.documentHandle, handle, geometry)
    this.emit({ type: 'objectModified', handle, entity: new AcDbEntity(this, handle), patch: geometry })
  }

  /** @internal SPLINE geometry proxy bridge. */
  async patchSplineGeometry(
    handle: AcDbHandle,
    geometry: AcadRustSplineGeometryPatch
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateSplineGeometry(this.documentHandle, handle, geometry)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch: geometry })
  }

  /** @internal Block-table proxy bridge. */
  async readBlockRecords(): Promise<AcadRustBlockRecordTable> {
    this.assertOpen()
    return this.client.section<AcadRustBlockRecordTable>(
      this.documentHandle,
      'tables-blockrecord'
    )
  }

  /** @internal Block-table mutation bridge. */
  async createBlockRecord(
    name: string,
    basePoint: AcadRustPoint3d,
    description = ''
  ): Promise<void> {
    this.assertOpen()
    await this.client.addBlockRecord(this.documentHandle, name, basePoint, description)
  }

  /** @internal Append an entity to a resident block definition. */
  async moveEntityToBlock(handle: AcDbHandle, blockName: string): Promise<void> {
    this.assertOpen()
    await this.client.moveEntityToBlock(this.documentHandle, handle, blockName)
    const entity = new AcDbEntity(this, handle)
    this.emit({
      type: 'objectModified',
      handle,
      entity,
      patch: { ownerBlockName: blockName }
    })
  }

  async appendBlockEntity(name: string, entity: AcadRustEntity): Promise<AcDbEntity> {
    this.assertOpen()
    const handle = await this.client.addEntityToBlock(
      this.documentHandle,
      name,
      JSON.stringify(entity)
    )
    const proxy = new AcDbEntity(this, normalizeDwgHandle(handle))
    this.emit({ type: 'objectAppended', handle: proxy.handle, entity: proxy })
    return proxy
  }

  /** @internal Only empty, unreferenced block records may be removed. */
  async removeEmptyBlockRecord(name: string): Promise<boolean> {
    this.assertOpen()
    return this.client.removeEmptyBlockRecord(this.documentHandle, name)
  }

  /** @internal Block-record proxy bridge. */
  async readBlockEntityHandles(name: string): Promise<string[]> {
    this.assertOpen()
    return this.client.blockEntityHandles(this.documentHandle, name)
  }

  async setEntityReactors(handle: AcDbHandle, reactorHandles: AcDbHandle[]): Promise<void> {
    this.assertOpen()
    await this.client.setEntityReactors(this.documentHandle, handle, reactorHandles)
  }

  async setExtensionDictionaryEntry(
    ownerHandle: AcDbHandle,
    key: string,
    targetHandle: AcDbHandle,
    hardOwner = false
  ): Promise<AcDbHandle> {
    this.assertOpen()
    return normalizeDwgHandle(
      await this.client.setExtensionDictionaryEntry(
        this.documentHandle, ownerHandle, key, targetHandle, hardOwner
      )
    )
  }

  async removeExtensionDictionaryEntry(ownerHandle: AcDbHandle, key: string): Promise<boolean> {
    this.assertOpen()
    return this.client.removeExtensionDictionaryEntry(this.documentHandle, ownerHandle, key)
  }

  async readObjectXdata<T = unknown>(handle: AcDbHandle): Promise<T | null> {
    this.assertOpen()
    return this.client.objectXdata<T>(this.documentHandle, handle)
  }

  async setObjectXdataRecord(
    handle: AcDbHandle,
    record: unknown
  ): Promise<void> {
    this.assertOpen()
    await this.client.setObjectXdataRecord(this.documentHandle, handle, record)
  }

  async removeObjectXdataRecord(handle: AcDbHandle, application: string): Promise<boolean> {
    this.assertOpen()
    return this.client.removeObjectXdataRecord(this.documentHandle, handle, application)
  }

  async ensureExtensionDictionary(handle: AcDbHandle): Promise<AcDbHandle> {
    this.assertOpen()
    return normalizeDwgHandle(
      await this.client.ensureExtensionDictionary(this.documentHandle, handle)
    )
  }

  async ensureXrecord(handle: AcDbHandle, key: string): Promise<AcDbHandle> {
    this.assertOpen()
    return normalizeDwgHandle(await this.client.ensureXrecord(this.documentHandle, handle, key))
  }

  async readXrecord<T = unknown>(handle: AcDbHandle, key: string): Promise<T | null> {
    this.assertOpen()
    return this.client.xrecord<T>(this.documentHandle, handle, key)
  }

  async setXrecordEntries(handle: AcDbHandle, key: string, entries: unknown[]): Promise<void> {
    this.assertOpen()
    await this.client.setXrecordEntries(this.documentHandle, handle, key, entries)
  }

  /** @internal Entity proxy bridge. */
  async readEntity(handle: AcDbHandle): Promise<AcadRustEntity> {
    this.assertOpen()
    return this.client.getEntity(this.documentHandle, handle)
  }

  /** @internal Entity proxy bridge. */
  async patchEntity(
    handle: AcDbHandle,
    patch: AcadRustEntityCommonPatch
  ): Promise<void> {
    this.assertOpen()
    await this.client.updateEntityCommon(this.documentHandle, handle, patch)
    const entity = new AcDbEntity(this, handle)
    this.emit({ type: 'objectModified', handle, entity, patch })
  }

  /** Return and clear callback failures captured during event delivery. */
  takeListenerErrors(): unknown[] {
    return this.listenerErrors.splice(0)
  }

  private emit(event: AcDbDatabaseEvent): void {
    // Snapshot listeners so subscribe/unsubscribe during delivery affects only
    // the next event. A callback runs after the resident Rust operation has
    // already committed; allowing it to throw would falsely reject that
    // operation and could trigger an unsafe retry by the caller.
    for (const listener of Array.from(this.listeners)) {
      try {
        listener(event)
      } catch (error) {
        this.listenerErrors.push(error)
        if (this.listenerErrors.length > 100) this.listenerErrors.shift()
      }
    }
  }

  private enterTransactionControl(): void {
    if (this.transactionControlInFlight) {
      throw new Error('A transaction operation is already in flight for this database.')
    }
    this.transactionControlInFlight = true
  }

  private leaveTransactionControl(): void {
    this.transactionControlInFlight = false
  }

  private assertOpen(): void {
    if (this.closed) throw new Error('AcDbDatabase is closed.')
  }
}

/**
 * Handle-stable AcDbEntity proxy. It fetches the authoritative entity from the
 * resident CadDocument on demand and never stores a second mutable entity copy.
 */
export class AcDbEntity {
  constructor(
    private readonly database: AcDbDatabase,
    readonly handle: AcDbHandle
  ) {}

  async getData(): Promise<AcadRustEntity> {
    return this.database.readEntity(this.handle)
  }

  async getClassName(): Promise<string> {
    const data = await this.getData()
    return acadRustEntityKind(data) ?? 'Unknown'
  }

  /**
   * Return the variant payload without duplicating it into a local entity store.
   * The returned object is a snapshot from the authoritative Rust document.
   */
  async getGeometryData<T extends Record<string, unknown> = Record<string, unknown>>(): Promise<T> {
    const data = await this.getData()
    const kind = acadRustEntityKind(data)
    if (kind == null) throw new Error(`Entity ${this.handle} has an invalid EntityType payload.`)
    const payload = data[kind]
    if (payload == null || typeof payload !== 'object' || Array.isArray(payload)) {
      throw new Error(`Entity ${this.handle} has an invalid ${kind} payload.`)
    }
    const { common: _common, ...geometry } = payload as Record<string, unknown>
    return geometry as T
  }

  async getPolylineVertices(): Promise<unknown[]> {
    const kind = await this.getClassName()
    if (!['LwPolyline', 'Polyline', 'Polyline2D', 'Polyline3D'].includes(kind)) {
      throw new Error(`Entity ${this.handle} is not a polyline (${kind}).`)
    }
    const geometry = await this.getGeometryData()
    if (!Array.isArray(geometry.vertices)) {
      throw new Error(`Polyline ${this.handle} has no vertices array.`)
    }
    return geometry.vertices
  }

  async getEllipseParameters(): Promise<{
    center: unknown
    major_axis: unknown
    minor_axis_ratio: number
    start_parameter: number
    end_parameter: number
    normal: unknown
  }> {
    if (await this.getClassName() !== 'Ellipse') {
      throw new Error(`Entity ${this.handle} is not an ELLIPSE.`)
    }
    const geometry = await this.getGeometryData()
    return {
      center: geometry.center,
      major_axis: geometry.major_axis,
      minor_axis_ratio: geometry.minor_axis_ratio as number,
      start_parameter: geometry.start_parameter as number,
      end_parameter: geometry.end_parameter as number,
      normal: geometry.normal
    }
  }

  /** Read INSERT/MINSERT payload and its ATTRIB/SEQEND chain by exact handle. */
  async getBlockReferenceData(): Promise<{
    kind: string
    payload: Record<string, unknown>
    sequence: AcadRustInsertSequence
  }> {
    const data = await this.getData()
    const kind = acadRustEntityKind(data)
    if (kind !== 'Insert' && kind !== 'MInsert') {
      throw new Error(`Entity ${this.handle} is not an INSERT/MINSERT (${kind ?? 'Unknown'}).`)
    }
    const raw = data[kind]
    if (raw == null || typeof raw !== 'object' || Array.isArray(raw)) {
      throw new Error(`INSERT ${this.handle} has an invalid payload.`)
    }
    const sequence = await this.database.getInsertSequence(this.handle)
    return { kind, payload: raw as Record<string, unknown>, sequence }
  }

  /** Resolve dynamic-block parameters/visibility links related to this handle. */
  async getDynamicBlockInfo(): ReturnType<AcDbDatabase['getDynamicBlockInfo']> {
    return this.database.getDynamicBlockInfo(this.handle)
  }

  async getLineEndpoints(): Promise<{ start: AcadRustPoint3d; end: AcadRustPoint3d }> {
    const data = await this.getData()
    if (acadRustEntityKind(data) !== 'Line') {
      throw new Error(`Entity ${this.handle} is not a LINE.`)
    }
    const line = data.Line as Record<string, unknown>
    return {
      start: line.start as AcadRustPoint3d,
      end: line.end as AcadRustPoint3d
    }
  }

  async setPolylineGeometry(geometry: AcadRustLwPolylineGeometryPatch): Promise<void> {
    await this.database.patchLwPolylineGeometry(this.handle, geometry)
  }

  async insertPolylineVertex(index: number, vertex: AcadRustLwPolylineVertex): Promise<void> {
    await this.setPolylineGeometry({ vertexOperations: [{ op: 'insert', index, vertex }] })
  }

  async updatePolylineVertex(index: number, vertex: import('../AcadRustSectionClient').AcadRustLwPolylineVertex): Promise<void> {
    await this.setPolylineGeometry({ vertexOperations: [{ op: 'update', index, vertex }] })
  }

  async removePolylineVertex(index: number): Promise<void> {
    await this.setPolylineGeometry({ vertexOperations: [{ op: 'remove', index }] })
  }

  async setLineEndpoints(start: AcadRustPoint3d, end: AcadRustPoint3d): Promise<void> {
    await this.database.patchLineGeometry(this.handle, { start, end })
  }

  /** Return the current measurement computed from native subtype geometry. */
  async getDimensionMeasurement(): Promise<{
    subtype: string
    measurement: number
    cachedMeasurement: number
    blockName: string
  }> {
    if (await this.getClassName() !== 'Dimension') {
      throw new Error(`Entity ${this.handle} is not DIMENSION.`)
    }
    return this.database.readDimensionMeasurement(this.handle)
  }

  async setDimensionProperties(patch: AcadRustDimensionBasePatch): Promise<void> {
    if (await this.getClassName() !== 'Dimension') {
      throw new Error(`Entity ${this.handle} is not DIMENSION.`)
    }
    await this.database.patchDimensionBase(this.handle, patch)
  }

  async setTextGeometry(geometry: AcadRustTextGeometryPatch): Promise<void> {
    if (await this.getClassName() !== 'Text') throw new Error(`Entity ${this.handle} is not TEXT.`)
    await this.database.patchTextGeometry(this.handle, geometry)
  }

  async setMTextGeometry(geometry: AcadRustMTextGeometryPatch): Promise<void> {
    if (await this.getClassName() !== 'MText') throw new Error(`Entity ${this.handle} is not MTEXT.`)
    await this.database.patchMTextGeometry(this.handle, geometry)
  }

  async setSplineGeometry(geometry: AcadRustSplineGeometryPatch): Promise<void> {
    if (await this.getClassName() !== 'Spline') {
      throw new Error(`Entity ${this.handle} is not a SPLINE.`)
    }
    await this.database.patchSplineGeometry(this.handle, geometry)
  }

  async setEllipseGeometry(geometry: AcadRustEllipseGeometryPatch): Promise<void> {
    if (await this.getClassName() !== 'Ellipse') {
      throw new Error(`Entity ${this.handle} is not an ELLIPSE.`)
    }
    await this.database.patchEllipseGeometry(this.handle, geometry)
  }

  async setCircleGeometry(geometry: { center?: AcadRustPoint3d; radius?: number }): Promise<void> {
    await this.database.patchCircleArcGeometry(this.handle, geometry)
  }

  async setAttributeValue(tag: string, value: string): Promise<void> {
    await this.database.patchInsertAttributeValue(this.handle, tag, value)
  }

  async setAttributeDefaultValue(value: string): Promise<void> {
    await this.database.patchAttdefDefaultValue(this.handle, value)
  }

  async setInsertPlacement(patch: AcadRustInsertPlacementPatch): Promise<void> {
    await this.database.patchInsertPlacement(this.handle, patch)
  }

  async setArcGeometry(geometry: {
    center?: AcadRustPoint3d
    radius?: number
    startAngle?: number
    endAngle?: number
  }): Promise<void> {
    await this.database.patchCircleArcGeometry(this.handle, geometry)
  }

  /** Read the complete shared AcDbEntity common-property block. */
  async getCommonData(): Promise<Record<string, unknown>> {
    const entity = await this.getData()
    const kind = acadRustEntityKind(entity)
    if (!kind) throw new Error('Invalid acadrust EntityType payload.')
    const payload = entity[kind]
    if (payload == null || typeof payload !== 'object') {
      throw new Error(`Invalid ${kind} entity payload.`)
    }
    const common = (payload as Record<string, unknown>).common
    if (common == null || typeof common !== 'object' || Array.isArray(common)) {
      throw new Error(`Entity ${kind} has no common property block.`)
    }
    return common as Record<string, unknown>
  }

  async getColor(): Promise<unknown> {
    return this.readCommon<unknown>('color')
  }

  async getLineWeight(): Promise<unknown> {
    return this.readCommon<unknown>('line_weight')
  }

  async getTransparency(): Promise<unknown> {
    return this.readCommon<unknown>('transparency')
  }

  async getReactors(): Promise<unknown[]> {
    const reactors = await this.readCommon<unknown>('reactors')
    return Array.isArray(reactors) ? reactors : []
  }

  async getExtendedData(): Promise<unknown[]> {
    const data = await this.readCommon<unknown>('extended_data')
    if (data == null || typeof data !== 'object') return []
    const records = (data as Record<string, unknown>).records
    return Array.isArray(records) ? records : []
  }

  async getLayer(): Promise<string> {
    return this.readCommon<string>('layer')
  }

  async moveToBlock(blockName: string): Promise<void> {
    await this.database.moveEntityToBlock(this.handle, blockName)
  }

  async setLayer(layer: string): Promise<void> {
    await this.database.patchEntity(this.handle, { layer })
  }

  async getLinetype(): Promise<string> {
    return this.readCommon<string>('linetype')
  }

  async setLinetype(linetype: string): Promise<void> {
    await this.database.patchEntity(this.handle, { linetype })
  }

  async getLinetypeScale(): Promise<number> {
    return this.readCommon<number>('linetype_scale')
  }

  async setLinetypeScale(linetypeScale: number): Promise<void> {
    await this.database.patchEntity(this.handle, { linetypeScale })
  }

  async isInvisible(): Promise<boolean> {
    return this.readCommon<boolean>('invisible')
  }

  async setInvisible(invisible: boolean): Promise<void> {
    await this.database.patchEntity(this.handle, { invisible })
  }

  async erase(): Promise<boolean> {
    return this.database.eraseEntity(this.handle)
  }

  async setReactors(reactorHandles: AcDbHandle[]): Promise<void> {
    await this.database.setEntityReactors(this.handle, reactorHandles)
  }

  async setExtensionDictionaryEntry(
    key: string,
    targetHandle: AcDbHandle,
    hardOwner = false
  ): Promise<AcDbHandle> {
    return this.database.setExtensionDictionaryEntry(this.handle, key, targetHandle, hardOwner)
  }

  async removeExtensionDictionaryEntry(key: string): Promise<boolean> {
    return this.database.removeExtensionDictionaryEntry(this.handle, key)
  }

  async getXdata<T = unknown>(): Promise<T | null> {
    return this.database.readObjectXdata<T>(this.handle)
  }

  async setXdataRecord(record: unknown): Promise<void> {
    await this.database.setObjectXdataRecord(this.handle, record)
  }

  async removeXdataRecord(application: string): Promise<boolean> {
    return this.database.removeObjectXdataRecord(this.handle, application)
  }

  async ensureExtensionDictionary(): Promise<AcDbHandle> {
    return this.database.ensureExtensionDictionary(this.handle)
  }

  async ensureXrecord(key: string): Promise<AcDbHandle> {
    return this.database.ensureXrecord(this.handle, key)
  }

  async getXrecord<T = unknown>(key: string): Promise<T | null> {
    return this.database.readXrecord<T>(this.handle, key)
  }

  async setXrecordEntries(key: string, entries: unknown[]): Promise<void> {
    await this.database.setXrecordEntries(this.handle, key, entries)
  }

  private async readCommon<T>(property: string): Promise<T> {
    const entity = await this.getData()
    const kind = acadRustEntityKind(entity)
    if (!kind) throw new Error('Invalid acadrust EntityType payload.')
    const payload = entity[kind]
    if (payload == null || typeof payload !== 'object') {
      throw new Error(`Invalid ${kind} entity payload.`)
    }
    const common = (payload as Record<string, unknown>).common
    if (common == null || typeof common !== 'object') {
      throw new Error(`Entity ${kind} has no common property block.`)
    }
    return (common as Record<string, unknown>)[property] as T
  }
}


/** AcDbBlockTable proxy over acadrust's resident BLOCK-RECORD table. */
export class AcDbBlockTable {
  constructor(private readonly database: AcDbDatabase) {}

  async getAt(name: string): Promise<AcDbBlockTableRecord | undefined> {
    const table = await this.database.readBlockRecords()
    const actualName = Object.keys(table.entries).find(
      key => key.toUpperCase() === name.toUpperCase()
    )
    if (actualName == null) return undefined
    return new AcDbBlockTableRecord(this.database, table.entries[actualName])
  }

  async has(name: string): Promise<boolean> {
    return (await this.getAt(name)) != null
  }

  /** Create a block definition with valid BLOCK/ENDBLK structural records. */
  async add(
    name: string,
    basePoint: AcadRustPoint3d = { x: 0, y: 0, z: 0 },
    description = ''
  ): Promise<AcDbBlockTableRecord> {
    await this.database.createBlockRecord(name, basePoint, description)
    const record = await this.getAt(name)
    if (record == null) throw new Error(`Created block '${name}' could not be read back.`)
    return record
  }

  /** Remove a block only when it has no child entities or INSERT references. */
  async remove(name: string): Promise<boolean> {
    return this.database.removeEmptyBlockRecord(name)
  }

  async getAll(): Promise<AcDbBlockTableRecord[]> {
    const table = await this.database.readBlockRecords()
    return Object.values(table.entries).map(
      record => new AcDbBlockTableRecord(this.database, record)
    )
  }

  async getNames(): Promise<string[]> {
    const table = await this.database.readBlockRecords()
    return Object.keys(table.entries)
  }
}

/** Handle-safe proxy for one BLOCK-RECORD; entity handles are fetched as hex. */
export class AcDbBlockTableRecord {
  constructor(
    private readonly database: AcDbDatabase,
    private readonly record: AcadRustBlockRecord
  ) {}

  get name(): string { return this.record.name }
  get description(): string { return this.record.description }
  get xrefPath(): string { return this.record.xref_path }
  get explodable(): boolean { return this.record.explodable }
  get scaleUniformly(): boolean { return this.record.scale_uniformly }
  get units(): number { return this.record.units }
  get basePoint(): AcadRustPoint3d { return this.record.base_point }
  get flags(): AcadRustBlockRecord['flags'] { return this.record.flags }

  async getEntityHandles(): Promise<string[]> {
    return this.database.readBlockEntityHandles(this.record.name)
  }

  async getEntities(): Promise<AcDbEntity[]> {
    const handles = await this.getEntityHandles()
    return handles.map(handle => this.database.getEntity(handle))
  }

  async getEntityCount(): Promise<number> {
    return (await this.getEntityHandles()).length
  }

  /** Move an existing entity into this block and update its Owner link. */
  async moveEntityHere(entityHandle: DwgHandleLike): Promise<void> {
    await this.database.moveEntityToBlock(normalizeDwgHandle(entityHandle), this.record.name)
  }

  /** Append an entity to this block definition; Rust assigns its Handle. */
  async appendEntity(entity: AcadRustEntity): Promise<AcDbEntity> {
    return this.database.appendBlockEntity(this.record.name, entity)
  }
}

