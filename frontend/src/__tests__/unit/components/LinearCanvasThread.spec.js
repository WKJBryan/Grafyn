import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import LinearCanvasThread from '@/components/companion/LinearCanvasThread.vue'

function tile(id, createdAt, modelId, content, extras = {}) {
  return {
    id,
    prompt: `Prompt ${id}`,
    models: [modelId],
    created_at: createdAt,
    responses: {
      [modelId]: {
        id: `response-${id}`,
        model_id: modelId,
        model_name: modelId,
        content,
        status: 'completed',
        created_at: createdAt,
      },
    },
    ...extras,
  }
}

describe('LinearCanvasThread', () => {
  it('renders tiles by created time and stable id, independent of persisted array order', () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [
          tile('b', '2026-09-01T01:00:00Z', 'model-b', 'Second at tie'),
          tile('z', '2026-09-01T00:00:00Z', 'model-z', 'First'),
          tile('a', '2026-09-01T01:00:00Z', 'model-a', 'First at tie'),
        ],
        streamingModels: new Set(),
      },
    })

    expect(wrapper.findAll('[data-tile-id]').map(node => node.attributes('data-tile-id')))
      .toEqual(['z', 'a', 'b'])
  })

  it('shows persisted Twin evidence provenance with the answer', () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('twin', '2026-09-01T00:00:00Z', 'model-a', 'Grounded answer', {
          twin_evidence_snapshot: {
            projection_snapshot_id: 'a'.repeat(64),
            reference_time: '2026-09-01T00:00:00Z',
            evidence_event_ids: ['b'.repeat(64), 'c'.repeat(64)],
            note_ids: ['note-1'],
          },
        })],
        streamingModels: new Set(),
      },
    })

    expect(wrapper.text()).toContain('Evidence snapshot')
    expect(wrapper.text()).toContain('2 events')
    expect(wrapper.text()).toContain('1 note')
  })

  it('keeps follow-up, regenerate, accept, and reject actions present without hover', async () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer')],
        streamingModels: new Set(),
      },
    })

    await wrapper.get('[aria-label="Follow up on tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Regenerate tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Accept tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject tile-a model-a"]').trigger('click')

    const ref = { tileId: 'tile-a', modelId: 'model-a' }
    expect(wrapper.emitted('follow-up')).toEqual([[ref]])
    expect(wrapper.emitted('regenerate')).toEqual([[ref]])
    expect(wrapper.emitted('feedback')).toEqual([
      [{ ...ref, feedbackType: 'accept' }],
      [{ ...ref, feedbackType: 'reject' }],
    ])
  })

  it('keeps actions visible but disables them until the exact response is terminal', async () => {
    const inFlight = tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Partial')
    inFlight.responses['model-a'].status = 'streaming'
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [inFlight],
        streamingModels: new Set(['tile-a:model-a']),
      },
    })

    for (const label of [
      'Follow up on tile-a model-a',
      'Regenerate tile-a model-a',
      'Accept tile-a model-a',
      'Reject tile-a model-a',
    ]) {
      expect(wrapper.get(`[aria-label="${label}"]`).attributes('disabled')).toBeDefined()
    }

    const failed = tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', '')
    failed.responses['model-a'].status = 'error'
    await wrapper.setProps({ tiles: [failed], streamingModels: new Set() })

    expect(wrapper.get('[aria-label="Regenerate tile-a model-a"]').attributes('disabled'))
      .toBeUndefined()
    expect(wrapper.get('[aria-label="Follow up on tile-a model-a"]').attributes('disabled'))
      .toBeDefined()
    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled'))
      .toBeDefined()
    expect(wrapper.get('[aria-label="Reject tile-a model-a"]').attributes('disabled'))
      .toBeDefined()
  })

  it('shows directed and bidirectional Twin contexts as distinct turn labels', () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [
          tile('directed', '2026-09-01T00:00:00Z', 'model-a', 'Directed', {
            twin_relationship_variant: { relationships: [{
              subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'directed',
            }] },
          }),
          tile('bidirectional', '2026-09-01T01:00:00Z', 'model-a', 'Bidirectional', {
            twin_relationship_variant: { relationships: [{
              subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'bidirectional',
            }] },
          }),
        ],
      },
    })

    expect(wrapper.get('[data-relationship-context="directed"]').text()).toContain('[directed]')
    expect(wrapper.get('[data-relationship-context="bidirectional"]').text()).toContain('[bidirectional]')
  })

  it('disables both feedback choices while the exact response feedback is in flight', async () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer')],
        feedbackInFlight: new Set(['tile-a:model-a']),
      },
    })

    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Reject tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Follow up on tile-a model-a"]').attributes('disabled')).toBeUndefined()
    expect(wrapper.get('[aria-label="Regenerate tile-a model-a"]').attributes('disabled')).toBeUndefined()

    await wrapper.get('[aria-label="Accept tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject tile-a model-a"]').trigger('click')
    expect(wrapper.emitted('feedback')).toBeUndefined()
  })
})
