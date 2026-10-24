<script setup lang="ts">
import type {UnwrapRef} from 'vue';import type {VaultUI} from '../useVault';import {kindName} from '../useVault';
const props=defineProps<{v:UnwrapRef<VaultUI>}>();defineEmits<{organize:[kind:'tag',previous?:string]}>();
</script>
<template>
  <section class="list-pane">
    <div class="search"><span>⌕</span><input id="search" v-model="v.query" placeholder="名称、账号、标签、地址" aria-label="搜索凭据"></div>
    <div class="list-filters"><select id="type-filter" v-model="v.typeFilter" aria-label="按类型筛选"><option value="">所有类型</option><option value="password">账号密码</option><option value="api">API 凭据</option><option value="database">数据库</option><option value="generic">通用</option></select><div class="chips"><button v-if="v.tagFilter" class="chip" @click="v.filterTag(v.tagFilter)">{{v.tagFilter}} ×</button></div></div>
    <div class="list-caption"><span>{{v.list.length}} 条凭据</span><span>置顶优先</span></div>
    <div class="items">
          <div v-for="entry in v.list" :key="entry.id" class="item" :class="{selected:v.selected===entry.id&&!v.creating}" :data-entry="entry.id">
            <button class="entry-select" :disabled="v.saving" @click="v.selectEntry(entry.id)"><span class="logo">{{entry.kind==='database'?'DB':entry.kind==='api'?'API':'人'}}</span><span class="entry-text"><strong>{{entry.title}}</strong><small>{{entry.account||kindName(entry.kind)}}</small><small>{{entry.database?.host ? `${entry.database.host}${entry.database.port?':'+entry.database.port:''}` : entry.url}}</small><span class="chips"><span v-for="tag in entry.tags" :key="tag" class="chip">{{tag}}</span></span></span></button>
            <button class="pin" :class="{pinned:entry.pinned}" :data-pin="entry.id" :aria-label="`${entry.pinned?'取消置顶':'置顶'} ${entry.title}`" :aria-pressed="entry.pinned" @click="v.pin(entry)"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M16 9V4l1-1H7l1 1v5l-3 3v2h14v-2l-3-3Z"/><path d="M12 14v7"/></svg></button>
          </div>
      <div v-if="!v.list.length" class="empty">{{v.loading?'正在解锁…':'没有匹配的凭据'}}<br><button class="text-btn" @click="v.allEntries()">清除筛选</button></div>
    </div>
  </section>
</template>
