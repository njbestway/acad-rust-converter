/**
 * Lossless identity bridge between MLightCAD ObjectIds and DWG handles.
 *
 * A DWG handle is an unsigned 64-bit integer. JavaScript numbers cannot
 * represent every u64 exactly, so this bridge stores canonical hexadecimal
 * handles and accepts bigint/string inputs. Numeric inputs are accepted only
 * while they are safe integers; silently rounding a handle would corrupt
 * object references during a round trip.
 *
 * This is deliberately an identity map, not a second object database. The
 * AcDb-facing proxy and the acadrust document remain the owners of their
 * respective objects; this class only maintains their cross-reference.
 */
export type DwgHandleLike = bigint | number | string

export interface ObjectIdentityEntry {
  readonly objectId: string
  /** Canonical uppercase hexadecimal, without a 0x prefix. */
  readonly handle: string
}

export class ObjectIdentityMap {
  private readonly handleByObjectId = new Map<string, string>()
  private readonly objectIdByHandle = new Map<string, string>()

  get size(): number {
    return this.handleByObjectId.size
  }

  /**
   * Register a one-to-one identity pair.
   *
   * Re-registering the same pair is idempotent. Rebinding either side to a
   * different identity is rejected: callers must explicitly unbind first so
   * an accidental handle collision cannot silently retarget references.
   */
  bind(objectId: string, handle: DwgHandleLike): ObjectIdentityEntry {
    const id = normalizeObjectId(objectId)
    const canonicalHandle = normalizeDwgHandle(handle)
    const existingHandle = this.handleByObjectId.get(id)
    const existingObjectId = this.objectIdByHandle.get(canonicalHandle)

    if (existingHandle === canonicalHandle && existingObjectId === id) {
      return { objectId: id, handle: canonicalHandle }
    }
    if (existingHandle != null && existingHandle !== canonicalHandle) {
      throw new Error(
        `ObjectId '${id}' is already bound to DWG handle ${existingHandle}.`
      )
    }
    if (existingObjectId != null && existingObjectId !== id) {
      throw new Error(
        `DWG handle ${canonicalHandle} is already bound to ObjectId '${existingObjectId}'.`
      )
    }

    this.handleByObjectId.set(id, canonicalHandle)
    this.objectIdByHandle.set(canonicalHandle, id)
    return { objectId: id, handle: canonicalHandle }
  }

  getHandle(objectId: string): string | undefined {
    return this.handleByObjectId.get(normalizeObjectId(objectId))
  }

  getObjectId(handle: DwgHandleLike): string | undefined {
    return this.objectIdByHandle.get(normalizeDwgHandle(handle))
  }

  hasObjectId(objectId: string): boolean {
    return this.handleByObjectId.has(normalizeObjectId(objectId))
  }

  hasHandle(handle: DwgHandleLike): boolean {
    return this.objectIdByHandle.has(normalizeDwgHandle(handle))
  }

  /** Remove a pair by either side; returns the removed pair when present. */
  unbindByObjectId(objectId: string): ObjectIdentityEntry | undefined {
    const id = normalizeObjectId(objectId)
    const handle = this.handleByObjectId.get(id)
    if (handle == null) return undefined
    this.handleByObjectId.delete(id)
    this.objectIdByHandle.delete(handle)
    return { objectId: id, handle }
  }

  unbindByHandle(handle: DwgHandleLike): ObjectIdentityEntry | undefined {
    const canonicalHandle = normalizeDwgHandle(handle)
    const objectId = this.objectIdByHandle.get(canonicalHandle)
    if (objectId == null) return undefined
    this.objectIdByHandle.delete(canonicalHandle)
    this.handleByObjectId.delete(objectId)
    return { objectId, handle: canonicalHandle }
  }

  clear(): void {
    this.handleByObjectId.clear()
    this.objectIdByHandle.clear()
  }

  entries(): ObjectIdentityEntry[] {
    return Array.from(this.handleByObjectId, ([objectId, handle]) => ({
      objectId,
      handle
    }))
  }

  /**
   * Return a detached copy for transaction snapshots. The maps are copied;
   * subsequent bind/unbind operations do not affect the snapshot.
   */
  clone(): ObjectIdentityMap {
    const copy = new ObjectIdentityMap()
    for (const [objectId, handle] of this.handleByObjectId) {
      copy.bind(objectId, handle)
    }
    return copy
  }
}

export function normalizeDwgHandle(handle: DwgHandleLike): string {
  let value: bigint
  try {
    if (typeof handle === 'bigint') {
      value = handle
    } else if (typeof handle === 'number') {
      if (!Number.isSafeInteger(handle) || handle < 0) {
        throw new Error('number handle must be a non-negative safe integer')
      }
      value = BigInt(handle)
    } else {
      const trimmed = handle.trim()
      const hex = trimmed.startsWith('0x') || trimmed.startsWith('0X')
        ? trimmed.slice(2)
        : trimmed
      if (!/^[0-9a-fA-F]+$/.test(hex)) {
        throw new Error('handle string must contain hexadecimal digits')
      }
      value = BigInt(`0x${hex}`)
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error)
    throw new Error(`Invalid DWG handle '${String(handle)}': ${detail}.`)
  }

  if (value < 0n || value > 0xffff_ffff_ffff_ffffn) {
    throw new Error('DWG handle must fit in an unsigned 64-bit integer.')
  }
  return value.toString(16).toUpperCase()
}

function normalizeObjectId(objectId: string): string {
  if (typeof objectId !== 'string' || objectId.trim().length === 0) {
    throw new Error('ObjectId must be a non-empty string.')
  }
  return objectId.trim()
}
