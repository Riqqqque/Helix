export function serverIdFromHash(hash: string): string | null {
  const candidate = hash.startsWith('#') ? hash.slice(1) : hash;
  const match = /^(?:servers|games)\/(.+)$/u.exec(candidate);
  const encoded = match?.[1];
  if (encoded === undefined || encoded.length === 0) return null;
  try {
    return decodeURIComponent(encoded);
  } catch {
    return encoded;
  }
}

export function serverDetailHash(id: string): string {
  return `#servers/${encodeURIComponent(id)}`;
}
