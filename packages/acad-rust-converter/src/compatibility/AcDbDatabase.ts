import {
  acadRustEntityKind,
  type AcadRustEntity,
  type AcadRustBlockRecord,
  type AcadRustBlockRecordTable,
  type AcadRustPoint3d
} from '../model/acadrust-model'
import {
  type AcadRustDocumentClient,
  type AcadRustEntityCommonPatch
} from '../AcadRustSectionClient'
import {
  normalizeDwgHandle,
  type DwgHandleLike
} from './ObjectIdentityMap'

export type AcDbHandle = string
export type AcDbEntityPatch = AcadRustEntityCommonPatch | { start?: AcadRustPoint3d; end?: AcadRustPoint3d } | AcadRustCircleArcGeometryPatch

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
  private closed = false

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
    await this.client.beginTransaction(this.documentHandle)
    this.emit({ type: 'transactionStarted' })
  }

  async commitTransaction(): Promise<void> {
    this.assertOpen()
    await this.client.commitTransaction(this.documentHandle)
    this.emit({ type: 'transactionCommitted' })
  }

  async abortTransaction(): Promise<void> {
    this.assertOpen()
    await this.client.abortTransaction(this.documentHandle)
    this.emit({ type: 'transactionAborted' })
  }

  async transaction<T>(action: () => Promise<T>): Promise<T> {
    await this.beginTransaction()
    try {
      const result = await action()
      await this.commitTransaction()
      return result
    } catch (error) {
      await this.abortTransaction()
      throw error
    }
  }

  async save(): Promise<ArrayBuffer> {
    this.assertOpen()
    const bytes = await this.client.save(this.documentHandle)
    this.emit({ type: 'databaseSaved' })
    return bytes
  }

  async close(): Promise<void> {
    if (this.closed) return
    await this.client.close(this.documentHandle)
    this.closed = true
    this.listeners.clear()
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

  /** @internal Block-table proxy bridge. */
  async readBlockRecords(): Promise<AcadRustBlockRecordTable> {
    this.assertOpen()
    return this.client.section<AcadRustBlockRecordTable>(
      this.documentHandle,
      'tables-blockrecord'
    )
  }

  /** @internal Block-record proxy bridge. */
  async readBlockEntityHandles(name: string): Promise<string[]> {
    this.assertOpen()
    return this.client.blockEntityHandles(this.documentHandle, name)
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

  private emit(event: AcDbDatabaseEvent): void {
    for (const listener of this.listeners) listener(event)
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

  async setLineEndpoints(start: AcadRustPoint3d, end: AcadRustPoint3d): Promise<void> {
    await this.database.patchLineGeometry(this.handle, { start, end })
  }

  async setCircleGeometry(geometry: { center?: AcadRustPoint3d; radius?: number }): Promise<void> {
    await this.database.patchCircleArcGeometry(this.handle, geometry)
  }

  async setArcGeometry(geometry: {
    center?: AcadRustPoint3d
    radius?: number
    startAngle?: number
    endAngle?: number
  }): Promise<void> {
    await this.database.patchCircleArcGeometry(this.handle, geometry)
  }

  async getLayer(): Promise<string> {
    return this.readCommon<string>('layer')
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


/** Read-only AcDbBlockTable view over acadrust's BLOCK-RECORD table. */
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
}

