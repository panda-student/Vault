// Native WebView2 acceptance. Requires two debug builds and agent-browser.
// All credentials, paths and releases are isolated test fixtures.
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {createServer} from 'node:http';
import {createServer as tcpServer} from 'node:net';
import {readFile,writeFile,mkdir,copyFile,mkdtemp} from 'node:fs/promises';
import {join,resolve,dirname} from 'node:path';
import {tmpdir} from 'node:os';
import {createPrivateKey,sign,createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';

const run = promisify(execFile), pause=ms=>new Promise(r=>setTimeout(r,ms));
const root=resolve('artifacts/update-acceptance');await mkdir(root,{recursive:true});
const fromVersion=process.env.UPDATE_FROM_VERSION||'0.5.4',toVersion=process.env.UPDATE_TO_VERSION||'0.5.5',failedVersion=process.env.UPDATE_FAILED_VERSION||'0.5.6';
const from=resolve(process.env.UPDATE_FROM||join(root,`builds/v${fromVersion}.exe`));
const to=resolve(process.env.UPDATE_TO||join(root,`builds/v${toVersion}.exe`));
const appRoot=await mkdtemp(join(tmpdir(),'vault-update-原生 验收-'));
const executable=join(appRoot,'vault.exe'), data=join(appRoot,'isolated data');
await copyFile(from,executable);
const binary=await readFile(to), original=await readFile(from);
const report={passed:false,root:appRoot,fromSha256:createHash('sha256').update(original).digest('hex'),toSha256:createHash('sha256').update(binary).digest('hex'),checks:[]};
const check=(name,ok)=>{report.checks.push({name,passed:!!ok});assert.ok(ok,name);console.log('PASS',name);};
const keyCommand="Add-Type -AssemblyName System.Security; $p=Join-Path $env:USERPROFILE '.codex/secrets/local-password-manager/update-signing-key.dpapi'; $b=[Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($p),$null,[Security.Cryptography.DataProtectionScope]::CurrentUser); [Console]::Write([Text.Encoding]::UTF8.GetString($b)); [Array]::Clear($b,0,$b.Length)";
const secret=(await run('powershell.exe',['-NoProfile','-Command',keyCommand],{windowsHide:true})).stdout.trim();
const privateKey=createPrivateKey({key:Buffer.from(secret,'base64'),format:'der',type:'pkcs8'});
async function zipRelease(bytes,version){
 const folder=await mkdtemp(join(appRoot,'archive-'));
 const member=join(folder,`vault-${version}`);await mkdir(member);await writeFile(join(member,'vault.exe'),bytes);
 const archive=join(folder,'portable.zip');
 await run('powershell.exe',['-NoProfile','-Command','Compress-Archive -LiteralPath $env:ZIP_MEMBER -DestinationPath $env:ZIP_OUTPUT'],{windowsHide:true,env:{...process.env,ZIP_MEMBER:member,ZIP_OUTPUT:archive}});
 return await readFile(archive);
}
function signedPayload(bytes,archive,version,notes){return Buffer.from(JSON.stringify({schema:2,version,platform:'windows-x86_64',url:`https://github.com/panda-student/vault/releases/download/v${version}/vault-${version}.zip`,sha256:createHash('sha256').update(bytes).digest('hex'),size:bytes.length,archive_sha256:createHash('sha256').update(archive).digest('hex'),archive_size:archive.length,notes}));}
const archive=await zipRelease(binary,toVersion);
const payload=signedPayload(binary,archive,toVersion,'原生 ZIP 更新验收：保留凭据、权限和 MCP 连接。');
const valid={payload:payload.toString('base64'),signature:sign(null,payload,privateKey).toString('base64')};
let activeEnvelope=valid,activeBinary=archive;
let mode='bad-signature';
const server=createServer((req,res)=>{
 if(req.url==='/latest.json'){
  if(mode==='offline'){res.writeHead(503);res.end();return;}
  res.setHeader('Content-Type','application/json');res.end(JSON.stringify(mode==='bad-signature'?{...activeEnvelope,signature:Buffer.alloc(64).toString('base64')}:activeEnvelope));return;
 }
 if(/^\/vault-[\d.]+\.zip$/.test(req.url)){
  const bytes=mode==='corrupt'?Buffer.from(activeBinary):activeBinary;if(mode==='corrupt')bytes[bytes.length-1]^=1;
  res.setHeader('Content-Length',bytes.length);let offset=0;
  const timer=setInterval(()=>{if(offset>=bytes.length){clearInterval(timer);res.end();return;}const end=Math.min(bytes.length,offset+256*1024);res.write(bytes.subarray(offset,end));offset=end;},15);
  res.on('close',()=>clearInterval(timer));return;
 }res.writeHead(404);res.end();
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const probe=tcpServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
const env={...process.env,VAULT_UI_DEBUG_PORT:String(port),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,VAULT_UPDATE_TEST_SOURCE:`http://127.0.0.1:${server.address().port}/latest.json`};
const agent=process.env.AGENT_BROWSER_BIN||join(dirname(process.execPath),'node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
let session=`vault-update-before-${process.pid}`,desktop,client,client2,ws,serial=0;
const pending=new Map();
async function connect(){
 const until=Date.now()+60000;
 while(Date.now()<until){try{
  const targets=await(await fetch(`http://127.0.0.1:${port}/json/list`,{signal:AbortSignal.timeout(1500)})).json();const target=targets.find(t=>t.type==='page');if(!target)throw Error('No page');
  ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.addEventListener('open',r,{once:true});ws.addEventListener('error',j,{once:true});});
  ws.addEventListener('message',e=>{const result=JSON.parse(e.data);if(result.id&&pending.has(result.id)){const p=pending.get(result.id);pending.delete(result.id);result.error?p.reject(Error(result.error.message)):p.resolve(result.result);}});
  await evaluate("window.__TAURI_INTERNALS__.invoke('desktop_info')");delete report.connectError;return;
 }catch(error){report.connectError=String(error);ws?.close();await pause(250);}}
 throw Error('Desktop CDP did not become ready');
}
async function cdp(method,params){const id=++serial;return new Promise((resolve,reject)=>{pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));setTimeout(()=>{if(pending.delete(id))reject(Error('CDP timeout: '+method));},20000).unref();});}
async function evaluate(expression){const result=await cdp('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(result.exceptionDetails)throw Error(result.exceptionDetails.text);return result.result.value;}
const invoke=(name,args={})=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`);
const status=()=>invoke('update_action',{action:'status'});
async function waitPhase(phase){const end=Date.now()+30000;let current;while(Date.now()<end){current=await status();if(current.phase===phase)return current;await pause(100);}report.lastStatus=current;throw Error('Expected phase '+phase+'; actual '+JSON.stringify(current));}
async function browser(...args){const result=await run(agent,['--session',session,'--cdp',String(port),...args],{windowsHide:true,timeout:30000,maxBuffer:2*1024*1024});return result.stdout;}
async function click(selector){await browser('click',selector);await pause(150);}
async function screenshot(name){await browser('screenshot',join(root,name+'.png'));}
async function waitText(selector,text){const end=Date.now()+15000;while(Date.now()<end){if((await evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent||''`)).includes(text))return;await pause(100);}throw Error('Missing UI text: '+text);}
async function mcp(){const c=new Client({name:'isolated-update-test',version:'1'});const transport=new StdioClientTransport({command:executable,args:['--mcp','--data-dir',data],env,stderr:'pipe'});await c.connect(transport);c.fixtureTransport=transport;return c;}
async function resumed(c){const end=Date.now()+15000;while(Date.now()<end){try{if((await c.listTools()).tools.length===11)return true;}catch{}await pause(100);}return false;}
async function coreProcesses(){const command="@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_TEST_EXE -and $_.CommandLine -notmatch '(?:^|\\s)--mcp(?:\\s|$)' -and $_.CommandLine.Contains($env:VAULT_TEST_DIR) } | Select-Object -ExpandProperty ProcessId) | ConvertTo-Json -Compress";const r=await run('powershell.exe',['-NoProfile','-Command',command],{windowsHide:true,env:{...process.env,VAULT_TEST_EXE:executable,VAULT_TEST_DIR:data}});const ids=JSON.parse(r.stdout.trim()||'[]');return Array.isArray(ids)?ids:[ids];}
async function killIsolated(){await run('powershell.exe',['-NoProfile','-Command',"Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:VAULT_TEST_EXE -or ($_.Path -like ($env:VAULT_TEST_ROOT+'\\.vault-update-*\\helper.exe')) } | Stop-Process -Force -ErrorAction SilentlyContinue"],{env:{...process.env,VAULT_TEST_EXE:executable,VAULT_TEST_ROOT:appRoot},windowsHide:true});}
try{
 client=await mcp();const created=await client.callTool({name:'create_entries',arguments:{kind:'password',title:'升级保留样例',account:'test-user',tags:['验收标签'],secret:'FAKE_UPDATE_SECRET_ONLY_6721'}});const entry=JSON.parse(created.content.find(c=>c.type==='text').text);const id=entry.id||entry.results?.[0]?.id;assert.ok(id);await client.close();client=null;
 desktop=spawn(executable,['--data-dir',data],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});
 desktop.stderr.on('data',bytes=>{report.desktopDiagnostics=((report.desktopDiagnostics||'')+String(bytes)).slice(-2000);});
 await connect();await browser('snapshot','-i');await click('[data-nav="settings"]');
 await waitPhase('error');check('invalid-signature-rejected-before-download',(await status()).error.includes('签名'));await screenshot('01-invalid-signature');
 mode='offline';await click('#check-update');await waitPhase('error');check('network-failure-is-retryable',(await status()).error.includes('网络'));
 mode='valid';await click('#check-update');await waitPhase('available');check('signed-newer-version-discovered',(await status()).version===toVersion);
 await click('#download-update');await waitPhase('downloading');await click('#cancel-update');await waitPhase('error');check('download-cancellation-is-retryable',(await status()).error.includes('取消'));
 mode='corrupt';await click('#download-update');await waitPhase('error');check('corrupted-binary-rejected',(await status()).error.includes('校验'));check('failed-download-preserves-executable',createHash('sha256').update(await readFile(executable)).digest('hex')===report.fromSha256);
 mode='valid';await click('#download-update');await waitPhase('ready');check('download-verified-and-ready',(await status()).received===archive.length);await screenshot('02-ready');
 await click('[data-nav="entries"]');await browser('fill','#f-title','未保存草稿');await click('[data-nav="settings"]');await click('#install-update');await click('#cancel-install-update');check('confirmation-cancel-keeps-app',(await invoke('desktop_info')).version===fromVersion);
 await click('#install-update');await click('#confirm-install-update');check('unsaved-draft-blocks-restart',(await evaluate("document.querySelector('#update-error')?.textContent||''")).includes('未保存'));await screenshot('03-draft-blocked');
 await click('[data-nav="entries"]');await click('#cancel-edit');await click('[data-nav="settings"]');
 client=await mcp();client2=await mcp();
 const bridgePids=[client.fixtureTransport.pid,client2.fixtureTransport.pid];
 report.bridgePids=bridgePids;
 const oldCores=await coreProcesses();check('two-clients-share-one-core-before-upgrade',oldCores.length===1);report.corePids=[oldCores[0]];
 // Configure a non-default permission and verify it survives the real restart.
 const oldInfo=await invoke('desktop_info');await invoke('save_settings',{settings:{...oldInfo.settings,mcpPermissions:{...oldInfo.settings.mcpPermissions,writeFile:false}}});
 await click('#install-update');await click('#confirm-install-update');ws.close();
 await pause(1600);session=`vault-update-after-${process.pid}`;await connect();
 const newInfo=await invoke('desktop_info');check('new-version-native-window-started',newInfo.version===toVersion);check('custom-data-directory-preserved',newInfo.dataDirectory===oldInfo.dataDirectory);check('permission-preserved',newInfo.settings.mcpPermissions.writeFile===false);
 check('executable-replaced-at-original-path',createHash('sha256').update(await readFile(executable)).digest('hex')===report.toSha256);
 const entries=await invoke('list_entries');const saved=entries.find(e=>e.id===id);check('credential-and-tags-preserved',saved?.title==='升级保留样例'&&saved.tags.includes('验收标签'));
 const preserved=await invoke('read_secret',{id,expectedRevision:saved.revision});check('encrypted-secret-decrypts-after-upgrade',preserved==='FAKE_UPDATE_SECRET_ONLY_6721');
 check('two-existing-mcp-connections-resume-after-upgrade',await resumed(client)&&await resumed(client2));
 const newCores=await coreProcesses();check('upgrade-restarts-exactly-one-shared-core',newCores.length===1&&newCores[0]!==oldCores[0]);report.corePids.push(newCores[0]);
 check('client-facing-processes-preserved',client.fixtureTransport.pid===bridgePids[0]&&client2.fixtureTransport.pid===bridgePids[1]);
 check('existing-mcp-reads-saved-entry',!(await client.callTool({name:'get_entries',arguments:{id}})).isError);
 await browser('snapshot','-i');await screenshot('05-upgraded');
 const width=await evaluate('window.innerWidth');await cdp('Emulation.setDeviceMetricsOverride',{width:900,height:580,deviceScaleFactor:1,mobile:false});await click('[data-nav="settings"]');
 check('update-actions-do-not-wrap-or-overflow-at-minimum-width',await evaluate("(()=>{const r=document.querySelector('#check-update').getBoundingClientRect(),p=document.querySelector('#update-panel').getBoundingClientRect();return r.right<=p.right&&r.left>=p.left&&document.documentElement.scrollWidth<=innerWidth})()"));await screenshot('06-minimum-width');await cdp('Emulation.clearDeviceMetricsOverride',{});
 // A correctly signed executable that immediately exits must roll back.
 const failingSource=join(root,'failed-start.rs'), failingBinary=join(root,'failed-start.exe');
 await writeFile(failingSource,'#![windows_subsystem="windows"]\nfn main(){std::process::exit(1);}\n');
 await run(join(process.env.USERPROFILE,'.cargo/bin/rustc.exe'),['--crate-name','failed_start','-C','opt-level=s','-C','strip=symbols',failingSource,'-o',failingBinary],{windowsHide:true});
 const failedBytes=await readFile(failingBinary);activeBinary=await zipRelease(failedBytes,failedVersion);
 const failedPayload=signedPayload(failedBytes,activeBinary,failedVersion,'仅验收：启动失败应回退');
 activeEnvelope={payload:failedPayload.toString('base64'),signature:sign(null,failedPayload,privateKey).toString('base64')};
 await click('#check-update');await waitPhase('available');await click('#download-update');await waitPhase('ready');
 await click('#install-update');await click('#confirm-install-update');ws.close();await pause(1600);session=`vault-update-rollback-${process.pid}`;await connect();
 check('failed-new-version-rolls-back-and-restarts',(await invoke('desktop_info')).version===toVersion);
 check('both-mcp-connections-resume-after-rollback',await resumed(client)&&await resumed(client2));
 const rolledBackCores=await coreProcesses();check('rollback-restarts-exactly-one-shared-core',rolledBackCores.length===1&&rolledBackCores[0]!==newCores[0]);report.corePids.push(rolledBackCores[0]);
 check('rollback-preserves-client-facing-processes',client.fixtureTransport.pid===bridgePids[0]&&client2.fixtureTransport.pid===bridgePids[1]);
 check('rollback-restores-previous-executable-hash',createHash('sha256').update(await readFile(executable)).digest('hex')===report.toSha256);
 check('rollback-preserves-encrypted-credentials',(await invoke('read_secret',{id,expectedRevision:saved.revision}))==='FAKE_UPDATE_SECRET_ONLY_6721');await screenshot('07-rolled-back');
 report.passed=true;
}catch(error){report.error=String(error);throw error;}
finally{
 await client?.close().catch(()=>{});await client2?.close().catch(()=>{});ws?.close();await killIsolated();server.closeAllConnections();await new Promise(r=>server.close(r));
 await writeFile(join(root,'native-update-report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({passed:report.passed,checks:report.checks.length,report:join(root,'native-update-report.json')}));
}
