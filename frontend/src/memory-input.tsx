import { useEffect, useState } from 'preact/hooks';

/**
 * Reads memory the way people type it: `6144`, `6G`, `6 GB`, `6.5GiB`, `512m`,
 * `512 MB`. A bare number below 256 means GiB; larger bare numbers mean MiB.
 * Returns whole MiB, or null when the text is not a size.
 */
export function parseMemoryInput(text: string): number | null {
  const match = /^\s*(\d+(?:[.,]\d+)?)\s*(t|tb|tib|g|gb|gib|m|mb|mib)?\s*$/iu.exec(text);
  if (match === null) return null;
  const amount = Number(match[1]?.replace(',', '.'));
  if (!Number.isFinite(amount) || amount <= 0) return null;
  const unit = (match[2] ?? '').toLowerCase();
  const factor = unit.startsWith('t') ? 1024 * 1024 : unit.startsWith('g') ? 1024 : unit.startsWith('m') ? 1 : amount < 256 ? 1024 : 1;
  return Math.round(amount * factor);
}

/** `6 GiB` for whole GiB, otherwise `6656 MiB`. */
export function formatMemoryInput(memoryMb: number): string {
  return memoryMb % 1024 === 0 ? `${memoryMb / 1024} GiB` : `${memoryMb} MiB`;
}

const PRESETS = [512, 1_024, 2_048, 3_072, 4_096, 6_144, 8_192, 10_240, 12_288, 16_384, 20_480, 24_576, 32_768, 49_152, 65_536];

/**
 * A memory field that accepts typed sizes and offers common presets. It reports
 * a value only when the text parses and fits the game's limits.
 */
export function MemoryInput({
  value,
  min,
  max,
  disabled,
  title,
  id,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  disabled?: boolean | undefined;
  title?: string | undefined;
  id?: string | undefined;
  onChange: (memoryMb: number) => void;
}) {
  const [draft, setDraft] = useState(() => formatMemoryInput(value));
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused) setDraft(formatMemoryInput(value));
  }, [focused, value]);
  const parsed = parseMemoryInput(draft);
  const problem = draft.trim() === ''
    ? 'Enter a size such as 6G or 6144'
    : parsed === null
      ? 'Use a size such as 6G, 6.5 GB, or 6144 MB'
      : parsed < min || parsed > max
        ? `Choose between ${formatMemoryInput(min)} and ${formatMemoryInput(max)}`
        : null;
  const listId = `${id ?? 'memory'}-presets`;
  return (
    <span class="memory-input">
      <input
        id={id}
        type="text"
        inputMode="decimal"
        list={listId}
        value={draft}
        disabled={disabled}
        title={title}
        aria-invalid={problem !== null}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          if (problem === null && parsed !== null) setDraft(formatMemoryInput(parsed));
        }}
        onInput={(event) => {
          const next = event.currentTarget.value;
          setDraft(next);
          const memory = parseMemoryInput(next);
          if (memory !== null && memory >= min && memory <= max) onChange(memory);
        }}
      />
      <datalist id={listId}>
        {PRESETS.filter((preset) => preset >= min && preset <= max).map((preset) => (
          <option key={preset} value={formatMemoryInput(preset)} />
        ))}
      </datalist>
      <small class={problem === null ? 'memory-input__hint' : 'memory-input__hint memory-input__hint--error'}>
        {problem ?? (parsed !== null && parsed % 1024 !== 0 ? `${(parsed / 1024).toFixed(2)} GiB` : `${parsed ?? value} MiB`)}
      </small>
    </span>
  );
}
