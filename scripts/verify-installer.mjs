import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {readFile,writeFile} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import assert from 'node:assert/strict';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
const run=promisify(execFile),artifacts=resolve(process.env.VAULT_ARTIFACTS||'artifacts');
const reportPath=join(artifacts,'installer-verification.json');
const readReport=async()=>JSON.parse((await readFile(reportPath,'utf8')).replace(/^\uFEFF/,''));
async function phase(name){
 const args=['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/verify-installer.ps1','-ArtifactDirectory',artifacts,'-Phase',name];
 if(process.env.VAULT_INSTALLER_NATIVE!=='1')args.push('-Silent');
 const result=await run('powershell.exe',args,{windowsHide:true,timeout:120000});console.log(result.stdout.trim());
}
let installed=false,client;
const checks=[];
try{
 await phase('Install');installed=true;
 const fixture=await readReport(),executable=join(fixture.directory,'vault.exe');
 const acceptance=await run(process.execPath,['scripts/verify-next.mjs'],{windowsHide:true,timeout:180000,env:{...process.env,VAULT_EXE:executable},maxBuffer:1024*1024});
 console.log(acceptance.stdout.trim());checks.push({name:'installed-mcp-and-native-desktop',passed:true});
 async function connect(){
  client=new Client({name:'installer-preservation',version:'1'});
  const transport=new StdioClientTransport({command:executable,args:['--mcp'],stderr:'pipe'});transport.stderr?.resume();await client.connect(transport);
 }
 async function call(name,args){const r=await client.callTool({name,arguments:args});assert.ok(!r.isError);return JSON.parse(r.content.find(c=>c.type==='text').text);}
 await connect();
 const created=await call('create_entries',{kind:'password',title:'Installer preservation fixture',secret:'FAKE_INSTALLER_SECRET_ONLY_5917'});
 const id=created.id;assert.ok(id);await client.close();client=null;await new Promise(r=>setTimeout(r,6000));
 await phase('Reinstall');await connect();
 await call('get_entries',{id});
 const target=join(fixture.fixture,'preserved.txt');
 await call('write_credential',{credentialId:id,field:'secret',target:{type:'file',path:target}});
 assert.equal(await readFile(target,'utf8'),'FAKE_INSTALLER_SECRET_ONLY_5917');
 checks.push({name:'encrypted-credential-decrypts-after-reinstall',passed:true});
 await client.close();client=null;await new Promise(r=>setTimeout(r,6000));
 const dbBefore=await readFile(join(fixture.directory,'data','vault.db'));
 await phase('Remove');installed=false;
 assert.deepEqual(await readFile(join(fixture.directory,'data','vault.db')),dbBefore);
 checks.push({name:'encrypted-database-unchanged-on-uninstall',passed:true});
 const report=await readReport();report.checks.push(...checks);await writeFile(reportPath,JSON.stringify(report,null,2));
 console.log('PASS installed application, reinstall and uninstall credential preservation');
}finally{
 await client?.close().catch(()=>{});
 if(installed){await new Promise(r=>setTimeout(r,6000));await phase('Remove').catch(()=>{});}
}
