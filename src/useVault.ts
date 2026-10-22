import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useUpdater } from './useUpdater';

export type Kind = 'password' | 'api' | 'database' | 'generic';
export interface DatabaseFields { databaseType:string; host:string; port:string; databaseName:string }
export interface Organization { groups:string[]; tags:string[] }
export interface TrashEntry extends Entry { deletedAt:number }
export type ApiProtocol = 'openai' | 'anthropic' | 'gemini' | 'custom';
export interface ApiService { id: string; protocol: ApiProtocol; url: string }
export const protocolName = (protocol: ApiProtocol) => ({openai:'OpenAI 兼容',anthropic:'Anthropic',gemini:'Gemini',custom:'自定义'})[protocol];
export interface Entry { id: string; kind: Kind; group:string; tags:string[]; database:DatabaseFields|null; title: string; account: string; url: string; services: ApiService[]; notes: string; pinned: boolean; createdAt: number; updatedAt: number; revision: number }
export interface McpPermissions { readMetadata:boolean; create:boolean; update:boolean; delete:boolean; fillInput:boolean; writeFile:boolean }
export type PermissionKey = 'access' | keyof McpPermissions;
export type PermissionRule = Partial<Record<PermissionKey, boolean>>;
export interface Settings { mcpEnabled: boolean; closeToTray: boolean; mcpPermissions:McpPermissions; mcpAccess:boolean; mcpTagRules:Record<string,PermissionRule>; mcpEntryRules:Record<string,PermissionRule> }
interface Info { version: string; executable: string; dataDirectory: string; webviewVersion: string; smokeTest: boolean; settings: Settings; startupEnabled: boolean }
export const kindName = (kind: Kind) => ({password:'账号密码',api:'API 凭据',database:'数据库',generic:'通用'})[kind];
export const secretName = (kind: Kind) => kind === 'api' ? 'API Key / Token' : '密码';
export const timestamp = (value: number) => new Date(value).toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });

export function useVault() {
  const page = ref<'entries'|'trash'|'settings'|'mcp'>('entries');
  const groupFilter=ref<string|null>(null), tagFilter=ref(''), typeFilter=ref<Kind|''>('');
  const organization=ref<Organization>({groups:[],tags:[]}), trash=ref<TrashEntry[]>([]);
  const query = ref(''), entries = ref<Entry[]>([]), selected = ref<string | null>(null), revealed = ref<string | null>(null);
  let testEpoch=0;
  function resetTests(){testEpoch++;}
  const info = ref<Info | null>(null), loading = ref(true), error = ref(''), toast = ref(''), saving = ref(false), configVisible = ref(false);
  const settingsSaving = ref(false);
  const permissionDraft=ref<Settings|null>(null);
  const deleteDialog = ref<HTMLDialogElement | null>(null);
  const editing = ref<Entry | null>(null), pendingDelete = ref<Entry | null>(null), formKind = ref<Kind>('password'), formError = ref('');
  const form = ref({ title: '', account: '', url: '', secret: '', notes: '', group:'', tags:[] as string[], database:{databaseType:'MySQL',host:'',port:'3306',databaseName:''} as DatabaseFields, services: [] as ApiService[] });
  const creating = ref(false), baseline = ref('');
  const dirty = computed(() => JSON.stringify(form.value) !== baseline.value);
  type Draft = { entry:Entry|null; kind:Kind; fields:typeof form.value; baseline:string };
  const drafts = new Map<string,Draft>();
  const updater = useUpdater(() => !!((editing.value||creating.value)&&dirty.value) || drafts.size>0 || !!permissionDraft.value || saving.value || settingsSaving.value || !!document.querySelector('dialog[open]:not(#update-dialog)'));
  let switching = false;
  const fieldsFor = (entry?:Entry) => ({group:entry?.group??'', tags:[...(entry?.tags??[])], database:entry?.database?{...entry.database}:{databaseType:'MySQL',host:'',port:'3306',databaseName:''}, title:entry?.title ?? '',account:entry?.account ?? '',url:entry?.url ?? '',secret:'',notes:entry?.notes ?? '',services:entry ? entry.services.map(service=>({...service})) : [{id:'',protocol:'openai' as ApiProtocol,url:''}]});
  function draftKey() { return creating.value ? `new:${formKind.value}` : editing.value?.id; }
  function stashDraft() {
    const key=draftKey(); if(!key) return;
    if(dirty.value) drafts.set(key,{entry:editing.value,kind:formKind.value,fields:JSON.parse(JSON.stringify(form.value)),baseline:baseline.value});
    else drafts.delete(key);
  }
  function loadEntry(entry?:Entry, restore=true) {
    hide();resetTests();formError.value='';creating.value=false;
    const draft=entry && restore ? drafts.get(entry.id) : undefined;
    editing.value=draft?.entry ?? (entry ? {...entry} : null);
    formKind.value=entry?.kind ?? 'password';
    form.value=draft?.fields ?? fieldsFor(entry);
    baseline.value=draft?.baseline ?? JSON.stringify(form.value);
    if(entry && draft?.entry?.revision!==undefined && draft.entry.revision!==entry.revision) reconcile(entry);
  }
  function reconcile(entry:Entry) {
    if(!editing.value || editing.value.id!==entry.id || editing.value.revision===entry.revision) return;
    if(!dirty.value) { loadEntry(entry,false); return; }
    if(JSON.stringify(fieldsFor(entry))===JSON.stringify(fieldsFor(editing.value))) {
      editing.value={...entry};
    } else formError.value='此条目已被其他操作修改。你的输入已保留，请撤销修改以载入最新内容；旧版本不会覆盖新版本。';
  }
  function selectEntry(id:string) {
    if(saving.value) return;stashDraft();switching=true;creating.value=false;selected.value=id;
    loadEntry(entries.value.find(entry=>entry.id===id));switching=false;
  }

  const list = computed(() => entries.value.filter(e =>
    (groupFilter.value===null||e.group===groupFilter.value)&&(!tagFilter.value||e.tags.includes(tagFilter.value))&&(!typeFilter.value||e.kind===typeFilter.value)&&
    [e.title,e.group,e.account,e.url,...e.tags,...Object.values(e.database??{}),...e.services.flatMap(s=>[s.url,protocolName(s.protocol)])].some(v=>v.toLowerCase().includes(query.value.trim().toLowerCase()))));
  const detail = computed(() => entries.value.find(e => e.id === selected.value));
  const title = computed(() => page.value === 'settings' ? '设置' : page.value==='mcp'?'MCP 管理':page.value === 'trash' ? '垃圾桶' : tagFilter.value?'# '+tagFilter.value:'全部凭据');
  const config = computed(() => info.value ? JSON.stringify({ mcpServers: { 'vault': { command: info.value.executable, args: ['--mcp', '--data-dir', info.value.dataDirectory] } } }, null, 2) : '');
  let fetching = false, timer: ReturnType<typeof setInterval> | undefined, toastTimer: ReturnType<typeof setTimeout> | undefined;
  const message = (value: unknown) => {
    if (!(value && typeof value === 'object')) return '操作失败，请检查本地文件权限后重试。';
    const failure = value as { message?: unknown; details?: unknown };
    const base = typeof failure.message === 'string' && failure.message ? failure.message : '操作失败，请检查本地文件权限后重试。';
    if (!Array.isArray(failure.details)) return base;
    const reasons = failure.details.map((issue) => {
      if (issue && typeof issue === 'object' && 'field' in issue && 'reason' in issue) {
        const named = issue as { field: unknown; reason: unknown };
        return `${String(named.field)}：${String(named.reason)}`;
      }
      return '';
    }).filter(Boolean).join('；');
    return reasons ? `${base}（${reasons}）` : base;
  };
  function notify(value: string) { toast.value = value; clearTimeout(toastTimer); toastTimer = setTimeout(() => { toast.value = ''; }, 3000); }
  function hide() { revealed.value = null; }
  function navigate(value: typeof page.value) { if(saving.value) return; if(page.value==='mcp'&&permissionDraft.value&&value!=='mcp')notify('未应用的权限修改已保留，返回 MCP 管理可继续编辑。');stashDraft();const same=value===page.value;creating.value=false;page.value=value;query.value='';hide();if(same) loadEntry(detail.value); }
  async function refresh() {
    if (fetching) return; fetching = true;
    try { const [items,org,deleted]=await Promise.all([invoke<Entry[]>('list_entries'),invoke<Organization>('get_organization'),invoke<TrashEntry[]>('list_trash')]); entries.value=items;organization.value=org;trash.value=deleted; error.value = ''; }
    catch (value) { error.value = message(value); }
    finally { fetching = false; loading.value = false; }
  }
  watch(list, values => { if (!values.some(e => e.id === selected.value)) selected.value = values[0]?.id ?? null; });
  watch(selected, () => { if(!switching && !creating.value) { stashDraft();loadEntry(detail.value); } }, {flush:'sync'});
  watch(() => detail.value?.revision, () => { hide();resetTests();if(!switching && !creating.value && detail.value) reconcile(detail.value); });
  watch(form, () => { hide();resetTests(); }, {deep:true,flush:'sync'});

  async function pin(entry: Entry) { try { await invoke('pin_entry', { id: entry.id, expectedRevision: entry.revision, pinned: !entry.pinned }); await refresh(); } catch (value) { notify(message(value)); await refresh(); } }
  async function reveal() {
    if (revealed.value !== null) { hide(); return; } if(form.value.secret) { revealed.value=form.value.secret;return; } const e = editing.value; if (!e) return;const epoch=testEpoch;
    try { const secret = await invoke<string>('read_secret', { id: e.id, expectedRevision: e.revision }); if (epoch===testEpoch && editing.value?.id === e.id && editing.value.revision === e.revision) revealed.value = secret; }
    catch (value) { notify(message(value)); await refresh(); }
  }
  function updateSecret(value:string) {
    const visible=revealed.value!==null;
    form.value.secret=value;
    if(visible) revealed.value=value;
  }
  async function copy(field:'account'|'secret'|'url'|'host'|'port'|'database_name',serviceId?:string,index?:number) {
    const e=editing.value;
    try {
      const text=field==='host'?form.value.database.host:field==='port'?form.value.database.port:field==='database_name'?form.value.database.databaseName:field==='account' ? form.value.account : field==='url' ? (formKind.value==='api' ? form.value.services[index ?? form.value.services.findIndex(s=>s.id===serviceId)]?.url ?? '' : form.value.url) : form.value.secret;
      if(field!=='secret' || text) await invoke('copy_text',{text});
      else if(e) await invoke('copy_field',{id:e.id,expectedRevision:e.revision,field});
      else return;
      notify('已复制');
    } catch(value) {notify(message(value));}
  }
  function openForm() {
    if(saving.value) return;stashDraft();hide();resetTests();formError.value='';creating.value=true;editing.value=null;
    formKind.value=typeFilter.value||'password';
    const draft=drafts.get(`new:${formKind.value}`);
    form.value=draft?.fields ?? {...fieldsFor(),group:groupFilter.value??'',tags:tagFilter.value?[tagFilter.value]:[]};baseline.value=draft?.baseline ?? JSON.stringify(form.value);
  }
  function addService() { if (form.value.services.length < 20) form.value.services.push({id:'',protocol:'openai',url:''}); }
  function closeForm() {
    if(saving.value) return;
    const key=draftKey();if(key) drafts.delete(key);
    loadEntry(detail.value,false);
  }
  async function save() {
    if (saving.value || (editing.value && !dirty.value)) return; saving.value = true; formError.value = '';
    try {
      let updated: Entry;
      const services = formKind.value === 'api' ? form.value.services.filter(service => service.url.trim()).map(service => ({...service,url:service.url.trim()})) : [];
      const payload = {...form.value, database:formKind.value==='database'?form.value.database:null, services, url:formKind.value === 'api' ? services[0]?.url ?? '' : form.value.url};
      if (editing.value) {
        // 非 API 条目不携带 services：后端要求 services 与 url 一致，传空数组会与之冲突。
        const { secret, services: entryServices, ...fields } = payload;
        const patch: Record<string, unknown> = { ...fields, id: editing.value.id, expectedRevision: editing.value.revision, ...(secret ? { secret } : {}) };
        if (formKind.value === 'api') patch.services = entryServices;
        updated = await invoke('update_entry', { patch });
      }
      else updated = await invoke('create_entry', { input: { ...payload, kind: formKind.value } });
      const key=draftKey();if(key) drafts.delete(key);
      switching=true;query.value='';groupFilter.value=null;tagFilter.value='';typeFilter.value='';creating.value=false;await refresh();selected.value=updated.id;loadEntry(updated,false);switching=false;notify('已加密保存');
    } catch (value) {
      formError.value = message(value);
      // 保存失败必须醒目：除了表单底部的红字，同时弹出顶部 toast 并滚动到错误区域。
      notify(formError.value);
      void nextTick().then(() => document.getElementById('form-error')?.scrollIntoView({ block: 'nearest' }));
    } finally { switching=false;saving.value = false; }
  }
  function askDelete() { if (!detail.value) return; hide(); pendingDelete.value = { ...detail.value }; deleteDialog.value?.showModal(); }
  async function confirmDelete() {
    if (!pendingDelete.value || saving.value) return; saving.value = true;
    try { drafts.delete(pendingDelete.value.id); await invoke('delete_entry', { id: pendingDelete.value.id, expectedRevision: pendingDelete.value.revision }); deleteDialog.value?.close(); pendingDelete.value = null; await refresh(); notify('已移入垃圾桶，可恢复'); }
    catch (value) { deleteDialog.value?.close(); notify(message(value)); await refresh(); } finally { saving.value = false; }
  }
  async function persistSettings(settings:Settings) {
    if(!info.value||settingsSaving.value)return false;settingsSaving.value=true;
    try{info.value.settings=await invoke<Settings>('save_settings',{settings});notify('设置已保存');return true;}
    catch(value){notify(message(value));return false;}finally{settingsSaving.value=false;}
  }
  async function updateSettings(key:'mcpEnabled'|'closeToTray') { if(info.value)await persistSettings({...info.value.settings,[key]:!info.value.settings[key]}); }
  async function updatePermission(key:keyof McpPermissions) { if(info.value)await persistSettings({...info.value.settings,mcpPermissions:{...info.value.settings.mcpPermissions,[key]:!info.value.settings.mcpPermissions[key]}}); }
  async function savePermissions(settings:Settings){return info.value?persistSettings({...settings,mcpEnabled:info.value.settings.mcpEnabled,closeToTray:info.value.settings.closeToTray}):false;}
  async function toggleStartup() { if (!info.value) return; try { info.value.startupEnabled = await invoke<boolean>('set_startup', { enabled: !info.value.startupEnabled }); notify('启动设置已更新'); } catch (value) { notify(message(value)); } }
  async function copyConfig() { try { await invoke('copy_text', { text: config.value }); notify('已复制 MCP 配置'); } catch (value) { notify(message(value)); } }
  function focus() { void refresh(); }
  function visibility() { if (document.visibilityState !== 'visible') hide(); }
  onMounted(async () => {
    try {
      info.value = await invoke<Info>('desktop_info'); await refresh();
      if (info.value.smokeTest) { const { runSmoke } = await import('./smoke'); await runSmoke(); }
      else { updater.start(); timer = setInterval(() => { if (document.visibilityState === 'visible') void refresh(); }, 2000); window.addEventListener('focus', focus); window.addEventListener('blur', hide); document.addEventListener('visibilitychange', visibility); }
    } catch (value) { error.value = message(value); loading.value = false; }
  });
  onUnmounted(() => { clearInterval(timer); clearTimeout(toastTimer); window.removeEventListener('focus', focus); window.removeEventListener('blur', hide); document.removeEventListener('visibilitychange', visibility); });
  async function restore(entry:TrashEntry){if(saving.value)return;saving.value=true;try{await invoke('restore_entry',{id:entry.id,expectedRevision:entry.revision});await refresh();notify('已恢复，标签已保留');}catch(v){notify(message(v));}finally{saving.value=false;}}
  async function emptyTrash(){if(saving.value)return;saving.value=true;try{const count=await invoke<number>('empty_trash');await refresh();if(count)notify(`已永久删除 ${count} 条凭据`);}catch(v){notify(message(v));}finally{saving.value=false;}}
  async function organize(kind:'group'|'tag',name:string,previous?:string){
    await invoke('organize',{kind,name,previous});
    if(previous){
      const rename=(tags:string[])=>[...new Set(tags.filter(t=>t!==previous).concat(tags.includes(previous)&&name?[name]:[]))];
      for(const draft of drafts.values()){
        draft.fields.tags=rename(draft.fields.tags);if(draft.entry)draft.entry.tags=rename(draft.entry.tags);
        const saved=JSON.parse(draft.baseline);saved.tags=rename(saved.tags);draft.baseline=JSON.stringify(saved);
      }
      const oldBaseline=JSON.parse(baseline.value||'{}');form.value.tags=rename(form.value.tags);if(oldBaseline.tags){oldBaseline.tags=rename(oldBaseline.tags);baseline.value=JSON.stringify(oldBaseline);}if(editing.value)editing.value.tags=rename(editing.value.tags);
      if(permissionDraft.value?.mcpTagRules[previous]){const rule=permissionDraft.value.mcpTagRules[previous];delete permissionDraft.value.mcpTagRules[previous];if(name)permissionDraft.value.mcpTagRules[name]=rule;}
    }
    if(tagFilter.value===previous)tagFilter.value=name;
    await refresh();if(info.value)info.value.settings=(await invoke<Info>('desktop_info')).settings;
  }
  function filterGroup(group:string|null){stashDraft();page.value='entries';groupFilter.value=group;}
  function filterTag(tag:string){stashDraft();page.value='entries';tagFilter.value=tagFilter.value===tag?'':tag;}
  function allEntries(){groupFilter.value=null;tagFilter.value='';typeFilter.value='';navigate('entries');}
  async function duplicate(sameSite=false){
    if(saving.value||!editing.value)return;
    const source=editing.value; const fields=JSON.parse(JSON.stringify(form.value));const kind=formKind.value;
    try{let secret='';if(!sameSite)secret=fields.secret||await invoke<string>('read_secret',{id:source.id,expectedRevision:source.revision});
      openForm();formKind.value=kind;form.value=sameSite?{...fieldsFor(),group:fields.group,url:fields.url,services:[]}:{...fields,title:fields.title+' · 副本',secret};
      if(sameSite)form.value.tags=[...fields.tags];baseline.value='';notify(sameSite?'已保留网站与标签，账号密码为空':'已复制为草稿，请修改环境和凭据');
    }catch(v){notify(message(v));}
  }
  function keyboard(e:KeyboardEvent){if(e.ctrlKey&&e.key==='k'){e.preventDefault();document.querySelector<HTMLInputElement>('#search')?.focus();}}
  onMounted(()=>window.addEventListener('keydown',keyboard));onUnmounted(()=>window.removeEventListener('keydown',keyboard));
  return { updater, permissionDraft, addService, page, query, entries, selected, revealed, info, loading, error, toast, saving, configVisible, creating, dirty, selectEntry, deleteDialog, editing, pendingDelete, formKind, formError, form, list, detail, title, config, navigate, refresh, pin, reveal, updateSecret, copy, openForm, closeForm, save, askDelete, confirmDelete, updateSettings, updatePermission, savePermissions, settingsSaving, toggleStartup, copyConfig, groupFilter,tagFilter,typeFilter,organization,trash,restore,emptyTrash,organize,filterGroup,filterTag,allEntries,duplicate,notify };
}
export type VaultUI=ReturnType<typeof useVault>;
