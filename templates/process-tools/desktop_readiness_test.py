# SPDX-License-Identifier: FSL-1.1-ALv2
"""The actual adapter publication sequence must prepare metadata before accepting clients.

A second real Node process connects while listen() is returning, before the adapter can
execute its next statement. Display programs and HTTP request handlers are outside this test.
"""
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = r"""
const fs=require('node:fs'), http=require('node:http'), vm=require('node:vm');
const {join}=require('node:path'), {spawnSync}=require('node:child_process');
const assert=require('node:assert/strict');
const config={scratch:process.argv[2],address:'pryx-publication-'+process.pid};
const environment={DISPLAY:':0',XAUTHORITY:join(config.scratch,'Xauthority')};
let server, observed;
const testingHttp={...http,createServer(...args) {
  server=http.createServer(...args);
  const listen=server.listen.bind(server);
  server.listen=(...args)=>{
    const result=listen(...args);
    const child=spawnSync(process.execPath,['-e',`
      const fs=require('node:fs'),net=require('node:net'),path=require('node:path');
      const socket=net.createConnection({path:'\\0'+process.argv[1]});
      socket.on('connect',()=>{
        let ready;
        try {ready=JSON.parse(fs.readFileSync(path.join(process.argv[2],'desktop-ready.json'),'utf8'))}catch{}
        console.log(JSON.stringify({connected:true,ready}));socket.destroy();
      });
      socket.on('error',error=>{console.error(error.message);process.exit(1)});
    `,config.address,config.scratch],{encoding:'utf8',timeout:5000});
    assert.equal(child.status,0,child.stderr);
    observed=JSON.parse(child.stdout);
    return result;
  };
  return server;
}};
try {
  const source=fs.readFileSync(process.argv[1],'utf8');
  const start=source.indexOf('const server = http.createServer(');
  assert(start>=0,'Exercise the committed adapter publication sequence');
  vm.runInNewContext(source.slice(start),{http:testingHttp,config,environment,
    writeFileSync:fs.writeFileSync,join,process,phase:()=>{}});
  assert(observed.connected,'Use an actual private Unix listener');
  assert.deepEqual(observed.ready,{display:environment.DISPLAY,
    xauthority:environment.XAUTHORITY,uid:process.getuid()},
    'Private listener accepted a connection before complete desktop readiness was published');
} finally {server?.close()}
"""


class DesktopPublication(unittest.TestCase):
    def test_connected_client_already_has_complete_readiness_metadata(self):
        adapter = Path(__file__).parent / 'desktop/start.mjs'
        with tempfile.TemporaryDirectory(prefix='pryx-desktop-publication-') as scratch:
            result = subprocess.run(['node', '-e', SCRIPT, str(adapter), scratch],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
