#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Measure default client waits against a listener that never writes; capped observations are censored.

Raw observations go to the configured output or a temporary file, never the repository by default.
This diagnostic collector does not define product timeout policy.
"""
import asyncio, json, os, pathlib, shutil, signal, subprocess, tempfile, time

CAP = 15
N = 3
async def main():
    root = pathlib.Path(tempfile.mkdtemp(prefix='pryx-edge-measure-'))
    sockets = []
    async def silent(reader, writer):
        sockets.append(writer)
        try: await reader.read()
        finally: writer.close()
    subprocess.run(['python3','-c','import requests'],check=True)
    server = await asyncio.start_server(silent, '127.0.0.1', 0)
    port = server.sockets[0].getsockname()[1]
    go_source = root / 'client.go'
    go_source.write_text('package main\nimport("net/http";"os";"fmt")\nfunc main(){_,err:=http.Get(os.Args[1]);if err!=nil {fmt.Fprintln(os.Stderr,err);os.Exit(1)}}\n')
    subprocess.run(['go','build','-o',str(root/'go-client'),str(go_source)],check=True)
    commands = {
      'curl': lambda url,i: ['curl','--noproxy','*','-sS',url],
      'Node fetch': lambda url,i: ['node','-e','fetch(process.argv[1]).catch(()=>process.exitCode=1)',url],
      'Python requests': lambda url,i: ['python3','-c','import requests,sys; s=requests.Session();s.trust_env=False;s.get(sys.argv[1])',url],
      'Go net/http': lambda url,i: [str(root/'go-client'),url],
      'Chrome': lambda url,i: ['chromium','--headless','--no-sandbox','--disable-dev-shm-usage','--disable-gpu',f'--user-data-dir={root}/chrome-{i}','--dump-dom',url],
    }
    semaphore = asyncio.Semaphore(10)
    async def measure(client,kind,i,command):
      async with semaphore:
        start=time.monotonic()
        process=await asyncio.create_subprocess_exec(*command,stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.PIPE,start_new_session=True,env={**os.environ,'NO_PROXY':'*','no_proxy':'*'})
        censored=False
        try: _,err=await asyncio.wait_for(process.communicate(),CAP)
        except asyncio.TimeoutError:
          censored=True;os.killpg(process.pid,signal.SIGKILL);_,err=await process.communicate()
        return {'client':client,'kind':kind,'sample':i,'seconds':round(time.monotonic()-start,4),'censored':censored,'exit':process.returncode,'stderr':err.decode(errors='replace')[-2000:]}
    jobs=[]
    for client,fn in commands.items():
      for kind,scheme in [('TLS','https'),('HTTP','http')]:
        for i in range(N):
          jobs.append(measure(client,kind,i,fn(f'{scheme}://127.0.0.1:{port}/',f'{client}-{kind}-{i}')))
    for i in range(N):
      jobs.append(measure('OpenSSH','banner',i,['ssh','-F','/dev/null','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','-p',str(port),'127.0.0.1','true']))
    results=await asyncio.gather(*jobs)
    output={'capSeconds':CAP,'n':N,'versions':{},'results':results}
    for name,args in [('curl',['curl','--version']),('Node fetch',['node','--version']),('Python requests',['python3','-c','import requests;print(requests.__version__)']),('Go net/http',['go','version']),('Chrome',['chromium','--version']),('OpenSSH',['ssh','-V'])]:
      p=subprocess.run(args,capture_output=True,text=True);output['versions'][name]=(p.stdout+p.stderr).splitlines()[0]
    output['missing']={'Safari':'Unavailable on this Linux host; no Safari measurement was run.'}
    path=pathlib.Path(os.environ.get('PRYX_EDGE_MEASUREMENTS', str(pathlib.Path(tempfile.gettempdir()) / 'pryx-edge-clients.json')));path.write_text(json.dumps(output,indent=2)+'\n')
    for client,kind in sorted({(r['client'],r['kind']) for r in results}):
      group=[r for r in results if r['client']==client and r['kind']==kind]
      values=sorted(r['seconds'] for r in group)
      print(client,kind,'n',len(values),'p50',values[len(values)//2],'p90',values[-1],'censored',sum(r['censored'] for r in group),flush=True)
    server.close();await server.wait_closed()
    for s in sockets:s.close()
    shutil.rmtree(root,ignore_errors=True)
asyncio.run(main())
