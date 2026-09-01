const IMAGE_MIME_TYPES = new Set(['image/jpeg', 'image/png', 'image/webp'])

export class GeneratedImageError extends Error {
  constructor(code, message, cause = null) {
    super(message)
    this.name = 'GeneratedImageError'
    this.code = code
    this.cause = cause
  }
}

export function createGeneratedImageUrl(preview, urlApi = URL) {
  if (
    !preview ||
    !IMAGE_MIME_TYPES.has(preview.mediaType) ||
    typeof preview.base64Data !== 'string' ||
    !preview.base64Data
  ) {
    throw new GeneratedImageError('INVALID_PREVIEW', 'Generated image preview is invalid.')
  }

  let binary
  try {
    binary = atob(preview.base64Data)
  } catch (cause) {
    throw new GeneratedImageError('INVALID_PREVIEW', 'Generated image preview is invalid.', cause)
  }

  const bytes = Uint8Array.from(binary, character => character.charCodeAt(0))
  return urlApi.createObjectURL(new Blob([bytes], { type: preview.mediaType }))
}

export function revokeGeneratedImageUrl(url, urlApi = URL) {
  if (typeof url === 'string' && url.startsWith('blob:')) {
    urlApi.revokeObjectURL(url)
  }
}

export function formatGeneratedImageCost(cost) {
  return cost?.status === 'exact_usd' && typeof cost.usd === 'string'
    ? `US$${cost.usd} exact`
    : 'Cost unavailable'
}

export function toGeneratedImageError(
  error,
  { online = globalThis.navigator?.onLine !== false } = {},
) {
  if (error instanceof GeneratedImageError) return error

  const rawMessage = typeof error === 'string' ? error : error?.message || 'Image generation failed.'
  const normalized = rawMessage.toLowerCase()

  if (/missing or expired/.test(normalized)) {
    return new GeneratedImageError(
      'RECEIPT_UNAVAILABLE',
      'This generation receipt is unavailable. Discard it and generate a new preview.',
      error,
    )
  }

  if (
    /do not re-?try|partially committed|authority advanced|recovery (?:is |remains )?pending|committed.*(?:pending|recovered)/
      .test(normalized)
  ) {
    return new GeneratedImageError(
      'COMMIT_UNCERTAIN',
      'The image save may already be committed. Do not retry until Grafyn recovers.',
      error,
    )
  }

  if (!online) {
    return new GeneratedImageError(
      'OFFLINE',
      'Image generation is unavailable while offline.',
      error,
    )
  }

  if (
    /failed to fetch|failed to (?:send|query)|failed while reading|could not (?:read|query)|timed? out|timeout|error sending request|connect(?:ion|ing)?/
      .test(normalized)
  ) {
    return new GeneratedImageError(
      'NETWORK',
      'The image provider could not be reached. Check your connection and try again.',
      error,
    )
  }

  if (/api key|not configured|unconfigured/.test(normalized)) {
    return new GeneratedImageError(
      'UNCONFIGURED',
      'Configure an OpenRouter API key before generating images.',
      error,
    )
  }

  return new GeneratedImageError('PROVIDER_ERROR', rawMessage, error)
}
