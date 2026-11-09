// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise the exposed endpoint with the real Playwright CDP client, without logging its URL. */
const {chromium}=require('playwright-core')
;(async()=>{
  const browser=await chromium.connectOverCDP(process.argv[2])
  try{const page=await browser.contexts()[0].newPage();await page.goto('data:text/html,<title>cdp-ok</title>');if(await page.title()!=='cdp-ok')throw new Error('CDP page failed');await page.close()}
  finally{await browser.close()}
})().catch(()=>{console.error('Exposed CDP connection failed');process.exit(1)})
