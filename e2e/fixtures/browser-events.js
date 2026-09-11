export function collectBrowserErrors(page) {
  const errors = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(`console: ${message.text()}`)
  })
  page.on('pageerror', (error) => {
    errors.push(`pageerror: ${error?.message || String(error)}`)
  })
  return errors
}

export function waitForInvokeResponse(page, command) {
  return page.waitForResponse((response) => {
    const request = response.request()
    if (request.method() !== 'POST' || !response.url().endsWith('/invoke')) return false
    try {
      return request.postDataJSON()?.command === command
    } catch {
      return false
    }
  })
}
