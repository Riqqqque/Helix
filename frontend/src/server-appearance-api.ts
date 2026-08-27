import { ApiError, expectNumber, expectRecord, expectString, requestJson } from './api';

export type ServerIconPreset = 'grass' | 'portal' | 'crystal' | 'fortress' | 'ember' | 'ocean';

export type ServerAppearance =
  | { kind: 'default'; revision: 0 }
  | { kind: 'preset'; revision: number; preset: ServerIconPreset; updatedAtUnixMs: number }
  | {
      kind: 'custom';
      revision: number;
      contentType: 'image/png' | 'image/jpeg';
      width: number;
      height: number;
      updatedAtUnixMs: number;
      imageUrl: string;
    };

const presets = new Set<ServerIconPreset>([
  'grass', 'portal', 'crystal', 'fortress', 'ember', 'ocean',
]);

export function parseServerAppearance(value: unknown): ServerAppearance {
  const context = 'Server appearance';
  const record = expectRecord(value, context);
  const kind = expectString(record, 'kind', context);
  const revision = expectNumber(record, 'revision', context, {
    integer: true,
    minimum: 0,
    maximum: Number.MAX_SAFE_INTEGER,
  });
  if (kind === 'default') {
    if (revision !== 0) throw new ApiError(`${context} returned an invalid default revision.`);
    return { kind, revision };
  }
  if (revision < 1) throw new ApiError(`${context} returned an invalid saved revision.`);
  const updatedAtUnixMs = expectNumber(record, 'updated_at_unix_ms', context, {
    integer: true,
    minimum: 0,
    maximum: Number.MAX_SAFE_INTEGER,
  });
  if (kind === 'preset') {
    const preset = expectString(record, 'preset', context) as ServerIconPreset;
    if (!presets.has(preset)) throw new ApiError(`${context} returned an unknown preset.`);
    return { kind, revision, preset, updatedAtUnixMs };
  }
  if (kind !== 'custom') throw new ApiError(`${context} returned an invalid kind.`);
  const contentType = expectString(record, 'content_type', context);
  if (contentType !== 'image/png' && contentType !== 'image/jpeg') {
    throw new ApiError(`${context} returned an invalid content type.`);
  }
  const width = expectNumber(record, 'width', context, { integer: true, minimum: 32, maximum: 2_048 });
  const height = expectNumber(record, 'height', context, { integer: true, minimum: 32, maximum: 2_048 });
  const imageUrl = expectString(record, 'image_url', context);
  if (!/^\/api\/v1\/servers\/[A-Za-z0-9:._-]{7,165}\/appearance\/image\?revision=[1-9]\d*$/u.test(imageUrl)) {
    throw new ApiError(`${context} returned an unsafe image URL.`);
  }
  const urlRevision = Number(new URL(imageUrl, 'http://helix.invalid').searchParams.get('revision'));
  if (urlRevision !== revision) throw new ApiError(`${context} returned a mismatched image revision.`);
  return { kind, revision, contentType, width, height, updatedAtUnixMs, imageUrl };
}

export function setServerIconPreset(
  instanceId: string,
  preset: ServerIconPreset,
  expectedRevision: number,
  csrfToken: string,
): Promise<ServerAppearance> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(instanceId)}/appearance`,
    parseServerAppearance,
    {
      method: 'PUT',
      csrfToken,
      body: { kind: 'preset', expected_revision: expectedRevision, preset },
      timeoutMs: 15_000,
    },
  );
}

export function setServerCustomIcon(
  instanceId: string,
  contentType: 'image/png' | 'image/jpeg',
  imageBase64: string,
  expectedRevision: number,
  csrfToken: string,
): Promise<ServerAppearance> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(instanceId)}/appearance`,
    parseServerAppearance,
    {
      method: 'PUT',
      csrfToken,
      body: {
        kind: 'custom',
        expected_revision: expectedRevision,
        content_type: contentType,
        image_base64: imageBase64,
      },
      timeoutMs: 20_000,
    },
  );
}

export function clearServerIcon(
  instanceId: string,
  expectedRevision: number,
  csrfToken: string,
): Promise<ServerAppearance> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(instanceId)}/appearance`,
    parseServerAppearance,
    {
      method: 'DELETE',
      csrfToken,
      body: { expected_revision: expectedRevision },
    },
  );
}
