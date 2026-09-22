<script setup lang="ts">
import { ArrowDownToLine, Plus, Radio, Server } from "@lucide/vue";
import type { SavedHost } from "../types";
import HostCard from "../components/HostCard.vue";

defineProps<{ hosts: SavedHost[]; importing: boolean; defaultHostId: string | null }>();
const emit = defineEmits<{
  add: [];
  import: [];
  open: [host: SavedHost];
  edit: [host: SavedHost];
  remove: [host: SavedHost];
  setDefault: [host: SavedHost];
}>();
</script>

<template>
  <main class="page dashboard-page">
    <header class="page-header">
      <div>
        <p class="eyebrow"><Radio :size="14" /> NOVA WATCH</p>
        <h1>GPU 监控</h1>
      </div>
      <div class="header-actions">
        <button class="secondary-button" :disabled="importing" @click="emit('import')">
          <ArrowDownToLine :size="16" />
          {{ importing ? '读取中' : '导入 SSH' }}
        </button>
        <button class="primary-button" @click="emit('add')"><Plus :size="17" />添加服务器</button>
      </div>
    </header>

    <section class="dashboard-hero" aria-label="NOVA Watch">
      <div class="hero-copy">
        <span class="hero-label">REMOTE GPU</span>
        <h2>每一台，都清晰可见。</h2>
        <p>连接服务器，查看实时 GPU 状态。</p>
        <button class="hero-button" @click="emit('add')"><Plus :size="16" />连接服务器</button>
      </div>
      <div class="hero-bloom" aria-hidden="true">
        <span class="bloom-core">NOVA<br /><small>WATCH</small></span>
      </div>
    </section>

    <section class="servers-section">
      <div class="section-heading">
        <div><p class="section-kicker">YOUR SERVERS</p><h2>服务器</h2></div>
        <span>{{ hosts.length }} 台</span>
      </div>

      <div v-if="hosts.length" class="host-grid">
        <HostCard
          v-for="host in hosts"
          :key="host.id"
          :host="host"
          :is-default="host.id === defaultHostId"
          @open="emit('open', host)"
          @edit="emit('edit', host)"
          @remove="emit('remove', host)"
          @set-default="emit('setDefault', host)"
        />
      </div>

      <div v-else class="empty-state">
        <div class="empty-orbit"><Server :size="26" /></div>
        <h3>还没有服务器</h3>
        <p>添加一台服务器开始监控。</p>
        <div class="empty-actions">
          <button class="primary-button" @click="emit('add')"><Plus :size="16" />添加服务器</button>
          <button class="secondary-button" @click="emit('import')"><ArrowDownToLine :size="16" />导入 SSH</button>
        </div>
      </div>
    </section>
  </main>
</template>
