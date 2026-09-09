import { ApiError, expectArray, expectBoolean, expectNumber, expectRecord, expectString, requestJson } from './api';

export interface ValheimSettings {
  world: string; password: string; public: boolean; crossplay: boolean;
  save_interval: number; backups: number; backup_short: number; backup_long: number;
  preset: string; modifiers: Record<string, string>; keys: string[];
}
export const defaultValheimSettings = (): ValheimSettings => ({ world: 'Dedicated', password: '', public: false, crossplay: false,
  save_interval: 1800, backups: 4, backup_short: 7200, backup_long: 43200, preset: '', modifiers: {}, keys: [] });
export interface ValheimMod { package: string; version: string; description: string; dependencies: string[]; url: string; enabled: boolean; deprecated: boolean }
export interface ValheimStatus { settings: ValheimSettings; expected_revision: string; mods: ValheimMod[]; running: boolean; runtime_current: boolean }

function text(value: unknown, context: string): string {
  if (typeof value !== 'string') throw new ApiError(`Invalid ${context} response`);
  return value;
}
export function parseValheimSettings(value: unknown): ValheimSettings {
  const r = expectRecord(value, 'Valheim settings');
  const modifiers = expectRecord(r.modifiers, 'world modifiers');
  return { world: expectString(r, 'world', 'Valheim'), password: text(r.password, 'password'),
    public: expectBoolean(r, 'public', 'Valheim'), crossplay: expectBoolean(r, 'crossplay', 'Valheim'),
    save_interval: expectNumber(r, 'save_interval', 'Valheim'), backups: expectNumber(r, 'backups', 'Valheim'),
    backup_short: expectNumber(r, 'backup_short', 'Valheim'), backup_long: expectNumber(r, 'backup_long', 'Valheim'),
    preset: text(r.preset, 'preset'), modifiers: Object.fromEntries(Object.entries(modifiers).map(([key, value]) => [key, text(value, 'modifier')])),
    keys: expectArray(r, 'keys', 'Valheim', 4).map(value => text(value, 'world key')) };
}
export function parseValheimMod(value: unknown): ValheimMod {
  const r = expectRecord(value, 'Thunderstore package');
  const name = expectString(r, 'package', 'Thunderstore');
  if (!/^[a-z0-9_]+-[a-z0-9_]+$/i.test(name)) throw new ApiError('Invalid Thunderstore package name');
  return { package: name, version: expectString(r, 'version', 'Thunderstore'), description: text(r.description, 'description'),
    dependencies: expectArray(r, 'dependencies', 'Thunderstore', 64).map(value => text(value, 'dependency')),
    url: `https://thunderstore.io/c/valheim/p/${name.replace('-', '/')}/`, enabled: expectBoolean(r, 'enabled', 'Thunderstore'),
    deprecated: expectBoolean(r, 'deprecated', 'Thunderstore') };
}
export function parseValheimStatus(value: unknown): ValheimStatus {
  const r = expectRecord(value, 'Valheim manager');
  return { settings: parseValheimSettings(r.settings), expected_revision: expectString(r, 'expected_revision', 'Valheim'),
    mods: expectArray(r, 'mods', 'Valheim', 128).map(parseValheimMod), running: expectBoolean(r, 'running', 'Valheim'),
    runtime_current: expectBoolean(r, 'runtime_current', 'Valheim') };
}
export function valheimRequest<T>(id: string, csrfToken: string, body: object, parse: (value: unknown) => T, signal?: AbortSignal): Promise<T> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/valheim`, parse, { method: 'POST', body, csrfToken, signal, timeoutMs: 60_000 });
}
