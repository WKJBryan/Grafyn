import { expect } from '@playwright/test'

function notesFrom(value) {
  return Array.isArray(value) ? value : value?.notes || []
}

function operationIds(bundle) {
  return bundle.envelopes.map((encoded) => JSON.parse(encoded).operation_id)
}

function materialNote(note) {
  return Object.fromEntries(
    Object.entries(note).filter(([field]) => field !== 'relative_path'),
  )
}

export async function verifyTwinEventSync({
  deviceA,
  deviceB,
  marker,
  attachmentDigest,
  reviewedMemoryId,
  expectedContext,
  knownPlaintexts,
}) {
  const sourceNotes = await Promise.all(
    notesFrom(await deviceA.invoke('list_notes', {}))
      .map(({ id }) => deviceA.invoke('get_note', { id })),
  )
  const sourceCaptured = sourceNotes.find((note) => note.content?.includes(marker))
  expect(sourceCaptured).toBeTruthy()

  const peerNotesBefore = await Promise.all(
    notesFrom(await deviceB.invoke('list_notes', {}))
      .map(({ id }) => deviceB.invoke('get_note', { id })),
  )
  const peerHadCapture = peerNotesBefore.some((note) => note.content?.includes(marker))
  const sourceBundle = await deviceA.invoke('export_sync_outbox', {})
  expect(sourceBundle.schemaVersion).toBe(1)
  expect(sourceBundle.envelopes.length).toBeGreaterThan(0)
  const encodedBundle = JSON.stringify(sourceBundle)
  for (const plaintext of knownPlaintexts) {
    expect(typeof plaintext).toBe('string')
    expect(plaintext.length).toBeGreaterThan(0)
    expect(encodedBundle).not.toContain(plaintext)
    expect(encodedBundle).not.toContain(Buffer.from(plaintext, 'utf8').toString('base64'))
  }

  const reversed = [...sourceBundle.envelopes].reverse()
  const duplicated = reversed.flatMap((envelope) => [envelope, envelope])
  const firstImport = await deviceB.invoke('import_sync_envelopes', {
    bundle: { schemaVersion: 1, envelopes: duplicated },
  })
  expect(firstImport.recoveryPending).toBe(false)
  expect(firstImport.applied + firstImport.duplicates).toBe(duplicated.length)
  if (peerHadCapture) {
    expect(firstImport.duplicates).toBeGreaterThanOrEqual(reversed.length)
  } else {
    expect(firstImport.applied).toBe(reversed.length)
    expect(firstImport.duplicates).toBe(reversed.length)
  }

  const replay = await deviceB.invoke('import_sync_envelopes', {
    bundle: { schemaVersion: 1, envelopes: reversed },
  })
  expect(replay.recoveryPending).toBe(false)
  expect(replay.applied).toBe(0)
  expect(replay.duplicates).toBe(reversed.length)

  const peerNoteMetadata = notesFrom(await deviceB.invoke('list_notes', {}))
  const peerNotes = await Promise.all(
    peerNoteMetadata.map(({ id }) => deviceB.invoke('get_note', { id })),
  )
  const captured = peerNotes.find((note) => note.content?.includes(marker))
  expect(captured).toBeTruthy()
  expect(materialNote(captured)).toEqual(materialNote(sourceCaptured))
  expect(captured.properties?.attachment_digests).toContain(attachmentDigest)

  const [sourceImage, loadedImage] = await Promise.all([
    deviceA.invoke('load_generated_image', { request: { attachmentDigest } }),
    deviceB.invoke('load_generated_image', { request: { attachmentDigest } }),
  ])
  expect(loadedImage).toEqual(sourceImage)
  expect(sourceImage).toMatchObject({
    attachmentDigest,
    mediaType: 'image/png',
    width: 2,
    height: 2,
  })
  expect(Buffer.from(loadedImage.base64Data, 'base64').length).toBe(loadedImage.byteSize)

  const referenceTime = new Date().toISOString()
  const [sourceProjection, peerProjection] = await Promise.all([
    deviceA.invoke('get_twin_state_projection', { request: { referenceTime } }),
    deviceB.invoke('get_twin_state_projection', { request: { referenceTime } }),
  ])
  const sourceMemory = sourceProjection.reviewed_memories.find(
    (item) => item.item_id === reviewedMemoryId,
  )
  const peerMemory = peerProjection.reviewed_memories.find(
    (item) => item.item_id === reviewedMemoryId,
  )
  expect(sourceMemory).toBeTruthy()
  expect(peerMemory).toEqual(sourceMemory)
  expect(peerProjection).toEqual(sourceProjection)

  const sourceObservation = sourceProjection.recent_observations.find(
    (item) => item.claim?.predicate === 'recorded_note'
      && item.claim?.object === sourceCaptured.id,
  )
  expect(sourceObservation).toBeTruthy()
  expect(sourceObservation.goals).toEqual([expectedContext.Goal])
  expect(sourceObservation.tags).toEqual(['inbox'])
  expect(sourceObservation.valid_from).toBeNull()
  expect(sourceObservation.valid_to).toBeNull()
  expect(Date.parse(sourceObservation.last_confirmed_at)).not.toBeNaN()
  expect(sourceObservation.relationship_variant.relationships).toHaveLength(1)
  expect(sourceObservation.relationship_variant.relationships[0]).toMatchObject({
    subject_id: 'owner',
    predicate: 'research_partner',
    direction: 'directed',
  })
  expect(sourceObservation.relationship_variant.relationships[0].object_id)
    .toMatch(/^person-[0-9a-f]{64}$/)

  const timelineRequest = {
    referenceTime,
    filter: { relationships: [], goals: [], tags: [] },
    cursor: null,
    limit: 100,
  }
  const [sourceTimeline, peerTimeline] = await Promise.all([
    deviceA.invoke('get_twin_event_timeline', { request: timelineRequest }),
    deviceB.invoke('get_twin_event_timeline', { request: timelineRequest }),
  ])
  expect(peerTimeline).toEqual(sourceTimeline)
  expect(sourceTimeline.snapshotId).toBe(sourceProjection.snapshot_id)
  const observationTimeline = sourceTimeline.items.find(
    (entry) => entry.item_id === sourceObservation.item_id && entry.state === 'observed',
  )
  expect(observationTimeline).toMatchObject({
    relationship_variant: sourceObservation.relationship_variant,
    valid_from: sourceObservation.valid_from,
    valid_to: sourceObservation.valid_to,
  })
  expect(observationTimeline.effective_at).toBe(sourceObservation.last_confirmed_at)
  const reviewedTimeline = sourceTimeline.items.find(
    (entry) => entry.item_id === reviewedMemoryId && entry.state === 'accepted',
  )
  expect(reviewedTimeline).toMatchObject({
    relationship_variant: sourceMemory.relationship_variant,
    valid_from: sourceMemory.valid_from,
    valid_to: sourceMemory.valid_to,
  })
  expect(Date.parse(reviewedTimeline.effective_at)).not.toBeNaN()

  const peerOutbox = await deviceB.invoke('export_sync_outbox', {})
  expect(peerOutbox).toEqual({ schemaVersion: 1, envelopes: [] })
  expect(new Set(operationIds(sourceBundle)).size).toBe(sourceBundle.envelopes.length)

  return {
    sourceBundle,
    firstImport,
    replay,
    peerNotes,
    sourceProjection,
    peerProjection,
    sourceTimeline,
    peerTimeline,
  }
}
