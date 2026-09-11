import { expect, test } from '@playwright/test'
import { collectBrowserErrors, waitForInvokeResponse } from '../fixtures/browser-events.js'
import { runtimeClientFor } from '../fixtures/runtime-client.js'
import { verifyTwinEventSync } from './twin-event-sync.spec.js'

const IMAGE_PROMPT = 'A cobalt orchid mapped as a living systems diagram'
const IMAGE_ANNOTATION = 'A visual trace for the temporal capture.'
const CAPTURE_MARKER = 'E2E temporal orchid 4d91c7'
const TWIN_NAME = 'Grafyn E2E Twin'
const TWIN_ROLE = 'A reviewed memory companion'
const REVIEWED_CLAIM = 'answers that include concrete implementation details such as files, commands, tests, or code.'
const SIMULATION_PROMPT = 'How would I verify this implementation?'
const ADVISOR_PROMPT = 'Use our history and tell me which files, commands, tests, and code matter now.'
const CAPTURE_CONTEXT = Object.freeze({
  Person: 'Mina',
  Role: 'Collaborator',
  Relationship: 'Research partner',
  Environment: 'Workshop',
  Activity: 'System mapping',
  Goal: 'Design a resilient companion',
})
const IMPLEMENTATION_PROMPTS = [
  'Show me the exact files and code that implement this behavior.',
  'List the commands and tests that verify this behavior end to end.',
  'Explain which code evidence proves this implementation is correct.',
]

async function setOpenRouterOnline(online) {
  const response = await fetch('http://127.0.0.1:18891/__control', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${process.env.GRAFYN_E2E_RUNTIME_TOKEN}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ online }),
  })
  expect(response.ok).toBe(true)
  expect(await response.json()).toEqual({ ok: true, online })
}

async function waitForBoot(page) {
  await expect(page.locator('.startup-splash')).toHaveCount(0, { timeout: 30_000 })
}

async function fullNotes(client) {
  const metadata = await client.invoke('list_notes', {})
  return Promise.all(metadata.map(({ id }) => client.invoke('get_note', { id })))
}

async function assertCompactLayout(page, { rootSelector, controlSelectors }) {
  const measurements = await page.evaluate(({ rootSelector: selector, controlSelectors: controls }) => {
    const root = document.querySelector(selector)
    const nav = document.querySelector('.companion-nav')
    const links = [...document.querySelectorAll('.companion-nav-link')]
    let rootPaddingSource = ''
    let navPaddingSource = ''

    const matches = (element, selectorText) => selectorText
      .split(',')
      .some((candidate) => {
        try {
          return element?.matches(candidate.trim()) === true
        } catch {
          return false
        }
      })
    const visitRules = (rules) => {
      for (const rule of rules) {
        if (rule.selectorText && rule.style) {
          const padding = rule.style.getPropertyValue('padding')
          if (padding.includes('safe-area-inset') && matches(root, rule.selectorText)) {
            rootPaddingSource = padding
          }
          if (padding.includes('safe-area-inset') && matches(nav, rule.selectorText)) {
            navPaddingSource = padding
          }
        }
        if (rule.cssRules) visitRules(rule.cssRules)
      }
    }
    for (const sheet of document.styleSheets) {
      try {
        visitRules(sheet.cssRules)
      } catch {
        // All production styles are same-origin; ignore browser-owned sheets.
      }
    }

    const measure = (element) => {
      const rect = element?.getBoundingClientRect()
      const style = element ? getComputedStyle(element) : null
      return {
        visible: Boolean(
          rect
          && rect.width > 0
          && rect.height > 0
          && style?.display !== 'none'
          && style?.visibility !== 'hidden'
        ),
        height: rect?.height ?? 0,
      }
    }
    return {
      viewportWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
      viewportMeta: document.querySelector('meta[name="viewport"]')?.getAttribute('content') ?? '',
      rootPaddingSource,
      navPaddingSource,
      navCount: links.length,
      activeNavCount: links.filter((link) => link.getAttribute('aria-current') === 'page').length,
      navHeights: links.map((link) => link.getBoundingClientRect().height),
      controls: controls.map((controlSelector) => ({
        selector: controlSelector,
        ...measure(document.querySelector(controlSelector)),
      })),
      desktopOnlyCount: document.querySelectorAll(
        '.sidebar-left, .sidebar-right, .full-graph-container',
      ).length,
    }
  }, { rootSelector, controlSelectors })

  expect(measurements.scrollWidth).toBeLessThanOrEqual(measurements.viewportWidth)
  expect(measurements.viewportMeta).toContain('viewport-fit=cover')
  for (const inset of ['top', 'right', 'left']) {
    expect(measurements.rootPaddingSource).toContain(`env(safe-area-inset-${inset})`)
  }
  for (const inset of ['right', 'bottom', 'left']) {
    expect(measurements.navPaddingSource).toContain(`env(safe-area-inset-${inset})`)
  }
  expect(measurements.navCount).toBe(4)
  expect(measurements.activeNavCount).toBe(1)
  for (const height of measurements.navHeights) expect(height).toBeGreaterThanOrEqual(44)
  for (const control of measurements.controls) {
    expect(control.visible, `${control.selector} should be visible`).toBe(true)
    expect(control.height, `${control.selector} should be at least 44px tall`).toBeGreaterThanOrEqual(44)
  }
  expect(measurements.desktopOnlyCount).toBe(0)
  await expect(page.getByText('MCP Integration', { exact: true })).toHaveCount(0)
  await expect(page.getByText('Import Content', { exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /check for updates|install update/i })).toHaveCount(0)
}

async function expectMinimumTouchTarget(locator, label) {
  await expect(locator, `${label} should be visible`).toBeVisible()
  const box = await locator.boundingBox()
  expect(box, `${label} should have a layout box`).not.toBeNull()
  expect(box.height, `${label} should be at least 44px tall`).toBeGreaterThanOrEqual(44)
  return box
}

function expectedStubResponse(question, priorTurns, candidateMemory) {
  return `History-aware response; prior turns: ${priorTurns}; candidate memory: ${candidateMemory}. I connected “${question}” to the reviewed evidence in your twin.`
}

async function clearTwinChatHistory(client) {
  const value = await client.invoke('list_sessions', {})
  const sessions = Array.isArray(value) ? value : value?.sessions || []
  for (const session of sessions.filter((item) => item.tags?.includes('companion-twin-chat'))) {
    await client.invoke('delete_session', { id: session.id })
  }
}

async function matchingTwinState(client) {
  const referenceTime = new Date().toISOString()
  const [proposalPage, projection] = await Promise.all([
    client.invoke('list_twin_proposals', {
      request: {
        referenceTime,
        filter: { relationships: [], goals: [], tags: [] },
        cursor: null,
        limit: 50,
      },
    }),
    client.invoke('get_twin_state_projection', {
      request: { referenceTime },
    }),
  ])
  return {
    proposalPage,
    projection,
    proposals: proposalPage.items.filter((item) => item.claim?.object === REVIEWED_CLAIM),
    reviewed: projection.reviewed_memories.filter(
      (item) => item.claim?.object === REVIEWED_CLAIM,
    ),
  }
}

test('Pixel companion persists, recalls, governs, and syncs a temporal multimodal twin', async ({ page }) => {
  test.setTimeout(600_000)
  const browserErrors = collectBrowserErrors(page)
  const deviceA = runtimeClientFor('device-a', 'android')
  const deviceB = runtimeClientFor('device-b', 'android')

  await setOpenRouterOnline(true)
  await deviceA.invoke('save_constitution_setup', {
    setup: {
      twin_name: TWIN_NAME,
      twin_role: TWIN_ROLE,
      source_boundaries: [],
      values: [],
      tastes: [],
      constraints: [],
      somatic_cues: [],
      action_tendencies: [],
      updated_at: null,
    },
  })

  await page.goto('/?grafynE2EProfile=android')
  await waitForBoot(page)
  await expect(page.getByRole('heading', { name: 'Capture what is happening' })).toBeVisible()

  await page.locator('.image-toggle').click()
  await page.getByLabel('Image prompt').fill(IMAGE_PROMPT)
  await expect(page.getByLabel('Image model')).toContainText('Grafyn E2E Image')
  await page.getByLabel('Image model').selectOption('grafyn/e2e-image')
  await expect(page.getByLabel('Image resolution')).toContainText('1024x1024')
  await page.getByLabel('Image resolution').selectOption('1024x1024')
  await page.getByLabel('Image aspect ratio').selectOption('1:1')
  await page.getByRole('button', { name: 'Generate image', exact: true }).click()

  const preview = page.getByAltText('Generated preview')
  await expect(preview).toBeVisible()
  await page.getByLabel('Image sync policy').selectOption('inherit')
  await expect.poll(() => preview.evaluate((image) => [image.naturalWidth, image.naturalHeight]))
    .toEqual([2, 2])
  await page.getByLabel('Image annotation').fill(IMAGE_ANNOTATION)
  await expect(page.getByRole('button', { name: 'Save generated image as a desktop file' }))
    .toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Share generated image' })).toHaveCount(0)
  await page.getByRole('button', { name: 'Save image to Grafyn' }).click()
  await expect(page.locator('[data-test="image-save-status"]')).toContainText('queued')

  const generatedImageNote = (await fullNotes(deviceA)).find(
    (note) => note.properties?.generation_prompt === IMAGE_PROMPT,
  )
  expect(generatedImageNote).toBeTruthy()
  const attachmentDigest = generatedImageNote.properties.attachment_digests?.[0]
  expect(attachmentDigest).toMatch(/^[0-9a-f]{64}$/)

  await setOpenRouterOnline(false)
  let capturedBeforeRestart = (await fullNotes(deviceA)).find(
    (note) => note.content?.includes(CAPTURE_MARKER),
  )
  if (!capturedBeforeRestart) {
    await page.locator('.capture-context summary').click()
    for (const [label, value] of Object.entries(CAPTURE_CONTEXT)) {
      await page.getByLabel(label, { exact: true }).fill(value)
    }
    await page.getByPlaceholder('A thought, encounter, decision, or observation…').fill(
      `${CAPTURE_MARKER}: I noticed that context changes which memory deserves attention.`,
    )
    await page.getByRole('button', { name: 'Capture', exact: true }).click()
    await expect(page.locator('.capture-success')).toContainText('Captured as')
    capturedBeforeRestart = (await fullNotes(deviceA)).find(
      (note) => note.content?.includes(CAPTURE_MARKER),
    )
  }
  expect(capturedBeforeRestart).toBeTruthy()
  expect(capturedBeforeRestart.properties?.attachment_digests).toContain(attachmentDigest)
  await assertCompactLayout(page, {
    rootSelector: '.capture-view',
    controlSelectors: ['.quick-capture-submit'],
  })

  expect(await deviceA.invoke('e2e_restart_runtime', {})).toBe(true)
  await page.reload()
  await waitForBoot(page)
  await expect(page.getByRole('heading', { name: 'Capture what is happening' })).toBeVisible()

  await page.getByRole('link', { name: 'Recall' }).click()
  await page.getByPlaceholder('Search your notes…').fill(CAPTURE_MARKER)
  const recallResult = page.locator('.recall-result').filter({ hasText: CAPTURE_MARKER }).first()
  await expect(recallResult).toBeVisible()
  await expect(recallResult.locator('.recall-attention')).toContainText(/\S/)
  await recallResult.click()
  await expect(page.locator('.recall-detail-body')).toContainText(CAPTURE_MARKER)
  await assertCompactLayout(page, {
    rootSelector: '.recall-view',
    controlSelectors: ['.recall-back'],
  })

  const capturedAfterRestart = await deviceA.invoke('get_note', { id: capturedBeforeRestart.id })
  expect(capturedAfterRestart.properties?.attachment_digests).toContain(attachmentDigest)
  const persistedImage = await deviceA.invoke('load_generated_image', {
    request: { attachmentDigest },
  })
  expect(persistedImage).toMatchObject({
    attachmentDigest,
    mediaType: 'image/png',
    width: 2,
    height: 2,
  })

  await setOpenRouterOnline(true)
  await page.getByRole('link', { name: 'Canvas' }).click()
  await expect(page.getByLabel('Canvas model')).toContainText('Grafyn E2E Text')
  await page.getByLabel('Canvas model').selectOption('grafyn/e2e-text')
  const canvasTurns = []
  for (const prompt of IMPLEMENTATION_PROMPTS) {
    await page.getByLabel('Canvas prompt').fill(prompt)
    await page.locator('.canvas-composer').getByRole('button', { name: 'Send' }).click()
    const turn = page.locator('.thread-turn').filter({ hasText: prompt }).last()
    await expect(turn.locator('.model-response-card.completed')).toBeVisible({ timeout: 30_000 })
    await expect(turn.locator('.content-text')).toContainText('History-aware response')
    canvasTurns.push(turn)
  }

  const regenerateAction = canvasTurns[0].getByRole('button', {
    name: /^Regenerate .* grafyn\/e2e-text$/,
  })
  await expectMinimumTouchTarget(regenerateAction, 'Canvas Regenerate action')
  await expect(regenerateAction).toBeEnabled()
  const regenerateResponsePromise = waitForInvokeResponse(page, 'regenerate_response')
  await regenerateAction.click()
  const regenerateResponse = await regenerateResponsePromise
  await regenerateResponse.finished()
  expect(regenerateResponse.ok()).toBe(true)
  await expect(canvasTurns[0].locator('.model-response-card.completed')).toBeVisible({
    timeout: 30_000,
  })
  await expect(regenerateAction).toBeEnabled()

  await canvasTurns[0].getByRole('button', { name: /Accept .* grafyn\/e2e-text/ }).click()
  const afterUseful = await matchingTwinState(deviceA)
  expect(afterUseful.proposals).toHaveLength(0)
  expect(afterUseful.reviewed).toHaveLength(0)

  let proposal = null
  for (const [index, turn] of canvasTurns.entries()) {
    const captureAction = turn.getByRole('button', { name: /Capture Twin preference from/ })
    const captureBox = await expectMinimumTouchTarget(captureAction, 'Capture Twin preference action')
    const turnActionsBox = await turn.locator('.turn-actions').boundingBox()
    expect(turnActionsBox, 'Canvas turn actions should have a layout box').not.toBeNull()
    expect(captureBox.width, 'Capture Twin preference action should fill the response actions row')
      .toBeGreaterThanOrEqual(turnActionsBox.width - 1)
    await captureAction.click()
    const dialog = page.getByRole('dialog', { name: 'Capture Twin preference' })
    await expect(dialog).toContainText('This records evidence, not memory.')
    await expect(dialog).toContainText('three matching captures from three different responses')
    await expectMinimumTouchTarget(
      dialog.getByRole('button', { name: 'Cancel' }),
      'Preference dialog Cancel action',
    )
    await expectMinimumTouchTarget(
      dialog.getByRole('button', { name: 'Record evidence' }),
      'Preference dialog Record evidence action',
    )
    const evidence = dialog.getByLabel('Twin preference evidence')
    await expect(evidence).toHaveValue('')
    await evidence.fill(REVIEWED_CLAIM)
    await dialog.getByRole('button', { name: 'Record evidence' }).click()
    await expect(dialog).toHaveCount(0, { timeout: 30_000 })
    await expect(page.locator('.canvas-notice')).toHaveText(
      'Evidence recorded. More matching evidence may be needed before Grafyn proposes a Twin memory for review.',
    )

    const state = await matchingTwinState(deviceA)
    expect(state.reviewed).toHaveLength(0)
    if (index < 2) {
      expect(state.proposals).toHaveLength(0)
    } else {
      expect(state.proposals).toHaveLength(1)
      expect(state.projection.pending_proposals).toContainEqual(state.proposals[0])
      proposal = state.proposals[0]
      expect(proposal).toMatchObject({
        kind: 'pending_proposal',
        support_count: 3,
        proposal_event_id: null,
        claim: {
          subject_id: 'owner',
          predicate: 'prefers',
          object: REVIEWED_CLAIM,
          polarity: 'affirmed',
        },
      })
    }
  }
  await assertCompactLayout(page, {
    rootSelector: '.canvas-companion',
    controlSelectors: ['.canvas-composer .send-button'],
  })

  expect(proposal).toBeTruthy()
  const reviewedMemoryId = proposal.item_id

  await clearTwinChatHistory(deviceA)
  await page.getByRole('link', { name: 'Twin' }).click()
  const proposalCard = page.locator(`[data-proposal-id="${reviewedMemoryId}"]`)
  await expect(proposalCard).toContainText('concrete implementation details')

  await page.getByRole('button', { name: 'Open Twin chat' }).click()
  await page.getByLabel('Twin answer mode').selectOption('simulation')
  await page.getByLabel('Twin message').fill(SIMULATION_PROMPT)
  await page.locator('.chat-composer').getByRole('button', { name: 'Send' }).click()
  const simulationResponse = page.locator('.twin-chat .model-response-card.completed').last()
  await expect(simulationResponse.locator('.content-text')).toHaveText(expectedStubResponse(
    SIMULATION_PROMPT,
    0,
    'excluded',
  ), { timeout: 30_000 })
  await expect(page.locator('.simulation-disclosure').first()).toBeVisible()

  await page.getByRole('button', { name: 'Open Twin review' }).click()
  const acceptProposal = page.getByRole('button', { name: `Accept proposal ${reviewedMemoryId}` })
  await expect(acceptProposal).toBeEnabled({ timeout: 30_000 })
  await acceptProposal.click()
  await expect(page.locator(`[data-proposal-id="${reviewedMemoryId}"]`)).toHaveCount(0, {
    timeout: 30_000,
  })

  await page.getByRole('button', { name: 'Open Twin chat' }).click()
  await page.getByLabel('Twin answer mode').selectOption('advisor')
  await page.getByLabel('Twin message').fill(ADVISOR_PROMPT)
  await page.locator('.chat-composer').getByRole('button', { name: 'Send' }).click()
  const advisorResponse = page.locator('.twin-chat .model-response-card.completed').last()
  await expect(advisorResponse.locator('.content-text')).toHaveText(
    expectedStubResponse(ADVISOR_PROMPT, 2, 'included'),
    { timeout: 30_000 },
  )
  await assertCompactLayout(page, {
    rootSelector: '.twin-companion',
    controlSelectors: ['.chat-composer button[type="submit"]'],
  })

  await verifyTwinEventSync({
    deviceA,
    deviceB,
    marker: CAPTURE_MARKER,
    attachmentDigest,
    reviewedMemoryId,
    expectedContext: CAPTURE_CONTEXT,
    knownPlaintexts: [
      IMAGE_PROMPT,
      IMAGE_ANNOTATION,
      TWIN_NAME,
      TWIN_ROLE,
      REVIEWED_CLAIM,
      CAPTURE_MARKER,
      ...Object.keys(CAPTURE_CONTEXT),
      ...Object.values(CAPTURE_CONTEXT),
    ],
  })

  const getSettingsResponsePromise = waitForInvokeResponse(page, 'get_settings')
  await page.getByLabel('Open companion settings').click()
  const getSettingsResponse = await getSettingsResponsePromise
  await getSettingsResponse.finished()
  expect(getSettingsResponse.ok()).toBe(true)
  const settingsDialog = page.getByRole('dialog', { name: 'Settings' })
  await expect(settingsDialog).toBeVisible()
  await expect(page.getByText('MCP Integration')).toHaveCount(0)
  await expect(page.getByText('Import Content')).toHaveCount(0)
  await expect(page.getByRole('button', { name: /check for updates|install update/i })).toHaveCount(0)
  await expect(settingsDialog.getByRole('radio', { checked: true })).toHaveCount(1)
  const darkTheme = settingsDialog.getByRole('radio', { name: 'Dark' })
  const targetTheme = await darkTheme.isChecked()
    ? settingsDialog.getByRole('radio', { name: 'Light' })
    : darkTheme
  await expect(targetTheme).not.toBeChecked()
  const targetThemeControl = targetTheme.locator('..')
  await expectMinimumTouchTarget(targetThemeControl, 'Companion theme control')
  const updateSettingsResponsePromise = waitForInvokeResponse(page, 'update_settings')
  await targetThemeControl.click()
  const updateSettingsResponse = await updateSettingsResponsePromise
  await updateSettingsResponse.finished()
  expect(updateSettingsResponse.ok()).toBe(true)
  await expect(targetTheme).toBeChecked()
  await expect(settingsDialog.getByText('Saving…')).toHaveCount(0)
  await expect(settingsDialog.getByRole('alert')).toHaveCount(0)
  expect(browserErrors).toEqual([])
})
