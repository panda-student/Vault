<script setup lang="ts">
import type {UnwrapRef} from 'vue';import type {VaultUI} from '../useVault';import {kindName,timestamp} from '../useVault';
defineProps<{v:UnwrapRef<VaultUI>}>();
</script>
<template>
  <section class="settings trash-panel">
    <div class="trash-heading"><p class="description">删除的凭据可恢复，不自动清理。<br>永久清空仅限人工操作，并需要再次确认；AI 没有永久删除工具。</p><button id="empty-trash" class="secondary danger" :disabled="v.saving||!v.trash.length" @click="v.emptyTrash()">清空垃圾桶</button></div>
    <div v-for="entry in v.trash" :key="entry.id" class="card trash-entry" :data-trash="entry.id"><div><h3>{{entry.title}}</h3><p class="description">{{entry.group||'未分组'}} · {{kindName(entry.kind)}} · {{entry.account||'无账号'}}</p><div class="chips"><span v-for="t in entry.tags" :key="t" class="chip">{{t}}</span></div><p class="metadata">删除于 {{timestamp(entry.deletedAt)}}</p></div><button class="secondary" :disabled="v.saving" :data-restore="entry.id" @click="v.restore(entry)">恢复</button></div>
    <div v-if="!v.trash.length" class="empty">垃圾桶为空</div>
  </section>
</template>
