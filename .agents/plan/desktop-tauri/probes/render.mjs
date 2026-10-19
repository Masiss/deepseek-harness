/**
 * Headless render probe: open the Desktop Host URL in headless Chromium and
 * report what the client rendered. Temporary P0 instrument.
 */
import { chromium } from 'playwright'

const url = process.argv[2]
if (url === undefined) {
  console.error('usage: node p0-render.mjs <url>')
  process.exit(2)
}

const browser = await chromium.launch()
const page = await browser.newPage()
const errors = []
page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
page.on('pageerror', (error) => { errors.push(String(error)) })

await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 90_000 })
await page.waitForFunction(() => globalThis.__DSH_BOOT__ !== undefined, null, { timeout: 90_000 })
const bootMs = Date.now()
// The client mounts asynchronously; wait until it has put real content on screen.
await page.waitForFunction(
  () => (document.body.innerText ?? '').trim().length > 0,
  null,
  { timeout: 90_000 },
)
const renderedMs = Date.now()

const metrics = await page.evaluate(() => {
  const paint = performance.getEntriesByName('first-contentful-paint')[0]
  const navigation = performance.getEntriesByType('navigation')[0]
  return {
    firstContentfulPaint: paint?.startTime ?? null,
    domContentLoaded: navigation?.domContentLoadedEventEnd ?? null,
  }
})

console.log(JSON.stringify({
  title: await page.title(),
  clientEntries: await page.evaluate(() => globalThis.__DSH_BOOT__?.entries?.length ?? null),
  bootMs,
  renderedMs,
  ...metrics,
  consoleErrors: errors.slice(0, 5),
  consoleErrorCount: errors.length,
  bodyHead: (await page.locator('body').innerText()).replace(/\s+/gu, ' ').slice(0, 160),
}, null, 2))

await browser.close()
