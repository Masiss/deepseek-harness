/**
 * P2 page probe: load the served application in a real browser and observe the
 * desktop bridge from inside the page.
 *
 * The bridge is Host-served, so the assertions here are the same ones the Tauri
 * WebView would satisfy: the global exists before client modules read it, the
 * page runs in desktop runtime, the client itself calls the shortcut document
 * instead of browser storage, and no console error reports a missing desktop
 * API.
 *
 * Usage: node bridge-page.mjs <token-url>
 */

import { chromium } from '../../../../apps/web/node_modules/playwright/index.mjs'

const url = process.argv[2]
if (url === undefined) throw new Error('usage: bridge-page.mjs <token-url>')

const browser = await chromium.launch()
const page = await browser.newPage()
const consoleMessages = []
const pageErrors = []
const bridgeCalls = []
const analyticsCalls = []
page.on('console', (message) => { consoleMessages.push(`${message.type()}: ${message.text()}`) })
page.on('pageerror', (error) => { pageErrors.push(String(error)) })
page.on('request', (request) => {
  const path = new URL(request.url()).pathname
  if (path.startsWith('/__dsh-desktop')) bridgeCalls.push(`${request.method()} ${path}`)
  // Product analytics opens its policy stream only when `dshDesktop` exists.
  if (path.includes('productAnalytics')) analyticsCalls.push(`${request.method()} ${path}`)
})

await page.goto(url, { waitUntil: 'load' })
await page.waitForTimeout(6000)

const observed = await page.evaluate(async () => {
  const bridge = globalThis.dshDesktop
  const deviceInfo = bridge === undefined ? null : await bridge.deviceInfo()
  const updates = bridge === undefined ? null : await bridge.updates.status()
  const definitions = [{ id: 'app.palette', defaults: {} }]
  const shortcuts = bridge === undefined ? null : await bridge.shortcuts.get(definitions)
  // The Settings editor saves through this same call; read and write in one
  // evaluate so no concurrent client read invalidates the revision.
  const saved = bridge === undefined || shortcuts === null
    ? null
    : await bridge.shortcuts.edit(
      { type: 'set', id: 'app.palette', binding: { code: 'KeyK', modifiers: ['control'] } },
      shortcuts.revision,
    )
  return {
    present: bridge !== undefined,
    protocolVersion: bridge?.protocolVersion ?? null,
    keyboard: bridge?.keyboard !== undefined,
    platform: document.documentElement.dataset.platform ?? null,
    deviceInfo,
    updatesPhase: updates?.phase ?? null,
    shortcutsStatus: shortcuts?.status ?? null,
    shortcutsSequence: shortcuts?.sequence ?? null,
    savedStatus: saved?.status ?? null,
    // What the user actually meets on a home with no configured credential.
    onboardingStep: document.querySelector('[data-desktop-onboarding]')?.getAttribute('data-desktop-onboarding') ?? null,
    onboardingVisible: document.querySelector('[data-desktop-onboarding]') !== null,
    workspaceComposer: document.querySelector('[data-composer-textarea], textarea, [contenteditable="true"]') !== null,
  }
})

const errors = consoleMessages.filter(entry => entry.startsWith('error'))
console.log(`present=${String(observed.present)} protocol=${String(observed.protocolVersion)} keyboard=${String(observed.keyboard)}`)
console.log(`platform=${String(observed.platform)} updates_phase=${String(observed.updatesPhase)}`)
console.log(`device_info=${String(observed.deviceInfo)}`)
console.log(`shortcuts_status=${String(observed.shortcutsStatus)} sequence=${String(observed.shortcutsSequence)} saved=${String(observed.savedStatus)}`)
console.log(`onboarding_visible=${String(observed.onboardingVisible)} step=${String(observed.onboardingStep)} composer=${String(observed.workspaceComposer)}`)
console.log(`client_bridge_get_calls=${String(bridgeCalls.filter(entry => entry.includes('shortcuts-get')).length)} client_bridge_calls=${String(bridgeCalls.length)}`)
// The policy stream multiplexes over /api/remote.mux, so a zero here proves
// nothing; the gate itself is the literal `'dshDesktop' in globalThis` at
// packages/client/product-analytics/src/client/index.ts:26.
console.log(`analytics_http_calls=${String(analyticsCalls.length)}`)
console.log(`console_errors=${errors.length} ${JSON.stringify(errors.slice(0, 5))}`)
console.log(`page_errors=${pageErrors.length} ${JSON.stringify(pageErrors.slice(0, 5))}`)

await browser.close()
