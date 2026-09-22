<script setup lang="ts">
import {
  Gauge,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Server,
  ServerCog,
} from "@lucide/vue";
import type { SavedHost } from "../types";
import WindowControls from "./WindowControls.vue";

const props = defineProps<{
  active: "overview" | "monitor";
  collapsed: boolean;
  hosts: SavedHost[];
  selectedHostId: string | null;
  connection: "connecting" | "connected" | "error" | "stopped";
}>();

const emit = defineEmits<{
  toggleSidebar: [];
  overview: [];
  monitor: [];
  switchHost: [hostId: string];
  editCurrent: [];
  add: [];
}>();

function switchHost(event: Event) {
  const select = event.target as HTMLSelectElement;
  const nextId = select.value;
  select.value = props.selectedHostId ?? "";
  if (nextId) emit("switchHost", nextId);
}

function connectionLabel() {
  if (props.connection === "connected") return "实时";
  if (props.connection === "connecting") return "连接中";
  if (props.connection === "error") return "重连中";
  return "已停止";
}
</script>

<template>
  <header class="app-topbar" data-tauri-drag-region>
    <button
      class="topbar-square-button"
      :aria-label="collapsed ? '展开侧栏' : '收起侧栏'"
      :title="collapsed ? '展开侧栏' : '收起侧栏'"
      @click="emit('toggleSidebar')"
    >
      <PanelLeftOpen v-if="collapsed" :size="18" />
      <PanelLeftClose v-else :size="18" />
    </button>

    <nav class="topbar-segment" aria-label="页面切换">
      <button :class="{ active: active === 'overview' }" @click="emit('overview')">
        <Gauge :size="15" />
        总览
      </button>
      <button :class="{ active: active === 'monitor' }" @click="emit('monitor')">
        <Server :size="15" />
        监控
      </button>
    </nav>

    <div class="topbar-drag-space" data-tauri-drag-region />

    <div v-if="selectedHostId" :class="['topbar-status', connection]">
      <i />
      {{ connectionLabel() }}
    </div>

    <label v-if="selectedHostId" class="topbar-server-switcher" aria-label="切换服务器">
      <Server :size="15" />
      <select :value="selectedHostId" @change="switchHost">
        <option v-for="host in hosts" :key="host.id" :value="host.id">{{ host.name }}</option>
      </select>
    </label>

    <button
      v-if="selectedHostId"
      class="topbar-icon-button"
      aria-label="编辑当前服务器"
      title="编辑当前服务器"
      @click="emit('editCurrent')"
    >
      <ServerCog :size="17" />
    </button>
    <button class="topbar-icon-button" aria-label="添加服务器" title="添加服务器" @click="emit('add')">
      <Plus :size="18" />
    </button>

    <span class="topbar-divider" />
    <WindowControls />
  </header>
</template>
