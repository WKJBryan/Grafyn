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
    twin_relationship_variant: { relationships: [] },
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

  it('hides Twin preference capture unless its surface explicitly opts in', () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer')],
      },
    })

    expect(wrapper.find('[aria-label="Capture Twin preference from tile-a model-a"]').exists())
      .toBe(false)
  })

  it('keeps response actions and the distinct global Twin evidence capture present without hover', async () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer')],
        streamingModels: new Set(),
        allowPreferenceCapture: true,
      },
    })

    await wrapper.get('[aria-label="Follow up on tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Regenerate tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Accept tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').trigger('click')

    const ref = { tileId: 'tile-a', modelId: 'model-a' }
    expect(wrapper.emitted('follow-up')).toEqual([[ref]])
    expect(wrapper.emitted('regenerate')).toEqual([[ref]])
    expect(wrapper.emitted('feedback')).toEqual([
      [{ ...ref, feedbackType: 'accept' }],
      [{ ...ref, feedbackType: 'reject' }],
    ])
    expect(wrapper.emitted('capture-preference')).toEqual([[
      {
        ...ref,
        responseId: 'response-tile-a',
        responseContent: 'Answer',
      },
    ]])
    expect(wrapper.get('.capture-preference-action').text()).toBe('Capture Twin preference')
  })

  it('keeps actions visible but disables them until the exact response is terminal', async () => {
    const inFlight = tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Partial')
    inFlight.responses['model-a'].status = 'streaming'
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [inFlight],
        streamingModels: new Set(['tile-a:model-a']),
        allowPreferenceCapture: true,
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
    expect(wrapper.find('[aria-label="Capture Twin preference from tile-a model-a"]').exists())
      .toBe(false)

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
    expect(wrapper.find('[aria-label="Capture Twin preference from tile-a model-a"]').exists())
      .toBe(false)
  })

  it('fails closed when the persisted relationship variant is absent or relationship-specific', () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [
          tile('missing', '2026-09-01T00:00:00Z', 'model-a', 'Missing', {
            twin_relationship_variant: undefined,
          }),
          tile('contextual', '2026-09-01T01:00:00Z', 'model-a', 'Contextual', {
            twin_relationship_variant: { relationships: [{
              subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'directed',
            }] },
          }),
        ],
        allowPreferenceCapture: true,
      },
    })

    expect(wrapper.find('[aria-label="Capture Twin preference from missing model-a"]').exists())
      .toBe(false)
    expect(wrapper.find('[aria-label="Capture Twin preference from contextual model-a"]').exists())
      .toBe(false)
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

  it('disables feedback and Twin evidence capture while exact response feedback is in flight', async () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer')],
        feedbackInFlight: new Set(['tile-a:model-a']),
        allowPreferenceCapture: true,
      },
    })

    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Reject tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Follow up on tile-a model-a"]').attributes('disabled')).toBeUndefined()
    expect(wrapper.get('[aria-label="Regenerate tile-a model-a"]').attributes('disabled')).toBeUndefined()

    await wrapper.get('[aria-label="Accept tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').trigger('click')
    expect(wrapper.emitted('feedback')).toBeUndefined()
    expect(wrapper.emitted('capture-preference')).toBeUndefined()
  })

  it('locks only the exact response capture while Twin evidence is being recorded', async () => {
    const wrapper = mount(LinearCanvasThread, {
      props: {
        tiles: [
          tile('tile-a', '2026-09-01T00:00:00Z', 'model-a', 'Answer A'),
          tile('tile-b', '2026-09-01T01:00:00Z', 'model-b', 'Answer B'),
        ],
        captureInFlight: new Set(['tile-a:model-a']),
        allowPreferenceCapture: true,
      },
    })

    expect(wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Regenerate tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled')).toBeUndefined()
    expect(wrapper.get('[aria-label="Regenerate tile-b model-b"]').attributes('disabled')).toBeUndefined()
    expect(wrapper.get('[aria-label="Capture Twin preference from tile-b model-b"]').attributes('disabled')).toBeUndefined()

    await wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').trigger('click')
    await wrapper.get('[aria-label="Capture Twin preference from tile-b model-b"]').trigger('click')
    expect(wrapper.emitted('capture-preference')).toEqual([[
      {
        tileId: 'tile-b',
        modelId: 'model-b',
        responseId: 'response-tile-b',
        responseContent: 'Answer B',
      },
    ]])
  })
})
