<script setup lang="ts">
defineProps<{ name: 'copy' | 'remove' | 'eye' | 'eye-off' }>();
</script>

<template>
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <template v-if="name === 'copy'"><rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></template>
    <template v-else-if="name === 'remove'"><path d="m6 6 12 12M18 6 6 18"/></template>
    <template v-else><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/><path v-if="name === 'eye-off'" d="m3 3 18 18"/></template>
  </svg>
</template>
