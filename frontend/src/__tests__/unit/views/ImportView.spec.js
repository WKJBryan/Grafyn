import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import ImportView from '@/views/ImportView.vue'

const dialog = vi.hoisted(() => ({
  open: vi.fn()
}))

const api = vi.hoisted(() => ({
  importApi: {
    preview: vi.fn(),
    apply: vi.fn()
  },
  zettelkasten: {
    discoverLinks: vi.fn(),
    applyLinks: vi.fn()
  },
  notes: {
    get: vi.fn()
  }
}))

vi.mock('@tauri-apps/api/dialog', () => dialog)
vi.mock('@/api/client', () => api)
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    success: vi.fn(),
    error: vi.fn()
  })
}))

function mountView() {
  return mount(ImportView, {
    global: {
      stubs: {
        RouterLink: {
          props: ['to'],
          template: '<a><slot /></a>'
        }
      }
    }
  })
}

describe('ImportView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    dialog.open.mockResolvedValue('C:/tmp/interview.md')
    api.importApi.preview.mockResolvedValue({
      platform: 'interview',
      total_conversations: 1,
      conversations: [
        {
          id: 'interview-1',
          title: 'Interview Transcript',
          messages: [
            { role: 'user', content: 'How do you decide?' },
            { role: 'interviewee', content: 'I need a demo.' }
          ],
          metadata: {},
          suggested_tags: ['interview', 'evidence']
        }
      ]
    })
    api.importApi.apply.mockResolvedValue({
      imported: 1,
      skipped: 0,
      note_ids: ['note-1'],
      errors: [],
      message: 'Imported 1 conversation as evidence notes'
    })
  })

  it('previews and imports labeled interview transcripts', async () => {
    const wrapper = mountView()

    expect(wrapper.text()).toContain('labeled .md/.txt/.docx interview transcripts')
    await wrapper.find('[data-guide="import-file-btn"]').trigger('click')
    await flushPromises()

    expect(dialog.open).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.arrayContaining([
        expect.objectContaining({ extensions: expect.arrayContaining(['md', 'txt', 'docx']) })
      ])
    }))
    expect(api.importApi.preview).toHaveBeenCalledWith('C:/tmp/interview.md')
    expect(wrapper.text()).toContain('interview')
    expect(wrapper.text()).toContain('Interview Transcript')

    await wrapper.findAll('button').find(button => button.text().includes('Import 1 Selected')).trigger('click')
    await flushPromises()

    expect(api.importApi.apply).toHaveBeenCalledWith('C:/tmp/interview.md', ['interview-1'])
    expect(wrapper.text()).toContain('Imported 1 conversation as evidence notes')
  })
})
