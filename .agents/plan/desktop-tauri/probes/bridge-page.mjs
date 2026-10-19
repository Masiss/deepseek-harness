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
page.on('console', (message) => { consoleMessages.push(`${message.type()}: ${message.text()}`) })
page.on('pageerror', (error) => { pageErrors.push(String(error)) })
page.on('request', (request) => {
  const path = new URL(request.url()).pathname
  if (path.startsWith('/__dsh-desktop')) bridgeCalls.push(`${request.method()} ${path}`)
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
  }
})

const errors = consoleMessages.filter(entry => entry.startsWith('error'))
console.log(`present=${String(observed.present)} protocol=${String(observed.protocolVersion)} keyboard=${String(observed.keyboard)}`)
console.log(`platform=${String(observed.platform)} updates_phase=${String(observed.updatesPhase)}`)
console.log(`device_info=${String(observed.deviceInfo)}`)
console.log(`shortcuts_status=${String(observed.shortcutsStatus)} sequence=${String(observed.shortcutsSequence)} saved=${String(observed.savedStatus)}`)
console.log(`client_bridge_calls=${JSON.stringify(bridgeCalls)}`)
console.log(`console_errors=${errors.length} ${JSON.stringify(errors.slice(0, 5))}`)
console.log(`page_errors=${pageErrors.length} ${JSON.stringify(pageErrors.slice(0, 5))}`)

await browser.close()
