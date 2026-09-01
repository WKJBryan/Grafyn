import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import TwinReviewQueue from '@/components/companion/TwinReviewQueue.vue'

function proposal(id, object = 'evidence first') {
  return {
    item_id: id,
    claim: {
      subject_id: 'owner',
      predicate: 'prefers',
      object,
      polarity: 'affirmed',
    },
    summary: null,
    support_count: 3,
    opposition_count: 0,
    relationship_variant: { relationships: [] },
  }
}

describe('TwinReviewQueue', () => {
  it('joins attention explanations to proposals by exact item id', () => {
    const wrapper = mount(TwinReviewQueue, {
      props: {
        proposals: [proposal('proposal-a'), proposal('proposal-b', 'speed first')],
        digestItems: [],
        attentionTrace: {
          selected: [
            { item_id: 'proposal-b', attention: { explanation: 'Repeated in this context.' } },
            { item_id: 'not-proposal-a', attention: { explanation: 'Wrong near-match.' } },
          ],
        },
      },
    })

    const cards = wrapper.findAll('[data-proposal-id]')
    expect(cards[0].text()).not.toContain('Wrong near-match.')
    expect(cards[1].text()).toContain('Repeated in this context.')
  })

  it('emits accept and reject decisions for the exact displayed proposal', async () => {
    const wrapper = mount(TwinReviewQueue, {
      props: { proposals: [proposal('proposal-a')], digestItems: [], attentionTrace: null },
    })

    await wrapper.get('[aria-label="Accept proposal proposal-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject proposal proposal-a"]').trigger('click')

    expect(wrapper.emitted('review-proposal')).toEqual([
      [{ itemId: 'proposal-a', decision: 'accept', reviewedClaim: null }],
      [{ itemId: 'proposal-a', decision: 'reject', reviewedClaim: null }],
    ])
  })

  it('accepts an edited claim without mutating the displayed proposal', async () => {
    const source = proposal('proposal-a')
    const wrapper = mount(TwinReviewQueue, {
      props: { proposals: [source], digestItems: [], attentionTrace: null },
    })

    await wrapper.get('[aria-label="Edit proposal proposal-a"]').trigger('click')
    await wrapper.get('[aria-label="Edited claim for proposal-a"]').setValue('careful experiments')
    await wrapper.get('[aria-label="Accept edited proposal proposal-a"]').trigger('click')

    expect(wrapper.emitted('review-proposal')[0][0]).toEqual({
      itemId: 'proposal-a',
      decision: 'accept',
      reviewedClaim: {
        subject_id: 'owner',
        predicate: 'prefers',
        object: 'careful experiments',
        polarity: 'affirmed',
      },
    })
    expect(source.claim.object).toBe('evidence first')
  })

  it('keeps digest review actions visible and identifies the exact item', async () => {
    const wrapper = mount(TwinReviewQueue, {
      props: {
        proposals: [],
        digestItems: [{ id: 'digest-a', pattern: 'Plans before acting', evidence_count: 4 }],
        attentionTrace: null,
      },
    })

    await wrapper.get('[aria-label="Keep digest digest-a"]').trigger('click')
    await wrapper.get('[aria-label="Reject digest digest-a"]').trigger('click')

    expect(wrapper.emitted('review-digest')).toEqual([
      [{ id: 'digest-a', action: 'keep' }],
      [{ id: 'digest-a', action: 'reject' }],
    ])
  })

  it('disables stale proposal, edit, and digest actions while the review page is loading', async () => {
    const wrapper = mount(TwinReviewQueue, {
      props: {
        proposals: [proposal('proposal-a')],
        digestItems: [{ id: 'digest-a', pattern: 'Plans before acting', evidence_count: 4 }],
        attentionTrace: null,
        loading: false,
      },
    })

    await wrapper.setProps({ loading: true })

    for (const label of [
      'Accept proposal proposal-a',
      'Edit proposal proposal-a',
      'Reject proposal proposal-a',
      'Keep digest digest-a',
      'Reject digest digest-a',
    ]) {
      expect(wrapper.get(`[aria-label="${label}"]`).attributes('disabled')).toBeDefined()
    }

    await wrapper.setProps({ loading: false })
    await wrapper.get('[aria-label="Edit proposal proposal-a"]').trigger('click')
    await wrapper.setProps({ loading: true })

    const cancel = wrapper.findAll('button').find(item => item.text() === 'Cancel')
    expect(cancel.attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Accept edited proposal proposal-a"]')
      .attributes('disabled')).toBeDefined()
  })

  it('shows directed and bidirectional proposal contexts as distinct labels', () => {
    const directed = proposal('proposal-directed')
    directed.relationship_variant = { relationships: [{
      subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'directed',
    }] }
    const bidirectional = proposal('proposal-bidirectional')
    bidirectional.relationship_variant = { relationships: [{
      subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'bidirectional',
    }] }
    const wrapper = mount(TwinReviewQueue, {
      props: { proposals: [directed, bidirectional], digestItems: [] },
    })

    expect(wrapper.get('[data-proposal-id="proposal-directed"] .relationship-label').text())
      .toContain('[directed]')
    expect(wrapper.get('[data-proposal-id="proposal-bidirectional"] .relationship-label').text())
      .toContain('[bidirectional]')
  })
})
