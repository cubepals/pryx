# SPDX-License-Identifier: FSL-1.1-ALv2
"""The desktop front serves nothing without the current, unexpired access token.

A real Node process runs the committed access check and request handlers of desktop/start.mjs
in front of a stand-in for websockify on a Unix socket, which records every request that
reaches it. Display programs, websockify itself and the edge are outside this test; the hosted
desktop_application acceptance runs them with the same refusals. Failure evidence retains
phase, error and byte counts with generated fixture credentials redacted.
"""
from functools import partial
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler
import json
import os
from pathlib import Path
import re
import secrets
import socketserver
import subprocess
import sys
import tempfile
import threading
import traceback
import unittest


FRONT = r"""
const fs=require('node:fs'), http=require('node:http'), net=require('node:net'), vm=require('node:vm');
const {join}=require('node:path'), {constants}=require('node:http2');
const {createHmac,randomBytes,timingSafeEqual}=require('node:crypto');
const [adapter,scratch,web,listen]=process.argv.slice(1);
const fixture=JSON.parse(fs.readFileSync(join(scratch,'desktop-fixture.json'),'utf8'));
const privateToken=fixture.private, token=fixture.access;
const started=performance.now();
const trace={node:process.version,phase:'load-adapter',events:[]};
function redacted(value) {
  return String(value).replaceAll(privateToken,'[fixture-private]').replaceAll(token,'[fixture-access]')
    .replace(/([?&]token=)[^&\s'"<>]+/g,'$1[fixture]')
    .replace(/(pryx-desktop=)[^;\s'"<>]+/g,'$1[fixture]');
}
function problem(error) {
  return {name:error?.name??typeof error,code:error?.code??null,syscall:error?.syscall??null,
    message:redacted(error?.message??error)};
}
function record(phase,event,detail={}) {
  if (trace.events.length<256) trace.events.push({phase,event,clock:Date.now(),
    milliseconds:Math.round((performance.now()-started)*1000)/1000,...detail});
}
function retain() {
  try {fs.writeFileSync(join(scratch,'desktop-front-trace.json'),JSON.stringify(trace),{mode:0o600})}
  catch(error) {trace.writeError=problem(error)}
}
function phase(name) {trace.phase=name;record(name,'start');retain()}
process.on('uncaughtExceptionMonitor',error=>{record(trace.phase,'uncaught-exception',problem(error));retain()});
process.once('exit',retain);
const source=fs.readFileSync(adapter,'utf8');
function part(from,to) {
  const start=source.indexOf(from), end=source.indexOf(to,start);
  if (start<0||end<0) throw new Error('Exercise the committed adapter front: '+from);
  return source.slice(start,end);
}
const code=part('function equal(','let stopping')+
  part('const server = http.createServer(',"writeFileSync(join(config.scratch, 'desktop-ready.json')")+
  ';server.listen(listen);globalThis.server=server;';
const context={http,net,constants,web,listen,readFileSync:fs.readFileSync,join,Buffer,URL,setTimeout,
  clearTimeout,createHmac,randomBytes,timingSafeEqual,
  protocol:JSON.parse(fs.readFileSync(join(adapter,'..','protocol.json'),'utf8')),
  config:{scratch,privateToken,maxRequestBytes:65536,timeoutMs:5000},
  run:async name=>'ran '+name};
function grant(token,seconds) {
  fs.writeFileSync(join(scratch,'desktop-access.json'),
    JSON.stringify({token,expiresAt:Math.floor(Date.now()/1000)+seconds}));
}
function get(name,path,headers={},method='GET',body) {
  phase(name);
  return new Promise((resolve,reject)=>{
    const request=http.request({socketPath:listen,path,method,headers,agent:false},response=>{
      let text='';
      record(name,'http-response',{status:response.statusCode});retain();
      response.setEncoding('utf8').on('data',chunk=>{
        text+=chunk;record(name,'http-data',{bytes:Buffer.byteLength(chunk)});retain();
      }).on('end',()=>{
        record(name,'http-end',{complete:response.complete});retain();
        resolve({status:response.statusCode,body:text,cookie:response.headers['set-cookie']?.[0]??null});
      }).on('close',()=>{record(name,'http-close',{complete:response.complete});retain()});
    });
    request.on('error',error=>{record(name,'http-error',problem(error));retain();reject(error)});
    request.end(body);
  });
}
function raw(name,path,upgrade) {
  phase(name);
  const headers=upgrade?'Connection: Upgrade\r\n':'Connection: close\r\n';
  return new Promise(resolve=>{
    const socket=net.createConnection(listen);
    let bytes='';
    const done=(reason,detail={})=>{
      record(name,reason,{bytes:bytes.length,greeting:bytes.includes('RFB 003.008'),...detail});retain();
      clearTimeout(timer);socket.destroy();resolve({head:bytes.split('\r\n')[0],
        greeting:bytes.includes('RFB 003.008')});
    };
    const timer=setTimeout(()=>done('socket-deadline'),3000);
    socket.setEncoding('latin1').on('connect',()=>record(name,'socket-connect'))
      .on('data',chunk=>{bytes+=chunk;record(name,'socket-data',{bytes:chunk.length})})
      .on('close',hadError=>done('socket-close',{hadError}))
      .on('error',error=>done('socket-error',problem(error)));
    socket.write(`GET ${path} HTTP/1.1\r\nHost: localhost\r\n${headers}Upgrade: websocket\r\n`+
      'Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: '+randomBytes(16).toString('base64')+'\r\n\r\n');
  });
}
(async()=>{
  phase('listen');
  vm.runInNewContext(code,context);
  await new Promise(resolve=>context.server.once('listening',resolve));
  const seen={};
  seen.neverOpened=await get('neverOpened','/vnc.html');
  seen.command=await get('command',context.protocol.commandPath,{[context.protocol.privateHeader]:privateToken},'POST',
    JSON.stringify({name:'xdotool',args:['getmouselocation']}));
  grant(token,60);
  seen.unauthenticatedPage=await get('unauthenticatedPage','/vnc.html');
  seen.unauthenticatedListing=await get('unauthenticatedListing','/');
  seen.wrongToken=await get('wrongToken','/vnc.html?token=not-'+token);
  seen.unauthenticatedPlainUpgrade=await raw('unauthenticatedPlainUpgrade','/websockify',false);
  seen.validPage=await get('validPage','/vnc.html?token='+token);
  const cookie=(seen.validPage.cookie??'').split(';')[0], value=cookie.slice(cookie.indexOf('=')+1);
  seen.cookieFile=await get('cookieFile','/app/ui.js',{cookie:'other=1; '+cookie});
  seen.cookieAsToken=await get('cookieAsToken','/app/ui.js?token='+encodeURIComponent(value));
  seen.validPlainUpgrade=await raw('validPlainUpgrade','/websockify?token='+token,false);
  seen.validUpgrade=await raw('validUpgrade','/websockify?token='+token,true);
  seen.cookieOnlyUpgrade=await raw('cookieOnlyUpgrade','/websockify',true);
  grant(token,-1);
  seen.expiredPage=await get('expiredPage','/vnc.html?token='+token);
  seen.expiredCookie=await get('expiredCookie','/app/ui.js',{cookie});
  seen.expiredUpgrade=await raw('expiredUpgrade','/websockify?token='+token,true);
  grant('next-'+token,60);
  seen.replacedCookie=await get('replacedCookie','/app/ui.js',{cookie});
  seen.replacedToken=await get('replacedToken','/vnc.html?token='+token);
  seen.token=token;
  phase('assertions');
  context.server.close();
  console.log(JSON.stringify(seen));
})().catch(error=>{
  record(trace.phase,'exception',problem(error));retain();
  console.error(redacted(error.stack??error));process.exit(1);
});
"""


def head(status):
    return f'HTTP/1.1 {status.value} {status.phrase}'


class Websockify(SimpleHTTPRequestHandler):
    """websockify 0.13.0's handler serves files through SimpleHTTPRequestHandler over HTTP/1.1
    and treats a request whose Upgrade header names websocket as a WebSocket, whatever
    Connection says (websocketserver.py); this one answers that with an RFB greeting."""
    protocol_version = 'HTTP/1.1'

    def do_GET(self):
        self.server.seen.append([self.path, self.headers.get('upgrade')])
        if self.headers.get('upgrade', '').lower() == 'websocket':
            self.send_response(HTTPStatus.SWITCHING_PROTOCOLS)
            self.send_header('Upgrade', 'websocket')
            self.send_header('Connection', 'Upgrade')
            self.end_headers()
            self.wfile.write(b'RFB 003.008\n')
            self.close_connection = True
            return
        super().do_GET()

    def log_message(self, *_):
        pass


def redacted(value, fixture):
    if isinstance(value, dict):
        return {name: redacted(item, fixture) for name, item in value.items()}
    if isinstance(value, list):
        return [redacted(item, fixture) for item in value]
    if not isinstance(value, (str, bytes)):
        return value
    value = value.decode(errors='replace') if isinstance(value, bytes) else value
    for name, token in fixture.items():
        value = value.replace(token, f'[fixture-{name}]')
    value = re.sub(r'([?&]token=)[^&\s\'"<>]+', r'\1[fixture]', value)
    return re.sub(r'(pryx-desktop=)[^;\s\'"<>]+', r'\1[fixture]', value)


def retain_failure(root, error, result, fixture, upstream):
    """Keep only owned, redacted fixture diagnostics, before temporary sockets are removed."""
    try:
        trace = json.loads((root / 'scratch/desktop-front-trace.json').read_text())
    except (OSError, ValueError) as problem:
        trace = {'phase': 'before-node-trace', 'readError': type(problem).__name__}
    stderr = result.stderr if result is not None else getattr(error, 'stderr', '') or ''
    frames = traceback.extract_tb(error.__traceback__)
    message = f'Owned Node process exceeded {error.timeout}s' if isinstance(error, subprocess.TimeoutExpired) else str(error)
    report = {
        'exception': {'type': type(error).__name__, 'message': redacted(message, fixture),
                      'frames': [{'file': Path(frame.filename).name, 'line': frame.lineno,
                                  'function': frame.name} for frame in frames]},
        'nodeReturncode': result.returncode if result is not None else None,
        'nodeStderr': redacted(stderr, fixture)[-8192:],
        'trace': trace,
        'upstreamRequests': [{'path': path.split('?', 1)[0], 'upgrade': upgrade is not None}
                             for path, upgrade in getattr(upstream, 'seen', [])],
    }
    encoded = json.dumps(redacted(report, fixture))
    destination = os.environ.get('PRYX_DESKTOP_FRONT_FAILURE')
    if not destination and os.environ.get('RUNNER_TEMP'):
        destination = str(Path(os.environ['RUNNER_TEMP']) / 'desktop-front-failures.jsonl')
    if destination:
        try:
            with Path(destination).open('a') as output:
                output.write(encoded + '\n')
        except OSError as problem:
            print(f'Desktop front evidence write failed: {type(problem).__name__}', file=sys.stderr)
    print('DESKTOP_FRONT_FAILURE ' + encoded, file=sys.stderr)


class DesktopFront(unittest.TestCase):
    def test_only_the_current_unexpired_token_reaches_websockify(self):
        adapter = Path(__file__).parent / 'desktop/start.mjs'
        with tempfile.TemporaryDirectory(prefix='pryx-desktop-front-') as directory:
            root = Path(directory)
            (root / 'scratch').mkdir()
            (root / 'web/app').mkdir(parents=True)
            (root / 'web/vnc.html').write_text('<title>noVNC</title>')
            (root / 'web/app/ui.js').write_text('export default 1')
            fixture = {'access': secrets.token_urlsafe(24), 'private': secrets.token_urlsafe(24)}
            (root / 'scratch/desktop-fixture.json').write_text(json.dumps(fixture))
            upstream = None
            serving = False
            result = None
            try:
                upstream = socketserver.ThreadingUnixStreamServer(
                    str(root / 'web.sock'), partial(Websockify, directory=str(root / 'web')))
                upstream.daemon_threads = True
                upstream.seen = []
                threading.Thread(target=upstream.serve_forever, daemon=True).start()
                serving = True
                result = subprocess.run(['node', '-e', FRONT, str(adapter), str(root / 'scratch'),
                                         str(root / 'web.sock'), str(root / 'front.sock')],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, redacted(result.stderr, fixture))
                seen = json.loads(result.stdout)
                token = seen['token']
                for name in ['neverOpened', 'unauthenticatedPage', 'unauthenticatedListing', 'wrongToken',
                             'cookieAsToken', 'expiredPage', 'expiredCookie', 'replacedCookie', 'replacedToken']:
                    self.assertEqual(seen[name]['status'], HTTPStatus.FORBIDDEN, name)
                    self.assertIsNone(seen[name]['cookie'], name)
                self.assertEqual(seen['unauthenticatedPlainUpgrade']['head'], head(HTTPStatus.FORBIDDEN))
                for name in ['cookieOnlyUpgrade', 'expiredUpgrade']:
                    self.assertEqual(seen[name], {'head': '', 'greeting': False}, name)
                self.assertEqual((seen['command']['status'], seen['command']['body']), (HTTPStatus.OK, 'ran xdotool'))
                self.assertEqual((seen['validPage']['status'], seen['validPage']['body']),
                                 (HTTPStatus.OK, '<title>noVNC</title>'))
                cookie = seen['validPage']['cookie']
                self.assertRegex(cookie, r'^pryx-desktop=[\w-]+; Max-Age=\d+; HttpOnly; SameSite=Strict$')
                self.assertNotIn(token, cookie)
                self.assertTrue(0 < int(re.search(r'Max-Age=(\d+)', cookie)[1]) <= 60)
                self.assertEqual((seen['cookieFile']['status'], seen['cookieFile']['body']),
                                 (HTTPStatus.OK, 'export default 1'))
                self.assertNotEqual(seen['validPlainUpgrade']['head'], head(HTTPStatus.SWITCHING_PROTOCOLS))
                self.assertFalse(seen['validPlainUpgrade']['greeting'])
                self.assertEqual(seen['validUpgrade']['head'], head(HTTPStatus.SWITCHING_PROTOCOLS))
                self.assertTrue(seen['validUpgrade']['greeting'])
                self.assertEqual(upstream.seen, [
                    ['/vnc.html?token=' + token, None],
                    ['/app/ui.js', None],
                    ['/websockify?token=' + token, None],
                    ['/websockify?token=' + token, 'websocket'],
                ], 'Only the current, unexpired token reaches websockify, and a plain request stays plain')
            except Exception as problem:
                retain_failure(root, problem, result, fixture, upstream)
                problem.args = tuple(redacted(value, fixture) for value in problem.args)
                raise
            finally:
                if upstream is not None:
                    if serving:
                        upstream.shutdown()
                    upstream.server_close()


if __name__ == '__main__':
    unittest.main()
