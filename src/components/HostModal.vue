<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { Check, Eye, EyeOff, FileKey, LoaderCircle, LockKeyhole, Server, ShieldCheck, X } from "@lucide/vue";
import type { ConnectionReport, RuntimeSecret, SavedHost } from "../types";
import { toTarget } from "../lib/hosts";

const props = defineProps<{ host?: SavedHost; secret?: RuntimeSecret; connectAfterSave?: boolean }>();
const emit = defineEmits<{ close: []; save: [host: SavedHost, secret: RuntimeSecret, connect: boolean] }>();

const form = reactive<SavedHost>({
  id: props.host?.id ?? crypto.randomUUID(),
  name: props.host?.name ?? "",
  host: props.host?.host ?? "",
  port: props.host?.port ?? 22,
  username: props.host?.username ?? "",
  authMethod: props.host?.authMethod ?? "password",
  privateKeyPath: props.host?.privateKeyPath ?? "~/.ssh/id_ed25519",
  expectedFingerprint: props.host?.expectedFingerprint,
  source: props.host?.source ?? "manual",
});
const secret = reactive<RuntimeSecret>({ password: props.secret?.password ?? "", passphrase: props.secret?.passphrase ?? "" });
const showSecret = ref(false);
const testing = ref(false);
const error = ref("");
const report = ref<ConnectionReport | null>(null);
const tested = ref(Boolean(props.host?.expectedFingerprint));

const canTest = computed(() => {
  if (!form.name.trim() || !form.host.trim() || !form.username.trim() || !form.port) return false;
  if (form.authMethod === "password") return Boolean(secret.password);
  return Boolean(form.privateKeyPath);
});

watch(
  () => [form.host, form.port, form.username, form.authMethod, form.privateKeyPath, secret.password, secret.passphrase],
  () => {
    report.value = null;
    error.value = "";
    tested.value = false;
  },
);

async function chooseKey() {
  const path = await open({ multiple: false, title: "选择 SSH 私钥" });
  if (typeof path === "string") form.privateKeyPath = path;
}

async function test() {
  testing.value = true;
  error.value = "";
  try {
    const result = await invoke<ConnectionReport>("test_connection", { target: toTarget(form, secret) });
    if (!result.nvidiaSmiAvailable) throw new Error("连接成功，但服务器没有找到 nvidia-smi");
    report.value = result;
    form.expectedFingerprint = result.fingerprint;
    tested.value = true;
  } catch (reason) {
    error.value = String(reason);
  } finally {
    testing.value = false;
  }
}

function save() {
  if (!tested.value) {
    error.value = "请先测试连接并确认服务器指纹";
    return;
  }
  emit("save", { ...form }, { ...secret }, Boolean(props.connectAfterSave));
}
</script>

<template>
  <div class="modal-backdrop" @mousedown.self="emit('close')">
    <section class="modal-panel" role="dialog" aria-modal="true" aria-labelledby="host-modal-title">
      <header class="modal-header">
        <div class="modal-title-icon"><Server :size="20" /></div>
        <div>
          <h2 id="host-modal-title">{{ host ? '编辑服务器' : '添加服务器' }}</h2>
          <p>建立安全 SSH 连接，并验证 GPU 工具。</p>
        </div>
        <button class="icon-button close-button" aria-label="关闭" @click="emit('close')"><X :size="20" /></button>
      </header>

      <div class="modal-body">
        <div class="field full">
          <label for="server-name">显示名称</label>
          <input id="server-name" v-model="form.name" placeholder="例如：训练节点 A100" autofocus />
        </div>
        <div class="form-grid host-address-grid">
          <div class="field wide">
            <label for="server-host">主机地址</label>
            <input id="server-host" v-model="form.host" placeholder="192.168.1.42 或 gpu.example.com" spellcheck="false" />
          </div>
          <div class="field">
            <label for="server-port">端口</label>
            <input id="server-port" v-model.number="form.port" type="number" min="1" max="65535" />
          </div>
        </div>
        <div class="field full">
          <label for="server-user">用户名</label>
          <input id="server-user" v-model="form.username" placeholder="root" autocomplete="username" />
        </div>

        <div class="field full">
          <label>认证方式</label>
          <div class="segmented-control">
            <button :class="{ active: form.authMethod === 'password' }" @click="form.authMethod = 'password'">
              <LockKeyhole :size="16" />密码
            </button>
            <button :class="{ active: form.authMethod === 'privateKey' }" @click="form.authMethod = 'privateKey'">
              <FileKey :size="16" />私钥
            </button>
          </div>
        </div>

        <div v-if="form.authMethod === 'password'" class="field full">
          <label for="server-password">SSH 密码</label>
          <div class="input-with-action">
            <input id="server-password" v-model="secret.password" :type="showSecret ? 'text' : 'password'" autocomplete="current-password" placeholder="SSH 密码" />
            <button aria-label="显示或隐藏密码" @click="showSecret = !showSecret">
              <EyeOff v-if="showSecret" :size="17" /><Eye v-else :size="17" />
            </button>
          </div>
          <p class="field-hint">密码保存在本机应用数据目录的 .env 文件中。</p>
        </div>

        <template v-else>
          <div class="field full">
            <label for="key-path">私钥文件</label>
            <div class="input-with-button">
              <input id="key-path" v-model="form.privateKeyPath" placeholder="~/.ssh/id_ed25519" spellcheck="false" />
              <button class="secondary-button compact" @click="chooseKey">浏览</button>
            </div>
          </div>
          <div class="field full">
            <label for="passphrase">私钥口令 <span>可选</span></label>
            <input id="passphrase" v-model="secret.passphrase" type="password" placeholder="加密私钥需要填写" />
          </div>
        </template>

        <div v-if="report" class="connection-result success">
          <ShieldCheck :size="18" />
          <div>
            <strong>连接成功 · {{ report.hostname }}</strong>
            <span>{{ report.nvitopAvailable ? 'nvitop 已就绪' : '未找到 nvitop，图表仍可用' }} · {{ report.fingerprint }}</span>
          </div>
        </div>
        <div v-else-if="error" class="connection-result error">
          <X :size="18" />
          <div><strong>连接检查失败</strong><span>{{ error }}</span></div>
        </div>
      </div>

      <footer class="modal-footer">
        <button class="secondary-button" :disabled="testing || !canTest" @click="test">
          <LoaderCircle v-if="testing" class="spin" :size="17" />
          <ShieldCheck v-else :size="17" />
          {{ testing ? '正在连接…' : '测试连接' }}
        </button>
        <div class="footer-spacer" />
        <button class="ghost-button" @click="emit('close')">取消</button>
        <button class="primary-button" :disabled="!tested" @click="save"><Check :size="17" />保存{{ connectAfterSave ? '并打开' : '' }}</button>
      </footer>
    </section>
  </div>
</template>
