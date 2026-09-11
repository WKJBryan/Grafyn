import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import RecallView from '@/views/companion/RecallView.vue'
import { memory, notes, twin } from '@/api/client'

function deferred() {
  let resolve
  let reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

async function search(wrapper, query) {
  await wrapper.get('input[type="search"]').setValue(query)
  await vi.advanceTimersByTimeAsync(300)
  await flushPromises()
}

describe('RecallView', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.spyOn(twin, 'rankAttention').mockResolvedValue({
      trace: { selected: [], excluded: [] },
      noteBindings: [],
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.useRealTimers()
  })

  it('starts with a useful idle state and then shows an explicit empty state', async () => {
    vi.spyOn(memory, 'recall').mockResolvedValue({ results: [] })
    const wrapper = mount(RecallView)

    expect(wrapper.get('[data-state="idle"]').text()).toContain('Search your notes')

    await search(wrapper, 'missing memory')

    expect(wrapper.get('[data-state="empty"]').text()).toContain('No notes found')
    expect(twin.rankAttention).not.toHaveBeenCalled()
  })

  it('joins attention through exact selected item and note bindings, never list position', async () => {
    const alphaItemId = `observation:${'a'.repeat(64)}:capture`
    const betaItemId = `observation:${'b'.repeat(64)}:capture`
    const excludedItemId = `observation:${'c'.repeat(64)}:capture`
    const unmatchedItemId = `observation:${'d'.repeat(64)}:capture`
    vi.spyOn(memory, 'recall').mockResolvedValue({
      results: [
        { note_id: 'note-a', title: 'Alpha', snippet: 'A', score: 0.9 },
        { note_id: 'note-b', title: 'Beta', snippet: 'B', score: 0.8 },
      ],
    })
    vi.spyOn(twin, 'rankAttention').mockResolvedValue({
      trace: {
        selected: [
          { item_id: betaItemId, attention: { explanation: 'Beta matches recent context.' } },
          { item_id: unmatchedItemId, attention: { explanation: 'Unbound selection.' } },
          { item_id: alphaItemId, attention: { explanation: 'Alpha matches the query.' } },
        ],
        excluded: [
          { item_id: excludedItemId, reason: 'not_eligible' },
        ],
      },
      noteBindings: [
        { itemId: betaItemId, noteId: 'note-b' },
        { itemId: excludedItemId, noteId: 'note-a' },
        { itemId: alphaItemId, noteId: 'note-a' },
        { itemId: 'missing-item', noteId: 'note-b' },
      ],
    })
    const wrapper = mount(RecallView)

    await search(wrapper, 'project memory')
    await flushPromises()

    const rows = wrapper.findAll('.recall-result')
    expect(rows[0].text()).toContain('Alpha matches the query.')
    expect(rows[0].text()).not.toContain('Beta matches recent context.')
    expect(rows[1].text()).toContain('Beta matches recent context.')
    expect(wrapper.text()).not.toContain('Unbound selection.')
    expect(twin.rankAttention).toHaveBeenCalledWith(expect.objectContaining({
      profile: 'recall',
      query: 'project memory',
      candidateNoteIds: ['note-a', 'note-b'],
    }))
  })

  it('keeps search results usable when optional attention loading fails', async () => {
    vi.spyOn(memory, 'recall').mockResolvedValue([
      { note_id: 'note-a', title: 'Alpha', snippet: 'Still visible', score: 0.9 },
    ])
    vi.spyOn(twin, 'rankAttention').mockRejectedValue(new Error('C:\\private\\trace'))
    const wrapper = mount(RecallView)

    await search(wrapper, 'alpha')
    await flushPromises()

    expect(wrapper.get('.recall-result').text()).toContain('Alpha')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('private')
  })

  it('loads and sanitizes a selected note detail', async () => {
    vi.spyOn(memory, 'recall').mockResolvedValue([
      { note_id: 'note-a', title: 'Alpha', snippet: 'Open me', score: 0.9 },
    ])
    vi.spyOn(notes, 'get').mockResolvedValue({
      id: 'note-a',
      title: 'Alpha',
      content: '# Detail\n<img src=x onerror="alert(1)"><script>alert(1)</script>',
      tags: ['inbox'],
    })
    const wrapper = mount(RecallView)
    await search(wrapper, 'alpha')

    await wrapper.get('.recall-result').trigger('click')
    await flushPromises()

    expect(notes.get).toHaveBeenCalledWith('note-a')
    expect(wrapper.get('.recall-detail-body').html()).toContain('<h1>Detail</h1>')
    expect(wrapper.get('.recall-detail-body').html()).not.toContain('onerror')
    expect(wrapper.get('.recall-detail-body').html()).not.toContain('<script')
  })

  it('shows safe local copy for recall and note-detail failures', async () => {
    vi.spyOn(memory, 'recall').mockRejectedValue(new Error('C:\\private\\recall'))
    const wrapper = mount(RecallView)
    await search(wrapper, 'alpha')

    expect(wrapper.get('[role="alert"]').text()).toBe('Recall is temporarily unavailable.')
    expect(wrapper.text()).not.toContain('private')
    expect(twin.rankAttention).not.toHaveBeenCalled()
  })

  it('ignores stale attention that resolves after a newer query', async () => {
    const alpha = deferred()
    const beta = deferred()
    const alphaItemId = `observation:${'a'.repeat(64)}:capture`
    const betaItemId = `observation:${'b'.repeat(64)}:capture`
    vi.spyOn(memory, 'recall').mockImplementation(async (query) => ([{
      note_id: `note-${query}`,
      title: query === 'alpha' ? 'Alpha' : 'Beta',
      snippet: query,
      score: 1,
    }]))
    vi.spyOn(twin, 'rankAttention')
      .mockReturnValueOnce(alpha.promise)
      .mockReturnValueOnce(beta.promise)
    const wrapper = mount(RecallView)

    await search(wrapper, 'alpha')
    await search(wrapper, 'beta')

    beta.resolve({
      trace: {
        selected: [{
          item_id: betaItemId,
          attention: { explanation: 'Newest beta explanation.' },
        }],
        excluded: [],
      },
      noteBindings: [{ itemId: betaItemId, noteId: 'note-beta' }],
    })
    await flushPromises()
    alpha.resolve({
      trace: {
        selected: [{
          item_id: alphaItemId,
          attention: { explanation: 'Stale alpha explanation.' },
        }],
        excluded: [],
      },
      noteBindings: [{ itemId: alphaItemId, noteId: 'note-alpha' }],
    })
    await flushPromises()

    expect(wrapper.get('.recall-result').text()).toContain('Newest beta explanation.')
    expect(wrapper.text()).not.toContain('Stale alpha explanation.')
  })

  it('ignores stale note detail that resolves after a newer selection', async () => {
    const alpha = deferred()
    const beta = deferred()
    vi.spyOn(memory, 'recall').mockResolvedValue([
      { note_id: 'note-alpha', title: 'Alpha', snippet: 'First', score: 1 },
      { note_id: 'note-beta', title: 'Beta', snippet: 'Second', score: 0.9 },
    ])
    vi.spyOn(notes, 'get').mockImplementation(noteId => (
      noteId === 'note-alpha' ? alpha.promise : beta.promise
    ))
    const wrapper = mount(RecallView)
    await search(wrapper, 'notes')

    await wrapper.findAll('.recall-result')[0].trigger('click')
    await wrapper.get('.recall-back').trigger('click')
    await wrapper.findAll('.recall-result')[1].trigger('click')

    beta.resolve({ id: 'note-beta', title: 'Beta', content: 'Newest detail' })
    await flushPromises()
    alpha.resolve({ id: 'note-alpha', title: 'Alpha', content: 'Stale detail' })
    await flushPromises()

    expect(wrapper.get('.recall-detail').text()).toContain('Newest detail')
    expect(wrapper.text()).not.toContain('Stale detail')
  })

  it('gives the search input an explicit accessible label', () => {
    const wrapper = mount(RecallView)

    expect(wrapper.get('label[for="companion-recall-query"]').text()).toContain('Search your notes')
  })

  it('wraps a long unbroken result title inside the compact result width', async () => {
    vi.spyOn(memory, 'recall').mockResolvedValue([{
      note_id: 'note-long',
      title: 'A'.repeat(300),
      snippet: '',
      score: 1,
    }])
    const wrapper = mount(RecallView)
    await search(wrapper, 'long')

    const heading = wrapper.get('.recall-result-heading').element
    const title = wrapper.get('.recall-result-heading strong').element
    expect(['0', '0px']).toContain(getComputedStyle(heading).minWidth)
    expect(getComputedStyle(title).overflowWrap).toBe('anywhere')
  })
})
