<script setup lang="ts">
import { ArrowUpRight, Fingerprint, KeyRound, MoreHorizontal, Pencil, Server, Trash2 } from "@lucide/vue";
import { ref } from "vue";
import type { SavedHost } from "../types";

defineProps<{ host: SavedHost; isDefault?: boolean }>();
const emit = defineEmits<{ open: []; edit: []; remove: []; setDefault: [] }>();
const menuOpen = ref(false);
</script>

<template>
  <article class="host-card">
    <div class="host-card-top">
      <div class="server-icon"><Server :size="20" /></div>
      <div class="host-menu-wrap">
        <button class="icon-button" aria-label="主机菜单" @click.stop="menuOpen = !menuOpen">
          <MoreHorizontal :size="19" />
        </button>
        <div v-if="menuOpen" class="mini-menu" @mouseleave="menuOpen = false">
          <button v-if="!isDefault" @click="emit('setDefault'); menuOpen = false"><Server :size="14" />设为默认</button>
          <button @click="emit('edit'); menuOpen = false"><Pencil :size="14" />编辑</button>
          <button class="danger" @click="emit('remove'); menuOpen = false"><Trash2 :size="14" />删除</button>
        </div>
      </div>
    </div>

    <div class="host-identity">
      <h3>{{ host.name }}<span v-if="isDefault" class="default-badge">默认</span></h3>
      <p>{{ host.username }}@{{ host.host }}:{{ host.port }}</p>
    </div>

    <div class="host-meta">
      <span><KeyRound :size="13" />{{ host.authMethod === 'password' ? '密码' : 'SSH 私钥' }}</span>
      <span v-if="host.expectedFingerprint"><Fingerprint :size="13" />已验证</span>
    </div>

    <button class="connect-button" @click="emit('open')">
      打开监控
      <ArrowUpRight :size="16" />
    </button>
  </article>
</template>
