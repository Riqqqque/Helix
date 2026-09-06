const HIDDEN_IMPORTED_SERVERS_KEY = 'helix.servers.hidden-imports';
const FORGOTTEN_IMPORTED_SERVERS_KEY = 'helix.servers.forgotten-imports';
const AMP_IMPORT_ID = /^amp:[0-9a-f-]{8,128}$/iu;

function readImportedServerIds(key: string): string[] {
  try {
    const value = JSON.parse(globalThis.localStorage?.getItem(key) ?? '[]') as unknown;
    if (!Array.isArray(value)) return [];
    return [...new Set(value.filter((item): item is string => typeof item === 'string' && AMP_IMPORT_ID.test(item)))].slice(0, 512);
  } catch {
    return [];
  }
}

function saveImportedServerIds(key: string, value: readonly string[]): void {
  try {
    globalThis.localStorage?.setItem(key, JSON.stringify([...new Set(value)].slice(0, 512)));
  } catch {
    // Display preference only; an unavailable browser store must not affect AMP.
  }
}

export function readHiddenImportedServers(): string[] {
  return readImportedServerIds(HIDDEN_IMPORTED_SERVERS_KEY);
}

export function saveHiddenImportedServers(value: readonly string[]): void {
  saveImportedServerIds(HIDDEN_IMPORTED_SERVERS_KEY, value);
}

export function readForgottenImportedServers(): string[] {
  return readImportedServerIds(FORGOTTEN_IMPORTED_SERVERS_KEY);
}

export function saveForgottenImportedServers(value: readonly string[]): void {
  saveImportedServerIds(FORGOTTEN_IMPORTED_SERVERS_KEY, value);
}

export function forgetImportedServer(id: string, hidden: readonly string[], forgotten: readonly string[]): {
  hidden: string[];
  forgotten: string[];
} {
  const nextHidden = hidden.filter((item) => item !== id);
  const nextForgotten = [...new Set([...forgotten, id])].slice(0, 512);
  saveHiddenImportedServers(nextHidden);
  saveForgottenImportedServers(nextForgotten);
  return { hidden: nextHidden, forgotten: nextForgotten };
}

export function rememberImportedServer(id: string, hidden: readonly string[], forgotten: readonly string[]): {
  hidden: string[];
  forgotten: string[];
} {
  const nextHidden = hidden.filter((item) => item !== id);
  const nextForgotten = forgotten.filter((item) => item !== id);
  saveHiddenImportedServers(nextHidden);
  saveForgottenImportedServers(nextForgotten);
  return { hidden: nextHidden, forgotten: nextForgotten };
}
