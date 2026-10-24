import { nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Entry, TrashEntry, Organization } from './useVault';
export async function runSmoke(){
 const checks:{name:string;passed:boolean}[]=[];
 const check=(name:string,passed:boolean)=>{checks.push({name,passed});if(!passed)throw Error(name);};
 const el=<T extends HTMLElement=HTMLElement>(s:string)=>{const e=document.querySelector<T>(s);if(!e)throw Error(`Missing ${s}`);return e;};
 const tick=async()=>{await nextTick();await new Promise(r=>setTimeout(r,150));await nextTick();};
 const until=async(fn:()=>boolean)=>{for(let i=0;i<80;i++){await tick();if(fn())return;}throw Error('UI condition timeout: '+(document.querySelector('#form-error')?.textContent||'no form error'));};
 const click=async(s:string)=>{const e=el(s);e.scrollIntoView({block:'center'});await tick();const r=e.getBoundingClientRect();const hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);check(`hit:${s}`,!!hit&&(hit===e||e.contains(hit)));e.click();await tick();};
 const input=async(s:string,value:string)=>{const e=el<HTMLInputElement>(s);e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}));await tick();};
 const select=async(s:string,value:string)=>{const e=el<HTMLSelectElement>(s);e.value=value;e.dispatchEvent(new Event('change',{bubbles:true}));await tick();};
 const entries=()=>invoke<Entry[]>('list_entries');
 try{
 await tick();check('all-entries-navigation',el('h1').textContent==='全部凭据');
 await click('[aria-label="新增标签"]');await input('#organization-name','Smoke System');await click('#organization-dialog button.primary');await until(()=>!el<HTMLDialogElement>('#organization-dialog').open);
 check('empty-tag-persists',(await invoke<Organization>('get_organization')).tags.includes('Smoke System'));
 await click('[data-tag="Smoke System"]');await click('#add-btn');await select('#f-kind','database');
 await input('#f-title','Smoke Oracle UVT');await input('#f-db-type','Oracle');await input('#f-db-host','db-smoke.invalid');await input('#f-db-port','1521');await input('#f-db-name','ORCLPDB1');await input('#f-account','smoke_user');
 await input('#f-tags','UVT');el('#f-tags').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await tick();
 check('oracle-label',document.body.textContent!.includes('服务名 / SID'));
 await click('#save-entry');await until(()=>!document.querySelector('.draft-state')&&el('#detail-title').textContent==='Smoke Oracle UVT');
 let db=(await entries()).find(e=>e.title==='Smoke Oracle UVT')!;
 check('database-saved-empty-password',!!db&&db.database?.port==='1521'&&db.tags.includes('Smoke System')&&db.tags.includes('UVT'));
 await click('#duplicate-entry');await input('#f-title','Smoke Oracle Test');await click('[aria-label="移除标签 UVT"]');await input('#f-tags','测试');el('#f-tags').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await tick();await click('#save-entry');await until(()=>el('#detail-title').textContent==='Smoke Oracle Test'&&!document.querySelector('.draft-state'));
 check('environment-copy-independent',(await entries()).filter(e=>e.tags.includes('Smoke System')).length===2);
 await click('[data-tag="UVT"]');check('tag-filter',document.querySelectorAll('.item').length===1&&!!document.querySelector(`[data-entry="${db.id}"]`));
 await click('[data-nav="entries"]');await input('#search','db-smoke.invalid');check('host-search',document.querySelectorAll('.item').length===2);await input('#search','');
 await click('[aria-label="重命名标签 Smoke System"]');await input('#organization-name','Smoke Renamed');await click('#organization-dialog button.primary');await until(()=>!el<HTMLDialogElement>('#organization-dialog').open);
 check('rename-updates-entries',(await entries()).filter(e=>e.tags.includes('Smoke Renamed')).length===2);
 await click(`[data-entry="${db.id}"] .entry-select`);await click('#delete-btn');await click('#cancel-delete');check('cancel-delete-keeps-entry',(await entries()).some(e=>e.id===db.id));
 await click('#delete-btn');await click('#confirm-delete');await until(()=>!el<HTMLDialogElement>('#delete-dialog').open);
 check('delete-is-logical',!(await entries()).some(e=>e.id===db.id)&&(await invoke<TrashEntry[]>('list_trash')).some(e=>e.id===db.id));
 await click('[data-nav="trash"]');await click(`[data-restore="${db.id}"]`);await until(()=>!document.querySelector(`[data-trash="${db.id}"]`));
 db=(await entries()).find(e=>e.id===db.id)!;check('restore-retains-fields',db.tags.includes('Smoke Renamed')&&db.tags.includes('UVT')&&db.database?.databaseName==='ORCLPDB1');
 await click('[data-nav="entries"]');await click('#add-btn');await select('#f-kind','password');await input('#f-title','Smoke Website');await input('#f-account','first-user');await input('#f-url','https://website-smoke.invalid');await input('#f-secret','FAKE_SMOKE_ONLY_123');await click('#save-entry');await until(()=>el('#detail-title').textContent==='Smoke Website'&&!document.querySelector('.draft-state'));
 await click('#same-site');check('same-site-clears-identity',el<HTMLInputElement>('#f-account').value===''&&el<HTMLInputElement>('#f-secret').value===''&&el<HTMLInputElement>('#f-url').value==='https://website-smoke.invalid');await input('#f-title','Smoke Second Account');await input('#f-account','second-user');await input('#f-secret','FAKE_SECOND_ONLY_123');await click('#save-entry');await until(()=>el('#detail-title').textContent==='Smoke Second Account'&&!document.querySelector('.draft-state'));
 const second=(await entries()).find(e=>e.title==='Smoke Second Account')!;
 const clipboardCheck=await invoke<boolean|null>('smoke_clipboard',{id:second.id});check('core-windows-clipboard-roundtrip',clipboardCheck===true||clipboardCheck===null);
 await click('[data-nav="entries"]');await input('#f-title','Unsaved draft');await click(`[data-entry="${db.id}"] .entry-select`);await click(`[data-entry="${second.id}"] .entry-select`);check('draft-survives-navigation',el<HTMLInputElement>('#f-title').value==='Unsaved draft');await click('#cancel-edit');check('revert-loads-saved-name',el<HTMLInputElement>('#f-title').value==='Smoke Second Account');
 await input('#f-notes','local draft');await invoke('update_entry',{patch:{id:second.id,expectedRevision:second.revision,title:'Concurrent name'}});await click('[data-nav="settings"]');await click('[data-nav="entries"]');await click(`[data-entry="${second.id}"] .entry-select`);await click('#save-entry');await until(()=>!!document.querySelector('#form-error'));check('stale-save-rejected',el('#form-error').textContent!.length>0);await click('#cancel-edit');
 await click('#add-btn');await select('#f-kind','api');await input('#f-title','Smoke API');await input('#f-secret','FAKE_API_ONLY_123');await input('#f-url','https://api-smoke.invalid/v1');await click('#add-service');await select('#f-protocol-1','anthropic');await input('#f-url-1','https://second-smoke.invalid');
 check('test-service-ui-removed',!document.querySelector('.service-test'));
 for(const [width,height] of [[900,580],[1120,740]]){await invoke('smoke_resize',{width,height});await tick();check(`no-horizontal-overflow-${width}`,document.documentElement.scrollWidth<=innerWidth+1);const controls=[...document.querySelectorAll('.service-controls')];check(`service-controls-one-line-${width}`,controls.every(row=>{const boxes=[...row.children].map(e=>e.getBoundingClientRect());return boxes.every(r=>Math.abs(r.top-boxes[0].top)<3);}));}
 await click('#save-entry');await until(()=>el('#detail-title').textContent==='Smoke API'&&!document.querySelector('.draft-state'));
 check('api-multiple-services',(await entries()).find(e=>e.title==='Smoke API')?.services.length===2);
 await click('#delete-btn');await click('#confirm-delete');await until(()=>!el<HTMLDialogElement>('#delete-dialog').open);await click('[data-nav="trash"]');
 const beforePurge=await invoke<TrashEntry[]>('list_trash');
 await click('#empty-trash');await until(()=>!el<HTMLButtonElement>('#empty-trash').disabled);check('cancel-native-purge-preserves-trash',(await invoke<TrashEntry[]>('list_trash')).length===beforePurge.length);
 const activeBeforePurge=(await entries()).length;
 await click('#empty-trash');await until(()=>document.querySelectorAll('[data-trash]').length===0);check('confirmed-native-purge-keeps-active',(await entries()).length===activeBeforePurge&&(await invoke<TrashEntry[]>('list_trash')).length===0);
 await click('[data-nav="mcp"]');await click('#connection-tab');await click('#show-config');const cfg=JSON.parse(el('#mcp-config').textContent!);const info=await invoke<{executable:string;dataDirectory:string;webviewVersion:string}>('desktop_info');check('mcp-settings-current-path',cfg.mcpServers['vault'].command===info.executable);
 await click('#permissions-tab');
 for(const key of ['readMetadata','create','update','delete','fillInput','writeFile']) {
  const selector=`#permission-${key}`;check(`permission-default:${key}`,el(selector).getAttribute('aria-checked')==='true');
  await click(selector);await click('#save-permissions');await until(()=>!el<HTMLInputElement>(selector).disabled&&el(selector).getAttribute('aria-checked')==='false'&&el<HTMLButtonElement>('#save-permissions').disabled);
  const current=await invoke<{settings:{mcpPermissions:Record<string,boolean>}}>('desktop_info');check(`permission-revocation-persists:${key}`,current.settings.mcpPermissions[key]===false);
  await click(selector);await click('#save-permissions');await until(()=>!el<HTMLInputElement>(selector).disabled&&el(selector).getAttribute('aria-checked')==='true'&&el<HTMLButtonElement>('#save-permissions').disabled);
  check(`permission-restores:${key}`,(await invoke<{settings:{mcpPermissions:Record<string,boolean>}}>('desktop_info')).settings.mcpPermissions[key]===true);
 }
 for(const [width,height] of [[900,580],[1120,740]]){await invoke('smoke_resize',{width,height});await tick();check(`permissions-no-horizontal-overflow-${width}`,document.documentElement.scrollWidth<=innerWidth+1);}
 check('restricted-operations-explained',el('#mcp-permissions').textContent!.includes('始终禁止'));
 await click('#mcp-toggle');check('mcp-disable-persists',!(await invoke<{settings:{mcpEnabled:boolean}}>('desktop_info')).settings.mcpEnabled);await click('#mcp-toggle');await click('[data-nav="settings"]');await click('#tray-toggle');check('tray-window-cycle',await invoke<boolean>('smoke_window_cycle'));await click('#tray-toggle');
 await invoke('finish_smoke',{report:{passed:true,checks,webviewVersion:info.webviewVersion,userAgent:navigator.userAgent}});
 }catch(error){await invoke('finish_smoke',{report:{passed:false,checks,error:String(error)}});}
}
