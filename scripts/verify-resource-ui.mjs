// Real WebView2 interactions on scripts/ui-audit-host.mjs's isolated fixture.
import {execFileSync} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import {join} from 'node:path';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
const runtime=JSON.parse(readFileSync('src-tauri/target/ui-audit-runtime.json','utf8'));
const browser=process.env.AGENT_BROWSER_BIN||'D:/nodejs/node_modules/agent-browser/bin/agent-browser-win32-x64.exe';
const report={passed:false,checks:[]};
const run=(...args)=>{const r=JSON.parse(execFileSync(browser,['--session','vault-resource-ui','--cdp',String(runtime.port),'--json',...args],{encoding:'utf8',windowsHide:true,maxBuffer:2e6}));assert.ok(r.success,r.error);return r.data};
const read=s=>run('eval',s).result;
const click=s=>{run('scrollintoview',s);run('click',s)};
const fill=(s,v)=>{run('scrollintoview',s);run('fill',s,v)};
const wait=s=>run('wait','--fn',s);
const check=(name,ok)=>{report.checks.push({name,passed:!!ok});assert.ok(ok,name);console.log('PASS '+name)};
const box=(kind,id,key,parent)=>`${parent?`[data-parent="${parent}"] `:''}[data-kind="${kind}"][data-id="${id}"][data-key="${key}"]`;
const client=new Client({name:'live-ui-policy-verifier',version:'1'});
const call=async(name,args={})=>{const r=await client.callTool({name,arguments:args});return {failed:!!r.isError,value:r.structuredContent??JSON.parse(r.content[0].text)}};
try{
 await client.connect(new StdioClientTransport({command:runtime.executable,args:['--mcp','--data-dir',runtime.directory],stderr:'ignore'}));
 const entries=(await call('search_entries')).value.entries,db=entries.find(e=>e.title==='订单数据库 · UVT');
 check('legacy-group-input-is-tag',db.group===''&&db.tags.includes('订单系统'));
 check('sidebar-has-only-tags',read('!document.querySelector(".sidebar").textContent.includes("分组")&&!document.querySelector("#f-group")'));
 click('[data-nav="mcp"]');wait('document.querySelectorAll("#permission-matrix tbody tr").length===1');
 check('default-row-only',read('document.querySelector("#customize").getAttribute("aria-expanded")==="false"'));
 check('seven-default-permissions-enabled',read('[...document.querySelectorAll("[data-kind=global]")].length===7&&[...document.querySelectorAll("[data-kind=global]")].every(e=>e.checked)'));
 click('#customize');click('[data-expand="订单系统"]');wait('document.querySelectorAll("tr.entry").length===2');check('expand-shares-same-table',read('document.querySelectorAll("tr.global").length===1&&document.querySelectorAll("tr.entry").length===2'));
 click(box('tag','订单系统','readMetadata'));wait('!document.querySelector("[data-kind=entry][data-key=readMetadata]").checked');click('#save-permissions');wait('document.querySelector("#save-permissions").disabled');
 check('saved-tag-rule-filters-live-mcp',(await call('search_entries')).value.entries.every(e=>!e.tags.includes('订单系统')));
 check('explicit-read-denied-through-same-live-session',(await call('get_entries',{id:db.id})).value.code==='PERMISSION_DENIED');
 click(box('tag','订单系统','writeFile'));click('#save-permissions');wait('document.querySelector("#save-permissions").disabled');const target=join(runtime.directory,'denied-file.txt');const denied=await call('write_credential',{credentialId:db.id,field:'secret',target:{type:'file',path:target}});check('tag-file-deny-prevents-side-effects',denied.value.code==='PERMISSION_DENIED'&&!existsSync(target));
 click(`[data-preview="${db.id}"]`);check('preview-source-from-core',read('[...document.querySelectorAll("#permission-preview tbody tr")].some(r=>r.innerText.includes("写入文件")&&r.innerText.includes("禁止")&&r.innerText.includes("订单系统"))'));click('#permission-preview .secondary');
 click('[data-expand="UVT"]');click(box('entry',db.id,'writeFile','订单系统'));wait(`[...document.querySelectorAll('[data-kind=entry][data-id="${db.id}"][data-key=writeFile]')].every(e=>e.checked)`);check('single-exception-syncs-across-tags',read(`document.querySelectorAll('[data-kind=entry][data-id="${db.id}"][data-key=writeFile]').length===2`));
 click('#customize');check('collapse-keeps-pending-rule',read('document.querySelectorAll("#permission-matrix tbody tr").length===1&&!document.querySelector("#save-permissions").disabled'));click('#save-permissions');wait('document.querySelector("#save-permissions").disabled');
 const allowedFile=join(runtime.directory,'allowed-file.txt');check('collapsed-entry-exception-still-authorizes-file',!(await call('write_credential',{credentialId:db.id,field:'secret',target:{type:'file',path:allowedFile}})).failed&&existsSync(allowedFile));
 click('#customize');click(`[data-parent="订单系统"] [data-reset="${db.id}"]`);wait(`!document.querySelector('[data-parent="订单系统"] [data-id="${db.id}"][data-key=writeFile]').checked`);click('#save-permissions');wait('document.querySelector("#save-permissions").disabled');
 check('restore-inheritance-revokes-exception',(await call('write_credential',{credentialId:db.id,field:'secret',target:{type:'file',path:join(runtime.directory,'deny-again.txt')}})).value.code==='PERMISSION_DENIED');
 click('[aria-label="重命名标签 订单系统"]');fill('#organization-name','订单平台');click('#organization-dialog button.primary');wait('!document.querySelector("#organization-dialog").open&&!!document.querySelector("[data-expand=订单平台]")');check('rename-preserves-policy-on-live-mcp',(await call('get_entries',{id:db.id})).value.code==='PERMISSION_DENIED');
 click('[data-expand="订单平台"]');wait('document.querySelector("tr.entry[data-parent=订单平台]")!==null');
 check('renamed-rule-shown-in-matrix',read('!document.querySelector("[data-kind=tag][data-id=订单平台][data-key=writeFile]").checked'));
 click('[aria-label="重命名标签 订单平台"]');click('#delete-tag');click('#delete-tag-dialog .secondary');check('cancel-tag-delete-preserves-label',read('!!document.querySelector("[data-tag=订单平台]")'));click('#delete-tag');click('#delete-tag-dialog .primary');wait('!document.querySelector("#delete-tag-dialog").open&&!document.querySelector("[data-tag=订单平台]")');check('delete-tag-removes-rule-but-keeps-credential',!(await call('get_entries',{id:db.id})).failed);check('tag-delete-keeps-item-count',read('document.querySelector("[data-nav=entries] .count").textContent==="5"'));
 click('#permission-update');click('[data-nav="settings"]');click('[data-nav="mcp"]');check('unsaved-permission-draft-survives-navigation',read('!document.querySelector("#permission-update").checked&&!document.querySelector("#save-permissions").disabled'));click('#revert-permissions');check('revert-restores-saved-default',read('document.querySelector("#permission-update").checked'));
 click('#customize');fill('.permission-search','no-such-label');check('search-retains-default-row-and-empty-state',read('document.querySelectorAll("tr.global").length===1&&document.querySelector("#permission-matrix").innerText.includes("没有匹配")'));fill('.permission-search','');
 for(const [width,height] of [[900,580],[1120,740]]){execFileSync('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/ui-window.ps1','-Action','Resize','-Width',String(width),'-Height',String(height)],{windowsHide:true});wait('document.querySelector("#permission-matrix")!==null');check('native-table-no-overflow-'+width,read('document.documentElement.scrollWidth<=innerWidth+1&&document.querySelector("#permission-matrix").scrollWidth<=document.querySelector(".permission-table-wrap").clientWidth+1'));}
 mkdirSync('docs/screenshots',{recursive:true});run('screenshot','docs/screenshots/current-permissions.png');
 click('[data-nav="entries"]');click('[data-tag="UVT"]');check('tag-navigation-filters-items',read('document.querySelectorAll(".item").length===1'));run('screenshot','docs/screenshots/current-database-small.png');click('[data-nav="entries"]');run('screenshot','docs/screenshots/current-ui.png');
 check('no-webview-runtime-errors',run('errors').errors.length===0);report.passed=true;
}catch(e){report.error=String(e);process.exitCode=1;console.error(e)}finally{await client.close();mkdirSync('docs/evidence',{recursive:true});writeFileSync('docs/evidence/resource-ui-verification.json',JSON.stringify(report,null,2));}
