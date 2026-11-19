import {
  AcDbDatabase,
  type AcadRustDocumentClient,
  type AcadRustEntity
} from '../src'

describe('AcDbDatabase compatibility proxies', () => {
  const line: AcadRustEntity = {
    Line: {
      common: {
        handle: 42,
        layer: '0',
        linetype: 'ByLayer',
        linetype_scale: 1,
        invisible: false
      },
      start: { x: 0, y: 0, z: 0 },
      end: { x: 10, y: 0, z: 0 }
    }
  }

  function createClient(): jest.Mocked<AcadRustDocumentClient> {
    return {
      open: jest.fn().mockResolvedValue(7),
      meta: jest.fn(),
      section: jest.fn(),
      sectionHandleIndex: jest.fn().mockResolvedValue([]),
      dynamicBlockIndex: jest.fn().mockResolvedValue({ dynamicObjects: [], visibilityParameters: [], representations: [] }),
      insertSequence: jest.fn().mockResolvedValue({ insertHandle: '2A', blockName: 'B', attributes: [], seqendHandle: null, seqendResolved: false, valid: true }),
      roundtripAudit: jest.fn().mockResolvedValue({ ok: true, dwgByteLength: 100, entityCount: 1, blockRecordCount: 1, objectCount: 0, opaqueObjectCount: 0, entitiesEqual: true, blockRecordsEqual: true, objectsEqual: true, opaqueObjectsEqual: true, objectXdataEqual: true, dynamicVisibilityEqual: true, blockRepresentationsEqual: true, tablesEqual: true, extensionDictionaryLinksEqual: true, reactorLinksEqual: true, contextScalesEqual: true, fieldDefinitionsEqual: true, dgnLineStyleDefinitionsEqual: true, dgnLineStyleComponentsEqual: true, vxControlEntriesEqual: true, eedRecordsEqual: true, blockOwnedHandlesEqual: true, dataStoreHandlesEqual: true, acdsDataEqual: true, metadataEqual: true, beforeFingerprint: 'AA', afterFingerprint: 'AA' }),
      entityPage: jest.fn(),
      blockEntityPage: jest.fn(),
      blockEntityHandles: jest.fn().mockResolvedValue(['2A', '2B']),
      blockEntitiesBatch: jest.fn(),
      getEntity: jest.fn().mockResolvedValue(line),
      updateEntityCommon: jest.fn().mockResolvedValue(undefined),
      updateLineGeometry: jest.fn().mockResolvedValue(undefined),
      updateLwPolylineGeometry: jest.fn().mockResolvedValue(undefined),
      updateCircleArcGeometry: jest.fn().mockResolvedValue(undefined),
      updateEllipseGeometry: jest.fn().mockResolvedValue(undefined),
      updateSplineGeometry: jest.fn().mockResolvedValue(undefined),
      updateTextGeometry: jest.fn().mockResolvedValue(undefined),
      updateMTextGeometry: jest.fn().mockResolvedValue(undefined),
      updateDimensionBase: jest.fn().mockResolvedValue(undefined),
      getDimensionMeasurement: jest.fn().mockResolvedValue({ subtype: 'DIMENSION_LINEAR', measurement: 25, cachedMeasurement: 24, blockName: '*D12' }),
      updateInsertPlacement: jest.fn().mockResolvedValue(undefined),
      updateInsertAttributeValue: jest.fn().mockResolvedValue(undefined),
      updateAttdefDefaultValue: jest.fn().mockResolvedValue(undefined),
      addEntity: jest.fn().mockResolvedValue('2A'),
      addBlockRecord: jest.fn().mockResolvedValue({ name: 'NEW_BLOCK', handle: '40', blockEntityHandle: '41', blockEndHandle: '42' }),
      addEntityToBlock: jest.fn().mockResolvedValue('43'),
      removeEmptyBlockRecord: jest.fn().mockResolvedValue(true),
      moveEntityToBlock: jest.fn().mockResolvedValue(undefined),
      objectXdata: jest.fn().mockResolvedValue(null),
      setObjectXdataRecord: jest.fn().mockResolvedValue(undefined),
      removeObjectXdataRecord: jest.fn().mockResolvedValue(true),
      ensureExtensionDictionary: jest.fn().mockResolvedValue('44'),
      ensureXrecord: jest.fn().mockResolvedValue('45'),
      xrecord: jest.fn().mockResolvedValue({ entries: [] }),
      setXrecordEntries: jest.fn().mockResolvedValue(undefined),
      setEntityReactors: jest.fn().mockResolvedValue(undefined),
      setExtensionDictionaryEntry: jest.fn().mockResolvedValue('44'),
      removeExtensionDictionaryEntry: jest.fn().mockResolvedValue(true),
      removeEntity: jest.fn().mockResolvedValue(true),
      save: jest.fn().mockResolvedValue(new ArrayBuffer(0)),
      beginTransaction: jest.fn().mockResolvedValue(undefined),
      commitTransaction: jest.fn().mockResolvedValue(undefined),
      abortTransaction: jest.fn().mockResolvedValue(undefined),
      close: jest.fn().mockResolvedValue(undefined),
      dispose: jest.fn()
    } as unknown as jest.Mocked<AcadRustDocumentClient>
  }

  it('runs a native save-reopen integrity audit', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const audit = await db.auditRoundtrip()
    expect(audit.ok).toBe(true)
    expect(client.roundtripAudit).toHaveBeenCalledWith(7)
  })

  it('reports table and relationship parity dimensions from the native roundtrip audit', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const audit = await db.auditRoundtrip()
    expect(audit.tablesEqual).toBe(true)
    expect(audit.extensionDictionaryLinksEqual).toBe(true)
    expect(audit.reactorLinksEqual).toBe(true)
    expect(audit.contextScalesEqual).toBe(true)
    expect(audit.vxControlEntriesEqual).toBe(true)
    expect(audit.eedRecordsEqual).toBe(true)
    expect(audit.blockOwnedHandlesEqual).toBe(true)
    expect(audit.dataStoreHandlesEqual).toBe(true)
    expect(audit.acdsDataEqual).toBe(true)
    expect(audit.metadataEqual).toBe(true)
  })

  it('exposes dynamic-block visibility and representation metadata', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const index = await db.getDynamicBlockIndex()
    expect(index.visibilityParameters).toEqual([])
    expect(client.dynamicBlockIndex).toHaveBeenCalledWith(7)
  })

  it('inspects INSERT ATTRIB/SEQEND associations using exact handles', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const sequence = await db.getInsertSequence('2a')
    expect(sequence.insertHandle).toBe('2A')
    expect(client.insertSequence).toHaveBeenCalledWith(7, '2A')
  })

  it('uses a stable hexadecimal handle and reads the resident entity on demand', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2a')

    expect(entity.handle).toBe('2A')
    expect(await entity.getClassName()).toBe('Line')
    expect(await entity.getLayer()).toBe('0')
    expect(client.getEntity).toHaveBeenCalledWith(7, '2A')
  })

  it('updates CIRCLE and ARC geometry through typed reference-safe patches', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    const center = { x: 1, y: 2, z: 0 }

    await entity.setCircleGeometry({ center, radius: 5 })
    expect(client.updateCircleArcGeometry).toHaveBeenNthCalledWith(
      1, 7, '2A', { center, radius: 5 }
    )

    await entity.setArcGeometry({ startAngle: 0.25, endAngle: 1.5 })
    expect(client.updateCircleArcGeometry).toHaveBeenNthCalledWith(
      2, 7, '2A', { startAngle: 0.25, endAngle: 1.5 }
    )
  })

  it('updates ELLIPSE parameters through a typed reference-safe patch', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    client.getEntity.mockResolvedValueOnce({
      Ellipse: {
        common: { handle: 42, layer: '0', linetype: 'ByLayer', linetype_scale: 1, invisible: false },
        center: { x: 0, y: 0, z: 0 },
        major_axis: { x: 10, y: 0, z: 0 },
        minor_axis_ratio: 0.5,
        start_parameter: 0,
        end_parameter: Math.PI * 2,
        normal: { x: 0, y: 0, z: 1 }
      }
    } as unknown as AcadRustEntity)
    const patch = {
      center: { x: 2, y: 3, z: 0 },
      majorAxis: { x: 12, y: 0, z: 0 },
      minorAxisRatio: 0.75,
      startParameter: 0.2,
      endParameter: 5.8,
      normal: { x: 0, y: 0, z: 1 }
    }

    await entity.setEllipseGeometry(patch)
    expect(client.updateEllipseGeometry).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('updates SPLINE NURBS data through a typed reference-safe patch', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    client.getEntity.mockResolvedValueOnce({
      Spline: {
        common: { handle: 42, layer: '0', linetype: 'ByLayer', linetype_scale: 1, invisible: false },
        degree: 2,
        flags: { closed: false, periodic: false, rational: false, planar: true, linear: false },
        knots: [0, 0, 0, 1, 1, 1],
        control_points: [{ x: 0, y: 0, z: 0 }, { x: 5, y: 10, z: 0 }, { x: 10, y: 0, z: 0 }],
        weights: [],
        fit_points: [],
        normal: { x: 0, y: 0, z: 1 },
        knot_tolerance: 0,
        control_tolerance: 0,
        fit_tolerance: 0,
        begin_tangent: { x: 0, y: 0, z: 0 },
        end_tangent: { x: 0, y: 0, z: 0 },
        knot_parameterization: 0,
        cv_frame_visible: false,
        dwg_flags1: 0
      }
    } as unknown as AcadRustEntity)
    const patch = {
      degree: 2,
      flags: { rational: true },
      knots: [0, 0, 0, 1, 1, 1],
      controlPoints: [
        { x: 0, y: 0, z: 0 },
        { x: 5, y: 10, z: 0 },
        { x: 10, y: 0, z: 0 }
      ],
      weights: [1, 0.8, 1],
      knotTolerance: 1e-7,
      beginTangent: { x: 1, y: 0, z: 0 }
    }

    await entity.setSplineGeometry(patch)
    expect(client.updateSplineGeometry).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('reads DIMENSION measurement computed from native subtype geometry', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    jest.spyOn(entity, 'getClassName').mockResolvedValue('Dimension')

    await expect(entity.getDimensionMeasurement()).resolves.toEqual({
      subtype: 'DIMENSION_LINEAR',
      measurement: 25,
      cachedMeasurement: 24,
      blockName: '*D12'
    })
    expect(client.getDimensionMeasurement).toHaveBeenCalledWith(7, '2A')
  })

  it('updates DIMENSION display properties through the native typed proxy', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    jest.spyOn(entity, 'getClassName').mockResolvedValue('Dimension')
    const patch = {
      textOverride: 'L=<>',
      textMiddlePoint: { x: 10, y: 20, z: 0 },
      textRotation: 0.25,
      styleName: 'Standard',
      lineSpacingFactor: 1.0,
      lineSpacingStyle: 1 as const,
      textUserPositioned: true,
      flipArrow1: false
    }
    await entity.setDimensionProperties(patch)
    expect(client.updateDimensionBase).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('updates TEXT through the native typed proxy', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    jest.spyOn(entity, 'getClassName').mockResolvedValue('Text')
    const patch = { value: 'UPDATED', insertionPoint: { x: 10, y: 20, z: 0 }, height: 2.5, rotation: 0.25, widthFactor: 0.9 }
    await entity.setTextGeometry(patch)
    expect(client.updateTextGeometry).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('updates MTEXT through the native typed proxy', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    jest.spyOn(entity, 'getClassName').mockResolvedValue('MText')
    const patch = { value: 'LINE 1\\PLINE 2', insertionPoint: { x: 10, y: 20, z: 0 }, height: 2.5, rectangleWidth: 80, rotation: 0.5 }
    await entity.setMTextGeometry(patch)
    expect(client.updateMTextGeometry).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('updates single-line ATTRIB values and ATTDEF defaults through typed APIs', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')

    await entity.setAttributeValue('PART_NO', 'P-100')
    await entity.setAttributeDefaultValue('DEFAULT')
    expect(client.updateInsertAttributeValue).toHaveBeenCalledWith(7, '2A', 'PART_NO', 'P-100')
    expect(client.updateAttdefDefaultValue).toHaveBeenCalledWith(7, '2A', 'DEFAULT')
  })

  it('updates INSERT placement without replacing block or attribute references', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    const patch = {
      insertPoint: { x: 10, y: 20, z: 30 },
      scale: [2, 3, 1] as [number, number, number],
      rotation: 0.5
    }

    await entity.setInsertPlacement(patch)
    expect(client.updateInsertPlacement).toHaveBeenCalledWith(7, '2A', patch)
  })

  it('patches common properties without replacing handle-bearing entity JSON', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity(42)
    const events: string[] = []
    db.on(event => events.push(event.type))

    await entity.setLayer('MECH')
    await entity.setLinetypeScale(2.5)
    await entity.setInvisible(true)

    expect(client.updateEntityCommon).toHaveBeenNthCalledWith(1, 7, '2A', { layer: 'MECH' })
    expect(client.updateEntityCommon).toHaveBeenNthCalledWith(2, 7, '2A', { linetypeScale: 2.5 })
    expect(client.updateEntityCommon).toHaveBeenNthCalledWith(3, 7, '2A', { invisible: true })
    expect(events).toEqual(['objectModified', 'objectModified', 'objectModified'])
  })

  it('updates LWPOLYLINE vertices and metadata through the typed geometry protocol', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const patch = { vertices: [
      { location: { x: 0, y: 0 }, bulge: 0.5, startWidth: 1, endWidth: 2, vertexId: 11 },
      { location: { x: 20, y: 10 }, bulge: -0.25, startWidth: 2, endWidth: 3, vertexId: 12 }
    ], isClosed: true, plinegen: true, constantWidth: 0, elevation: 5, thickness: 1, normal: { x: 0, y: 0, z: 1 } }
    const events: string[] = []
    db.on(event => events.push(event.type))
    await db.getEntity('FFFFFFFFFFFFFFFF').setPolylineGeometry(patch)
    expect(client.updateLwPolylineGeometry).toHaveBeenCalledWith(7, 'FFFFFFFFFFFFFFFF', patch)
    expect(events).toEqual(['objectModified'])
  })

  it('supports indexed LWPOLYLINE vertex edits without coercing entity handles to numbers', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('FFFFFFFFFFFFFFFF')
    const vertex = { location: { x: 3, y: 4 }, bulge: 0.75, startWidth: 2, endWidth: 4, vertexId: 99 }
    await entity.insertPolylineVertex(1, vertex)
    await entity.updatePolylineVertex(2, vertex)
    await entity.removePolylineVertex(0)
    expect(client.updateLwPolylineGeometry).toHaveBeenNthCalledWith(1, 7, 'FFFFFFFFFFFFFFFF', { vertexOperations: [{ op: 'insert', index: 1, vertex }] })
    expect(client.updateLwPolylineGeometry).toHaveBeenNthCalledWith(2, 7, 'FFFFFFFFFFFFFFFF', { vertexOperations: [{ op: 'update', index: 2, vertex }] })
    expect(client.updateLwPolylineGeometry).toHaveBeenNthCalledWith(3, 7, 'FFFFFFFFFFFFFFFF', { vertexOperations: [{ op: 'remove', index: 0 }] })
  })

  it('updates LINE endpoints through the typed geometry protocol', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    const start = { x: -5, y: 2, z: 0 }
    const end = { x: 25, y: 2, z: 0 }

    await entity.setLineEndpoints(start, end)

    expect(client.updateLineGeometry).toHaveBeenCalledWith(7, '2A', { start, end })
  })

  it('isolates listener exceptions from already-committed database mutations', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const observed: string[] = []
    const failure = new Error('consumer callback failed')

    db.on(() => { throw failure })
    db.on(event => observed.push(event.type))

    // The Rust mutation succeeds even when an observer is faulty. Rejecting
    // here would invite callers to retry an operation that already happened.
    await expect(db.getEntity('2A').setLayer('MECH')).resolves.toBeUndefined()
    expect(client.updateEntityCommon).toHaveBeenCalledWith(7, '2A', { layer: 'MECH' })
    expect(observed).toEqual(['objectModified'])
    expect(db.takeListenerErrors()).toEqual([failure])
    expect(db.takeListenerErrors()).toEqual([])
  })

  it('routes append, erase, transactions and save to the same resident document', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const events: string[] = []
    db.on(event => events.push(event.type))

    const appended = await db.addEntity(line)
    expect(appended.handle).toBe('2A')
    expect(client.addEntity).toHaveBeenCalledWith(7, JSON.stringify(line))
    expect(await appended.erase()).toBe(true)

    await db.transaction(async () => {
      await db.getEntity(42).setLayer('TEMP')
    })
    await db.save()

    expect(client.removeEntity).toHaveBeenCalledWith(7, '2A')
    expect(client.beginTransaction).toHaveBeenCalledWith(7)
    expect(client.commitTransaction).toHaveBeenCalledWith(7)
    expect(client.save).toHaveBeenCalledWith(7)
    expect(events).toEqual([
      'objectAppended',
      'objectErased',
      'transactionStarted',
      'objectModified',
      'transactionCommitted',
      'databaseSaved'
    ])
  })

  it('updates reactor links and extension dictionary entries by exact handles', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')

    await entity.setReactors(['2B', 'FFFFFFFFFFFFFFFF'])
    expect(client.setEntityReactors).toHaveBeenCalledWith(7, '2A', ['2B', 'FFFFFFFFFFFFFFFF'])

    expect(await entity.setExtensionDictionaryEntry('APP_DATA', '2B', true)).toBe('44')
    expect(client.setExtensionDictionaryEntry).toHaveBeenCalledWith(
      7, '2A', 'APP_DATA', '2B', true
    )
    expect(await entity.removeExtensionDictionaryEntry('APP_DATA')).toBe(true)
  })

  it('routes XData and Extension Dictionary operations through Rust handles', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    const record = {
      application_name: 'MY_APP',
      values: [{ String: 'payload' }]
    }

    await entity.setXdataRecord(record)
    expect(client.setObjectXdataRecord).toHaveBeenCalledWith(7, '2A', record)
    expect(await entity.getXdata()).toBeNull()
    expect(await entity.removeXdataRecord('MY_APP')).toBe(true)
    expect(await entity.ensureExtensionDictionary()).toBe('44')
    expect(await entity.ensureXrecord('CUSTOM')).toBe('45')
    expect(await entity.getXrecord('CUSTOM')).toEqual({ entries: [] })

    const entries = [{ code: 1, value: { String: 'value' } }]
    await entity.setXrecordEntries('CUSTOM', entries)
    expect(client.setXrecordEntries).toHaveBeenCalledWith(7, '2A', 'CUSTOM', entries)
  })

  it('moves an entity between blocks through the owner-safe resident operation', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    await db.getEntity('2A').moveToBlock('MECHANICAL')
    expect(client.moveEntityToBlock).toHaveBeenCalledWith(7, '2A', 'MECHANICAL')
  })

  it('creates blocks, appends members and delegates safe empty-block removal', async () => {
    const client = createClient()
    client.section.mockResolvedValue({
      handle: 1,
      entries: {
        NEW_BLOCK: {
          handle: 64,
          block_entity_handle: 65,
          block_end_handle: 66,
          name: 'NEW_BLOCK',
          flags: {
            anonymous: false,
            has_attributes: false,
            is_xref: false,
            is_xref_overlay: false,
            is_external: false,
            is_xref_unloaded: false
          },
          layout: 0,
          units: 0,
          explodable: true,
          scale_uniformly: false,
          entity_handles: [],
          xref_path: '',
          description: '',
          insert_handles: [],
          base_point: { x: 0, y: 0, z: 0 }
        }
      }
    } as never)
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const block = await db.blockTable.add('NEW_BLOCK', { x: 1, y: 2, z: 3 }, 'test')
    expect(block.name).toBe('NEW_BLOCK')
    expect(client.addBlockRecord).toHaveBeenCalledWith(7, 'NEW_BLOCK', { x: 1, y: 2, z: 3 }, 'test')

    const appended = await block.appendEntity(line)
    expect(appended.handle).toBe('43')
    expect(client.addEntityToBlock).toHaveBeenCalledWith(7, 'NEW_BLOCK', JSON.stringify(line))
    expect(await db.blockTable.remove('NEW_BLOCK')).toBe(true)
    expect(client.removeEmptyBlockRecord).toHaveBeenCalledWith(7, 'NEW_BLOCK')
  })

  it('reads block definitions and returns entity proxies with lossless handles', async () => {
    const client = createClient()
    client.section.mockResolvedValue({
      handle: 1,
      entries: {
        '*Model_Space': {
          handle: 1,
          block_entity_handle: 2,
          block_end_handle: 3,
          name: '*Model_Space',
          flags: {
            anonymous: false,
            has_attributes: false,
            is_xref: false,
            is_xref_overlay: false,
            is_external: false,
            is_xref_unloaded: false
          },
          layout: 0,
          units: 0,
          explodable: true,
          scale_uniformly: false,
          entity_handles: [42, 43],
          xref_path: '',
          description: '',
          insert_handles: [],
          base_point: { x: 0, y: 0, z: 0 }
        }
      }
    } as never)
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const record = await db.blockTable.getAt('*model_space')

    expect(record?.name).toBe('*Model_Space')
    expect(await record?.getEntityHandles()).toEqual(['2A', '2B'])
    expect((await record?.getEntities())?.map(entity => entity.handle)).toEqual(['2A', '2B'])
    expect(client.blockEntityHandles).toHaveBeenCalledWith(7, '*Model_Space')
  })

  it('keeps ObjectId-to-handle identities exact and rolls them back with nested transactions', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const exactHandle = 'FFFFFFFFFFFFFFFF'
    db.bindObjectIdentity('mlight-object-1', exactHandle)
    expect(db.getHandleForObjectId('mlight-object-1')).toBe(exactHandle)
    expect(db.getObjectIdForHandle('0xffffffffffffffff')).toBe('mlight-object-1')

    await db.beginTransaction()
    db.bindObjectIdentity('temporary-object', '1000000000000000')
    await db.beginTransaction()
    db.bindObjectIdentity('nested-object', '1000000000000001')
    await db.abortTransaction()
    expect(db.getObjectIdForHandle('1000000000000001')).toBeUndefined()
    expect(db.getObjectIdForHandle('1000000000000000')).toBe('temporary-object')
    await db.abortTransaction()
    expect(db.getObjectIdForHandle('1000000000000000')).toBeUndefined()
    expect(db.getObjectIdForHandle(exactHandle)).toBe('mlight-object-1')
  })

  it('validates transaction depth before mutating the resident document', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))

    await expect(db.commitTransaction()).rejects.toThrow('No active transaction')
    await expect(db.abortTransaction()).rejects.toThrow('No active transaction')
    expect(client.commitTransaction).not.toHaveBeenCalled()
    expect(client.abortTransaction).not.toHaveBeenCalled()
    expect(db.transactionDepth).toBe(0)
  })

  it('retains the identity snapshot when the Rust rollback request fails', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    db.bindObjectIdentity('stable', 'AB')
    await db.beginTransaction()
    db.bindObjectIdentity('temporary', 'AC')
    client.abortTransaction.mockRejectedValueOnce(new Error('worker unavailable'))

    await expect(db.abortTransaction()).rejects.toThrow('worker unavailable')
    expect(db.transactionDepth).toBe(1)
    expect(db.getObjectIdForHandle('AC')).toBe('temporary')

    await db.abortTransaction()
    expect(db.transactionDepth).toBe(0)
    expect(db.getObjectIdForHandle('AC')).toBeUndefined()
    expect(db.getObjectIdForHandle('AB')).toBe('stable')
  })

  it('rolls back a failed transaction and closes idempotently', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))

    await expect(db.transaction(async () => {
      throw new Error('abort')
    })).rejects.toThrow('abort')

    await db.close()
    await db.close()
    expect(client.abortTransaction).toHaveBeenCalledWith(7)
    expect(client.close).toHaveBeenCalledTimes(1)
  })
  it('reads polyline geometry snapshots through the resident entity proxy', async () => {
    const client = createClient()
    client.getEntity.mockResolvedValueOnce({
      LwPolyline: {
        common: { handle: 42, layer: '0' },
        vertices: [
          { location: { x: 0, y: 0 }, bulge: 0, start_width: 0, end_width: 0, vertex_id: 0 },
          { location: { x: 10, y: 5 }, bulge: 0.25, start_width: 0, end_width: 0, vertex_id: 1 }
        ],
        is_closed: false
      }
    } as never)
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const vertices = await db.getEntity('2A').getPolylineVertices()
    expect(vertices).toHaveLength(2)
    expect((vertices[1] as { bulge: number }).bulge).toBe(0.25)
    expect(client.getEntity).toHaveBeenCalledWith(7, '2A')
  })

  it('reads INSERT payload and validates its ATTRIB/SEQEND sequence', async () => {
    const client = createClient()
    client.getEntity.mockResolvedValueOnce({
      Insert: {
        common: { handle: 42, layer: '0' },
        block_header: 99,
        insertion_point: { x: 1, y: 2, z: 0 },
        scale: { x: 1, y: 1, z: 1 },
        rotation: 0
      }
    } as never)
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const info = await db.getEntity('2A').getBlockReferenceData()
    expect(info.kind).toBe('Insert')
    expect(info.payload.block_header).toBe(99)
    expect(info.sequence.valid).toBe(true)
    expect(client.insertSequence).toHaveBeenCalledWith(7, '2A')
  })

  it('filters dynamic-block graph data for a requested handle', async () => {
    const client = createClient()
    client.dynamicBlockIndex.mockResolvedValueOnce({
      dynamicObjects: [
        { handle: 'A0', owner: '2A', dxfName: 'BLOCKVISIBILITYPARAMETER', cppClassName: 'AcDbBlockVisibilityParameter', data: {} },
        { handle: 'B0', owner: 'FF', dxfName: 'OTHER', cppClassName: 'Other', data: {} }
      ],
      visibilityParameters: [
        { handle: 'A1', owner: '2A', name: 'Visibility1', description: '', allBlocks: [], states: [] }
      ],
      representations: [
        { representationHandle: 'A2', dynamicBlockHandle: 'A0' },
        { representationHandle: 'B2', dynamicBlockHandle: 'B0' }
      ]
    })
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const info = await db.getEntity('2A').getDynamicBlockInfo()
    expect(info.objects).toHaveLength(1)
    expect(info.visibilityParameters).toHaveLength(1)
    expect(info.representations).toHaveLength(1)
  })

  it('exposes closed lifecycle state and rejects entity reads after close', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    expect(db.isClosed).toBe(false)
    await db.close()
    expect(db.isClosed).toBe(true)
    expect(() => db.getEntity('2A')).toThrow('AcDbDatabase is closed')
  })

  it('exposes shared entity common properties without a local mutable copy', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2A')
    const common = await entity.getCommonData()
    expect(common.layer).toBe('0')
    expect(common.linetype).toBe('ByLayer')
    expect(await entity.getLayer()).toBe('0')
    expect(await entity.getReactors()).toEqual([])
    expect(await entity.getExtendedData()).toEqual([])
  })

  it('rejects overlapping transaction-control operations without corrupting depth', async () => {
    const client = createClient()
    let releaseBegin!: () => void
    client.beginTransaction.mockImplementationOnce(() => new Promise<void>(resolve => {
      releaseBegin = resolve
    }))
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const first = db.beginTransaction()
    await expect(db.beginTransaction()).rejects.toThrow('already in flight')
    releaseBegin()
    await first
    expect(db.transactionDepth).toBe(1)
    await db.abortTransaction()
    expect(db.transactionDepth).toBe(0)
  })

  it('does not close a database while a transaction is active', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    await db.beginTransaction()
    await expect(db.close()).rejects.toThrow('active transactions')
    expect(client.close).not.toHaveBeenCalled()
    await db.abortTransaction()
    await db.close()
    expect(client.close).toHaveBeenCalledTimes(1)
  })

  it('retains transaction depth when the native commit fails', async () => {
    const client = createClient()
    client.commitTransaction.mockRejectedValueOnce(new Error('native commit failed'))
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    await db.beginTransaction()
    await expect(db.commitTransaction()).rejects.toThrow('native commit failed')
    expect(db.transactionDepth).toBe(1)
    await db.abortTransaction()
    expect(db.transactionDepth).toBe(0)
  })


})
