import { describe, expect, it, vi } from 'vitest'
import {
  GeneratedImageError,
  createGeneratedImageUrl,
  formatGeneratedImageCost,
  revokeGeneratedImageUrl,
  toGeneratedImageError,
} from '@/utils/generatedImage'

describe('generated image utilities', () => {
  it('creates a MIME-typed Blob URL from the exact decoded preview and revokes it', async () => {
    const urlApi = {
      createObjectURL: vi.fn(() => 'blob:grafyn-preview'),
      revokeObjectURL: vi.fn(),
    }

    const url = createGeneratedImageUrl({
      mediaType: 'image/png',
      base64Data: 'AQID',
    }, urlApi)

    expect(url).toBe('blob:grafyn-preview')
    expect(urlApi.createObjectURL).toHaveBeenCalledOnce()
    const blob = urlApi.createObjectURL.mock.calls[0][0]
    expect(blob).toBeInstanceOf(Blob)
    expect(blob.type).toBe('image/png')
    const bytes = await new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.addEventListener('load', () => resolve(new Uint8Array(reader.result)))
      reader.addEventListener('error', () => reject(reader.error))
      reader.readAsArrayBuffer(blob)
    })
    expect([...bytes]).toEqual([1, 2, 3])

    revokeGeneratedImageUrl(url, urlApi)
    expect(urlApi.revokeObjectURL).toHaveBeenCalledWith('blob:grafyn-preview')
  })

  it('formats exact provider cost without rounding and labels unavailable cost honestly', () => {
    expect(formatGeneratedImageCost({ status: 'exact_usd', usd: '0.004200' }))
      .toBe('US$0.004200 exact')
    expect(formatGeneratedImageCost({ status: 'unavailable' })).toBe('Cost unavailable')
  })

  it.each([
    ['Failed to fetch', { online: false }, 'OFFLINE'],
    ['Failed to send OpenRouter image request', { online: true }, 'NETWORK'],
    ['Image model capability query timed out', { online: true }, 'NETWORK'],
    ['Could not read image capability response', { online: true }, 'NETWORK'],
    ['OpenRouter API key is not configured', { online: true }, 'UNCONFIGURED'],
    ['Save committed but repair is pending; do not retry', { online: true }, 'COMMIT_UNCERTAIN'],
    ['Provider rejected the request', { online: true }, 'PROVIDER_ERROR'],
  ])('turns %s into the typed %s state', (message, environment, code) => {
    const error = toGeneratedImageError(message, environment)

    expect(error).toBeInstanceOf(GeneratedImageError)
    expect(error.code).toBe(code)
    expect(error.message).toBeTruthy()
  })

  it.each([
    ['Generated image save committed before a later failure; do not retry automatically', 'COMMIT_UNCERTAIN'],
    ['Generated image save partially committed before a network connection failed; do not retry', 'COMMIT_UNCERTAIN'],
    ['mutation authority advanced and durable recovery remains pending: digest', 'COMMIT_UNCERTAIN'],
    ['generated image save partially committed and recovery is pending', 'COMMIT_UNCERTAIN'],
    ['generated image receipt is missing or expired', 'RECEIPT_UNAVAILABLE'],
  ])('classifies terminal receipt outcome %s as %s', (message, code) => {
    expect(toGeneratedImageError(message, { online: true }).code).toBe(code)
  })
})
