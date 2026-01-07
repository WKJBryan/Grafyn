/**
 * E2E Tests: API Integration
 *
 * Tests API endpoints directly to ensure backend functionality
 */

import { test, expect } from '@playwright/test'

const API_BASE = 'http://localhost:8080/api'

test.describe('API Integration', () => {
  test.describe('Notes API', () => {
    test('GET /notes should return notes list', async ({ request }) => {
      const response = await request.get(`${API_BASE}/notes`)

      expect(response.ok()).toBe(true)
      expect(response.status()).toBe(200)

      const notes = await response.json()
      expect(Array.isArray(notes)).toBe(true)
    })

    test('POST /notes should create a note', async ({ request }) => {
      const noteData = {
        title: `API Test Note ${Date.now()}`,
        content: 'Created via API test',
        status: 'draft',
        tags: ['api', 'test'],
      }

      const response = await request.post(`${API_BASE}/notes`, {
        data: noteData,
      })

      expect(response.ok()).toBe(true)

      const created = await response.json()
      expect(created.title).toBe(noteData.title)
      expect(created.content).toBe(noteData.content)
      expect(created.id).toBeTruthy()

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(created.id)}`)
    })

    test('GET /notes/:id should return a specific note', async ({ request }) => {
      // First create a note
      const noteData = {
        title: `Get Test Note ${Date.now()}`,
        content: 'Test content',
        status: 'draft',
        tags: [],
      }

      const createResponse = await request.post(`${API_BASE}/notes`, {
        data: noteData,
      })
      const created = await createResponse.json()

      // Then get it
      const response = await request.get(
        `${API_BASE}/notes/${encodeURIComponent(created.id)}`
      )

      expect(response.ok()).toBe(true)

      const note = await response.json()
      expect(note.id).toBe(created.id)
      expect(note.title).toBe(noteData.title)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(created.id)}`)
    })

    test('PUT /notes/:id should update a note', async ({ request }) => {
      // Create a note
      const createResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: `Update Test Note ${Date.now()}`,
          content: 'Original content',
          status: 'draft',
          tags: [],
        },
      })
      const created = await createResponse.json()

      // Update it
      const updateData = {
        title: 'Updated Title',
        content: 'Updated content',
        status: 'canonical',
        tags: ['updated'],
      }

      const response = await request.put(
        `${API_BASE}/notes/${encodeURIComponent(created.id)}`,
        { data: updateData }
      )

      expect(response.ok()).toBe(true)

      const updated = await response.json()
      expect(updated.title).toBe(updateData.title)
      expect(updated.status).toBe(updateData.status)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(created.id)}`)
    })

    test('DELETE /notes/:id should delete a note', async ({ request }) => {
      // Create a note
      const createResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: `Delete Test Note ${Date.now()}`,
          content: 'To be deleted',
          status: 'draft',
          tags: [],
        },
      })
      const created = await createResponse.json()

      // Delete it
      const response = await request.delete(
        `${API_BASE}/notes/${encodeURIComponent(created.id)}`
      )

      expect(response.ok()).toBe(true)

      // Verify it's gone
      const getResponse = await request.get(
        `${API_BASE}/notes/${encodeURIComponent(created.id)}`
      )
      expect(getResponse.ok()).toBe(false)
    })

    test('should handle special characters in note IDs', async ({ request }) => {
      const noteData = {
        title: 'Special/Characters?Test',
        content: 'Testing special chars',
        status: 'draft',
        tags: [],
      }

      const createResponse = await request.post(`${API_BASE}/notes`, {
        data: noteData,
      })

      if (createResponse.ok()) {
        const created = await createResponse.json()

        // Get with encoded ID
        const getResponse = await request.get(
          `${API_BASE}/notes/${encodeURIComponent(created.id)}`
        )
        expect(getResponse.ok()).toBe(true)

        // Cleanup
        await request.delete(`${API_BASE}/notes/${encodeURIComponent(created.id)}`)
      }
    })
  })

  test.describe('Search API', () => {
    test('GET /search should return search results', async ({ request }) => {
      // Create a note to search for
      const createResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: `Searchable Note ${Date.now()}`,
          content: 'This contains the word unique12345',
          status: 'draft',
          tags: [],
        },
      })
      const created = await createResponse.json()

      // Wait for indexing
      await new Promise(resolve => setTimeout(resolve, 1000))

      // Search
      const response = await request.get(`${API_BASE}/search?q=unique12345`)

      expect(response.ok()).toBe(true)

      const results = await response.json()
      expect(Array.isArray(results)).toBe(true)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(created.id)}`)
    })

    test('GET /search should respect limit parameter', async ({ request }) => {
      const response = await request.get(`${API_BASE}/search?q=test&limit=3`)

      expect(response.ok()).toBe(true)

      const results = await response.json()
      expect(results.length).toBeLessThanOrEqual(3)
    })

    test('GET /search/similar should return similar notes', async ({ request }) => {
      // Create notes
      const note1Response = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Machine Learning Intro',
          content: 'Neural networks and deep learning',
          status: 'draft',
          tags: [],
        },
      })
      const note1 = await note1Response.json()

      const note2Response = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Deep Learning Guide',
          content: 'Advanced neural network architectures',
          status: 'draft',
          tags: [],
        },
      })
      const note2 = await note2Response.json()

      // Wait for indexing
      await new Promise(resolve => setTimeout(resolve, 1000))

      // Get similar
      const response = await request.get(
        `${API_BASE}/search/similar/${encodeURIComponent(note1.id)}`
      )

      expect(response.ok()).toBe(true)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(note1.id)}`)
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(note2.id)}`)
    })
  })

  test.describe('Graph API', () => {
    test('GET /graph/backlinks should return backlinks', async ({ request }) => {
      // Create target note
      const targetResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Backlink Target',
          content: 'Target content',
          status: 'draft',
          tags: [],
        },
      })
      const target = await targetResponse.json()

      // Create note with wikilink
      const sourceResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Backlink Source',
          content: 'Links to [[Backlink Target]]',
          status: 'draft',
          tags: [],
        },
      })
      const source = await sourceResponse.json()

      // Get backlinks
      const response = await request.get(
        `${API_BASE}/graph/backlinks/${encodeURIComponent(target.id)}`
      )

      expect(response.ok()).toBe(true)

      const backlinks = await response.json()
      expect(Array.isArray(backlinks)).toBe(true)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(target.id)}`)
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(source.id)}`)
    })

    test('GET /graph/outgoing should return outgoing links', async ({ request }) => {
      // Create notes
      const note1Response = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Link Source Note',
          content: 'Links to [[Link Target A]] and [[Link Target B]]',
          status: 'draft',
          tags: [],
        },
      })
      const note1 = await note1Response.json()

      // Get outgoing
      const response = await request.get(
        `${API_BASE}/graph/outgoing/${encodeURIComponent(note1.id)}`
      )

      expect(response.ok()).toBe(true)

      const outgoing = await response.json()
      expect(Array.isArray(outgoing)).toBe(true)

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(note1.id)}`)
    })

    test('GET /graph/neighbors should return graph neighbors', async ({ request }) => {
      // Create a note
      const noteResponse = await request.post(`${API_BASE}/notes`, {
        data: {
          title: 'Neighbor Test',
          content: 'Test content',
          status: 'draft',
          tags: [],
        },
      })
      const note = await noteResponse.json()

      // Get neighbors
      const response = await request.get(
        `${API_BASE}/graph/neighbors/${encodeURIComponent(note.id)}?depth=1`
      )

      expect(response.ok()).toBe(true)

      const result = await response.json()
      expect(result).toHaveProperty('nodes')
      expect(result).toHaveProperty('edges')

      // Cleanup
      await request.delete(`${API_BASE}/notes/${encodeURIComponent(note.id)}`)
    })

    test('POST /graph/rebuild should rebuild graph index', async ({ request }) => {
      const response = await request.post(`${API_BASE}/graph/rebuild`)

      expect(response.ok()).toBe(true)
    })
  })

  test.describe('Error Handling', () => {
    test('should return 404 for non-existent note', async ({ request }) => {
      const response = await request.get(
        `${API_BASE}/notes/non-existent-note-id-12345`
      )

      expect(response.ok()).toBe(false)
      expect(response.status()).toBe(404)
    })

    test('should return 422 for invalid note data', async ({ request }) => {
      const response = await request.post(`${API_BASE}/notes`, {
        data: {
          // Missing required title
          content: 'Content without title',
        },
      })

      expect(response.ok()).toBe(false)
      // Could be 400 or 422 depending on validation
      expect([400, 422]).toContain(response.status())
    })

    test('should handle malformed JSON', async ({ request }) => {
      const response = await request.post(`${API_BASE}/notes`, {
        headers: { 'Content-Type': 'application/json' },
        data: 'not valid json{',
      })

      expect(response.ok()).toBe(false)
    })
  })

  test.describe('Rate Limiting', () => {
    test('should handle rapid requests', async ({ request }) => {
      // Make several rapid requests
      const promises = []
      for (let i = 0; i < 10; i++) {
        promises.push(request.get(`${API_BASE}/notes`))
      }

      const responses = await Promise.all(promises)

      // Should mostly succeed (rate limiting might kick in)
      const successCount = responses.filter(r => r.ok()).length
      expect(successCount).toBeGreaterThan(0)
    })
  })
})
