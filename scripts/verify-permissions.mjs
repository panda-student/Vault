// All data are synthetic. Direct fixture-settings injection simulates another
// desktop process saving policy; no production-only policy mutation tool exists.
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp,readFile,writeFile,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const run=promisify(execFile), executable=resolve(process.env.VAULT_EXE||'src-tauri/target/release/local-password-manager.exe');
const root=await mkdtemp(join(tmpdir(),'vault-permissions-')),data=join(root,'data');
const report={passed:false,checks:[],executableSha256:createHash('sha256').update(await readFile(executable)).digest('hex')};
const check=(name,passed)=>{report.checks.push({name,passed:!!passed});assert.ok(passed,name);console.log('PASS',name);};
const client=new Client({name:'permission-acceptance',version:'1'}), outputs=[];
const transport=new StdioClientTransport({command:executable,args:['--mcp','--data-dir',data],stderr:'pipe'});
transport.stderr?.on('data',b=>outputs.push(String(b)));
const secret='FAKE_PERMISSION_FIXTURE_ONLY_6104';
const call=async(name,args={})=>{
 const result=await client.callTool({name,arguments:args});outputs.push(JSON.stringify(result));
 return {failed:!!result.isError,value:result.structuredContent??JSON.parse(result.content[0].text)};
};
const names=async()=>(await client.listTools()).tools.map(tool=>tool.name);
const inject=`import sqlite3,json,sys
c=sqlite3.connect(sys.argv[1]);s=json.loads(c.execute("SELECT value FROM metadata WHERE name='settings'").fetchone()[0]);s.update(json.loads(sys.argv[2]));c.execute("UPDATE metadata SET value=? WHERE name='settings'",(json.dumps(s).encode(),));c.commit();c.close()`;
const defaults={readMetadata:true,create:true,update:true,delete:true,fillInput:true,writeFile:true};
const policy=async(changes={},enabled=true)=>run('python',['-c',inject,join(data,'vault.db'),JSON.stringify({mcpEnabled:enabled,mcpPermissions:{...defaults,...changes}})],{windowsHide:true});
try {
 await client.connect(transport);
 check('default-twelve-tools',(await names()).length===12);
 const created=await call('create_entries',{kind:'password',title:'Permission fixture',secret});
 const {id,revision}=created.value;
 check('core-mutation-projection-is-minimal',!created.failed&&Object.keys(created.value).sort().join(',')==='id,revision,status');
 const scenarios=[
  ['readMetadata',['search_entries','get_entries'],[['get_entries',{id}],['search_entries',{query:'Permission'}]]],
  ['create',['create_entries'],[['create_entries',{kind:'password',title:'denied',secret}]]],
  ['update',['update_entries'],[['update_entries',{id,expectedRevision:revision,title:'denied'}]]],
  ['delete',['delete_entries'],[['delete_entries',{id,expectedRevision:revision}]]],
  ['fillInput',['capture_fill_target','fill_credential'],[['capture_fill_target',{}],['fill_credential',{credentialId:id,field:'secret',targetId:'invalid'}],['write_credential',{credentialId:id,field:'secret',target:{type:'input',targetId:'invalid'}}]]],
  ['writeFile',[],[['write_credential',{credentialId:id,field:'secret',target:{type:'file',path:join(root,'denied.txt')}}]]],
 ];
 for(const [key,hidden,calls] of scenarios){
  await policy({[key]:false});
  const discovered=await names();check(`discovery-revoked:${key}`,hidden.every(name=>!discovered.includes(name)));
  for(const [name,args] of calls){const result=await call(name,args);check(`live-core-rejects:${key}:${name}`,result.failed&&result.value.code==='PERMISSION_DENIED');}
  if(key==='fillInput'){
   const path=join(root,'allowed-file.txt'),result=await call('write_credential',{credentialId:id,field:'secret',target:{type:'file',path}});
   check('window-denial-does-not-disable-file',!result.failed&&(await readFile(path,'utf8'))===secret);
  }
  if(key==='writeFile'){
   const result=await call('write_credential',{credentialId:id,field:'secret',target:{type:'input',targetId:'invalid'}});
   check('file-denial-does-not-disable-window',!result.failed&&result.value.status==='target_unavailable');
  }
  await policy();check(`same-connection-restores:${key}`,(await names()).length===12);
 }
 await policy(Object.fromEntries(Object.keys(defaults).map(key=>[key,false])));
 check('all-revoked-only-config-discovered',(await names()).join(',')==='get_mcp_config');
 check('all-revoked-write-not-executed',(await call('write_credential',{credentialId:id,field:'secret',target:{type:'file',path:join(root,'all-denied.txt')}})).value.code==='PERMISSION_DENIED');
 await policy({},false);check('master-off-hides-all-tools',(await names()).length===0);
 check('master-off-rejects-existing-connection',(await call('get_mcp_config')).value.code==='MCP_DISABLED');
 await policy();
 const current=await call('get_entries',{id});check('denied-mutations-preserve-original-entry',!current.failed&&current.value.revision===revision&&current.value.title==='Permission fixture');
 const made=await call('create_entries',{items:[{kind:'password',title:'scoped allowed',tags:['work'],secret},{kind:'password',title:'scoped hidden',tags:['work','production'],secret}]});
 const [allowed,hidden]=made.value.results;
 const objectPolicy=async patch=>run('python',['-c',inject,join(data,'vault.db'),JSON.stringify(patch)],{windowsHide:true});
 await objectPolicy({mcpAccess:false,mcpTagRules:{work:{access:true},production:{access:false}},mcpEntryRules:{}});
 const scoped=await call('search_entries',{});check('tag-scope-filters-wire-search',scoped.value.entries.length===1&&scoped.value.entries[0].id===allowed.id);
 check('mixed-scope-batch-read-is-denied',(await call('get_entries',{items:[{id:allowed.id},{id:hidden.id}]})).value.code==='PERMISSION_DENIED');
 check('hidden-file-write-denied',(await call('write_credential',{credentialId:hidden.id,field:'secret',target:{type:'file',path:join(root,'hidden-write.txt')}})).value.code==='PERMISSION_DENIED');
 await objectPolicy({mcpEntryRules:{[hidden.id]:{access:true}}});check('entry-override-restores-wire-read',!(await call('get_entries',{id:hidden.id})).failed);
 await objectPolicy({mcpAccess:true,mcpTagRules:{production:{update:false,delete:false,create:false}},mcpEntryRules:{}});
 const deniedUpdate=await call('update_entries',{items:[{id:allowed.id,expectedRevision:allowed.revision,title:'should roll back'},{id:hidden.id,expectedRevision:hidden.revision,title:'denied'}]});
 check('scoped-batch-update-rolls-back',deniedUpdate.failed&&(await call('get_entries',{id:allowed.id})).value.title==='scoped allowed');
 const deniedDelete=await call('delete_entries',{items:[{id:allowed.id,expectedRevision:allowed.revision},{id:hidden.id,expectedRevision:hidden.revision}]});check('scoped-batch-delete-rolls-back',deniedDelete.failed&&!(await call('get_entries',{id:allowed.id})).failed);
 await objectPolicy({mcpTagRules:{},mcpEntryRules:{},mcpAccess:true});
 const spoof=await call('create_entries',{kind:'password',title:'spoof',secret,caller:'desktop'});check('caller-payload-cannot-forge-identity',spoof.failed&&spoof.value.code==='INVALID_INPUT');
 check('all-output-excludes-protected-plaintext-and-base64',outputs.every(text=>!text.includes(secret)&&!text.includes(Buffer.from(secret).toString('base64'))));
 report.passed=true;
} catch(error){report.error=String(error);process.exitCode=1;console.error(report.error);}
finally{await client.close();await mkdir('docs/evidence',{recursive:true});await writeFile('docs/evidence/permissions-verification.json',JSON.stringify(report,null,2));}
