import type { JSX } from 'preact';

export type GameMarkId = 'minecraft' | 'vrising';

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
      {game === 'minecraft' ? <MinecraftMark /> : <VRisingMark />}
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
      <circle cx="16" cy="13" r="8.2" fill="#7a1c22" />
      <circle cx="16" cy="13" r="6.1" fill="#c43b3a" />
      <circle cx="18.2" cy="11.1" r="2.2" fill="#f2c2b6" opacity="0.35" />
      <path fill="#140c10" d="M6.4 28.4 16 8.8 25.6 28.4h-3.4L16 14.8 9.8 28.4Z" />
      <path fill="#2a1418" d="M11.2 28.4h9.6v-3.2l-1.4-1.2h-6.8l-1.4 1.2z" />
      <path fill="#3d1c22" d="M12.4 22.6h7.2v2.6h-7.2zm1.2-3.8h4.8v2.4h-4.8z" />
      <path fill="#f0d4a4" d="M15.2 20.2h1.6v2h-1.6z" />
      <path fill="#8a1f28" d="M14.8 8.2 16 5.4 17.2 8.2 16 10Z" />
    </>
  );
}

export function gameMarkForSoftware(software: string, kind?: string): GameMarkId | null {
  if (kind === 'vrising' || /v\s*rising/iu.test(software)) return 'vrising';
  if (kind === 'minecraft' || /minecraft|paper|purpur|folia|leaves|fabric|vanilla|spigot|bukkit|forge/iu.test(software)) {
    return 'minecraft';
  }
  return null;
}
