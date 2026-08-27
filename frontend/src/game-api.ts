import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from './api';
import type {
  GameHostingAvailability,
  GameHostingBlocker,
  GameHostingBlockerStatus,
  GameHostingReadiness,
} from './game-types';

const MAX_GAME_FEATURES = 32;
const MAX_GAME_BLOCKERS = 16;

function expectCanonicalFeature(value: unknown, context: string): string {
  if (
    typeof value !== 'string' ||
    value.length > 96 ||
    !/^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/.test(value)
  ) {
    throw new ApiError(`${context} returned an invalid feature identifier.`);
  }
  return value;
}

export function parseGameHostingReadiness(value: unknown): GameHostingReadiness {
  const context = 'Game hosting readiness API';
  const record = expectRecord(value, context);
  const schemaVersion = expectNumber(record, 'schema_version', context, {
    integer: true,
    minimum: 1,
    maximum: 1,
  });
  const availability = expectString(record, 'availability', context);
  if (!['ready', 'degraded', 'unavailable'].includes(availability)) {
    throw new ApiError(`${context} returned an invalid availability value.`);
  }

  const availableFeatures = expectArray(
    record,
    'available_features',
    context,
    MAX_GAME_FEATURES,
  ).map((feature) => expectCanonicalFeature(feature, context));
  if (new Set(availableFeatures).size !== availableFeatures.length) {
    throw new ApiError(`${context} returned duplicate feature identifiers.`);
  }

  const blockers = expectArray(
    record,
    'blockers',
    context,
    MAX_GAME_BLOCKERS,
  ).map((value, index): GameHostingBlocker => {
    const blockerContext = `${context} blocker ${index + 1}`;
    const blocker = expectRecord(value, blockerContext);
    const code = expectCanonicalFeature(blocker.code, blockerContext);
    const status = expectString(blocker, 'status', blockerContext);
    if (!['required', 'in_progress', 'ready'].includes(status)) {
      throw new ApiError(`${blockerContext} returned an invalid status value.`);
    }
    return { code, status: status as GameHostingBlockerStatus };
  });
  if (new Set(blockers.map((blocker) => blocker.code)).size !== blockers.length) {
    throw new ApiError(`${context} returned duplicate blockers.`);
  }
  if (availability === 'ready' && blockers.some((blocker) => blocker.status !== 'ready')) {
    throw new ApiError(`${context} marked unresolved blockers as ready.`);
  }

  return {
    schemaVersion: schemaVersion as 1,
    availability: availability as GameHostingAvailability,
    availableFeatures,
    blockers,
    collectedAtUnixMs: expectNumber(record, 'collected_at_unix_ms', context, {
      integer: true,
      minimum: 0,
    }),
  };
}

export function getGameHostingReadiness(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<GameHostingReadiness> {
  return requestJson('/api/v1/games/readiness', parseGameHostingReadiness, {
    csrfToken,
    signal,
  });
}
