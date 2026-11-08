// SPDX-License-Identifier: FSL-1.1-ALv2
/** Screenshots delegate navigation, rendering and PNG encoding to official Playwright. */
const {chromium}=require('playwright-core')
;(async()=>{
  const browser=await chromium.connectOverCDP(`http://127.0.0.1:${process.env.PRYX_CDP_PORT}`)
  try{
    const page=await browser.contexts()[0].newPage()
    try{await page.goto(process.argv[2],{waitUntil:'load'});await page.screenshot({path:process.argv[3],type:'png'})}
    finally{await page.close()}
  }finally{await browser.close()}
})().catch(()=>{console.error('Browser screenshot failed');process.exit(1)})
