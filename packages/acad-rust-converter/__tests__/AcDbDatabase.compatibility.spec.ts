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
      start: [0, 0, 0],
      end: [10, 0, 0]
    }
  }

  function createClient(): jest.Mocked<AcadRustDocumentClient> {
    return {
      open: jest.fn().mockResolvedValue(7),
      meta: jest.fn(),
      section: jest.fn(),
      entityPage: jest.fn(),
      blockEntityPage: jest.fn(),
      blockEntitiesBatch: jest.fn(),
      getEntity: jest.fn().mockResolvedValue(line),
      updateEntityCommon: jest.fn().mockResolvedValue(undefined),
      addEntity: jest.fn().mockResolvedValue('2A'),
      removeEntity: jest.fn().mockResolvedValue(true),
      save: jest.fn().mockResolvedValue(new ArrayBuffer(0)),
      beginTransaction: jest.fn().mockResolvedValue(undefined),
      commitTransaction: jest.fn().mockResolvedValue(undefined),
      abortTransaction: jest.fn().mockResolvedValue(undefined),
      close: jest.fn().mockResolvedValue(undefined),
      dispose: jest.fn()
    } as unknown as jest.Mocked<AcadRustDocumentClient>
  }

  it('uses a stable hexadecimal handle and reads the resident entity on demand', async () => {
    const client = createClient()
    const db = await AcDbDatabase.open(client, new ArrayBuffer(0))
    const entity = db.getEntity('2a')

    expect(entity.handle).toBe('2A')
    expect(await entity.getClassName()).toBe('Line')
    expect(await entity.getLayer()).toBe('0')
    expect(client.getEntity).toHaveBeenCalledWith(7, '2A')
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
})
