import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {mkdtemp,readFile,writeFile} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {DatabaseSync} from 'node:sqlite';
import {createHash} from 'node:crypto';
const oldExe=resolve(process.env.VAULT_OLD_EXE||'artifacts/vault-0.2.0/Vault.exe');
const newExe=resolve(process.env.VAULT_EXE||'src-tauri/target/release/local-password-manager.exe');
const root=await mkdtemp(join(tmpdir(),'vault-migration-')),data=join(root,'data');
const report={passed:false,checks:[]};let client;
function check(name,passed){report.checks.push({name,passed});if(!passed)throw Error(name);console.log('PASS',name);}
async function connect(exe){const c=new Client({name:'migration-fixture',version:'1'});await c.connect(new StdioClientTransport({command:exe,args:['--mcp','--data-dir',data],stderr:'ignore'}));return c;}
async function call(name,args){const r=await client.callTool({name,arguments:args});if(r.isError)throw Error('Fixture call failed');return r.structuredContent??JSON.parse(r.content[0].text);}
try{
 client=await connect(oldExe);
 const a=await call('create_entries',{kind:'api',title:'Legacy fixture',secret:'FAKE_LEGACY_ONLY_777',services:[{protocol:'figma',url:'https://figma-fixture.invalid/v1'},{protocol:'gitea',url:'https://gitea-fixture.invalid/api/v1'}]});
 const before=await call('get_entries',{id:a.id});await client.close();
 const oldDb=new DatabaseSync(join(data,'vault.db'),{readOnly:true});check('fixture-is-schema-one',oldDb.prepare('PRAGMA user_version').get().user_version===1);oldDb.close();
 client=await connect(newExe);const after=await call('get_entries',{id:a.id});
 check('existing-id-and-revision-preserved',before.id===after.id&&before.revision===after.revision);
 check('rest-protocols-normalized-with-addresses-ids-preserved',after.services.every((s,i)=>s.protocol==='custom'&&s.url===before.services[i].url&&s.id===before.services[i].id));
 check('old-entries-default-ungrouped-untagged',after.group===''&&after.tags.length===0);
 const path=join(root,'local-secret-check.txt');await call('write_credential',{credentialId:a.id,field:'secret',target:{type:'file',path}});
 check('secret-survives-schema-migration',(await readFile(path,'utf8'))==='FAKE_LEGACY_ONLY_777');
 await client.close();
 const migrated=new DatabaseSync(join(data,'vault.db'),{readOnly:true});check('schema-two-and-trash-table',migrated.prepare('PRAGMA user_version').get().user_version===2&&!!migrated.prepare("SELECT name FROM sqlite_master WHERE name='trash'").get());migrated.close();
 report.executableSha256=createHash('sha256').update(await readFile(newExe)).digest('hex');report.passed=true;
}catch(e){report.error=String(e);process.exitCode=1;console.error(report.error);}
finally{try{await client?.close();}catch{}await writeFile('docs/evidence/migration-verification.json',JSON.stringify(report,null,2));}
