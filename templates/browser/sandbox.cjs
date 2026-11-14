// SPDX-License-Identifier: FSL-1.1-ALv2
/** Initial browser policy comes from the agent's private runtime directory. Missing legacy
 * policy and process-tier invocations always retain Chromium's sandbox. */
const {readFileSync}=require('node:fs')
const {join}=require('node:path')
module.exports=function sandboxed(root){
  if(Object.hasOwn(process.env,'PRYX_PROCESS_POLICY'))return true
  try{return JSON.parse(readFileSync(join(root,'browser-access.json'),'utf8')).chromiumSandbox!==false}
  catch{return true}
}
