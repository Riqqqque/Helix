import type { ManagedGameKind } from './control-api';

export interface ManagedGameInfo {
  id: ManagedGameKind;
  label: string;
  /** Chooser card subtitle. */
  blurb: string;
  /** One-line intro shown at the top of the create dialog. */
  intro: string;
  memory: { min: number; max: number; default: number };
  players: { min: number; max: number; default: number };
  defaultGamePort: number;
  /** Extra ports auto-allocated after the game port (offsets 1..n). */
  extraPorts: number;
  /** Whether the second slot can be entered manually as a query port. */
  manualQueryPort: boolean;
  queryPortLabel: string;
  /** Primary join protocol for detail-page diagnostics. */
  joinProtocol: 'udp' | 'tcp';
  /** Short protocol description used in firewall/port copy. */
  portNoun: string;
  /** Port-pool dialog placeholder examples. */
  poolRangeHint: string;
  poolPortsHint: string;
  listOnBrowser: boolean;
  serverPassword: boolean;
  adminPassword: boolean;
  clusterToken: boolean;
  caves: boolean;
  worldSeed: boolean;
  worldSize: boolean;
  worldName: boolean;
  wineNotice: boolean;
}

export const MANAGED_GAMES: ReadonlyArray<ManagedGameInfo> = [
  {
    id: 'satisfactory',
    label: 'Satisfactory',
    blurb: 'SteamCMD dedicated server. TCP/UDP game port plus a UDP LAN beacon port.',
    intro:
      'Helix installs the Satisfactory dedicated server from Steam in an isolated container. It needs the game port (TCP and UDP) and the LAN beacon port (UDP); Helix reserves a consecutive pair. Public Direct Connect needs those ports forwarded in your router.',
    memory: { min: 6_144, max: 32_768, default: 12_288 },
    players: { min: 1, max: 16, default: 4 },
    defaultGamePort: 7_777,
    extraPorts: 1,
    manualQueryPort: true,
    queryPortLabel: 'Beacon UDP',
    joinProtocol: 'udp',
    portNoun: 'the TCP+UDP game port and UDP beacon port',
    poolRangeHint: '7777-7811',
    poolPortsHint: '7777, 7779',
    listOnBrowser: false,
    serverPassword: false,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: false,
  },
  {
    id: 'project_zomboid',
    label: 'Project Zomboid',
    blurb: 'SteamCMD dedicated server with an admin account and Steam query port.',
    intro:
      'Helix installs the Project Zomboid dedicated server from Steam in an isolated container. It uses two consecutive UDP ports for players and Steam data. The admin password is required to claim the admin account in-game; leave it blank to have Helix generate one.',
    memory: { min: 4_096, max: 32_768, default: 8_192 },
    players: { min: 1, max: 128, default: 8 },
    defaultGamePort: 16_261,
    extraPorts: 1,
    manualQueryPort: true,
    queryPortLabel: 'Steam UDP',
    joinProtocol: 'udp',
    portNoun: 'UDP game ports',
    poolRangeHint: '16261-16295',
    poolPortsHint: '16261, 16264',
    listOnBrowser: true,
    serverPassword: true,
    adminPassword: true,
    clusterToken: false,
    caves: false,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: false,
  },
  {
    id: 'seven_days_to_die',
    label: '7 Days to Die',
    blurb: 'SteamCMD dedicated server. TCP+UDP game port plus two reserved UDP ports.',
    intro:
      'Helix installs the 7 Days to Die dedicated server from Steam in an isolated container. It reserves three consecutive ports starting at the game port (TCP+UDP, then two UDP). Public play needs those ports forwarded in your router.',
    memory: { min: 6_144, max: 49_152, default: 12_288 },
    players: { min: 1, max: 64, default: 8 },
    defaultGamePort: 26_900,
    extraPorts: 2,
    manualQueryPort: true,
    queryPortLabel: 'Query UDP',
    joinProtocol: 'tcp',
    portNoun: 'the TCP+UDP game port and two UDP ports',
    poolRangeHint: '26900-26965',
    poolPortsHint: '26900, 26903',
    listOnBrowser: true,
    serverPassword: true,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: true,
    worldSize: false,
    worldName: true,
    wineNotice: false,
  },
  {
    id: 'rust',
    label: 'Rust',
    blurb: 'SteamCMD dedicated server with UDP game/query ports and a TCP RCON port.',
    intro:
      'Helix installs the Rust dedicated server from Steam in an isolated container. It reserves three consecutive ports: UDP game, UDP query, and TCP RCON. Helix generates a private RCON password automatically.',
    memory: { min: 8_192, max: 65_536, default: 12_288 },
    players: { min: 1, max: 500, default: 50 },
    defaultGamePort: 28_015,
    extraPorts: 2,
    manualQueryPort: true,
    queryPortLabel: 'Query UDP',
    joinProtocol: 'udp',
    portNoun: 'UDP game and query ports plus the TCP RCON port',
    poolRangeHint: '28015-28090',
    poolPortsHint: '28015, 28018',
    listOnBrowser: false,
    serverPassword: false,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: true,
    worldSize: true,
    worldName: false,
    wineNotice: false,
  },
  {
    id: 'sons_of_the_forest',
    label: 'Sons of the Forest',
    blurb: 'Windows dedicated server under an isolated Wine runtime. Three UDP ports.',
    intro:
      'Helix installs the Windows-only Sons of the Forest dedicated server from Steam and runs it under an isolated Wine runtime in a container. It reserves three consecutive UDP ports (game, query, blob sync). Wine adds startup time and memory overhead.',
    memory: { min: 6_144, max: 32_768, default: 8_192 },
    players: { min: 1, max: 8, default: 8 },
    defaultGamePort: 8_766,
    extraPorts: 2,
    manualQueryPort: true,
    queryPortLabel: 'Query UDP',
    joinProtocol: 'udp',
    portNoun: 'UDP game, query, and sync ports',
    poolRangeHint: '8766-8840',
    poolPortsHint: '8766, 8769',
    listOnBrowser: false,
    serverPassword: true,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: true,
  },
  {
    id: 'factorio',
    label: 'Factorio',
    blurb: 'Official headless server — tiny download, single UDP port, no Steam needed.',
    intro:
      'Helix downloads the official Factorio headless server in an isolated container. It uses a single UDP port and starts fast. Public listing requires a Factorio account token, which can be added to the settings file later.',
    memory: { min: 1_024, max: 16_384, default: 4_096 },
    players: { min: 1, max: 255, default: 16 },
    defaultGamePort: 34_197,
    extraPorts: 0,
    manualQueryPort: false,
    queryPortLabel: '',
    joinProtocol: 'udp',
    portNoun: 'the UDP game port',
    poolRangeHint: '34197-34250',
    poolPortsHint: '34197, 34200',
    listOnBrowser: true,
    serverPassword: true,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: false,
  },
  {
    id: 'dont_starve_together',
    label: "Don't Starve Together",
    blurb: 'Klei dedicated server with an optional Caves shard on a second UDP port.',
    intro:
      'Helix installs the Don\'t Starve Together dedicated server from Steam in an isolated container. The overworld shard uses one UDP port; enabling the Caves shard reserves the next port too. A Klei cluster token is needed for public listing and persistent player accounts.',
    memory: { min: 1_024, max: 8_192, default: 2_048 },
    players: { min: 1, max: 64, default: 8 },
    defaultGamePort: 10_999,
    extraPorts: 0,
    manualQueryPort: false,
    queryPortLabel: '',
    joinProtocol: 'udp',
    portNoun: 'the UDP game port (plus a second for Caves)',
    poolRangeHint: '10999-11050',
    poolPortsHint: '10999, 11002',
    listOnBrowser: true,
    serverPassword: true,
    adminPassword: false,
    clusterToken: true,
    caves: true,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: false,
  },
  {
    id: 'vintage_story',
    label: 'Vintage Story',
    blurb: 'Official .NET dedicated server on a single TCP port.',
    intro:
      'Helix downloads the latest stable Vintage Story dedicated server and runs it on the .NET runtime in an isolated container. It uses a single TCP port for players.',
    memory: { min: 2_048, max: 32_768, default: 6_144 },
    players: { min: 1, max: 64, default: 16 },
    defaultGamePort: 42_420,
    extraPorts: 0,
    manualQueryPort: false,
    queryPortLabel: '',
    joinProtocol: 'tcp',
    portNoun: 'the TCP game port',
    poolRangeHint: '42420-42470',
    poolPortsHint: '42420, 42423',
    listOnBrowser: true,
    serverPassword: true,
    adminPassword: false,
    clusterToken: false,
    caves: false,
    worldSeed: false,
    worldSize: false,
    worldName: false,
    wineNotice: false,
  },
];

export function managedGameInfo(game: ManagedGameKind): ManagedGameInfo {
  const info = MANAGED_GAMES.find((entry) => entry.id === game);
  if (info === undefined) throw new Error(`Unknown managed game: ${game}`);
  return info;
}

export function managedGameLabel(game: string): string {
  return MANAGED_GAMES.find((entry) => entry.id === game)?.label ?? game;
}
