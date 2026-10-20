<script setup lang="ts">
import {computed,ref} from 'vue';import type {UnwrapRef} from 'vue';import type {VaultUI} from '../useVault';import {kindName} from '../useVault';
const props=defineProps<{v:UnwrapRef<VaultUI>}>();defineEmits<{organize:[kind:'group'|'tag',previous?:string]}>();
const collapsed=ref(new Set<string>());
const grouped=computed(()=>[...new Set(props.v.list.map(e=>e.group))].map(name=>({name,entries:props.v.list.filter(e=>e.group===name)})));
function toggle(name:string){if(collapsed.value.has(name))collapsed.value.delete(name);else collapsed.value.add(name);}
function add(group:string){props.v.filterGroup(group);props.v.openForm();}
</script>
<template>
  <section class="list-pane">
    <div class="search"><span>⌕</span><input id="search" v-model="v.query" placeholder="名称、账号、标签、地址" aria-label="搜索凭据"></div>
    <div class="list-filters"><select id="type-filter" v-model="v.typeFilter" aria-label="按类型筛选"><option value="">所有类型</option><option value="password">账号密码</option><option value="api">API 凭据</option><option value="database">数据库</option><option value="generic">通用</option></select><div class="chips"><button v-if="v.groupFilter!==null" class="chip" @click="v.filterGroup(null)">{{v.groupFilter||'未分组'}} ×</button><button v-if="v.tagFilter" class="chip" @click="v.filterTag(v.tagFilter)">{{v.tagFilter}} ×</button></div></div>
    <div class="list-caption"><span>{{v.list.length}} 条凭据</span><span>组内置顶优先</span></div>
    <div class="items">
      <div v-for="group in grouped" :key="group.name">
        <div class="group-heading"><button class="text-btn group-toggle" :aria-expanded="!collapsed.has(group.name)" @click="toggle(group.name)">{{collapsed.has(group.name)?'▸':'▾'}} {{group.name||'未分组'}} · {{group.entries.length}}</button><button v-if="group.name" class="text-btn" :aria-label="`重命名 ${group.name}`" @click="$emit('organize','group',group.name)">⋯</button><button class="text-btn" :aria-label="`在 ${group.name||'未分组'} 新增`" @click="add(group.name)">＋</button></div>
        <template v-if="!collapsed.has(group.name)">
          <div v-for="entry in group.entries" :key="entry.id" class="item" :class="{selected:v.selected===entry.id&&!v.creating}" :data-entry="entry.id">
            <button class="entry-select" :disabled="v.saving" @click="v.selectEntry(entry.id)"><span class="logo">{{entry.kind==='database'?'DB':entry.kind==='api'?'API':'人'}}</span><span class="entry-text"><strong>{{entry.title}}</strong><small>{{entry.account||kindName(entry.kind)}}</small><small>{{entry.database?.host ? `${entry.database.host}${entry.database.port?':'+entry.database.port:''}` : entry.url}}</small><span class="chips"><span v-for="tag in entry.tags" :key="tag" class="chip">{{tag}}</span></span></span></button>
            <button class="pin" :class="{pinned:entry.pinned}" :data-pin="entry.id" :aria-label="`${entry.pinned?'取消置顶':'置顶'} ${entry.title}`" :aria-pressed="entry.pinned" @click="v.pin(entry)"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M16 9V4l1-1H7l1 1v5l-3 3v2h14v-2l-3-3Z"/><path d="M12 14v7"/></svg></button>
          </div>
        </template>
      </div>
      <div v-if="!v.list.length" class="empty">{{v.loading?'正在解锁…':'没有匹配的凭据'}}<br><button class="text-btn" @click="v.allEntries()">清除筛选</button></div>
    </div>
  </section>
</template>
