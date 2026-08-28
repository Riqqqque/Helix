import type { JSX } from 'preact';

export type GameMarkId = 'minecraft' | 'vrising' | 'valheim' | 'terraria';

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
      {game === 'minecraft' ? <MinecraftMark /> : game === 'vrising' ? <VRisingMark /> : game === 'valheim' ? <ValheimMark /> : <TerrariaMark />}
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

export function gameMarkForSoftware(software: string, kind?: string): GameMarkId | null {
  if (kind === 'vrising' || /v\s*rising/iu.test(software)) return 'vrising';
  if (kind === 'valheim' || /valheim/iu.test(software)) return 'valheim';
  if (kind === 'terraria' || /terraria|tmodloader/iu.test(software)) return 'terraria';
  if (kind === 'minecraft' || /minecraft|paper|purpur|folia|leaves|fabric|vanilla|spigot|bukkit|forge|quilt|pufferfish/iu.test(software)) {
    return 'minecraft';
  }
  if (/imported|amp/iu.test(software)) return null;
  return null;
}
