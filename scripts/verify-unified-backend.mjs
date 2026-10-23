// Actual Tauri WebView + two stdio MCP clients, with isolated synthetic data.
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp,mkdir,copyFile,readFile,writeFile,realpath,readdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve,dirname} from 'node:path';
import {createHash} from 'node:crypto';
import net from 'node:net';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
const run=promisify(execFile),sleep=ms=>new Promise(r=>setTimeout(r,ms));
const root=await realpath(await mkdtemp(join(tmpdir(),'vault-unified-backend-')));
const agent=process.env.AGENT_BROWSER_EXE||join(dirname(process.execPath),'node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
const source=resolve(process.env.VAULT_EXE||'src-tauri/target/debug/local-password-manager.exe');
const report={passed:false,root,executableSha256:createHash('sha256').update(await readFile(source)).digest('hex'),checks:[]};
const check=(name,ok)=>{report.checks.push({name,passed:!!ok});console.log(`${ok?'PASS':'FAIL'} ${name}`);if(!ok)throw Error(name);};
const call=async(c,name,args={})=>{const r=await c.callTool({name,arguments:args});return {failed:!!r.isError,value:r.structuredContent??JSON.parse(r.content.find(c=>c.type==='text').text)};};
try {
for(const [index,order]of ['desktop-first','mcp-first'].entries()){
 const folder=join(root,order),data=join(folder,'data'),exe=join(folder,'vault.exe'),port=18461+index;
 await mkdir(folder,{recursive:true});await copyFile(source,exe);
 const env={...process.env,VAULT_UI_DEBUG_PORT:String(port)},clients=[];
 const session='vault-unified-'+process.pid+'-'+index;
 const browser=async(...args)=>{
  for(let attempt=0;;attempt++){
   try{return (await run(agent,['--session',session,'--cdp',String(port),...args],{windowsHide:true,timeout:30000,maxBuffer:1024*1024})).stdout.trim();}
   catch(error){if(attempt>=1||!String(error.stderr).includes('different daemon configuration'))throw error;await sleep(500);}
  }
 };
 const evaluate=async expression=>JSON.parse(await browser('eval',expression));
 const invoke=(name,args={})=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`);
 const hosts=async()=>{const result=await run('powershell.exe',['-NoProfile','-Command',`@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_AUDIT_EXE -and $_.CommandLine -notmatch '(?:^|\\s)--mcp(?:\\s|$)' } | Select-Object -ExpandProperty ProcessId)|ConvertTo-Json -Compress`],{windowsHide:true,env:{...env,VAULT_AUDIT_EXE:exe}});const ids=JSON.parse(result.stdout.trim()||'[]');return Array.isArray(ids)?ids:[ids];};
 const connect=async()=>{const c=new Client({name:'unified-backend-audit',version:'1'});const t=new StdioClientTransport({command:exe,args:['--mcp','--data-dir',data],env,stderr:'pipe'});t.stderr?.resume();await c.connect(t);clients.push(c);return c;};
 const start=()=>spawn(exe,['--data-dir',data],{env,windowsHide:true,stdio:'ignore'});
 let initial,launcher;
 try{
  if(order==='desktop-first')launcher=start();
  const a=await connect(),b=await connect();
  initial=await hosts();check(order+': one backend before activation',initial.length===1);
  if(order==='mcp-first'){
   let noWebview=false;try{await fetch(`http://127.0.0.1:${port}/json/list`,{signal:AbortSignal.timeout(500)});}catch{noWebview=true;}
   check(order+': backend initially has no WebView',noWebview);
   launcher=start();
  }
  let ready=false;for(let attempt=0;attempt<160;attempt++){
   try{const targets=await(await fetch(`http://127.0.0.1:${port}/json/list`,{signal:AbortSignal.timeout(500)})).json();if(targets.some(t=>t.type==='page')){ready=true;break;}}catch{}await sleep(150);
  }
  check(order+': actual desktop WebView is ready',ready);
  await browser('wait','--fn','!!window.__TAURI_INTERNALS__');
  const info=await invoke('desktop_info');
  check(order+': desktop and both MCP clients use the same host PID',info.backendPid===initial[0]&&JSON.stringify(await hosts())===JSON.stringify(initial));
  if(order==='mcp-first'){for(let n=0;n<30&&launcher.exitCode===null;n++)await sleep(100);check(order+': second launcher exits without creating another backend',launcher.exitCode===0);}
  const canonical='\\\\?\\'+await realpath(exe);
  const base=join(process.env.LOCALAPPDATA,'LocalPasswordManager','updates',createHash('sha256').update(canonical.toLowerCase()).digest('hex'));
  const names=await readdir(join(base,'services'));
  let activationReply='';
  await new Promise(resolve=>{
   const socket=net.createConnection('\\\\.\\pipe\\vault-'+names[0]+'-desktop');
   const timer=setTimeout(()=>{socket.destroy();resolve();},3000);
   socket.on('connect',()=>socket.write('{"caller":"desktop","operation":"ReadField"}\n'));
   socket.on('data',data=>{activationReply+=String(data);});
   socket.on('error',()=>{});socket.on('close',()=>{clearTimeout(timer);resolve();});
  });
  check(order+': non-vault process cannot use the desktop activation channel',activationReply==='');
  const fromAi=await call(a,'create_entries',{kind:'password',title:'created by MCP',secret:'FAKE_UNIFIED_ONLY_5319'});
  const list=await invoke('list_entries',{});
  check(order+': desktop reads MCP-created credential',list.some(e=>e.id===fromAi.value.id));
  const desktop=await invoke('create_entry',{input:{kind:'password',title:'created by desktop',account:'fixture',secret:'FAKE_DESKTOP_ONLY_6420'}});
  const read=await call(b,'get_entries',{id:desktop.id});
  check(order+': MCP reads desktop-created metadata without plaintext',!read.failed&&read.value.id===desktop.id&&!JSON.stringify(read.value).includes('FAKE_DESKTOP_ONLY_6420'));
  const settings=info.settings;settings.mcpPermissions.create=false;
  await invoke('save_settings',{settings});
  check(order+': desktop permission change immediately rejects existing MCP connection',(await call(a,'create_entries',{kind:'password',title:'must fail',secret:'FAKE_DENIED_ONLY'})).failed);
  const allowed=await invoke('create_entry',{input:{kind:'password',title:'desktop retains permission',secret:'FAKE_ALLOWED_ONLY'}});
  check(order+': desktop identity retains its permissions',!!allowed.id);
  settings.mcpPermissions.create=true;await invoke('save_settings',{settings});
  check(order+': same MCP connection recovers after permission restore',!(await call(a,'create_entries',{kind:'password',title:'restored permission',secret:'FAKE_RESTORED_ONLY'})).failed);
  await browser('screenshot',resolve(`artifacts/unified-backend-${order}.png`));
  settings.closeToTray=false;await invoke('save_settings',{settings});
  await run('powershell.exe',['-NoProfile','-Command',`
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class FixtureClose{[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);}'
$target=Get-Process -Id $env:VAULT_AUDIT_PID
if($target.Path -ne $env:VAULT_AUDIT_EXE -or $target.MainWindowHandle -eq 0){throw 'Not the isolated desktop'}
[void][FixtureClose]::PostMessage($target.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
`],{windowsHide:true,env:{...env,VAULT_AUDIT_PID:String(initial[0]),VAULT_AUDIT_EXE:exe}});
  await sleep(1000);
  check(order+': closing desktop keeps existing MCP backend alive',JSON.stringify(await hosts())===JSON.stringify(initial)&&!(await call(a,'get_entries',{id:desktop.id})).failed);
  const reopen=start();for(let n=0;n<50&&reopen.exitCode===null;n++)await sleep(100);
  check(order+': reopening desktop reuses the retained backend',reopen.exitCode===0&&JSON.stringify(await hosts())===JSON.stringify(initial));
 }finally{
  for(const c of clients)await c.close().catch(()=>{});
  await run('powershell.exe',['-NoProfile','-Command',`Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_AUDIT_EXE } | ForEach-Object {Stop-Process -Id $_.ProcessId -Force}`],{windowsHide:true,env:{...env,VAULT_AUDIT_EXE:exe}}).catch(()=>{});
 }
}
report.passed=true;
} catch(error) {report.error=String(error);process.exitCode=1;console.error(report.error);}
finally {await mkdir('artifacts',{recursive:true});await writeFile('artifacts/unified-backend-verification.json',JSON.stringify(report,null,2));}
