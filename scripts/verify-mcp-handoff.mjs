// Isolated real-process regression: shared core, drain, pause, resume, no write replay.
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {mkdtemp,copyFile,rename,readFile,writeFile,readdir,unlink,mkdir,realpath} from 'node:fs/promises';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createHash,randomUUID} from 'node:crypto';
import assert from 'node:assert/strict';
const run=promisify(execFile),pause=ms=>new Promise(r=>setTimeout(r,ms));
const root=await mkdtemp(join(tmpdir(),'vault-mcp-shared-'));
const exe=join(root,'Vault.exe'),data=join(root,'data');
await copyFile(resolve(process.env.VAULT_EXE||'src-tauri/target/release/local-password-manager.exe'),exe);
const clients=[],report={passed:false,root,checks:[]};
let lock;
const check=(name,ok)=>{report.checks.push({name,passed:!!ok});assert.ok(ok,name);console.log('PASS',name);};
async function connect(){const c=new Client({name:'shared-core-regression',version:'1'});const t=new StdioClientTransport({command:exe,args:['--mcp','--data-dir',data],stderr:'pipe'});await c.connect(t);c.transportFixture=t;clients.push(c);return c;}
const call=async(c,name,args={})=>{const r=await c.callTool({name,arguments:args});assert(!r.isError);return r.structuredContent??JSON.parse(r.content[0].text);};
async function services(){const command="@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:VAULT_TEST_EXE -and $_.CommandLine.Contains('--mcp-service') -and $_.CommandLine.Contains($env:VAULT_TEST_DIR) } | Select-Object -ExpandProperty ProcessId) | ConvertTo-Json -Compress";const r=await run('powershell.exe',['-NoProfile','-Command',command],{windowsHide:true,env:{...process.env,VAULT_TEST_EXE:exe,VAULT_TEST_DIR:data}});const v=JSON.parse(r.stdout.trim()||'[]');return Array.isArray(v)?v:[v];}
async function resumed(c){const end=Date.now()+20000;while(Date.now()<end){try{if((await c.listTools()).tools.length===12)return;}catch{}await pause(100);}throw Error('Original MCP connection did not resume');}
try{
 const [a,b]=await Promise.all([connect(),connect()]);
 const pids=clients.map(c=>c.transportFixture.pid);
 const first=await services();check('two-connections-share-exactly-one-core',first.length===1);report.firstCore=first[0];
 const entry=await call(a,'create_entries',{kind:'password',title:'shared fixture',secret:'FAKE_SHARED_CORE_ONLY_9157'});
 check('second-connection-reads-first-connection-entry',(await call(b,'get_entries',{id:entry.id})).id===entry.id);
 const canonical='\\\\?\\'+await realpath(exe);
 const base=join(process.env.LOCALAPPDATA,'LocalPasswordManager','updates',createHash('sha256').update(canonical.toLowerCase()).digest('hex'));
 const token=randomUUID(),flag=join(base,'pause.json');
 lock=spawn('python',['-c',"import sqlite3,sys,time; c=sqlite3.connect(sys.argv[1]); c.execute('BEGIN IMMEDIATE'); print('locked',flush=True); time.sleep(3); c.commit(); c.close()",join(data,'vault.db')],{windowsHide:true,stdio:['ignore','pipe','pipe']});
 await new Promise((res,rej)=>{lock.stdout.once('data',res);lock.once('error',rej);});
 const running=call(a,'create_entries',{kind:'password',title:'accepted before pause',secret:'FAKE_RUNNING_WRITE_3416'});
 await pause(150);
 await writeFile(flag,JSON.stringify({token,expires:Math.floor(Date.now()/1000)+300}));
 await pause(200);
 const registrations=(await readdir(join(base,'bridges'))).map(name=>join(base,'bridges',name,'paused'));
 const ack=async()=>Promise.all(registrations.map(p=>readFile(p,'utf8').catch(()=>'')));
 check('pause-does-not-acknowledge-in-flight-write',(await ack()).some(v=>v!==token));
 let rejected=false;try{await b.callTool({name:'create_entries',arguments:{kind:'password',title:'must not replay',secret:'FAKE_REJECTED_WRITE_5664'}});}catch(e){rejected=String(e).includes('UPDATE_IN_PROGRESS');}
 check('new-write-is-explicitly-rejected-during-pause',rejected);
 const committed=await running;check('accepted-write-completes-before-pause',!!committed.id);
 const end=Date.now()+12000;while(Date.now()<end){const files=await readdir(join(base,'bridges'));const values=await Promise.all(files.map(p=>readFile(join(base,'bridges',p,'paused'),'utf8').catch(()=>'')));if(values.length>=2&&values.every(v=>v===token))break;await pause(75);}
 check('shared-core-exits-after-all-connections-drain',(await services()).length===0);
 await rename(exe,join(root,'previous.exe'));await copyFile(join(root,'previous.exe'),exe);
 await unlink(flag);
 await Promise.all([resumed(a),resumed(b)]);
 const next=await services();check('resume-starts-one-new-shared-core',next.length===1&&next[0]!==first[0]);report.resumedCore=next[0];
 check('client-facing-processes-unchanged',clients.every((c,i)=>c.transportFixture.pid===pids[i]));
 const found=await call(a,'search_entries',{});
 check('completed-write-occurs-once-and-rejected-write-is-not-replayed',found.entries.filter(e=>e.title==='accepted before pause').length===1&&!found.entries.some(e=>e.title==='must not replay'));
 await a.close();await resumed(b);check('closing-one-client-keeps-shared-core',JSON.stringify(await services())===JSON.stringify(next));
 report.passed=true;
}catch(e){report.error=String(e);process.exitCode=1;console.error(report.error);}
finally{for(const c of clients)await c.close().catch(()=>{});lock?.kill();await mkdir('artifacts',{recursive:true});await writeFile('artifacts/mcp-shared-verification.json',JSON.stringify(report,null,2)+'\n');}
