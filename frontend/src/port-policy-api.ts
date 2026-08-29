import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from './api';
import type { GamePortPolicy } from './control-api';

function parseGamePortPolicy(expectedGame: 'minecraft' | 'vrising' | 'valheim' | 'terraria') {
  return (value: unknown): GamePortPolicy => {
    const root = expectRecord(value, 'game port policy');
    if (expectNumber(root, 'schema_version', 'game port policy', { integer: true, minimum: 0 }) !== 1) {
      throw new ApiError('Unsupported game port policy schema.');
    }
    const policy = expectRecord(root.policy, 'game port policy details');
    if (expectString(policy, 'game', 'game port policy details') !== expectedGame) {
      throw new ApiError('Game port policy returned the wrong game.');
    }
    const ranges = expectArray(policy, 'ranges', 'game port policy details', 32).map((entry) => {
      const range = expectRecord(entry, 'game port range');
      return {
        start: expectNumber(range, 'start', 'game port range', { integer: true, minimum: 0 }),
        end: expectNumber(range, 'end', 'game port range', { integer: true, minimum: 0 }),
      };
    });
    const portList = (record: Record<string, unknown>, key: string, limit: number) =>
      expectArray(record, key, 'game port policy', limit).map((entry) => {
        if (typeof entry !== 'number' || !Number.isInteger(entry)) {
          throw new ApiError('Game port policy returned an invalid port.');
        }
        return entry;
      });
    const autoForward = policy.auto_forward_on_create;
    if (typeof autoForward !== 'boolean') throw new ApiError('Game port policy returned an invalid auto-forward flag.');
    const next = root.next_available_port;
    return {
      ranges,
      ports: portList(policy, 'ports', 256),
      autoForwardOnCreate: autoForward,
      capacity: expectNumber(root, 'capacity', 'game port policy', { integer: true, minimum: 0 }),
      assignedPorts: portList(root, 'assigned_ports', 4096),
      ampClaimedPorts: portList(root, 'amp_claimed_ports', 4096),
      availableCount: expectNumber(root, 'available_count', 'game port policy', { integer: true, minimum: 0 }),
      nextAvailablePort: next === null || next === undefined
        ? null
        : expectNumber(root, 'next_available_port', 'game port policy', { integer: true, minimum: 0 }),
    };
  };
}

export function getMinecraftPortPolicy(csrfToken: string, signal?: AbortSignal): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/minecraft', parseGamePortPolicy('minecraft'), { csrfToken, signal });
}

export function saveMinecraftPortPolicy(
  input: Pick<GamePortPolicy, 'ranges' | 'ports' | 'autoForwardOnCreate'>,
  csrfToken: string,
): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/minecraft', parseGamePortPolicy('minecraft'), {
    method: 'PUT',
    body: {
      game: 'minecraft',
      ranges: input.ranges,
      ports: input.ports,
      auto_forward_on_create: input.autoForwardOnCreate,
    },
    csrfToken,
  });
}

export function getVRisingPortPolicy(csrfToken: string, signal?: AbortSignal): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/vrising', parseGamePortPolicy('vrising'), { csrfToken, signal });
}

export function saveVRisingPortPolicy(
  input: Pick<GamePortPolicy, 'ranges' | 'ports' | 'autoForwardOnCreate'>,
  csrfToken: string,
): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/vrising', parseGamePortPolicy('vrising'), {
    method: 'PUT',
    body: {
      game: 'vrising',
      ranges: input.ranges,
      ports: input.ports,
      auto_forward_on_create: false,
    },
    csrfToken,
  });
}

export function getValheimPortPolicy(csrfToken: string, signal?: AbortSignal): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/valheim', parseGamePortPolicy('valheim'), { csrfToken, signal });
}

export function saveValheimPortPolicy(
  input: Pick<GamePortPolicy, 'ranges' | 'ports' | 'autoForwardOnCreate'>,
  csrfToken: string,
): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/valheim', parseGamePortPolicy('valheim'), {
    method: 'PUT',
    body: {
      game: 'valheim',
      ranges: input.ranges,
      ports: input.ports,
      auto_forward_on_create: false,
    },
    csrfToken,
  });
}

export function getTerrariaPortPolicy(csrfToken: string, signal?: AbortSignal): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/terraria', parseGamePortPolicy('terraria'), { csrfToken, signal });
}

export function saveTerrariaPortPolicy(
  input: Pick<GamePortPolicy, 'ranges' | 'ports' | 'autoForwardOnCreate'>,
  csrfToken: string,
): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/terraria', parseGamePortPolicy('terraria'), {
    method: 'PUT',
    body: {
      game: 'terraria',
      ranges: input.ranges,
      ports: input.ports,
      auto_forward_on_create: input.autoForwardOnCreate,
    },
    csrfToken,
  });
}
