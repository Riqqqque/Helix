import type { JSX } from 'preact';

export type GameMarkId =
  | 'minecraft'
  | 'vrising'
  | 'valheim'
  | 'terraria'
  | 'palworld'
  | 'satisfactory'
  | 'project_zomboid'
  | 'seven_days_to_die'
  | 'rust'
  | 'sons_of_the_forest'
  | 'factorio'
  | 'dont_starve_together'
  | 'vintage_story';

export function GameMark({
  game,
  size = 34,
}: {
  game: GameMarkId;
  size?: number;
}): JSX.Element {
  return (
    <svg
      class={`game-mark game-mark--${game}`}
      width={size}
      height={size}
      viewBox="0 0 32 32"
      aria-hidden="true"
    >
      {game === 'minecraft' ? (
        <MinecraftMark />
      ) : game === 'vrising' ? (
        <VRisingMark />
      ) : game === 'valheim' ? (
        <ValheimMark />
      ) : game === 'terraria' ? (
        <TerrariaMark />
      ) : game === 'satisfactory' ? (
        <SatisfactoryMark />
      ) : game === 'project_zomboid' ? (
        <ProjectZomboidMark />
      ) : game === 'seven_days_to_die' ? (
        <SevenDaysMark />
      ) : game === 'rust' ? (
        <RustMark />
      ) : game === 'sons_of_the_forest' ? (
        <SonsOfTheForestMark />
      ) : game === 'factorio' ? (
        <FactorioMark />
      ) : game === 'dont_starve_together' ? (
        <DontStarveMark />
      ) : game === 'vintage_story' ? (
        <VintageStoryMark />
      ) : (
        <PalworldMark />
      )}
    </svg>
  );
}

function MinecraftMark(): JSX.Element {
  return (
    <>
      <path fill="#3d6e1e" d="M16 3.2 28.4 10.2 16 17.2 3.6 10.2Z" />
      <path fill="#5ea32d" d="M16 4.4 26.8 10.2 16 16.1 5.2 10.2Z" />
      <path fill="#2f4f16" d="M8.4 8.6h3.2v3.2H8.4zm6.4-2.4h3.1v3.1h-3.1zm7.1 3.6h2.6v2.6h-2.6z" opacity="0.35" />
      <path fill="#8a5a2b" d="M3.6 10.2 16 17.2v11.2L3.6 21.4Z" />
      <path fill="#6e4522" d="M16 17.2 28.4 10.2v11.2L16 28.4Z" />
      <path fill="#c48a3a" d="M6.4 13.6h3.1v3.6H6.4zm4.8 4.2h2.4v4.8H11.2z" opacity="0.35" />
      <path fill="#4a2f16" d="M19.2 16.4h3.4v6.2h-3.4zm4.8 3.1h2.6v4.2h-2.6z" opacity="0.28" />
      <path fill="none" stroke="rgba(0,0,0,0.28)" stroke-width="0.7" d="M16 3.2 28.4 10.2 16 17.2 3.6 10.2Z" />
      <path fill="none" stroke="rgba(0,0,0,0.22)" stroke-width="0.7" d="M3.6 10.2 16 17.2v11.2M16 17.2 28.4 10.2v11.2" />
    </>
  );
}

function VRisingMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#1a1014" />
      <path fill="#7a1f28" d="M6 26h20v2H6z" />
      <path fill="#4a141a" d="M8 18h16l-2 8H10z" />
      <path fill="#c43b3a" d="M16 5 22 18h-4l-2-6-2 6h-4z" />
      <path fill="#f0d4a4" d="M15.2 11h1.6v3h-1.6z" />
      <circle cx="16" cy="8.2" r="1.5" fill="#f2c2b6" />
    </>
  );
}

function ValheimMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#1b2420" />
      <path fill="#c4a35a" d="M16 4 26 14v10l-10 4L6 24V14Z" />
      <path fill="#2c3b34" d="M16 7.2 23.2 14v8.2L16 25.4l-7.2-3.2V14Z" />
      <path fill="#d7c28a" d="M16 9.4 21.4 14v6.8L16 23.4l-5.4-2.6V14Z" />
      <path fill="#1b2420" d="M16 12.2 19.4 15v4.4L16 21.2l-3.4-1.8V15Z" />
    </>
  );
}

function TerrariaMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#12301c" />
      <path fill="#3d9a48" d="M6 22h20v4H6z" />
      <path fill="#2f7a38" d="M8 18h16v4H8z" />
      <path fill="#c48a3a" d="M14.4 6h3.2v14h-3.2z" />
      <path fill="#e0c070" d="M10 6h12l-2 4H12z" />
      <path fill="#8a5a2b" d="M15.2 20h1.6v6h-1.6z" />
    </>
  );
}

function PalworldMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#10241f" />
      <path fill="#2c6e5f" d="M4 22h24v5H4z" />
      <path fill="#58b98a" d="M9 15c0-5.5 3.1-9 7-9s7 3.5 7 9-3.1 7-7 7-7-1.5-7-7z" />
      <path fill="#10241f" d="M11 14.4c0-3.6 2.2-6 5-6s5 2.4 5 6-2.2 4.6-5 4.6-5-1-5-4.6z" />
      <circle cx="13.6" cy="12.6" r="1.5" fill="#9fe8c4" />
      <circle cx="18.4" cy="12.6" r="1.5" fill="#9fe8c4" />
      <path fill="#3d8f6f" d="M14.6 16.6h2.8l-1.4 2.2z" />
    </>
  );
}

function SatisfactoryMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#1d2330" />
      <path fill="#e8a33d" d="M7 20h18l-2 6H9z" />
      <path fill="#c9822e" d="M10 14h12l2 6H8z" />
      <path fill="#5c6b8a" d="M13 8h6l3 6H10z" />
      <path fill="#dfe7f5" d="M15.4 9.4h1.4v4h-1.4z" />
      <circle cx="11" cy="24" r="1.6" fill="#1d2330" />
      <circle cx="21" cy="24" r="1.6" fill="#1d2330" />
    </>
  );
}

function ProjectZomboidMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#14181c" />
      <circle cx="16" cy="13" r="7" fill="#9aa5a0" />
      <circle cx="13.6" cy="12" r="1.6" fill="#14181c" />
      <circle cx="18.4" cy="12" r="1.6" fill="#14181c" />
      <path fill="#14181c" d="M13.4 16.2h5.2l-.8 2h-3.6z" />
      <path fill="#4e5a52" d="M8 20h16v7H8z" />
      <path fill="#7d8b80" d="M8 20h16v2H8z" />
    </>
  );
}

function SevenDaysMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#26160f" />
      <path fill="#8c3b1f" d="M5 22h22v5H5z" />
      <path fill="#d97a2b" d="M16 4l7 8h-4.4L22 18H10l3.4-6H9z" />
      <path fill="#f0b34a" d="M16 8.4l3.6 4.4H16z" />
      <circle cx="24.4" cy="8" r="2" fill="#d97a2b" />
    </>
  );
}

function RustMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#241d16" />
      <circle cx="16" cy="16" r="11" fill="none" stroke="#b3562b" stroke-width="3" />
      <circle cx="16" cy="16" r="4.4" fill="#8c4322" />
      <path fill="#d98d54" d="M15 4h2v4h-2zM15 24h2v4h-2zM4 15h4v2H4zM24 15h4v2h-4zM8.2 7.4l2.8 1.4-1 2-2.8-1.4zM21.2 21.2l2.8 1.4-1 2-2.8-1.4zM22.8 7.4l1.4 2.8-2 1-1.4-2.8zM7.4 21.2l1.4 2.8-2 1-1.4-2.8z" />
    </>
  );
}

function SonsOfTheForestMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#0e1a12" />
      <path fill="#1f3a24" d="M4 18h24v9H4z" />
      <path fill="#2f5a35" d="M7 10l4 8H4zM21 8l5 10h-7z" />
      <path fill="#48804a" d="M13 6l6 12H8z" />
      <path fill="#7fae6f" d="M15.4 8.4l3.6 7.2h-3.6z" />
      <circle cx="16" cy="22.4" r="1.8" fill="#c9e2b0" />
    </>
  );
}

function FactorioMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#1c2026" />
      <circle cx="16" cy="16" r="9" fill="none" stroke="#c98a2e" stroke-width="2.4" />
      <circle cx="16" cy="16" r="3.4" fill="#e0a33c" />
      <path fill="#e0a33c" d="M10.4 6.6l2 2.4a11 11 0 0 0-3 3l-2.8-1.2a12.6 12.6 0 0 1 3.8-4.2zM21.6 6.6a12.6 12.6 0 0 1 3.8 4.2l-2.8 1.2a11 11 0 0 0-3-3zM6.6 20.4l2.8-1.2a11 11 0 0 0 3 3l-2 2.4a12.6 12.6 0 0 1-3.8-4.2zM25.4 20.4a12.6 12.6 0 0 1-3.8 4.2l-2-2.4a11 11 0 0 0 3-3z" />
    </>
  );
}

function DontStarveMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#191019" />
      <path fill="#e8dfc8" d="M16 5c-4.6 0-7.4 3.2-7.4 7.4 0 3 1.6 5.2 3.4 6.6L10.4 25h11.2L20 19c1.8-1.4 3.4-3.6 3.4-6.6C23.4 8.2 20.6 5 16 5z" />
      <circle cx="13.4" cy="11.6" r="1.7" fill="#191019" />
      <circle cx="18.6" cy="11.6" r="1.7" fill="#191019" />
      <path fill="#191019" d="M14.2 15h3.6l-.6 2.4h-2.4z" />
      <path fill="#8f2f3f" d="M11.6 25h8.8l1.2 2.4H10.4z" />
    </>
  );
}

function VintageStoryMark(): JSX.Element {
  return (
    <>
      <rect width="32" height="32" rx="6" fill="#1a1712" />
      <path fill="#c4a468" d="M8 6h16l-2 4H10z" />
      <path fill="#8f6f3f" d="M10 10h12v14l-6 3-6-3z" />
      <path fill="#1a1712" d="M12.4 13h7.2l-1 9-2.6 1.4-2.6-1.4z" />
      <path fill="#d8bd85" d="M14.6 15h2.8l-.6 6.4h-1.6z" />
    </>
  );
}

export function gameMarkForSoftware(software: string, kind?: string): GameMarkId | null {
  if (kind === 'palworld' || /palworld/iu.test(software)) return 'palworld';
  if (kind === 'vrising' || /v\s*rising/iu.test(software)) return 'vrising';
  if (kind === 'valheim' || /valheim/iu.test(software)) return 'valheim';
  if (kind === 'terraria' || /terraria|tmodloader/iu.test(software)) return 'terraria';
  if (kind === 'satisfactory' || /satisfactory/iu.test(software)) return 'satisfactory';
  if (kind === 'project_zomboid' || /project\s*zomboid|\bzomboid\b/iu.test(software)) return 'project_zomboid';
  if (kind === 'seven_days_to_die' || /7\s*days\s*to\s*die|7dtd|7d2d/iu.test(software)) return 'seven_days_to_die';
  if (kind === 'rust' || /\brust\b|rustdedicated/iu.test(software)) return 'rust';
  if (kind === 'sons_of_the_forest' || /sons\s*of\s*the\s*forest|sonsoftheforest/iu.test(software)) return 'sons_of_the_forest';
  if (kind === 'factorio' || /factorio/iu.test(software)) return 'factorio';
  if (kind === 'dont_starve_together' || /don.?t\s*starve|donotstarve/iu.test(software)) return 'dont_starve_together';
  if (kind === 'vintage_story' || /vintage\s*story|vintagestory/iu.test(software)) return 'vintage_story';
  if (kind === 'minecraft' || /minecraft|paper|purpur|folia|leaves|fabric|vanilla|spigot|bukkit|forge|quilt|pufferfish/iu.test(software)) {
    return 'minecraft';
  }
  if (/imported|amp/iu.test(software)) return null;
  return null;
}
