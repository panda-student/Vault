<script setup lang="ts">
import { computed, ref } from 'vue';
import type { UnwrapRef } from 'vue';
import type { VaultUI } from '../useVault';
const props = defineProps<{v: UnwrapRef<VaultUI>}>();
const dialog = ref<HTMLDialogElement|null>(null);
const status = computed(() => props.v.updater.status);
const labels: Record<string,string> = {idle:'尚未检查',checking:'正在检查更新…',available:'发现新版本',downloading:'正在下载更新…',ready:'已下载，等待重启',installing:'正在等待操作完成并更新…','up-to-date':'当前已是最新版本',error:'更新未完成'};
const progress = computed(() => status.value.total ? Math.min(100,Math.round(status.value.received/status.value.total*100)) : 0);
async function install() { const done = await props.v.updater.action('install'); if (!done) dialog.value?.close(); }
</script>
<template>
  <div class="card update-card" id="update-panel">
    <h3>软件更新</h3>
    <div class="setting-row update-heading"><div><span id="update-status" role="status">{{labels[status.phase]}}</span><small>当前 v{{v.info?.version}}<template v-if="status.version"> · 新版 v{{status.version}}</template></small></div><button id="check-update" class="secondary" :disabled="v.updater.busy" @click="v.updater.action('check')">检查更新</button></div>
    <div v-if="status.version" class="update-details">
      <p class="update-notes">{{status.notes}}</p>
      <div v-if="status.phase==='downloading'" class="update-progress"><progress :value="status.received" :max="status.total||1" aria-label="更新下载进度"></progress><span>{{progress}}%</span></div>
      <div class="update-actions"><button v-if="['available','error'].includes(status.phase)" id="download-update" class="primary" @click="v.updater.action('download')">下载更新</button><button v-if="status.phase==='downloading'" id="cancel-update" class="secondary" @click="v.updater.action('cancel')">取消下载</button><button v-if="status.phase==='ready'" id="install-update" class="primary" :disabled="v.saving||v.settingsSaving" @click="dialog?.showModal()">重启并更新</button></div>
    </div>
    <p v-if="v.updater.error||status.error" id="update-error" class="form-error" role="alert">{{v.updater.error||status.error}}</p>
    <p class="description update-hint">启动时自动检查。下载后验证签名，由你决定何时重启；更新保留现有密码库和 MCP 路径。</p>
    <dialog ref="dialog" id="update-dialog"><h2>重启并更新到 v{{status.version}}？</h2><p class="dialog-description">程序将退出、替换并重新打开。请先保存修改。已连接的 MCP 会等待当前操作完成，更新期间暂停内部服务，完成后自动恢复，无需退出 AI 客户端。</p><div class="modal-actions"><button id="cancel-install-update" class="secondary" autofocus :disabled="status.phase==='installing'" @click="dialog?.close()">稍后</button><button id="confirm-install-update" class="primary" :disabled="status.phase==='installing'" @click="install">重启并更新</button></div></dialog>
  </div>
</template>
<style scoped>
.update-heading{gap:16px}.update-heading button{flex-shrink:0}.update-notes{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;line-height:1.7;color:#536079;max-height:180px;overflow-y:auto}.update-actions{display:flex;gap:12px;flex-wrap:wrap}.update-progress{display:flex;align-items:center;gap:12px;margin:12px 0;font-size:12px}.update-progress progress{flex:1;min-width:0;accent-color:#5364dc}.update-hint{margin:16px 0 0;font-size:11px}.update-details{padding-bottom:4px}
</style>
