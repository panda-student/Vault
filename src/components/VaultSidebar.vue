<script setup lang="ts">
import type {UnwrapRef} from 'vue';import type {VaultUI} from '../useVault';
defineProps<{v:UnwrapRef<VaultUI>}>();
defineEmits<{organize:[kind:'group'|'tag',previous?:string]}>();
</script>
<template>
  <aside class="sidebar">
    <div class="brand"><span class="brand-icon">◇</span>本地密码库</div>
    <div class="nav-scroll">
      <button class="nav" :class="{active:v.page==='entries'&&!v.tagFilter}" data-nav="entries" @click="v.allEntries()">▦ 全部凭据<span class="count">{{v.entries.length}}</span></button>
      <div class="nav-heading"><span>标签</span><button class="text-btn" aria-label="新增标签" @click="$emit('organize','tag')">＋</button></div>
      <div v-for="t in v.organization.tags" :key="t" class="nav-line"><button class="nav" :class="{active:v.page==='entries'&&v.tagFilter===t}" :data-tag="t" @click="v.filterTag(t)"># {{t}}</button><button class="text-btn" :aria-label="`重命名标签 ${t}`" @click="$emit('organize','tag',t)">⋯</button></div>
    </div>
    <div class="sidebar-bottom"><button class="nav" :class="{active:v.page==='trash'}" data-nav="trash" @click="v.navigate('trash')">♲ 垃圾桶<span class="count">{{v.trash.length}}</span></button><button class="nav" :class="{active:v.page==='mcp'}" data-nav="mcp" @click="v.navigate('mcp')">⌘ MCP 管理</button><button class="nav" :class="{active:v.page==='settings'}" data-nav="settings" @click="v.navigate('settings')">⚙ 设置</button><div class="local-label">安全保存，便捷填写<small>本地加密 · AI 接口不返回原文</small></div></div>
  </aside>
</template>
