<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Maximize2, RefreshCw, TerminalSquare } from "@lucide/vue";
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SshTarget, TerminalEvent } from "../types";

const props = defineProps<{ target: SshTarget; active: boolean }>();
const container = ref<HTMLElement | null>(null);
const status = ref<"connecting" | "connected" | "error" | "stopped">("connecting");
const error = ref("");
let terminal: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let unlisten: UnlistenFn | undefined;
let resizeObserver: ResizeObserver | undefined;
let started = false;
let runId = crypto.randomUUID();
let disposed = false;

function decodeBase64(value: string) {
  const padded = value + "=".repeat((4 - value.length % 4) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

async function resizeRemote() {
  if (!props.active || !container.value || container.value.clientWidth < 32 || container.value.clientHeight < 32 || !fitAddon || !terminal || !started) return;
  fitAddon.fit();
  if (terminal.cols < 1 || terminal.rows < 1) return;
  await invoke("resize_terminal", { targetId: props.target.id, runId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
}

async function start() {
  if (!terminal || !fitAddon || disposed) return;
  const currentRunId = crypto.randomUUID();
  runId = currentRunId;
  started = false;
  status.value = "connecting";
  error.value = "";
  fitAddon.fit();
  terminal.reset();
  terminal.writeln("\x1b[38;2;229;154;176mNOVA Watch\x1b[0m  正在建立远程 PTY…\r\n");
  await invoke("start_terminal", {
    target: props.target,
    runId: currentRunId,
    cols: terminal.cols,
    rows: terminal.rows,
  });
  if (disposed) {
    await invoke("stop_terminal", { targetId: props.target.id, runId: currentRunId }).catch(() => undefined);
    return;
  }
  started = true;
}

onMounted(async () => {
  await nextTick();
  terminal = new Terminal({
    cursorBlink: true,
    cursorStyle: "bar",
    fontFamily: "'Cascadia Code', 'SFMono-Regular', Consolas, monospace",
    fontSize: 13,
    lineHeight: 1.16,
    scrollback: 3000,
    theme: {
      background: "#282532",
      foreground: "#f2ebf2",
      cursor: "#efa2b5",
      cursorAccent: "#282532",
      selectionBackground: "#a99de066",
      black: "#302b38",
      red: "#f28e9b",
      green: "#a9d9c8",
      yellow: "#e9c27e",
      blue: "#9db9e8",
      magenta: "#c5a8e8",
      cyan: "#9fd2dc",
      white: "#fff8fc",
    },
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(container.value!);
  terminal.onData((data) => {
    if (started && !disposed) invoke("terminal_input", { targetId: props.target.id, runId, data }).catch(() => undefined);
  });

  unlisten = await listen<TerminalEvent>("terminal-update", ({ payload }) => {
    if (disposed || payload.targetId !== props.target.id || payload.runId !== runId || !terminal) return;
    if (payload.kind === "output" && payload.data) terminal.write(decodeBase64(payload.data));
    if (payload.kind === "connected") status.value = "connected";
    if (payload.kind === "connecting") status.value = "connecting";
    if (payload.kind === "stopped") status.value = "stopped";
    if (payload.kind === "error") {
      status.value = "error";
      error.value = payload.message ?? "终端连接失败";
      terminal.writeln(`\r\n\x1b[31m${error.value}\x1b[0m`);
    }
  });
  if (disposed) { unlisten(); return; }

  resizeObserver = new ResizeObserver(() => void resizeRemote());
  resizeObserver.observe(container.value!);
  try {
    await start();
  } catch (reason) {
    status.value = "error";
    error.value = String(reason);
    terminal.writeln(`\r\n\x1b[31m${error.value}\x1b[0m`);
  }
});

onBeforeUnmount(() => {
  disposed = true;
  resizeObserver?.disconnect();
  unlisten?.();
  invoke("stop_terminal", { targetId: props.target.id, runId }).catch(() => undefined);
  terminal?.dispose();
});

watch(() => props.active, async (active) => {
  if (!active) return;
  await nextTick();
  window.requestAnimationFrame(() => void resizeRemote());
});
</script>

<template>
  <section class="terminal-shell">
    <header class="terminal-toolbar">
      <div class="terminal-title"><TerminalSquare :size="16" /><strong>nvitop</strong><span>{{ target.username }}@{{ target.host }}</span></div>
      <div class="terminal-actions">
        <span :class="['terminal-status', status]"><i />{{ status === 'connected' ? '已连接' : status === 'connecting' ? '连接中' : status === 'error' ? '连接错误' : '已停止' }}</span>
        <button class="icon-button" title="重新连接" @click="start"><RefreshCw :size="16" /></button>
        <button class="icon-button" title="适应窗口" @click="resizeRemote"><Maximize2 :size="16" /></button>
      </div>
    </header>
    <div ref="container" class="terminal-container" />
  </section>
</template>
