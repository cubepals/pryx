// SPDX-License-Identifier: FSL-1.1-ALv2
/** Official Playwright owns Chromium and CDP; http-proxy owns HTTP/WebSocket forwarding.
 * The adapter adds a short-lived capability and rewrites discovery through the exposed prefix. */
const { chromium } = require('playwright-core')
const http = require('node:http')
const {HTTP_STATUS_BAD_GATEWAY,HTTP_STATUS_FORBIDDEN}=require('node:http2').constants
const { readFileSync } = require('node:fs')
const { timingSafeEqual } = require('node:crypto')
const httpProxy = require('http-proxy')
const { join } = require('node:path')
const root=process.env.PRYX_TOOL_DIRECTORY
const cdp=Number(process.env.PRYX_CDP_PORT)
const publicPort=Number(process.env.PRYX_BROWSER_PORT)
function access(request){
  try{
    const url=new URL(request.url,'http://localhost'),token=url.searchParams.get('token')??''
    const policy=JSON.parse(readFileSync(join(root,'browser-access.json'),'utf8'))
    const a=Buffer.from(token),b=Buffer.from(policy.token)
    if(policy.expiresAt*1000<=Date.now()||a.length!==b.length||!timingSafeEqual(a,b))return
    url.searchParams.delete('token')
    request.url=url.pathname+url.search
    return policy
  }catch{return}
}
async function main(){
  const browser=await chromium.launchPersistentContext(join(root,'chromium'),{headless:true,chromiumSandbox:true,args:[`--remote-debugging-port=${cdp}`,'--remote-debugging-address=127.0.0.1'],viewport:{width:1280,height:720}})
  const proxy=httpProxy.createProxyServer({target:`http://127.0.0.1:${cdp}`,ws:true,changeOrigin:true,followRedirects:false})
  proxy.on('error',(_error,_request,response)=>{if(response.writeHead)response.writeHead(HTTP_STATUS_BAD_GATEWAY);response.end()})
  const server=http.createServer(async(request,response)=>{
    const policy=access(request)
    if(!policy){response.writeHead(HTTP_STATUS_FORBIDDEN);response.end();return}
    response.setHeader('cache-control','no-store')
    if(request.url.split('?')[0].replace(/\/$/,'')==='/json/version'){
      try{
        const upstream=await fetch(`http://127.0.0.1:${cdp}/json/version`)
        const version=await upstream.json(),external=new URL(policy.url)
        external.protocol=external.protocol==='https:'?'wss:':'ws:'
        external.pathname=external.pathname.replace(/\/$/,'')+new URL(version.webSocketDebuggerUrl).pathname
        external.search=new URLSearchParams({token:policy.token}).toString()
        version.webSocketDebuggerUrl=external.toString()
        response.setHeader('content-type','application/json');response.end(JSON.stringify(version))
      }catch{response.writeHead(HTTP_STATUS_BAD_GATEWAY);response.end()}
    }else proxy.web(request,response)
  })
  server.on('upgrade',(request,socket,head)=>{
    if(!access(request)){socket.destroy();return}
    proxy.ws(request,socket,head)
  })
  server.listen(publicPort,'127.0.0.1')
  let stopping=false
  const stop=async()=>{stopping=true;server.close();proxy.close();await browser.close();process.exit(0)}
  process.on('SIGTERM',stop);process.on('SIGINT',stop)
  browser.on('close',()=>{if(!stopping)process.exit(1)})
}
main().catch(()=>{console.error('Sandboxed Chromium failed to start; check the host native-tools seccomp policy');process.exit(1)})
