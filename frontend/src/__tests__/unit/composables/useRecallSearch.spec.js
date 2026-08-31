import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useRecallSearch } from '@/composables/useRecallSearch'

function deferred() {
  let resolve
  let reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function mountSearch(search) {
  let state
  const wrapper = mount(defineComponent({
    setup() {
      state = useRecallSearch({ search, debounceMs: 300, limit: 5 })
      return state
    },
    template: '<div :data-status="status">{{ results.map(item => item.title).join(",") }}</div>',
  }))
  return { wrapper, state }
}

describe('useRecallSearch', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('does not search blank input and debounces the final nonblank query', async () => {
    const search = vi.fn().mockResolvedValue([])
    const { wrapper, state } = mountSearch(search)

    state.query.value = '   '
    await vi.advanceTimersByTimeAsync(300)
    expect(search).not.toHaveBeenCalled()

    state.query.value = 'first'
    await vi.advanceTimersByTimeAsync(200)
    state.query.value = 'final'
    await vi.advanceTimersByTimeAsync(299)
    expect(search).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    await flushPromises()

    expect(search).toHaveBeenCalledOnce()
    expect(search).toHaveBeenCalledWith('final', 5)
    wrapper.unmount()
  })

  it('normalizes wrapped snake-case and camel-case backend results', async () => {
    const search = vi.fn().mockResolvedValue({
      results: [
        {
          note_id: 'note-1',
          title: 'First',
          content: 'Body one',
          relevance_score: 0.7,
          connection_type: 'graph',
          tags: ['work'],
        },
        {
          noteId: 'note-2',
          title: 'Second',
          snippet: 'Body two',
          totalScore: 0.8,
          graphBoost: 0.2,
        },
      ],
    })
    const { wrapper, state } = mountSearch(search)

    state.query.value = 'body'
    await vi.advanceTimersByTimeAsync(300)
    await flushPromises()

    expect(state.status.value).toBe('ready')
    expect(state.results.value).toEqual([
      {
        note_id: 'note-1',
        title: 'First',
        snippet: 'Body one',
        score: 0.7,
        total_score: 0.7,
        graph_boost: 1,
        tags: ['work'],
      },
      {
        note_id: 'note-2',
        title: 'Second',
        snippet: 'Body two',
        score: 0.8,
        total_score: 0.8,
        graph_boost: 0.2,
        tags: [],
      },
    ])
    wrapper.unmount()
  })

  it('ignores an older response that finishes after a newer query', async () => {
    const first = deferred()
    const second = deferred()
    const search = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const { wrapper, state } = mountSearch(search)

    state.query.value = 'first'
    await vi.advanceTimersByTimeAsync(300)
    state.query.value = 'second'
    await vi.advanceTimersByTimeAsync(300)

    second.resolve([{ note_id: 'new', title: 'Newest', snippet: '', score: 1 }])
    await flushPromises()
    first.resolve([{ note_id: 'old', title: 'Stale', snippet: '', score: 1 }])
    await flushPromises()

    expect(state.results.value.map(item => item.note_id)).toEqual(['new'])
    expect(state.completedQuery.value).toBe('second')
    wrapper.unmount()
  })

  it('clears prior results and enters loading during a new query debounce', async () => {
    const search = vi.fn().mockResolvedValueOnce([
      { note_id: 'old', title: 'Old result', snippet: '', score: 1 },
    ])
    const { wrapper, state } = mountSearch(search)

    state.query.value = 'old'
    await vi.advanceTimersByTimeAsync(300)
    await flushPromises()
    expect(state.status.value).toBe('ready')

    state.query.value = 'new'

    expect(state.status.value).toBe('loading')
    expect(state.results.value).toEqual([])
    expect(state.completedQuery.value).toBe('')
    wrapper.unmount()
  })

  it('exposes a safe error without leaking backend details', async () => {
    const search = vi.fn().mockRejectedValue(new Error('C:\\private\\vault'))
    const { wrapper, state } = mountSearch(search)

    state.query.value = 'private'
    await vi.advanceTimersByTimeAsync(300)
    await flushPromises()

    expect(state.status.value).toBe('error')
    expect(state.error.value).toBe('Recall is temporarily unavailable.')
    expect(state.error.value).not.toContain('private')
    wrapper.unmount()
  })
})
