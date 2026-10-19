/**
 * PTY probe: drive the application's own terminal in a real browser against the
 * Host the Tauri shell loads.
 *
 * This is the end-to-end path of the PTY invariant: keyboard input -> client
 * shortcut -> terminal RPC stream -> Host -> node-pty -> /dev/pts -> screen.
 *
 * Usage: node terminal-page.mjs <token-url>
 */

import { chromium } from '../../../../apps/web/node_modules/playwright/index.mjs'

const url = process.argv[2]
if (url === undefined) throw new Error('usage: terminal-page.mjs <token-url>')

const browser = await chromium.launch()
const page = await browser.newPage()
const consoleErrors = []
const failedRequests = []
const bridgePaths = []
const failedPaths = []
let bridgeInFlight = 0
let bridgePeak = 0
page.on('console', (message) => { if (message.type() === 'error') consoleErrors.push(message.text()) })
page.on('pageerror', (error) => { consoleErrors.push(String(error)) })
page.on('request', (request) => {
  const path = new URL(request.url()).pathname
  if (!path.startsWith('/__dsh-desktop')) return
  bridgePaths.push(path)
  bridgeInFlight += 1
  if (bridgeInFlight > bridgePeak) bridgePeak = bridgeInFlight
})
page.on('requestfinished', (request) => {
  if (new URL(request.url()).pathname.startsWith('/__dsh-desktop')) bridgeInFlight -= 1
})
page.on('requestfailed', (request) => {
  const path = new URL(request.url()).pathname
  if (path.startsWith('/__dsh-desktop')) bridgeInFlight -= 1
  failedPaths.push(path)
  failedRequests.push(`${request.failure()?.errorText ?? '?'} ${request.url()}`)
})

await page.goto(url, { waitUntil: 'load' })
await page.waitForTimeout(4000)

// A terminal tab needs a Session target, so open one first.
const newSession = page.getByText('New Session', { exact: false }).first()
if (await newSession.count() > 0) {
  await newSession.click({ timeout: 5000 }).catch(() => {})
  await page.waitForTimeout(2500)
}

// `terminal.new` default on desktop:linux and web:linux.
await page.keyboard.press('Control+Backquote')
await page.waitForTimeout(4000)

const terminalCount = await page.locator('.xterm').count()
console.log(`xterm_present=${String(terminalCount > 0)}`)

if (terminalCount > 0) {
  await page.locator('.xterm').first().click({ timeout: 5000 }).catch(() => {})
  await page.keyboard.type('echo PTY_OK; tty; echo ROWS=$(tput lines) COLS=$(tput cols)')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(3000)
  const screen = await page.locator('.xterm-rows').first().innerText().catch(() => '')
  console.log(`terminal_text=${JSON.stringify(screen.slice(0, 400))}`)

  // Signal delivery: an interrupted foreground process must return the prompt.
  await page.keyboard.type('sleep 30')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(1500)
  await page.keyboard.press('Control+c')
  await page.waitForTimeout(1500)
  await page.keyboard.type('echo AFTER_SIGNAL')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(2000)
  const afterSignal = await page.locator('.xterm-rows').first().innerText().catch(() => '')
  console.log(`signal_text=${JSON.stringify(afterSignal.slice(-220))}`)
} else {
  const shell = await page.evaluate(() => document.body.innerText.slice(0, 300))
  console.log(`page_text=${JSON.stringify(shell)}`)
}

console.log(`console_errors=${String(consoleErrors.length)} ${JSON.stringify(consoleErrors.slice(0, 3))}`)
console.log(`bridge_requests=${String(bridgePaths.length)} peak_in_flight=${String(bridgePeak)} failed=${String(failedPaths.length)}`)
console.log(`failed_requests=${String(failedRequests.length)} ${JSON.stringify(failedRequests.slice(0, 4))}`)
await browser.close()
