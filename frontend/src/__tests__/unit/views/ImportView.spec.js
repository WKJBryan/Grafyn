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
      total_items: 1,
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
      ],
      items: [
        {
          id: 'interview-1',
          title: 'Interview Transcript',
          platform: 'interview',
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
      semantic_link_suggestions: [],
      message: 'Imported 1 content item as evidence notes'
    })
  })

  it('previews and imports labeled interview transcripts', async () => {
    const wrapper = mountView()

    expect(wrapper.text()).toContain('Import Content')
    expect(wrapper.text()).toContain('Markdown, TXT, DOCX, PDF')
    await wrapper.find('[data-guide="import-file-btn"]').trigger('click')
    await flushPromises()

    expect(dialog.open).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.arrayContaining([
        expect.objectContaining({ extensions: expect.arrayContaining(['md', 'txt', 'docx', 'pdf']) })
      ])
    }))
    expect(api.importApi.preview).toHaveBeenCalledWith('C:/tmp/interview.md')
    expect(wrapper.text()).toContain('interview')
    expect(wrapper.text()).toContain('Interview Transcript')

    await wrapper.findAll('button').find(button => button.text().includes('Import 1 Selected')).trigger('click')
    await flushPromises()

    expect(api.importApi.apply).toHaveBeenCalledWith('C:/tmp/interview.md', ['interview-1'])
    expect(wrapper.text()).toContain('Imported 1 content item as evidence notes')
  })

  it('shows review-only semantic wikilink suggestions after document import', async () => {
    api.importApi.preview.mockResolvedValue({
      platform: 'document',
      total_items: 2,
      total_conversations: 2,
      items: [
        {
          id: 'doc-parent',
          title: 'Example Document',
          platform: 'document',
          messages: [{ role: 'system', content: '# Example Document' }],
          metadata: { model_info: ['document_index'] },
          suggested_tags: ['document']
        },
        {
          id: 'doc-section',
          title: 'Decision-Making Style',
          platform: 'document',
          messages: [{ role: 'source', content: 'Part of: [[Example Document]]' }],
          metadata: { model_info: ['document_section'] },
          suggested_tags: ['document']
        }
      ],
      conversations: []
    })
    api.importApi.apply.mockResolvedValue({
      imported: 2,
      skipped: 0,
      note_ids: ['note-parent', 'note-section'],
      errors: [],
      semantic_link_suggestions: [
        {
          from_title: 'Decision-Making Style',
          to_title: 'Example Document',
          reason: 'Section frames the document topic.'
        }
      ],
      message: 'Imported 2 content items as evidence notes (1 section note)'
    })

    const wrapper = mountView()
    await wrapper.find('[data-guide="import-file-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('2 content items found')
    expect(wrapper.text()).toContain('Decision-Making Style')

    await wrapper.findAll('button').find(button => button.text().includes('Import 2 Selected')).trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Semantic Wikilink Suggestions')
    expect(wrapper.text()).toContain('[[Decision-Making Style]]')
    expect(wrapper.text()).toContain('[[Example Document]]')
  })
})
