// Installer acceptance uses only synthetic credentials and an isolated path.
import {spawn,execFile} from 'node:child_process';
import {existsSync} from 'node:fs';
import {promisify} from 'node:util';
import {mkdtemp,mkdir,copyFile,readFile,writeFile,readdir,realpath} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
const run=promisify(execFile),sleep=ms=>new Promise(r=>setTimeout(r,ms));
// CIM reports long paths even when Windows TEMP uses an 8.3 alias.
const root=await realpath(await mkdtemp(join(tmpdir(),'vault-installer-handoff-')));
const folder=join(root,'custom install','vault'),data=join(root,'custom data'),executable=join(folder,'vault.exe');
await mkdir(folder,{recursive:true});
await copyFile(resolve(process.env.VAULT_OLD_EXE||'artifacts/github-release-0.5.5/unpacked/vault-0.5.5/Vault.exe'),executable);
const helper=resolve(process.env.VAULT_HELPER_EXE||'src-tauri/target/debug/local-password-manager.exe');
const expectedPayloadHash=createHash('sha256').update(await readFile(helper)).digest('hex');
const oldHash=createHash('sha256').update(await readFile(executable)).digest('hex');
const rollback=process.env.VAULT_TEST_ROLLBACK==='1';
const report={passed:false,root,checks:[]},clients=[],transports=[];
const check=(name,passed)=>{report.checks.push({name,passed:!!passed});console.log(`${passed?'PASS':'FAIL'} ${name}`);assert.ok(passed,name);};
async function connect(){const c=new Client({name:'installer-handoff-audit',version:'1'});const t=new StdioClientTransport({command:executable,args:['--mcp','--data-dir',data],stderr:'pipe'});t.stderr?.resume();await c.connect(t);clients.push(c);transports.push(t);return c;}
async function call(c,name,args){const r=await c.callTool({name,arguments:args});assert.ok(!r.isError);return r.structuredContent??JSON.parse(r.content.find(c=>c.type==='text').text);}
const hash=async p=>createHash('sha256').update(await readFile(p)).digest('hex');
let desktop,watcher,requestFinishedAt,desktopExitedAt,writeLock;
// Wait for the installer's own exit, not for pipe EOF held by GUI descendants.
async function install(command,args){
 return new Promise((resolve,reject)=>{
  const child=spawn(command,args,{windowsHide:true,windowsVerbatimArguments:args.some(a=>a.startsWith('/D=')),stdio:['ignore','pipe','pipe']});
  let diagnostic='';for(const stream of [child.stderr,child.stdout])stream.on('data',data=>{diagnostic=(diagnostic+String(data)).slice(-8192);});
  const timer=setTimeout(()=>{child.kill();reject(Error('Installer timed out'));},150000);
  child.once('error',error=>{clearTimeout(timer);reject(error);});child.once('exit',code=>{clearTimeout(timer);child.stdout.destroy();child.stderr.destroy();if(code!==0){report.installDiagnostic=diagnostic;console.log(diagnostic);}resolve(code);});
 });
}
try{
 const a=await connect(),b=await connect();const ids=transports.map(t=>t.pid);
 const entry=await call(a,'create_entries',{kind:'password',title:'Installer fixture',secret:'FAKE_INSTALLER_ONLY_6134'});
 // Exercise the old application's close-to-tray setting as well as a custom data path.
 await run('python',['-c',`import sqlite3,json,sys
c=sqlite3.connect(sys.argv[1]);s=json.loads(c.execute("SELECT value FROM metadata WHERE name='settings'").fetchone()[0]);s['closeToTray']=True;c.execute("UPDATE metadata SET value=? WHERE name='settings'",(json.dumps(s).encode(),));c.commit();c.close()`,join(data,'vault.db')],{windowsHide:true});
 desktop=spawn(executable,['--data-dir',data,'--background'],{windowsHide:true,stdio:'ignore'});
 desktop.once('exit',()=>{desktopExitedAt=Date.now();});
 await sleep(3000);
 check('old desktop and two MCP transports are alive',desktop.exitCode===null&&transports.every(t=>t.pid));
 let inFlight;
 if(process.env.VAULT_TEST_DRAIN==='1'){
  writeLock=spawn('python',['-c',"import sqlite3,sys,time; c=sqlite3.connect(sys.argv[1]); c.execute('BEGIN IMMEDIATE'); print('locked',flush=True); time.sleep(3); c.commit(); c.close()",join(data,'vault.db')],{windowsHide:true,stdio:['ignore','pipe','ignore']});
  await new Promise((resolve,reject)=>{writeLock.stdout.once('data',resolve);writeLock.once('error',reject);});
  inFlight=call(a,'create_entries',{kind:'password',title:'accepted installer in-flight write',secret:'FAKE_IN_FLIGHT_WRITE_8153'}).then(value=>{requestFinishedAt=Date.now();return value;});
  inFlight.catch(()=>{});
  await sleep(150);
  check('MCP write is still in flight when installation starts',requestFinishedAt===undefined);
 }
 if(rollback){
  // Cause the new desktop's health marker to fail, without changing vault data.
  watcher=setInterval(async()=>{for(const name of await readdir(folder)){if(name.startsWith('.vault-update-install-'))await mkdir(join(folder,name,'healthy')).catch(()=>{});}},5);
 }
 let code;
 if(process.env.VAULT_INSTALLER){
  const installer=resolve(process.env.VAULT_INSTALLER);
  const regName=process.env.VAULT_INSTALLER_REG_NAME||'vault-installer-audit';
  await run('powershell.exe',['-NoProfile','-Command',`if(Test-Path ('HKCU:/Software/'+$env:VAULT_AUDIT_REG)){throw 'Existing installer registration; refusing to overwrite it'}; if(!([Diagnostics.FileVersionInfo]::GetVersionInfo($env:VAULT_AUDIT_SETUP).FileDescription.StartsWith($env:VAULT_AUDIT_REG+' '))){throw 'Installer registration namespace differs from test namespace'}`],{windowsHide:true,env:{...process.env,VAULT_AUDIT_REG:regName,VAULT_AUDIT_SETUP:installer}});
  if(process.env.VAULT_INSTALLER_NATIVE==='1'){
   code=await install('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/verify-installer-handoff.ps1','-Installer',installer,'-Directory',folder,'-Evidence',resolve('artifacts/installer-native-handoff.png')]);
  }else code=await install(installer,['/S',`/D=${folder}`]);
  check(rollback?'actual NSIS reports failed startup':'actual NSIS overwrite installation succeeds',rollback?code!==0:code===0);
 }else{
  code=await install(helper,['--installer-replace',executable,join(root,'install.json')]);
  check(rollback?'replacement coordinator reports rollback':'replacement coordinator succeeds',rollback?code!==0:code===0);
  report.install=JSON.parse(await readFile(join(root,'install.json'),'utf8'));
 }
 check('old desktop exits automatically even with close-to-tray',desktop.exitCode===0);
 if(process.env.VAULT_INSTALLER&&!rollback){
  check('follow-up overwrite supports the previously restarted desktop',await install(resolve(process.env.VAULT_INSTALLER),['/S',`/D=${folder}`])===0);
 }
 if(inFlight){
  const result=await inFlight;report.drain={result,requestFinishedAt,desktopExitedAt};
  console.log('Drain result:',JSON.stringify(report.drain));
  check('in-flight MCP write finishes before desktop shutdown',!!result.id&&requestFinishedAt<=desktopExitedAt);
  const matches=await call(a,'search_entries',{query:'accepted installer in-flight write'});
  check('in-flight write is committed once without replay',matches.entries.length===1&&matches.entries[0].id===result.id);
 }
 check(rollback?'old executable is restored after failed startup':'installed executable matches the packaged payload',await hash(executable)===(rollback?oldHash:expectedPayloadHash));
 const observed=JSON.parse((await run('powershell.exe',['-NoProfile','-Command',`$items=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_AUDIT_IMAGE -and $_.CommandLine -notlike '*--mcp*' }); @{count=$items.Count;customData=(@($items | Where-Object { $_.CommandLine.Contains($env:VAULT_AUDIT_DATA) }).Count -eq 1)} | ConvertTo-Json -Compress`],{windowsHide:true,env:{...process.env,VAULT_AUDIT_IMAGE:executable,VAULT_AUDIT_DATA:data}})).stdout);
 report.desktop=observed;console.log('Restarted desktop:',JSON.stringify(observed));
 check('exactly one desktop restarts with the original custom data path',observed.count===1&&observed.customData);
 for(const [index,c]of clients.entries()){
  let ready=false;for(let i=0;i<100;i++){try{await call(c,'get_entries',{id:entry.id});ready=true;break;}catch{await sleep(150);}}
  check(`original MCP connection ${index+1} resumes without reconnect`,ready&&transports[index].pid===ids[index]);
 }
 const config=await call(a,'get_mcp_config',{});
 if(!rollback)check('new MCP configuration uses lowercase vault',!!config.mcpServers?.vault&&!config.mcpServers?.Vault);
 const output=join(root,'decrypted.txt');await call(b,'write_credential',{credentialId:entry.id,field:'secret',target:{type:'file',path:output}});
 check('original encrypted credential decrypts after replacement',await readFile(output,'utf8')==='FAKE_INSTALLER_ONLY_6134');
 report.executableSha256=await hash(executable);report.passed=true;
}finally{
 clearInterval(watcher);
 writeLock?.kill();
 for(const c of clients)await c.close().catch(()=>{});
 // Only processes whose exact image is inside this unique fixture are cleanup targets.
 await run('powershell.exe',['-NoProfile','-Command',`Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_AUDIT_IMAGE } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }`],{windowsHide:true,env:{...process.env,VAULT_AUDIT_IMAGE:executable}}).catch(()=>{});
 if(process.env.VAULT_INSTALLER){
  // Registration is compiled into an audit namespace locally; never touch D:\\vault.
  try{await sleep(800);if(existsSync(join(folder,'Uninstall.exe')))await install(join(folder,'Uninstall.exe'),['/S']);}catch{}
 }
 await mkdir('artifacts',{recursive:true});await writeFile(process.env.VAULT_HANDOFF_REPORT||'artifacts/installer-handoff-verification.json',JSON.stringify(report,null,2));
}
