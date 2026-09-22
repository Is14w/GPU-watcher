<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Activity, ChartNoAxesCombined, CircleAlert, ListTree, Radio, RotateCw, TerminalSquare } from "@lucide/vue";
import { onBeforeUnmount, onMounted, ref } from "vue";
import GpuCard from "../components/GpuCard.vue";
import SessionSummary from "../components/SessionSummary.vue";
import TerminalPanel from "../components/TerminalPanel.vue";
import { recordSessionSnapshot } from "../lib/session";
import type { MonitorEvent, Snapshot, SshTarget } from "../types";

const props = defineProps<{ target: SshTarget }>();
const emit = defineEmits<{
  editCredentials: [];
  status: [value: "connecting" | "connected" | "error" | "stopped"];
}>();

const tab = ref<"overview" | "processes" | "summary" | "terminal">("terminal");
const connection = ref<"connecting" | "connected" | "error" | "stopped">("connecting");
const snapshot = ref<Snapshot | null>(null);
const error = ref("");
const history = ref<Record<string, number[]>>({});
let unlisten: UnlistenFn | undefined;
let runId = crypto.randomUUID();
let disposed = false;

function ingest(data: Snapshot) {
  recordSessionSnapshot(props.target.id, data);
  snapshot.value = data;
  const next = { ...history.value };
  for (const gpu of data.gpus) {
    next[gpu.uuid] = [...(next[gpu.uuid] ?? []), gpu.gpuUtilization ?? 0].slice(-60);
  }
  history.value = next;
}

async function startMonitor() {
  runId = crypto.randomUUID();
  connection.value = "connecting";
  emit("status", "connecting");
  error.value = "";
  await invoke("start_monitor", { target: props.target, runId });
  if (disposed) await invoke("stop_monitor", { targetId: props.target.id, runId }).catch(() => undefined);
}

onMounted(async () => {
  unlisten = await listen<MonitorEvent>("monitor-update", ({ payload }) => {
    if (disposed || payload.targetId !== props.target.id || payload.runId !== runId) return;
    if (payload.kind === "snapshot" && payload.snapshot) {
      connection.value = "connected";
      emit("status", "connected");
      error.value = "";
      ingest(payload.snapshot);
    } else if (payload.kind === "error") {
      connection.value = "error";
      emit("status", "error");
      error.value = payload.message ?? "监控连接异常";
    } else if (payload.kind === "connecting" || payload.kind === "connected" || payload.kind === "stopped") {
      connection.value = payload.kind;
      emit("status", payload.kind);
    }
  });
  if (disposed) { unlisten(); return; }
  try {
    await startMonitor();
  } catch (reason) {
    connection.value = "error";
    emit("status", "error");
    error.value = String(reason);
  }
});

onBeforeUnmount(() => {
  disposed = true;
  unlisten?.();
  invoke("stop_monitor", { targetId: props.target.id, runId }).catch(() => undefined);
});
</script>

<template>
  <main class="page monitor-page">
    <div v-if="error" class="error-banner">
      <CircleAlert :size="19" />
      <div><strong>监控连接中断</strong><span>{{ error }}</span></div>
      <button class="ghost-button" @click="emit('editCredentials')">更新凭据</button>
      <button class="secondary-button compact" @click="startMonitor"><RotateCw :size="15" />重连</button>
    </div>

    <nav class="tab-bar">
      <button :class="{ active: tab === 'overview' }" @click="tab = 'overview'"><Activity :size="16" />指标总览</button>
      <button :class="{ active: tab === 'processes' }" @click="tab = 'processes'"><ListTree :size="16" />GPU 进程 <span>{{ snapshot?.processes.length ?? 0 }}</span></button>
      <button :class="{ active: tab === 'summary' }" @click="tab = 'summary'"><ChartNoAxesCombined :size="16" />运行总结</button>
      <button :class="{ active: tab === 'terminal' }" @click="tab = 'terminal'"><TerminalSquare :size="16" />nvitop</button>
    </nav>

    <section v-if="tab === 'overview'" class="monitor-content">
      <div v-if="snapshot" class="gpu-grid">
        <GpuCard v-for="gpu in snapshot.gpus" :key="gpu.uuid" :gpu="gpu" :history="history[gpu.uuid] ?? []" />
      </div>
      <div v-else class="loading-monitor">
        <div class="radar-loader"><Radio :size="26" /></div>
        <h3>正在读取 GPU 状态</h3>
        <p>正在连接 {{ target.host }}</p>
      </div>
    </section>

    <section v-else-if="tab === 'processes'" class="process-panel">
      <div class="table-header"><div><h2>计算进程</h2></div><span>{{ snapshot?.processes.length ?? 0 }} 个进程</span></div>
      <div class="process-table">
        <div class="process-row process-head"><span>PID</span><span>进程</span><span>GPU</span><span>显存</span></div>
        <div v-for="process in snapshot?.processes" :key="`${process.gpuUuid}-${process.pid}`" class="process-row">
          <span class="mono">{{ process.pid }}</span>
          <strong :title="process.processName">{{ process.processName }}</strong>
          <span>GPU {{ snapshot?.gpus.find((gpu) => gpu.uuid === process.gpuUuid)?.index ?? '?' }}</span>
          <span>{{ process.usedMemory?.toFixed(0) ?? '—' }} MiB</span>
        </div>
        <div v-if="!snapshot?.processes.length" class="table-empty">当前没有可见的 GPU 计算进程</div>
      </div>
    </section>

    <SessionSummary v-else-if="tab === 'summary'" :host-id="target.id" />

    <TerminalPanel v-show="tab === 'terminal'" :target="target" :active="tab === 'terminal'" />
  </main>
</template>
