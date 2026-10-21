<script setup lang="ts">
import {computed,ref,watch,onUnmounted} from 'vue';
import type {UnwrapRef} from 'vue';
import {invoke} from '@tauri-apps/api/core';
import type {VaultUI,Settings,PermissionKey} from '../useVault';
const props=defineProps<{v:UnwrapRef<VaultUI>}>();
const clone=<T,>(x:T):T=>JSON.parse(JSON.stringify(x));
const draft=ref<Settings|null>(props.v.permissionDraft?clone(props.v.permissionDraft):props.v.info?clone(props.v.info.settings):null);
const open=ref(false),expanded=ref(new Set<string>()),query=ref(''),previewId=ref(''),previewDialog=ref<HTMLDialogElement|null>(null),previewError=ref('');
const columns:{key:PermissionKey;label:string}[]=[{key:'access',label:'允许访问'},{key:'readMetadata',label:'读取信息'},{key:'create',label:'新增'},{key:'update',label:'修改'},{key:'delete',label:'移入垃圾桶'},{key:'fillInput',label:'窗口填写'},{key:'writeFile',label:'写入文件'}];
const policy=(s:Settings|null|undefined)=>s?JSON.stringify([s.mcpPermissions,s.mcpAccess,s.mcpTagRules,s.mcpEntryRules]):'';
const dirty=computed(()=>policy(draft.value)!==policy(props.v.info?.settings));
watch(draft,s=>{props.v.permissionDraft=s&&dirty.value?clone(s):null;},{deep:true});
watch(()=>props.v.info?.settings,s=>{if(!s)return;if(!props.v.permissionDraft)draft.value=clone(s);else if(draft.value){draft.value.mcpEnabled=s.mcpEnabled;draft.value.closeToTray=s.closeToTray;}},{deep:true});
type Effective={configured:boolean;allowed:boolean;source:string};
type Projection={id:string;permissions:Record<PermissionKey,Effective>};
const projections=ref<Record<string,Projection['permissions']>>({});
let epoch=0,timer:ReturnType<typeof setTimeout>|undefined;
watch([draft,()=>props.v.entries],()=>{clearTimeout(timer);const current=++epoch;timer=setTimeout(async()=>{if(!draft.value)return;try{const rows=await invoke<Projection[]>('permission_preview',{settings:clone(draft.value)});if(current!==epoch)return;projections.value=Object.fromEntries(rows.map(e=>[e.id,e.permissions]));previewError.value='';}catch{if(current===epoch)previewError.value='无法计算生效权限，请重试。';}},80);},{deep:true,immediate:true});
onUnmounted(()=>{clearTimeout(timer);epoch++;});
type Row={kind:'global'|'tag'|'entry'|'untagged';id:string;parent:string;title:string;note:string};
const rows=computed(()=>{
 const result:Row[]=[{kind:'global',id:'global',parent:'',title:'默认规则',note:'适用于未单独设置的操作'}];
 if(!open.value)return result;const q=query.value.trim().toLowerCase();
 for(const tag of props.v.organization.tags){const related=props.v.entries.filter(e=>e.tags.includes(tag));const match=related.filter(e=>!q||tag.toLowerCase().includes(q)||[e.title,e.account,...e.tags].join(' ').toLowerCase().includes(q));if(q&&!match.length&&!tag.toLowerCase().includes(q))continue;
  result.push({kind:'tag',id:tag,parent:'',title:tag,note:related.length+' 条凭据'});
  if(expanded.value.has(tag)||q)for(const e of match)result.push({kind:'entry',id:e.id,parent:tag,title:e.title,note:e.tags.join(' · ')});
 }
 const untagged=props.v.entries.filter(e=>!e.tags.length&&(!q||[e.title,e.account].join(' ').toLowerCase().includes(q)));
 if(untagged.length){result.push({kind:'untagged',id:'untagged',parent:'',title:'无标签凭据',note:''});for(const e of untagged)result.push({kind:'entry',id:e.id,parent:'untagged',title:e.title,note:'继承默认规则'});}
 return result;
});
function own(row:Row,key:PermissionKey){if(!draft.value)return false;return row.kind==='global'||Object.hasOwn(row.kind==='tag'?draft.value.mcpTagRules[row.id]||{}:draft.value.mcpEntryRules[row.id]||{},key);}
function hasRule(row:Row){return draft.value&&row.kind!=='global'&&Object.keys(row.kind==='tag'?draft.value.mcpTagRules[row.id]||{}:draft.value.mcpEntryRules[row.id]||{}).length>0;}
function defaultValue(key:PermissionKey){return draft.value?(key==='access'?draft.value.mcpAccess:draft.value.mcpPermissions[key]):false;}
function value(row:Row,key:PermissionKey):boolean{
 if(!draft.value)return false;if(row.kind==='global')return defaultValue(key);
 if(row.kind==='tag')return draft.value.mcpTagRules[row.id]?.[key]??defaultValue(key);
 return projections.value[row.id]?.[key]?.configured??false;
}
function source(row:Row,key:PermissionKey){return row.kind==='entry'?projections.value[row.id]?.[key]?.source||'正在计算':own(row,key)?row.kind==='global'?'默认规则':'标签设置':'继承默认规则';}
function set(row:Row,key:PermissionKey,event:Event){if(!draft.value)return;const checked=(event.target as HTMLInputElement).checked;
 if(row.kind==='global'){if(key==='access')draft.value.mcpAccess=checked;else draft.value.mcpPermissions[key]=checked;}
 else{const map=row.kind==='tag'?draft.value.mcpTagRules:draft.value.mcpEntryRules;(map[row.id]||={})[key]=checked;}
}
function reset(row:Row){if(!draft.value)return;delete (row.kind==='tag'?draft.value.mcpTagRules:draft.value.mcpEntryRules)[row.id];}
function toggleTag(id:string){expanded.value.has(id)?expanded.value.delete(id):expanded.value.add(id);}
function toggle(){open.value=!open.value;if(!open.value)query.value='';}
async function save(){if(!draft.value)return;const success=await props.v.savePermissions(clone(draft.value));if(success&&props.v.info){props.v.permissionDraft=null;draft.value=clone(props.v.info.settings);}}
function revert(){if(props.v.info){props.v.permissionDraft=null;draft.value=clone(props.v.info.settings);}}
const counts=computed(()=>({tags:Object.values(draft.value?.mcpTagRules||{}).filter(r=>Object.keys(r).length).length,entries:Object.values(draft.value?.mcpEntryRules||{}).filter(r=>Object.keys(r).length).length}));
const allowedCount=computed(()=>Object.values(projections.value).filter(p=>p.access.allowed).length);
const selectedPreview=computed(()=>projections.value[previewId.value]);
function preview(id:string){previewId.value=id;previewDialog.value?.showModal();}
</script>
<template>
<section class="permission-editor" id="mcp-permissions" v-if="draft">
 <div class="permission-heading"><div><h3>凭据访问与操作权限</h3><p class="description">未单独设置的操作使用默认规则。权限按条目生效。</p></div><span class="permission-count">{{allowedCount}} / {{v.entries.length}} 条凭据可供 AI 使用</span></div>
 <div class="permission-toolbar"><div><button class="permission-disclosure" id="customize" :aria-expanded="open" aria-controls="permission-matrix" @click="toggle">{{open?'⌄':'›'}} 按标签单独设置</button><p class="permission-caption">已有 {{counts.tags}} 个标签、{{counts.entries}} 条凭据单独配置，收起后仍然生效。</p></div><input v-if="open" class="permission-search" v-model="query" aria-label="搜索标签或凭据" placeholder="搜索标签或凭据"></div>
 <p v-if="!draft.mcpEnabled" class="permission-notice">MCP 已暂停。可以编辑并保存规则，重新开启后生效。</p>
 <p v-if="previewError" class="form-error" role="alert">{{previewError}}</p>
 <div class="permission-table-wrap"><table id="permission-matrix" aria-label="MCP 凭据权限矩阵"><colgroup><col style="width:28%"><col span="7"><col style="width:88px"></colgroup><thead><tr><th>默认规则<span v-if="open"> / 标签 / 凭据</span></th><th v-for="c in columns" :key="c.key">{{c.label}}</th><th>规则来源</th></tr></thead><tbody>
 <template v-for="row in rows" :key="row.kind+row.parent+row.id"><tr v-if="row.kind==='untagged'" class="untagged"><td colspan="9">无标签凭据 · 可设置单条例外</td></tr><tr v-else :data-row="row.id" :data-parent="row.parent" :class="['permission-row',row.kind]">
  <td class="permission-resource"><div class="permission-name"><button v-if="row.kind==='tag'" class="text-btn" :data-expand="row.id" :aria-expanded="expanded.has(row.id)||!!query" :aria-label="`${expanded.has(row.id)||query?'收起':'展开'}标签 ${row.title}`" @click="toggleTag(row.id)">{{expanded.has(row.id)||query?'⌄':'›'}}</button><span v-if="row.kind==='tag'" class="permission-hash">#</span><div><button v-if="row.kind==='entry'" class="permission-entry-name" :data-preview="row.id" @click="preview(row.id)">{{row.title}}</button><strong v-else>{{row.title}}</strong><small>{{row.note}}</small></div></div></td>
  <td v-for="c in columns" :key="c.key"><span v-if="row.kind==='entry'&&c.key==='create'" class="muted">—</span><label v-else class="permission-check" :title="source(row,c.key)"><input type="checkbox" :id="row.kind==='global'&&c.key!=='access'?`permission-${c.key}`:undefined" :data-kind="row.kind" :data-id="row.id" :data-key="c.key" :aria-label="`${row.title} · ${c.label}`" :aria-checked="value(row,c.key)" :checked="value(row,c.key)" :class="{inherited:!own(row,c.key)}" :disabled="v.settingsSaving||!!previewError||(row.kind==='entry'&&!projections[row.id])||(c.key!=='access'&&!value(row,'access'))" @change="set(row,c.key,$event)"></label></td>
  <td class="permission-source">{{row.kind==='global'?'全局默认':hasRule(row)?row.kind==='tag'?'标签设置':'单条设置':row.kind==='tag'?'继承默认':'标签 / 默认'}}<button v-if="hasRule(row)" class="text-btn" :data-reset="row.id" :aria-label="`${row.title} 恢复继承`" @click="reset(row)">↶</button></td>
 </tr></template>
 <tr v-if="open&&rows.length===1"><td colspan="9" class="empty">没有匹配的标签或凭据</td></tr>
 </tbody></table></div>
 <p class="permission-caption rule-order">生效顺序：单条设置 → 标签规则 → 默认规则。多标签冲突时禁止优先。修改标签规则不会清除单条例外。</p>
 <p class="permission-boundary"><strong>始终禁止 MCP：</strong>读取密码原文、修改权限、永久删除。“读取信息”仅指受控元数据。</p>
 <div class="permission-actions"><span>{{dirty?'有修改待应用':'权限已保存'}}</span><button id="revert-permissions" class="secondary" :disabled="!dirty||v.settingsSaving" @click="revert">撤销更改</button><button id="save-permissions" class="primary" :disabled="!dirty||v.settingsSaving" @click="save">保存并应用</button></div>
 <dialog ref="previewDialog" id="permission-preview"><h2>{{v.entries.find(e=>e.id===previewId)?.title}}</h2><p class="dialog-description">根据当前编辑内容计算；保存后生效。</p><table class="permission-preview-table"><thead><tr><th>操作</th><th>生效权限</th><th>来源</th></tr></thead><tbody><tr v-for="c in columns.filter(c=>c.key!=='create')" :key="c.key"><td>{{c.label}}</td><td>{{selectedPreview?.[c.key]?.allowed?'允许':'禁止'}}</td><td>{{selectedPreview?.[c.key]?.source}}</td></tr><tr><td>读取密码原文</td><td>始终禁止</td><td>固定安全规则</td></tr><tr><td>永久删除</td><td>始终禁止</td><td>固定安全规则</td></tr></tbody></table><div class="modal-actions"><button class="secondary" @click="previewDialog?.close()">关闭</button></div></dialog>
</section>
</template>
<style>
.permission-editor{min-width:0}.permission-heading{display:flex;justify-content:space-between;align-items:flex-start;gap:16px}.permission-count{white-space:nowrap;color:#6574b8;font-size:10px;padding-top:5px}.permission-toolbar{display:flex;justify-content:space-between;align-items:center;gap:16px;margin:22px 0 15px}.permission-disclosure{background:none;border:0;color:#5364dc;font-size:12px;padding:6px 0}.permission-caption{font-size:10px;line-height:1.9;color:#8994a9;margin-top:6px}.permission-search{font:inherit;font-size:11px;width:205px;min-width:100px;border:1px solid #e2e6f0;border-radius:5px;padding:9px}.permission-table-wrap{border:1px solid #e6eaf3;border-radius:6px;overflow:hidden}.permission-table-wrap table{table-layout:fixed;border-collapse:collapse;width:100%;font-size:11px}.permission-table-wrap th{font-weight:500;color:#8390a6;background:#f7f8fc;font-size:10px;padding:12px 3px;line-height:1.6}.permission-table-wrap th:first-child{text-align:left;padding-left:12px}.permission-table-wrap td{padding:12px 3px;text-align:center;border-top:1px solid #eef0f6}.permission-row.global{background:#f1f3ff}.permission-row.tag{background:#fafbfe}.permission-table-wrap .permission-resource{text-align:left;padding-left:12px}.permission-row.entry .permission-resource{padding-left:37px}.permission-name{display:flex;align-items:center;gap:5px;min-width:0}.permission-name>div{min-width:0}.permission-name strong{font-weight:550}.permission-name small{display:block;color:#9aa5b9;font-size:9px;margin-top:5px;overflow-wrap:anywhere}.permission-name .text-btn{padding:2px 4px}.permission-hash{color:#96a2c0}.permission-entry-name{text-align:left;overflow-wrap:anywhere;font:inherit;color:#4c5f89;background:none;border:0;padding:0}.permission-check{display:inline-flex;padding:7px;cursor:pointer}.permission-check input{accent-color:#5364dc;width:14px;height:14px;margin:0;cursor:pointer}.permission-check input.inherited{accent-color:#939fb9}.permission-check input:disabled{opacity:.4;cursor:default}.permission-source{color:#919bb0;font-size:9px;line-height:1.7;overflow-wrap:anywhere}.permission-source .text-btn{padding:3px}.rule-order{margin:13px 0 20px}.permission-boundary{font-size:10px;color:#8894aa;line-height:1.9;border-top:1px solid #edf0f6;padding-top:14px}.permission-boundary strong{font-weight:500}.permission-actions{display:flex;gap:10px;align-items:center;margin-top:22px;padding-top:16px;border-top:1px solid #edf0f6}.permission-actions span{margin-right:auto;color:#929bb0;font-size:11px}.permission-notice{color:#9c7a3e;background:#fffaf1;border-radius:5px;padding:10px;font-size:11px;margin-bottom:14px}.permission-table-wrap .untagged td{text-align:left;padding:13px 15px;color:#8e9bb3;background:#fafbfe;font-size:10px}.permission-preview-table{width:100%;font-size:11px;border-collapse:collapse}.permission-preview-table th,.permission-preview-table td{padding:11px 7px;text-align:left;border-bottom:1px solid #edf0f6}.permission-preview-table th{color:#8994a9;font-weight:500}@media(max-width:1050px){.permission-table-wrap col:last-child{width:64px!important}.permission-table-wrap col:first-child{width:25%!important}.permission-row.entry .permission-resource{padding-left:20px}.permission-check{padding:6px 3px}.permission-count{white-space:normal;text-align:right}.permission-name small{font-size:9px}}
</style>
