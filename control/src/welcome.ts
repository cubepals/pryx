// SPDX-License-Identifier: FSL-1.1-ALv2
/** Self-contained install welcome UI. Public HTML contains version, optional versioned guides and key location;
 * authenticated details are fetched on demand. Credentials live only in the tab's closure. */

import { createHash } from 'node:crypto'
import type { Env, Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import type { Db } from './database.ts'
import { PRYX_VERSION } from './generated/release.ts'
import type { Isolation } from './isolation.ts'
import { authenticate } from './keys.ts'
import { welcomeConnectGuides } from './site-address.ts'
import { welcomeExamples } from './welcome-examples.ts'
import { WELCOME_DETAILS_PATH } from './welcome-path.ts'

const locations: Record<string, string> = {
  render: 'PRYX_BOOTSTRAP_API_KEY in Render → service → Environment',
  railway: 'PRYX_BOOTSTRAP_API_KEY in Railway → service → Variables',
  coolify: 'SERVICE_PASSWORD_64_PRYX in Coolify → service → Configuration → Environment Variables',
}
const script = `document.querySelector('form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const input = document.querySelector('input');
  const key = input.value; input.value = '';
  const status = document.querySelector('[role="status"]');
  const details = document.querySelector('#details'); details.replaceChildren();
  try {
    const response = await fetch(${JSON.stringify(WELCOME_DETAILS_PATH)}, {headers: {Authorization: 'Bearer ' + key}, cache: 'no-store', credentials: 'omit', redirect: 'error'});
    if (!response.ok) throw new Error('A valid, active API key is required.');
    const data = await response.json();
    const report = document.createElement('p'); report.textContent = data.capabilities; details.append(report);
    for (const example of data.examples) {
      const section = document.createElement('section');
      const title = document.createElement('h2'); title.textContent = example.title;
      const setup = document.createElement('p'); setup.textContent = example.setup;
      const pre = document.createElement('pre'); const code = document.createElement('code'); code.textContent = example.code; pre.append(code);
      const copy = document.createElement('button'); copy.textContent = 'Copy'; copy.type = 'button';
      copy.addEventListener('click', async () => {try {await navigator.clipboard.writeText(example.code); copy.textContent = 'Copied';} catch {status.textContent = 'Select and copy the block.';}});
      section.append(title, setup, pre, copy); details.append(section);
    }
    status.textContent = 'Connected. Blocks reference PRYX_API_KEY; they contain no key.';
  } catch (error) { status.textContent = error.message; }
});`
const scriptHash = createHash('sha256').update(script).digest('base64')
const htmlText = (value: string) =>
  value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;')

export function withWelcome<E extends Env>(
  app: Hono<E>,
  options: {
    db: Db
    origin?: string
    siteUrl?: string
    isolation?: Isolation
    platform?: string
    platformKey?: boolean
    boundary?: string
  },
) {
  const guides = welcomeConnectGuides(options.siteUrl)
  const guideLinks = guides.length
    ? `<nav aria-label="Connect guides"><p>${guides.map((guide) => `<a rel="noreferrer" href="${htmlText(guide.url)}">${htmlText(guide.title)}</a>`).join(' · ')}</p></nav>`
    : ''
  const location = options.platformKey
    ? (locations[options.platform ?? ''] ?? 'PRYX_BOOTSTRAP_API_KEY in the service environment settings')
    : 'the first successful startup in the service log (apiKey); on a server, the initial pryx up output'
  app.use(WELCOME_DETAILS_PATH, async (c, next) => {
    c.header('Cache-Control', 'no-store')
    c.header('Referrer-Policy', 'no-referrer')
    await next()
  })
  app.get('/', (c) => {
    c.header('Cache-Control', 'no-store')
    c.header('Referrer-Policy', 'no-referrer')
    c.header('X-Content-Type-Options', 'nosniff')
    c.header(
      'Content-Security-Policy',
      `default-src 'none'; script-src 'sha256-${scriptHash}'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`,
    )
    return c.html(
      `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>pryx runs here</title><style>:root{color-scheme:light dark;font:17px system-ui}body{max-width:54rem;margin:4rem auto;padding:0 1rem}input,button{font:inherit;padding:.6rem}pre{overflow:auto;padding:1rem;background:light-dark(#eee,#222)}section{margin:2rem 0}</style><h1>pryx runs here</h1><p>Version ${htmlText(PRYX_VERSION)}</p><p>Find this install's key in ${htmlText(location)}.</p>${guideLinks}<form><label>API key <input type="password" autocomplete="off" required maxlength="4096"></label> <button>Connect</button></form><p role="status" aria-live="polite"></p><main id="details"></main><script>${script}</script></html>`,
    )
  })
  app.get(WELCOME_DETAILS_PATH, async (c) => {
    const header = c.req.header('authorization') ?? ''
    const identity = header.startsWith('Bearer ') ? await authenticate(options.db, header.slice(7)) : null
    if (!identity || identity.expired)
      return c.json({ error: 'A valid, active API key is required' }, StatusCodes.UNAUTHORIZED)
    return c.json({
      capabilities: `Default tier: ${options.isolation ?? 'container'}.${options.boundary ? ` Boundary: ${options.boundary}.` : ''} Requests still undergo node admission and fail if their policy cannot be enforced. This release has no host probe report: additional tiers, namespaces, cgroup limits, KVM, browsers and desktops are unverified here. Process needs Landlock and seccomp; it cannot run container images. Process sleep retains memory only while its supervisor survives; snapshots retain files. Shared UID process workloads must be mutually trusted, and internet-enabled process workloads share host networking even with separate UIDs. Wasm uses a virtual kernel and shared sidecar; it refuses container images, and guest ingress is HTTP only. Container needs an engine and the host firewall policy. See the hosting guide for these boundaries.`,
      examples: welcomeExamples(options.origin ?? new URL(c.req.url).origin, options.isolation),
    })
  })
}
