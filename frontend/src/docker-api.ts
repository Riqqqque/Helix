import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type DockerContainerAction = 'start' | 'stop' | 'restart';

export interface DockerContainer {
  name: string;
  image: string;
  state: string;
  status: string;
  ports: string;
  running: boolean;
  protected: boolean;
  cpuPercent: number | null;
  memoryUsedBytes: number | null;
  memoryLimitBytes: number | null;
  pids: number | null;
  panelPort: number | null;
}

export interface PortainerHint {
  detected: boolean;
  container: string | null;
  running: boolean;
  panelPort: number | null;
  panelScheme: 'http' | 'https' | null;
}

export interface DockerInventory {
  availability: 'ready' | 'unavailable';
  dockerInstalled: boolean;
  containers: DockerContainer[];
  truncated: boolean;
  portainer: PortainerHint;
  error: string | null;
  note: string | null;
  collectedAtUnixMs: number;
}

export interface HomarrWidgetCandidate {
  name: string;
  url: string;
  icon: string | null;
}

export interface HomarrCatalog {
  availability: 'ready' | 'not_found' | 'unsupported_format';
  container: string | null;
  widgets: HomarrWidgetCandidate[];
  note: string | null;
  collectedAtUnixMs: number;
}

const NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/u;
const actions = ['start', 'stop', 'restart'] as const;

function bool(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function optionalText(record: Record<string, unknown>, key: string, maximum: number): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > maximum || Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError(`Docker inventory returned an invalid ${key} value.`);
  }
  return value;
}

function optionalNumber(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    throw new ApiError(`Docker inventory returned an invalid ${key} value.`);
  }
  return value;
}

function parseContainer(value: unknown): DockerContainer {
  const item = expectRecord(value, 'docker container');
  const name = expectString(item, 'name', 'docker container');
  if (!NAME.test(name)) throw new ApiError('Docker inventory returned an invalid container name.');
  return {
    name,
    image: expectString(item, 'image', 'docker container'),
    state: expectString(item, 'state', 'docker container'),
    status: expectString(item, 'status', 'docker container'),
    ports: expectString(item, 'ports', 'docker container'),
    running: bool(item, 'running', 'docker container'),
    protected: bool(item, 'protected', 'docker container'),
    cpuPercent: optionalNumber(item, 'cpu_percent'),
    memoryUsedBytes: optionalNumber(item, 'memory_used_bytes'),
    memoryLimitBytes: optionalNumber(item, 'memory_limit_bytes'),
    pids: optionalNumber(item, 'pids'),
    panelPort: optionalNumber(item, 'panel_port'),
  };
}

export function parseDockerInventory(value: unknown): DockerInventory {
  const root = expectRecord(value, 'docker inventory');
  if (expectNumber(root, 'schema_version', 'docker inventory', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Docker inventory returned an unsupported schema.');
  }
  const availability = expectString(root, 'availability', 'docker inventory');
  if (availability !== 'ready' && availability !== 'unavailable') {
    throw new ApiError('Docker inventory returned an invalid availability value.');
  }
  const portainer = expectRecord(root.portainer, 'portainer hint');
  return {
    availability,
    dockerInstalled: bool(root, 'docker_installed', 'docker inventory'),
    containers: expectArray(root, 'containers', 'docker inventory', 64).map(parseContainer),
    truncated: bool(root, 'truncated', 'docker inventory'),
    portainer: {
      detected: bool(portainer, 'detected', 'portainer hint'),
      container: optionalText(portainer, 'container', 128),
      running: portainer.running === undefined ? false : bool(portainer, 'running', 'portainer hint'),
      panelPort: optionalNumber(portainer, 'panel_port'),
      panelScheme: portainer.panel_scheme === 'https' || portainer.panel_scheme === 'http'
        ? portainer.panel_scheme
        : null,
    },
    error: optionalText(root, 'error', 500),
    note: optionalText(root, 'note', 500),
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'docker inventory', { integer: true, minimum: 0 }),
  };
}

export function parseHomarrCatalog(value: unknown): HomarrCatalog {
  const root = expectRecord(value, 'Homarr catalog');
  if (expectNumber(root, 'schema_version', 'Homarr catalog', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Homarr catalog returned an unsupported schema.');
  }
  const availability = expectString(root, 'availability', 'Homarr catalog');
  if (availability !== 'ready' && availability !== 'not_found' && availability !== 'unsupported_format') {
    throw new ApiError('Homarr catalog returned an invalid availability value.');
  }
  const widgets = expectArray(root, 'widgets', 'Homarr catalog', 64).map((entry) => {
    const item = expectRecord(entry, 'Homarr widget');
    const name = expectString(item, 'name', 'Homarr widget');
    const url = expectString(item, 'url', 'Homarr widget');
    if (name.trim().length === 0 || (!url.startsWith('http://') && !url.startsWith('https://'))) {
      throw new ApiError('Homarr catalog returned an invalid shortcut.');
    }
    return { name, url, icon: optionalText(item, 'icon', 2_048) };
  });
  const container = root.container === null || root.container === undefined
    ? null
    : expectString(root, 'container', 'Homarr catalog');
  return {
    availability,
    container,
    widgets,
    note: optionalText(root, 'note', 500),
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'Homarr catalog', { integer: true, minimum: 0 }),
  };
}

export function getDockerInventory(csrfToken: string, signal?: AbortSignal): Promise<DockerInventory> {
  return requestJson('/api/v1/docker/inventory', parseDockerInventory, { csrfToken, signal, timeoutMs: 25_000 });
}

export function runDockerContainerAction(
  name: string,
  action: DockerContainerAction,
  confirmation: string,
  csrfToken: string,
): Promise<Record<string, unknown>> {
  if (!NAME.test(name) || !actions.includes(action) || confirmation !== name) {
    return Promise.reject(new ApiError('That Docker action is not valid.'));
  }
  return requestJson('/api/v1/docker/actions', (value) => expectRecord(value, 'docker action'), {
    method: 'POST',
    csrfToken,
    body: { name, action, confirmation },
  });
}

export function getHomarrCatalog(csrfToken: string, signal?: AbortSignal): Promise<HomarrCatalog> {
  return requestJson('/api/v1/docker/homarr', parseHomarrCatalog, { csrfToken, signal, timeoutMs: 20_000 });
}

export function portainerHref(hostname: string, panelPort: number | null, panelScheme: 'http' | 'https' | null): string | null {
  if (panelPort === null) return null;
  const scheme = panelScheme ?? (panelPort === 9443 ? 'https' : 'http');
  return `${scheme}://${hostname}:${panelPort}/`;
}
