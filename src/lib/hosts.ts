import type { RuntimeSecret, SavedHost, SshTarget } from "../types";

const STORAGE_KEY = "gpu-watcher.hosts.v1";
const DEFAULT_HOST_KEY = "gpu-watcher.default-host.v1";

export type StoredCredentials = Record<string, RuntimeSecret>;

export function loadHosts(): SavedHost[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as SavedHost[];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function persistHosts(hosts: SavedHost[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(hosts));
}

export function loadDefaultHostId(): string | null {
  return localStorage.getItem(DEFAULT_HOST_KEY);
}

export function persistDefaultHostId(hostId: string | null) {
  if (hostId) localStorage.setItem(DEFAULT_HOST_KEY, hostId);
  else localStorage.removeItem(DEFAULT_HOST_KEY);
}

export function toTarget(host: SavedHost, secret?: RuntimeSecret): SshTarget {
  return {
    id: host.id,
    name: host.name,
    host: host.host,
    port: host.port,
    username: host.username,
    authMethod: host.authMethod,
    privateKeyPath: host.privateKeyPath,
    expectedFingerprint: host.expectedFingerprint,
    password: secret?.password,
    passphrase: secret?.passphrase,
  };
}
