import { computed, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
export interface UpdateStatus {
  phase: 'idle'|'checking'|'available'|'downloading'|'ready'|'installing'|'up-to-date'|'error';
  version: string|null; notes: string; received: number; total: number; error: string|null;
}
export function useUpdater(hasUnsaved: () => boolean) {
  const status = ref<UpdateStatus>({phase:'idle',version:null,notes:'',received:0,total:0,error:null});
  const error = ref('');
  const busy = computed(() => ['checking','downloading','installing'].includes(status.value.phase));
  let polling: ReturnType<typeof setInterval>|undefined, checking: ReturnType<typeof setInterval>|undefined;
  let refreshing = false;
  async function refresh() {
    if (refreshing) return; refreshing = true;
    try { status.value = await invoke<UpdateStatus>('update_action', {action:'status'}); }
    catch { /* A process exiting for an update no longer answers IPC. */ }
    finally { refreshing = false; }
  }
  async function action(action: 'check'|'download'|'cancel'|'install') {
    if (action === 'install' && hasUnsaved()) { error.value = '还有未保存的凭据或权限修改。请先保存或撤销，再重启更新。'; return false; }
    if (busy.value && action !== 'cancel') return false;
    error.value = '';
    if (action === 'check') status.value.phase = 'checking';
    if (action === 'download') { status.value.phase = 'downloading'; status.value.received = 0; }
    if (action === 'install') status.value.phase = 'installing';
    if (action !== 'cancel') polling = setInterval(() => { void refresh(); }, 350);
    try { await invoke('update_action', {action}); return true; }
    catch (value) { error.value = (value as {message?: string})?.message || '更新未完成，请重试。'; return false; }
    finally { if (action !== 'cancel') clearInterval(polling); await refresh(); }
  }
  function start() {
    void action('check');
    checking = setInterval(() => { if (!busy.value && status.value.phase !== 'ready') void action('check'); }, 60 * 60 * 1000);
  }
  onUnmounted(() => { clearInterval(polling); clearInterval(checking); });
  return {status,error,busy,action,start};
}
