<script setup lang="ts">
import { Gauge, Plus, Server } from "@lucide/vue";
import LogoMark from "./LogoMark.vue";

defineProps<{ active: "overview" | "monitor" }>();
const emit = defineEmits<{ overview: []; monitor: []; add: [] }>();
</script>

<template>
  <aside class="sidebar">
    <button class="brand" aria-label="NOVA Watch home" @click="emit('overview')">
      <LogoMark />
      <span>NOVA <em>Watch</em></span>
    </button>

    <nav class="nav-group" aria-label="主导航">
      <p>工作区</p>
      <button :class="['nav-item', { active: active === 'overview' }]" @click="emit('overview')">
        <Gauge :size="18" />
        <span>总览</span>
      </button>
      <button :class="['nav-item', { active: active === 'monitor' }]" @click="emit('monitor')">
        <Server :size="18" />
        <span>实时监控</span>
      </button>
    </nav>

    <div class="sidebar-spacer" />

    <button class="add-server-button" @click="emit('add')">
      <Plus :size="17" />
      <span>添加服务器</span>
    </button>

  </aside>
</template>
