<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, X } from "@lucide/vue";
import { computed, nextTick, onMounted, reactive, ref } from "vue";
import HostModal from "./components/HostModal.vue";
import Sidebar from "./components/Sidebar.vue";
import Topbar from "./components/Topbar.vue";
import { loadDefaultHostId, loadHosts, persistDefaultHostId, persistHosts, toTarget } from "./lib/hosts";
import type { RuntimeSecret, SavedHost, SshConfigHost } from "./types";
import DashboardView from "./views/DashboardView.vue";
import MonitorView from "./views/MonitorView.vue";

const hosts = ref<SavedHost[]>(loadHosts());
const secrets = reactive<Record<string, RuntimeSecret>>({});
const storedDefaultHostId = loadDefaultHostId();
const defaultHostId = ref<string | null>(hosts.value.some((host) => host.id === storedDefaultHostId) ? storedDefaultHostId : hosts.value[0]?.id ?? null);
const selectedHostId = ref<string | null>(null);
const modalHost = ref<SavedHost | undefined>();
const modalOpen = ref(false);
const connectAfterSave = ref(false);
const importing = ref(false);
const toast = ref("");
const sidebarCollapsed = ref(false);
const monitorConnection = ref<"connecting" | "connected" | "error" | "stopped">("stopped");
let toastTimer: number | undefined;
let credentialRevision = 0;

const selectedHost = computed(() => hosts.value.find((host) => host.id === selectedHostId.value));
const selectedTarget = computed(() => selectedHost.value ? toTarget(selectedHost.value, secrets[selectedHost.value.id]) : null);

if (defaultHostId.value !== storedDefaultHostId) persistDefaultHostId(defaultHostId.value);

function notify(message: string) {
  toast.value = message;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => { toast.value = ""; }, 3200);
}

async function persistCredentials() {
  credentialRevision += 1;
  await invoke("save_credentials", {
    credentials: Object.entries(secrets).map(([id, secret]) => ({ id, password: secret.password, passphrase: secret.passphrase })),
    revision: credentialRevision,
  });
}

function showAdd() {
  modalHost.value = undefined;
  connectAfterSave.value = false;
  modalOpen.value = true;
}

function showOverview() {
  selectedHostId.value = null;
  monitorConnection.value = "stopped";
}

function showEdit(host: SavedHost, shouldConnect = false) {
  modalHost.value = host;
  connectAfterSave.value = shouldConnect;
  modalOpen.value = true;
}

function openHost(host: SavedHost) {
  const needsPassword = host.authMethod === "password" && !secrets[host.id]?.password;
  if (needsPassword || !host.expectedFingerprint) {
    showEdit(host, true);
    return;
  }
  monitorConnection.value = "connecting";
  selectedHostId.value = host.id;
}

function openDefault() {
  const host = hosts.value.find((item) => item.id === defaultHostId.value);
  if (host) openHost(host);
  else showOverview();
}

function openMonitor() {
  if (!selectedHost.value) openDefault();
}

function switchHost(hostId: string) {
  const host = hosts.value.find((item) => item.id === hostId);
  if (host) openHost(host);
}

function setDefault(host: SavedHost) {
  defaultHostId.value = host.id;
  persistDefaultHostId(host.id);
  notify(`已将 ${host.name} 设为默认服务器`);
}

async function saveHost(host: SavedHost, secret: RuntimeSecret, shouldConnect: boolean) {
  const index = hosts.value.findIndex((item) => item.id === host.id);
  if (index >= 0) hosts.value[index] = host;
  else hosts.value.push(host);
  if (!defaultHostId.value) {
    defaultHostId.value = host.id;
    persistDefaultHostId(host.id);
  }
  secrets[host.id] = secret;
  try {
    await persistCredentials();
  } catch (reason) {
    notify(`凭据保存失败：${String(reason)}`);
    return;
  }
  persistHosts(hosts.value);
  modalOpen.value = false;
  notify(index >= 0 ? "服务器配置已更新" : "服务器已添加");
  if (shouldConnect) {
    monitorConnection.value = "connecting";
    selectedHostId.value = null;
    await nextTick();
    selectedHostId.value = host.id;
  }
}

async function removeHost(host: SavedHost) {
  if (!window.confirm(`确定删除“${host.name}”吗？`)) return;
  hosts.value = hosts.value.filter((item) => item.id !== host.id);
  delete secrets[host.id];
  await persistCredentials().catch((reason) => notify(`凭据更新失败：${String(reason)}`));
  if (defaultHostId.value === host.id) {
    defaultHostId.value = hosts.value[0]?.id ?? null;
    persistDefaultHostId(defaultHostId.value);
  }
  persistHosts(hosts.value);
  notify("服务器已删除");
}

async function importSshConfig() {
  importing.value = true;
  try {
    const imported = await invoke<SshConfigHost[]>("read_ssh_config_hosts");
    let added = 0;
    for (const item of imported) {
      const duplicate = hosts.value.some((host) => host.host === item.host && host.port === item.port && host.username === (item.username || "root"));
      if (duplicate) continue;
      hosts.value.push({
        id: crypto.randomUUID(),
        name: item.alias,
        host: item.host,
        port: item.port,
        username: item.username || "root",
        authMethod: item.identityFile ? "privateKey" : "password",
        privateKeyPath: item.identityFile,
        source: "ssh-config",
      });
      added += 1;
    }
    if (!defaultHostId.value && hosts.value[0]) {
      defaultHostId.value = hosts.value[0].id;
      persistDefaultHostId(defaultHostId.value);
    }
    persistHosts(hosts.value);
    notify(imported.length ? `已导入 ${added} 台新主机${added < imported.length ? `，跳过 ${imported.length - added} 台重复项` : ""}` : "~/.ssh/config 中没有可导入的主机");
  } catch (reason) {
    notify(`导入失败：${String(reason)}`);
  } finally {
    importing.value = false;
  }
}

onMounted(async () => {
  const stored = await invoke<Array<{ id: string; password?: string; passphrase?: string }>>("load_credentials").catch(() => []);
  for (const credential of stored) secrets[credential.id] = { password: credential.password, passphrase: credential.passphrase };
  openDefault();
});
</script>

<template>
  <div :class="['app-shell', { 'sidebar-collapsed': sidebarCollapsed }]">
    <Topbar
      :active="selectedHost ? 'monitor' : 'overview'"
      :collapsed="sidebarCollapsed"
      :hosts="hosts"
      :selected-host-id="selectedHostId"
      :connection="monitorConnection"
      @toggle-sidebar="sidebarCollapsed = !sidebarCollapsed"
      @overview="showOverview"
      @monitor="openMonitor"
      @switch-host="switchHost"
      @edit-current="selectedHost && showEdit(selectedHost, true)"
      @add="showAdd"
    />

    <div class="app-body">
      <Sidebar :active="selectedHost ? 'monitor' : 'overview'" @overview="showOverview" @monitor="openMonitor" @add="showAdd" />
      <DashboardView
        v-if="!selectedHost || !selectedTarget"
        :hosts="hosts"
        :importing="importing"
        :default-host-id="defaultHostId"
        @add="showAdd"
        @import="importSshConfig"
        @open="openHost"
        @edit="showEdit"
        @remove="removeHost"
        @set-default="setDefault"
      />
      <MonitorView
        v-else
        :key="`${selectedTarget.id}-${selectedTarget.expectedFingerprint ?? ''}`"
        :target="selectedTarget"
        @status="monitorConnection = $event"
        @edit-credentials="showEdit(selectedHost, true)"
      />
    </div>

    <HostModal
      v-if="modalOpen"
      :host="modalHost"
      :secret="modalHost ? secrets[modalHost.id] : undefined"
      :connect-after-save="connectAfterSave"
      @close="modalOpen = false"
      @save="saveHost"
    />

    <Transition name="toast">
      <div v-if="toast" class="toast-message">
        <CheckCircle2 :size="18" />
        <span>{{ toast }}</span>
        <button aria-label="关闭通知" @click="toast = ''"><X :size="15" /></button>
      </div>
    </Transition>
  </div>
</template>
