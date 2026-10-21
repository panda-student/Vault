import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,readFile,writeFile,mkdir,realpath} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
const executable=resolve(process.env.VAULT_EXE||'src-tauri/target/release/local-password-manager.exe');
const root=await mkdtemp(join(tmpdir(),'vault-next-'));
const data=join(root,'data'),report={passed:false,checks:[],root};
const check=(name,ok)=>{report.checks.push({name,passed:!!ok});assert.ok(ok,name);console.log('PASS',name);};
const client=new Client({name:'next-acceptance',version:'1'});
let transport,desktop,dialogHelper;
const secret='FAKE_NEXT_ACCEPTANCE_ONLY_4821',outputs=[];
async function call(name,args){
 let r;try{r=await client.callTool({name,arguments:args});}catch(e){if(e.code===-32602)return {failed:true,value:{code:'UNKNOWN_TOOL'}};throw e;}outputs.push(JSON.stringify(r));
 const text=r.content?.filter(c=>c.type==='text').map(c=>c.text).join('')||'{}';
 return {failed:!!r.isError,value:JSON.parse(text)};
}
try{
 transport=new StdioClientTransport({command:executable,args:['--mcp','--data-dir',data],stderr:'pipe'});
 await client.connect(transport);transport.stderr?.on('data',b=>outputs.push(String(b)));
 check('mcp-server-name',client.getServerVersion()?.name==='Vault');
 const config=await call('get_mcp_config',{});
 const launch=config.value.mcpServers?.Vault;
 check('mcp-config-name-and-current-path',!config.failed&&launch&&await realpath(launch.command)===await realpath(executable)&&launch.args[0]==='--mcp'&&launch.args[1]==='--data-dir'&&await realpath(launch.args[2])===await realpath(data));
 const tools=(await client.listTools()).tools;
 check('twelve-tools-no-network-or-purge',tools.length===12&&!tools.some(t=>/execute_api|trash|purge/.test(t.name)));
 const created=await call('create_entries',{items:[
  {kind:'database',title:'MCP DB UVT',group:'MCP System',tags:['UVT'],database:{databaseType:'Oracle',host:'db-next.invalid',port:'1521',databaseName:'ORCL'},secret},
  {kind:'database',title:'MCP DB Local',database:{databaseType:'Redis',host:'127.0.0.1',port:'6379',databaseName:'0'},secret:''},
  {kind:'api',title:'MCP API',services:[{protocol:'openai',url:'https://one.invalid/v1'},{protocol:'anthropic',url:'https://two.invalid'}],secret}
 ]});
 check('batch-create-database-and-api',!created.failed&&created.value.results.length===3);
 const [db,local,api]=created.value.results;
 const meta=await call('get_entries',{id:db.id});
 check('metadata-shows-group-tags-database-not-secret',!meta.failed&&meta.value.tags.includes('MCP System')&&meta.value.database.databaseName==='ORCL'&&!('secret' in meta.value)&&!('notes' in meta.value));
 for(const query of ['MCP System','UVT','db-next.invalid']){
  const r=await call('search_entries',{query});check(`search:${query}`,r.value.entries.some(e=>e.id===db.id));
 }
 for(const [field,expected] of [['host','db-next.invalid'],['port','1521'],['database_name','ORCL'],['secret',secret]]){
  const path=join(root,`${field}.txt`);
  const r=await call('write_credential',{credentialId:db.id,field,target:{type:'file',path}});
  check(`local-field-write:${field}`,!r.failed&&(await readFile(path,'utf8'))===expected);
 }
 const poisoned=await call('update_entries',{id:db.id,expectedRevision:1,group:secret,tags:[secret]});
 check('metadata-update',!poisoned.failed);
 const guarded=await call('get_entries',{id:db.id});
 check('new-fields-filter-protected-values',guarded.value.group===''&&guarded.value.tags.some(t=>t===null));
 const oracle=await call('search_entries',{query:secret});
 check('redacted-fields-not-search-oracle',oracle.value.entries.length===0);
 const bad=await call('delete_entries',{items:[{id:db.id,expectedRevision:2},{id:api.id,expectedRevision:999}]});
 check('batch-delete-rollback',bad.failed&&!(await call('get_entries',{id:db.id})).failed);
 const removed=await call('delete_entries',{id:db.id,expectedRevision:2});
 check('soft-delete',!removed.failed&&(await call('get_entries',{id:db.id})).failed);
 check('deleted-write-blocked',(await call('write_credential',{credentialId:db.id,field:'secret',target:{type:'file',path:join(root,'deleted.txt')}})).failed);
 check('purge-tool-unavailable',(await call('empty_trash',{})).failed);
 check('removed-api-tool-unavailable',(await call('execute_api_request',{credentialId:api.id,path:'/'})).failed);
 // P1: 校验失败必须携带字段级 details，AI 才能定位字段自纠而不是盲试。
 const detailTarget=(await call('create_entries',{kind:'password',title:'Acceptance Restore',tags:['AcceptanceTag'],secret})).value;
 const mismatch=await call('update_entries',{id:detailTarget.id,expectedRevision:detailTarget.revision,url:'https://mismatch.invalid',services:[]});
 check('invalid-input-details-name-field',mismatch.failed&&Array.isArray(mismatch.value.details)&&mismatch.value.details.some(d=>d.field==='services'&&typeof d.reason==='string'&&d.reason.length>0));
 // P2: MCP 恢复垃圾桶条目（更新权限范围内），输出保持受控元数据（无备注、无密钥）。
 check('delete-for-restore-cycle',!(await call('delete_entries',{id:detailTarget.id,expectedRevision:detailTarget.revision})).failed);
 const restored=await call('restore_entries',{id:detailTarget.id,expectedRevision:detailTarget.revision});
 check('mcp-restore-returns-controlled-metadata',!restored.failed&&restored.value.id===detailTarget.id&&!('notes' in restored.value)&&!('secret' in restored.value));
 const pinned=await call('pin_entries',{id:detailTarget.id,expectedRevision:restored.value.revision,pinned:true});
 check('mcp-pin-returns-controlled-metadata',!pinned.failed&&pinned.value.pinned===true&&pinned.value.revision===restored.value.revision+1);
 // P2: 全局标签重命名与删除；标签无权限规则时不构成提权，应放行并全局生效。
 check('mcp-organize-rename',!(await call('organize',{kind:'tag',name:'AcceptanceTagRenamed',previous:'AcceptanceTag'})).failed);
 check('organize-rename-applies-globally',(await call('get_entries',{id:detailTarget.id})).value.tags.includes('AcceptanceTagRenamed'));
 check('mcp-organize-delete',!(await call('organize',{kind:'tag',name:'',previous:'AcceptanceTagRenamed'})).failed);
 check('organize-delete-clears-tag',(await call('get_entries',{id:detailTarget.id})).value.tags.length===0);
 const beforeEmptyTagDelete=(await call('get_entries',{id:detailTarget.id})).value;
 check('mcp-create-unused-tag',!(await call('organize',{kind:'tag',name:'UnusedCatalogTag'})).failed);
 check('mcp-delete-unused-tag-directly',!(await call('organize',{kind:'tag',name:'',previous:'UnusedCatalogTag'})).failed);
 const afterEmptyTagDelete=(await call('get_entries',{id:detailTarget.id})).value;
 check('unused-tag-delete-preserves-entry',JSON.stringify(beforeEmptyTagDelete)===JSON.stringify(afterEmptyTagDelete));
 const missingTag=await call('organize',{kind:'tag',name:'',previous:'UnusedCatalogTag'});
 check('deleted-tag-is-not-found',missingTag.failed&&missingTag.value.code==='NOT_FOUND');
 check('all-ai-output-excludes-original-and-base64',outputs.every(o=>!o.includes(secret)&&!o.includes(Buffer.from(secret).toString('base64'))));
 await client.close();
 // Isolated desktop fixture: no real data, no display of protected values in reports.
 const smokePath=join(root,'desktop-smoke.json');
 desktop=spawn(executable,['--data-dir',join(root,'desktop-data'),'--smoke-test',smokePath],{windowsHide:true,stdio:'ignore'});
 const dialogReport=join(root,'native-trash.json');
 dialogHelper=spawn('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('scripts/verify-native-trash.ps1'),'-ProcessId',String(desktop.pid),'-ReportPath',dialogReport],{windowsHide:true,stdio:'ignore'});
 const closed=await Promise.race([once(desktop,'close'),new Promise(r=>setTimeout(()=>r(null),100000).unref())]);
 if(!closed){desktop.kill();throw Error('Desktop test timeout');}
 report.desktop=JSON.parse(await readFile(smokePath,'utf8'));
 check('native-desktop-suite',closed[0]===0&&report.desktop.passed);
 report.nativeTrash=JSON.parse((await readFile(dialogReport,'utf8')).replace(/^\uFEFF/,''));check('native-human-confirmation-cancel-and-confirm',report.nativeTrash.passed);
 report.executableSha256=createHash('sha256').update(await readFile(executable)).digest('hex');
 report.passed=true;
}catch(e){report.error=String(e);process.exitCode=1;console.error(report.error);}
finally{try{await client.close();}catch{}await mkdir('docs/evidence',{recursive:true});await writeFile('docs/evidence/next-verification.json',JSON.stringify(report,null,2));}
