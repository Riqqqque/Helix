import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';
import type { MinecraftSoftware } from './control-api';

export type InstallableMinecraftSoftware = Extract<MinecraftSoftware, 'paper' | 'purpur' | 'folia' | 'leaves' | 'vanilla' | 'fabric' | 'neoforge' | 'forge' | 'quilt' | 'pumpkin' | 'pufferfish' | 'custom'>;
export type MinecraftCatalogStatus = 'ready' | 'validation_pending' | 'manual_build_required' | 'publisher_source_required' | 'topology_planned' | 'retired' | 'not_recommended' | 'platform_planned';

export interface MinecraftSoftwareCatalogEntry {
  id: string;
  name: string;
  kind: 'plugin_server' | 'vanilla_server' | 'mod_server' | 'custom_server' | 'proxy' | 'hybrid_server' | 'native_server' | 'bedrock_server';
  status: MinecraftCatalogStatus;
  installable: boolean;
  recommended: boolean;
  appeal: string;
  note: string;
}

export type ServerManagerReadiness =
  | {
      availability: 'ready';
      supportedMinecraftSoftware: InstallableMinecraftSoftware[];
      minecraftSoftwareCatalog: MinecraftSoftwareCatalogEntry[];
      features: string[];
      collectedAtUnixMs: number;
    }
  | {
      availability: 'unavailable';
      supportedMinecraftSoftware: [];
      minecraftSoftwareCatalog: [];
      blockers: Array<{ code: string; status: string }>;
      collectedAtUnixMs: number;
    };

const installableSoftware = new Set<InstallableMinecraftSoftware>(['paper', 'purpur', 'folia', 'leaves', 'vanilla', 'fabric', 'neoforge', 'forge', 'quilt', 'pumpkin', 'pufferfish', 'custom']);
const catalogStatuses = new Set<MinecraftCatalogStatus>(['ready', 'validation_pending', 'manual_build_required', 'publisher_source_required', 'topology_planned', 'retired', 'not_recommended', 'platform_planned']);
const catalogKinds = new Set<MinecraftSoftwareCatalogEntry['kind']>(['plugin_server', 'vanilla_server', 'mod_server', 'custom_server', 'proxy', 'hybrid_server', 'native_server', 'bedrock_server']);

function boolean(record: Record<string, unknown>, key: string, context: string): boolean {
  if (typeof record[key] !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return record[key];
}

function stringList(record: Record<string, unknown>, key: string, context: string, maximum: number): string[] {
  return expectArray(record, key, context, maximum).map((entry) => {
    if (typeof entry !== 'string' || entry.trim().length === 0) throw new ApiError(`${context} returned an invalid ${key} value.`);
    return entry;
  });
}

export function parseServerManagerReadiness(value: unknown): ServerManagerReadiness {
  const root = expectRecord(value, 'server manager readiness');
  if (expectNumber(root, 'schema_version', 'server manager readiness', { integer: true }) !== 1) {
    throw new ApiError('Server manager readiness returned an unsupported schema.');
  }
  const availability = expectString(root, 'availability', 'server manager readiness');
  const collectedAtUnixMs = expectNumber(root, 'collected_at_unix_ms', 'server manager readiness', { integer: true, minimum: 0 });
  if (availability === 'unavailable') {
    return {
      availability,
      supportedMinecraftSoftware: [],
      minecraftSoftwareCatalog: [],
      blockers: expectArray(root, 'blockers', 'server manager readiness', 16).map((value) => {
        const item = expectRecord(value, 'server manager blocker');
        return { code: expectString(item, 'code', 'server manager blocker'), status: expectString(item, 'status', 'server manager blocker') };
      }),
      collectedAtUnixMs,
    };
  }
  if (availability !== 'ready') throw new ApiError('Server manager readiness returned an invalid availability.');
  const supported = stringList(root, 'supported_minecraft_software', 'server manager readiness', 24)
    .filter((entry): entry is InstallableMinecraftSoftware => installableSoftware.has(entry as InstallableMinecraftSoftware));
  const catalog = expectArray(root, 'minecraft_software_catalog', 'server manager readiness', 64).map((value) => {
    const item = expectRecord(value, 'Minecraft software catalog entry');
    const id = expectString(item, 'id', 'Minecraft software catalog entry');
    const kind = expectString(item, 'kind', 'Minecraft software catalog entry') as MinecraftSoftwareCatalogEntry['kind'];
    const status = expectString(item, 'status', 'Minecraft software catalog entry') as MinecraftCatalogStatus;
    // A newer broker can advertise choices this dashboard cannot create yet.
    if (!installableSoftware.has(id as InstallableMinecraftSoftware) && (!catalogKinds.has(kind) || !catalogStatuses.has(status))) return null;
    if (!catalogKinds.has(kind) || !catalogStatuses.has(status)) throw new ApiError('Minecraft software catalog returned an invalid classification.');
    const parsed: MinecraftSoftwareCatalogEntry = {
      id,
      name: expectString(item, 'name', 'Minecraft software catalog entry'),
      kind,
      status,
      installable: boolean(item, 'installable', 'Minecraft software catalog entry'),
      recommended: boolean(item, 'recommended', 'Minecraft software catalog entry'),
      appeal: expectString(item, 'appeal', 'Minecraft software catalog entry'),
      note: expectString(item, 'note', 'Minecraft software catalog entry'),
    };
    if (parsed.installable !== (parsed.status === 'ready')) throw new ApiError('Minecraft software catalog returned inconsistent availability.');
    return parsed;
  }).filter((entry): entry is MinecraftSoftwareCatalogEntry => entry !== null);
  const catalogById = new Map(catalog.map((entry) => [entry.id, entry]));
  if (new Set(supported).size !== supported.length || supported.some((id) => catalogById.get(id)?.installable !== true)) {
    throw new ApiError('Server manager readiness returned inconsistent installable software.');
  }
  return {
    availability,
    supportedMinecraftSoftware: supported,
    minecraftSoftwareCatalog: catalog,
    features: stringList(root, 'features', 'server manager readiness', 64),
    collectedAtUnixMs,
  };
}

export function getServerManagerReadiness(csrfToken: string, signal?: AbortSignal): Promise<ServerManagerReadiness> {
  return requestJson('/api/v1/servers/manager/readiness', parseServerManagerReadiness, { csrfToken, signal, timeoutMs: 25_000 });
}
