<script setup lang="ts">
import { reactive, ref } from 'vue';
import { useVault } from './useVault';
import VaultSidebar from './components/VaultSidebar.vue';
import EntryList from './components/EntryList.vue';
import EntryEditor from './components/EntryEditor.vue';
import TrashPanel from './components/TrashPanel.vue';
import SettingsPanel from './components/SettingsPanel.vue';
const state=useVault();const v=reactive(state);const {deleteDialog}=state;
const organizationDialog=ref<HTMLDialogElement|null>(null), organizationKind=ref<'group'|'tag'>('group');
const organizationName=ref(''),previousName=ref<string|undefined>(),organizationError=ref(''),organizing=ref(false);
function openOrganization(kind:'group'|'tag',previous?:string){organizationKind.value=kind;previousName.value=previous;organizationName.value=previous??'';organizationError.value='';organizationDialog.value?.showModal();}
async function saveOrganization(){if(organizing.value)return;organizing.value=true;try{await v.organize(organizationKind.value,organizationName.value,previousName.value);organizationDialog.value?.close();}catch(e){organizationError.value=(e as {message?:string}).message||'保存失败';}finally{organizing.value=false;}}
</script>
<template>
<div class="app-shell"><VaultSidebar :v="v" @organize="openOrganization"/><main>
<header><div><h1>{{v.title}}</h1><p>本地加密保存，AI 按引用填写。</p></div><button v-if="v.page==='entries'" id="add-btn" class="primary" :disabled="v.loading||!!v.error||v.saving" @click="v.openForm()">＋ 新增凭据</button></header>
<div v-if="v.error" class="error" role="alert">{{v.error}}<button class="text-btn" @click="v.refresh()">重试</button></div>
<div v-if="v.page==='entries'" class="split"><EntryList :v="v" @organize="openOrganization"/><EntryEditor :v="v"/></div><TrashPanel v-else-if="v.page==='trash'" :v="v"/><SettingsPanel v-else :v="v"/>
<footer><span><span class="dot"></span>{{v.info?'本地密码库已解锁':'正在连接本地核心'}}</span><span>v{{v.info?.version}}</span></footer></main></div>
<dialog ref="deleteDialog" id="delete-dialog"><h2>移入垃圾桶？</h2><p class="dialog-description">“{{v.pendingDelete?.title}}”将从正常列表移除。你可以在垃圾桶恢复。未保存的修改不会保留。</p><div class="modal-actions"><button id="cancel-delete" autofocus class="secondary" :disabled="v.saving" @click="deleteDialog?.close()">取消</button><button id="confirm-delete" class="primary danger-fill" :disabled="v.saving" @click="v.confirmDelete()">移入垃圾桶</button></div></dialog>
<dialog ref="organizationDialog" id="organization-dialog"><form @submit.prevent="saveOrganization"><h2>{{previousName?'重命名':'新增'}}{{organizationKind==='group'?'分组':'标签'}}</h2><div class="form-field" style="margin-top:20px"><label for="organization-name">名称</label><input id="organization-name" v-model="organizationName" required maxlength="64" autofocus></div><p v-if="organizationError" class="form-error">{{organizationError}}</p><div class="modal-actions"><button type="button" class="secondary" :disabled="organizing" @click="organizationDialog?.close()">取消</button><button class="primary" :disabled="organizing">保存</button></div></form></dialog>
<div v-if="v.toast" class="toast" role="status">{{v.toast}}</div></template>