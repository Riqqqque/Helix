export const DASHBOARD_ICONS_PNG =
  'https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png';

const GENERIC_SLUGS = new Set([
  'avatar',
  'blank',
  'default',
  'file',
  'icon',
  'image',
  'img',
  'logo',
  'media',
  'placeholder',
  'thumbnail',
  'upload',
]);

const NAME_ALIASES: ReadonlyArray<readonly [RegExp, string]> = [
  [/\bhome\s*assistant\b|\bhass\b|^ha$/iu, 'home-assistant'],
  [/\bpi[\s-]?hole\b/u, 'pi-hole'],
  [/\badguard(\s*home)?\b/u, 'adguard-home'],
  [/\bqbittorrent\b|\bqbit\b/u, 'qbittorrent'],
  [/\bnginx\s*proxy\s*manager\b/u, 'nginx-proxy-manager'],
  [/\buptime\s*kuma\b/u, 'uptime-kuma'],
  [/\bpaperless(\s*ngx)?\b/u, 'paperless-ngx'],
  [/\bnode[\s-]?red\b/u, 'nodered'],
  [/\bbitwarden\s*rs\b|\bvaultwarden\b/u, 'vaultwarden'],
  [/\boverseerr\b/u, 'overseerr'],
  [/\bjellyseerr\b/u, 'jellyseerr'],
  [/\btautulli\b/u, 'tautulli'],
  [/\bprowlarr\b/u, 'prowlarr'],
  [/\bbazarr\b/u, 'bazarr'],
  [/\blidarr\b/u, 'lidarr'],
  [/\breadarr\b/u, 'readarr'],
  [/\bradarr\b/u, 'radarr'],
  [/\bsonarr\b/u, 'sonarr'],
  [/\bwhisparr\b/u, 'whisparr'],
  [/\bsabnzbd\b/u, 'sabnzbd'],
  [/\bnzbget\b/u, 'nzbget'],
  [/\btransmission\b/u, 'transmission'],
  [/\bdeluge\b/u, 'deluge'],
  [/\bflood\b/u, 'flood'],
  [/\bjellyfin\b/u, 'jellyfin'],
  [/\bemby\b/u, 'emby'],
  [/\bplex\b/u, 'plex'],
  [/\bnavidrome\b/u, 'navidrome'],
  [/\bimmich\b/u, 'immich'],
  [/\bphotoprism\b/u, 'photoprism'],
  [/\blychee\b/u, 'lychee'],
  [/\bnextcloud\b/u, 'nextcloud'],
  [/\bsyncthing\b/u, 'syncthing'],
  [/\bportainer\b/u, 'portainer'],
  [/\byacht\b/u, 'yacht'],
  [/\btraefik\b/u, 'traefik'],
  [/\bcaddy\b/u, 'caddy'],
  [/\bnginx\b/u, 'nginx'],
  [/\bgrafana\b/u, 'grafana'],
  [/\bprometheus\b/u, 'prometheus'],
  [/\binfluxdb\b/u, 'influxdb'],
  [/\bnetdata\b/u, 'netdata'],
  [/\bglances\b/u, 'glances'],
  [/\bcockpit\b/u, 'cockpit'],
  [/\bproxmox\b/u, 'proxmox'],
  [/\bunraid\b/u, 'unraid'],
  [/\btruenas\b/u, 'truenas'],
  [/\bopenmediavault\b|\bomv\b/u, 'openmediavault'],
  [/\bgitea\b/u, 'gitea'],
  [/\bforgejo\b/u, 'forgejo'],
  [/\bgitlab\b/u, 'gitlab'],
  [/\bauthentik\b/u, 'authentik'],
  [/\bkeycloak\b/u, 'keycloak'],
  [/\bauthelia\b/u, 'authelia'],
  [/\btailscale\b/u, 'tailscale'],
  [/\bwireguard\b/u, 'wireguard'],
  [/\bcloudflare\b/u, 'cloudflare'],
  [/\bhomarr\b/u, 'homarr'],
  [/\bdashy\b/u, 'dashy'],
  [/\bhomepage\b/u, 'homepage'],
  [/\bflaresolverr\b/u, 'flaresolverr'],
  [/\bunifi\b/u, 'unifi'],
  [/\bomada\b/u, 'omada'],
  [/\bopnsense\b/u, 'opnsense'],
  [/\bpfsense\b/u, 'pfsense'],
  [/\bspotify\b/u, 'spotify'],
  [/\bdiscord\b/u, 'discord'],
  [/\bsteam\b/u, 'steam'],
  [/\bminecraft\b/u, 'minecraft'],
  [/\bpterodactyl\b/u, 'pterodactyl'],
  [/\bcrafty\b/u, 'crafty-controller'],
  [/\bamp\b|\bcube[\s-]?coders\b/u, 'cubecoders-amp'],
  [/\bvalheim\b/u, 'valheim'],
  [/\bterraria\b/u, 'terraria'],
  [/\bpalworld\b/u, 'palworld'],
  [/\bollama\b/u, 'ollama'],
  [/\bopen-?webui\b/u, 'open-webui'],
  [/\bkavita\b/u, 'kavita'],
  [/\bkomga\b/u, 'komga'],
  [/\baudiobookshelf\b/u, 'audiobookshelf'],
  [/\bcalibre(\s*web)?\b/u, 'calibre-web'],
  [/\bfilebrowser\b/u, 'filebrowser'],
  [/\bduplicati\b/u, 'duplicati'],
  [/\brestic\b/u, 'restic'],
  [/\bwatchtower\b/u, 'watchtower'],
  [/\bdozzle\b/u, 'dozzle'],
  [/\bchangedetection\b/u, 'changedetection'],
  [/\bmealie\b/u, 'mealie'],
  [/\btandoor\b/u, 'tandoor'],
  [/\bhomebox\b/u, 'homebox'],
  [/\bwikijs\b|\bwiki\.js\b/u, 'wikijs'],
  [/\bbookstack\b/u, 'bookstack'],
  [/\boutline\b/u, 'outline'],
  [/\bminio\b/u, 'minio'],
  [/\bseafile\b/u, 'seafile'],
  [/\besphome\b/u, 'esphome'],
  [/\bzigbee2mqtt\b/u, 'zigbee2mqtt'],
  [/\bmosquitto\b/u, 'mosquitto'],
  [/\bfrigate\b/u, 'frigate'],
  [/\bscrypted\b/u, 'scrypted'],
  [/\bhomebridge\b/u, 'homebridge'],
  [/\bstationeers\b/u, 'stationeers'],
  [/\bvrising\b|\bv rising\b/u, 'v-rising'],
];

const HOST_ALIASES: Readonly<Record<string, string>> = {
  amp: 'cubecoders-amp',
  bazarr: 'bazarr',
  deluge: 'deluge',
  emby: 'emby',
  grafana: 'grafana',
  ha: 'home-assistant',
  hass: 'home-assistant',
  homeassistant: 'home-assistant',
  homarr: 'homarr',
  immich: 'immich',
  jellyfin: 'jellyfin',
  jellyseerr: 'jellyseerr',
  lidarr: 'lidarr',
  navidrome: 'navidrome',
  nextcloud: 'nextcloud',
  overseerr: 'overseerr',
  pihole: 'pi-hole',
  plex: 'plex',
  portainer: 'portainer',
  prowlarr: 'prowlarr',
  qbittorrent: 'qbittorrent',
  radarr: 'radarr',
  readarr: 'readarr',
  sabnzbd: 'sabnzbd',
  sonarr: 'sonarr',
  syncthing: 'syncthing',
  tautulli: 'tautulli',
  transmission: 'transmission',
  unifi: 'unifi',
  uptime: 'uptime-kuma',
  vaultwarden: 'vaultwarden',
};

const PORT_SLUGS: Readonly<Record<number, string>> = {
  19999: 'netdata',
  2283: 'immich',
  2342: 'photoprism',
  3001: 'uptime-kuma',
  32400: 'plex',
  3579: 'kavita',
  4533: 'navidrome',
  5055: 'overseerr',
  5056: 'jellyseerr',
  6767: 'bazarr',
  7878: 'radarr',
  8096: 'jellyfin',
  8112: 'deluge',
  8123: 'home-assistant',
  8191: 'flaresolverr',
  8384: 'syncthing',
  8443: 'unifi',
  8686: 'lidarr',
  8787: 'readarr',
  8888: 'jupyter',
  8989: 'sonarr',
  9000: 'portainer',
  9091: 'transmission',
  9443: 'portainer',
  9696: 'prowlarr',
  11434: 'ollama',
};

export function dashboardIconUrl(slug: string): string {
  return `${DASHBOARD_ICONS_PNG}/${slug}.png`;
}

export function shortcutLetter(name: string): string {
  const match = name.trim().match(/\p{L}|\p{N}/u);
  return (match?.[0] ?? '?').toUpperCase();
}

export function shortcutIconUrl(input: {
  name: string;
  url: string;
  icon?: string | null;
}): string | null {
  const explicit = httpIcon(input.icon);
  if (explicit !== null) return explicit;
  const fromField = slugFromIconField(input.icon);
  if (fromField !== null) return dashboardIconUrl(fromField);
  const fromAlias = aliasFromName(input.name);
  if (fromAlias !== null) return dashboardIconUrl(fromAlias);
  const fromHost = slugFromHostname(input.url);
  if (fromHost !== null) return dashboardIconUrl(fromHost);
  const fromPort = slugFromPort(input.url);
  if (fromPort !== null) return dashboardIconUrl(fromPort);
  const fromName = sanitizeSlug(input.name);
  if (fromName !== null) return dashboardIconUrl(fromName);
  return null;
}

function httpIcon(value: string | null | undefined): string | null {
  const raw = value?.trim() ?? '';
  if (raw.length === 0 || raw.length > 2_048) return null;
  try {
    const parsed = new URL(raw);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:' ? parsed.href : null;
  } catch {
    return null;
  }
}

function slugFromIconField(value: string | null | undefined): string | null {
  const raw = value?.trim() ?? '';
  if (raw.length === 0 || raw.includes(':') || raw.startsWith('http://') || raw.startsWith('https://')) return null;
  return sanitizeSlug(fileStem(raw));
}

function aliasFromName(name: string): string | null {
  const lowered = name.trim().toLowerCase();
  if (lowered.length === 0) return null;
  for (const [pattern, slug] of NAME_ALIASES) {
    if (pattern.test(lowered)) return slug;
  }
  return null;
}

function slugFromHostname(url: string): string | null {
  try {
    const hostname = new URL(url).hostname.toLowerCase();
    if (/^\d{1,3}(?:\.\d{1,3}){3}$/u.test(hostname) || hostname.includes(':')) return null;
    const host = hostname.split('.')[0] ?? '';
    if (/^\d+$/u.test(host)) return null;
    if (host in HOST_ALIASES) return HOST_ALIASES[host] ?? null;
    return null;
  } catch {
    return null;
  }
}

function slugFromPort(url: string): string | null {
  try {
    const port = Number(new URL(url).port);
    if (!Number.isInteger(port) || port <= 0) return null;
    return PORT_SLUGS[port] ?? null;
  } catch {
    return null;
  }
}

function fileStem(value: string): string {
  const last = value.split(/[/\\]/u).at(-1) ?? value;
  const withoutQuery = last.split('?')[0] ?? last;
  const dot = withoutQuery.lastIndexOf('.');
  if (dot <= 0) return withoutQuery;
  const extension = withoutQuery.slice(dot + 1).toLowerCase();
  return ['png', 'svg', 'webp', 'jpg', 'jpeg', 'gif', 'ico'].includes(extension)
    ? withoutQuery.slice(0, dot)
    : withoutQuery;
}

function sanitizeSlug(value: string): string | null {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/['’]/gu, '')
    .replace(/[^a-z0-9]+/gu, '-')
    .replace(/^-+|-+$/gu, '');
  if (slug.length < 2 || slug.length > 80) return null;
  if (GENERIC_SLUGS.has(slug)) return null;
  if (/^[0-9a-f-]{32,}$/u.test(slug) && slug.replace(/-/gu, '').length >= 32) return null;
  return slug;
}
