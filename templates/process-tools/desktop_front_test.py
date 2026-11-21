# SPDX-License-Identifier: FSL-1.1-ALv2
"""The desktop front serves nothing without the current, unexpired access token.

A real Node process runs the committed access check and request handlers of desktop/start.mjs
in front of a stand-in for websockify on a Unix socket, which records every request that
reaches it. Display programs, websockify itself and the edge are outside this test; the hosted
desktop_application acceptance runs them with the same refusals.

The stand-in closes connections at different points in a response so the front must deliver
complete responses and terminate an interrupted response promptly.
"""
from functools import partial
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler
import json
from pathlib import Path
import re
import socket
import socketserver
import subprocess
import tempfile
import threading
import unittest


FRONT = r"""
const fs=require('node:fs'), http=require('node:http'), net=require('node:net'), vm=require('node:vm');
const {join}=require('node:path'), {constants}=require('node:http2');
const {createHmac,randomBytes,timingSafeEqual}=require('node:crypto');
const [adapter,scratch,web,listen,scenario]=process.argv.slice(1);
const source=fs.readFileSync(adapter,'utf8');
function part(from,to) {
  const start=source.indexOf(from), end=source.indexOf(to,start);
  if (start<0||end<0) throw new Error('Exercise the committed adapter front: '+from);
  return source.slice(start,end);
}
const code=part('function equal(','let stopping')+
  part('const server = http.createServer(',"writeFileSync(join(config.scratch, 'desktop-ready.json')")+
  ';server.listen(listen);globalThis.server=server;';
// A loaded host may run websockify between any two of the front's writes: a write after a
// complete request waits until the stand-in has answered and closed that connection.
const answered=join(scratch,'..','websockify-closed'), idle=new Int32Array(new SharedArrayBuffer(4));
const late={...net,createConnection(...options) {
  const connection=net.createConnection(...options), write=connection.write.bind(connection);
  let sent=Buffer.alloc(0);
  connection.write=(chunk,...rest)=>{
    for (const deadline=Date.now()+5000; sent.includes('\r\n\r\n')&&!fs.existsSync(answered)&&
      Date.now()<deadline;) Atomics.wait(idle,0,0,5);
    sent=Buffer.concat([sent,Buffer.from(chunk)]);
    return write(chunk,...rest);
  };
  return connection;
}};
const context={http,net:late,constants,web,listen,readFileSync:fs.readFileSync,join,Buffer,URL,setTimeout,
  clearTimeout,createHmac,randomBytes,timingSafeEqual,
  protocol:JSON.parse(fs.readFileSync(join(adapter,'..','protocol.json'),'utf8')),
  config:{scratch,privateToken:'private',maxRequestBytes:65536,timeoutMs:5000},
  run:async name=>'ran '+name};
function grant(token,seconds) {
  fs.writeFileSync(join(scratch,'desktop-access.json'),
    JSON.stringify({token,expiresAt:Math.floor(Date.now()/1000)+seconds}));
}
function get(path,headers={},{method='GET',body,signal}={}) {
  return new Promise((resolve,reject)=>{
    const request=http.request({socketPath:listen,path,method,headers,agent:false,signal},response=>{
      let text='';
      response.setEncoding('utf8').on('data',chunk=>{text+=chunk}).on('end',()=>resolve({
        status:response.statusCode,body:text,cookie:response.headers['set-cookie']?.[0]??null}));
      response.on('error',reject);
    });
    request.on('error',reject);
    request.end(body);
  });
}
function raw(path,upgrade) {
  const headers=upgrade?'Connection: Upgrade\r\n':'Connection: close\r\n';
  return new Promise(resolve=>{
    const socket=net.createConnection(listen);
    let bytes='';
    const done=()=>{clearTimeout(timer);socket.destroy();resolve({head:bytes.split('\r\n')[0],
      greeting:bytes.includes('RFB 003.008')})};
    const timer=setTimeout(done,3000);
    socket.setEncoding('latin1').on('data',chunk=>{bytes+=chunk}).on('close',done).on('error',done);
    socket.write(`GET ${path} HTTP/1.1\r\nHost: localhost\r\n${headers}Upgrade: websocket\r\n`+
      'Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n');
  });
}
async function verifyAccess() {
  const token='current-access-token', seen={};
  seen.neverOpened=await get('/vnc.html');
  seen.command=await get(context.protocol.commandPath,{[context.protocol.privateHeader]:'private'},
    {method:'POST',body:JSON.stringify({name:'xdotool',args:['getmouselocation']})});
  grant(token,60);
  seen.unauthenticatedPage=await get('/vnc.html');
  seen.unauthenticatedListing=await get('/');
  seen.wrongToken=await get('/vnc.html?token=not-'+token);
  seen.unauthenticatedPlainUpgrade=await raw('/websockify',false);
  seen.validPage=await get('/vnc.html?token='+token);
  const cookie=(seen.validPage.cookie??'').split(';')[0], value=cookie.slice(cookie.indexOf('=')+1);
  seen.cookieFile=await get('/app/ui.js',{cookie:'other=1; '+cookie});
  seen.cookieAsToken=await get('/app/ui.js?token='+encodeURIComponent(value));
  seen.validPlainUpgrade=await raw('/websockify?token='+token,false);
  seen.validUpgrade=await raw('/websockify?token='+token,true);
  seen.cookieOnlyUpgrade=await raw('/websockify',true);
  grant(token,-1);
  seen.expiredPage=await get('/vnc.html?token='+token);
  seen.expiredCookie=await get('/app/ui.js',{cookie});
  seen.expiredUpgrade=await raw('/websockify?token='+token,true);
  grant('next-'+token,60);
  seen.replacedCookie=await get('/app/ui.js',{cookie});
  seen.replacedToken=await get('/vnc.html?token='+token);
  seen.token=token;
  return seen;
}
async function verifyInterruption() {
  const token='current-access-token';
  grant(token,60);
  const signal=AbortSignal.timeout(3000);
  let interruption;
  try {
    const response=await get('/interrupted.html?token='+token,{}, {signal});
    interruption=response.status;
  } catch {
    interruption=signal.aborted?'deadline':'closed';
  }
  return {interruption,validPage:await get('/vnc.html?token='+token),token};
}
(async()=>{
  vm.runInNewContext(code,context);
  await new Promise(resolve=>context.server.once('listening',resolve));
  const seen=await (scenario==='interrupted' ? verifyInterruption() : verifyAccess());
  context.server.close();
  console.log(JSON.stringify(seen));
})().catch(error=>{console.error(error);process.exit(1)});
"""


def head(status):
    return f'HTTP/1.1 {status.value} {status.phrase}'


class Websockify(SimpleHTTPRequestHandler):
    """websockify 0.13.0's handler serves files through SimpleHTTPRequestHandler over HTTP/1.1
    and treats a request whose Upgrade header names websocket as a WebSocket, whatever
    Connection says (websocketserver.py); this one answers that with an RFB greeting."""
    protocol_version = 'HTTP/1.1'
    upgraded = False

    def handle(self):
        super().handle()
        # Like websockify, it closes once a request asks for that, but a scheduler decides when the
        # close lands: hold it until the front sends more on this connection or closes it.
        if not self.upgraded:
            self.connection.recv(1, socket.MSG_PEEK)

    def do_GET(self):
        self.server.seen.append([self.path, self.headers.get('upgrade')])
        if self.path.split('?', 1)[0] == '/interrupted.html':
            body = b'<title>noVNC</title>'
            self.send_response(HTTPStatus.OK)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body[:-1])
            self.connection.shutdown(socket.SHUT_WR)
            self.close_connection = True
            return
        if self.headers.get('upgrade', '').lower() == 'websocket':
            self.send_response(HTTPStatus.SWITCHING_PROTOCOLS)
            self.send_header('Upgrade', 'websocket')
            self.send_header('Connection', 'Upgrade')
            self.end_headers()
            self.wfile.write(b'RFB 003.008\n')
            # Answered and closed before the front's next write can run.
            self.connection.shutdown(socket.SHUT_RDWR)
            Path(self.directory, '..', 'websockify-closed').touch()
            self.close_connection = self.upgraded = True
            return
        super().do_GET()

    def log_message(self, *_):
        pass


class DesktopFront(unittest.TestCase):
    def exercise_front(self, scenario):
        adapter = Path(__file__).parent / 'desktop/start.mjs'
        with tempfile.TemporaryDirectory(prefix='pryx-desktop-front-') as directory:
            root = Path(directory)
            (root / 'scratch').mkdir()
            (root / 'web/app').mkdir(parents=True)
            (root / 'web/vnc.html').write_text('<title>noVNC</title>')
            (root / 'web/app/ui.js').write_text('export default 1')
            upstream = socketserver.ThreadingUnixStreamServer(
                str(root / 'web.sock'), partial(Websockify, directory=str(root / 'web')))
            upstream.daemon_threads = True
            upstream.seen = []
            threading.Thread(target=upstream.serve_forever, daemon=True).start()
            try:
                result = subprocess.run(['node', '-e', FRONT, str(adapter), str(root / 'scratch'),
                                         str(root / 'web.sock'), str(root / 'front.sock'), scenario],
                                        capture_output=True, text=True, timeout=60)
            finally:
                upstream.shutdown()
                upstream.server_close()
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout), upstream.seen

    def test_only_the_current_unexpired_token_reaches_websockify(self):
        seen, requests = self.exercise_front('access')
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
        self.assertEqual(requests, [
            ['/vnc.html?token=' + token, None],
            ['/app/ui.js', None],
            ['/websockify?token=' + token, None],
            ['/websockify?token=' + token, 'websocket'],
        ], 'Only the current, unexpired token reaches websockify, and a plain request stays plain')

    def test_interrupted_response_ends_before_client_deadline(self):
        seen, requests = self.exercise_front('interrupted')
        token = seen['token']
        self.assertEqual(requests, [
            ['/interrupted.html?token=' + token, None],
            ['/vnc.html?token=' + token, None],
        ])
        self.assertEqual((seen['validPage']['status'], seen['validPage']['body']),
                         (HTTPStatus.OK, '<title>noVNC</title>'))
        self.assertIn(seen['interruption'], ('closed', HTTPStatus.BAD_GATEWAY),
                      'The front must end the interrupted response before the client deadline')


if __name__ == '__main__':
    unittest.main()
