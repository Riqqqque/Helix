import { useEffect, useState } from 'preact/hooks';
import { ApiError } from './api';
import { serverIsLive, type ManagedServer } from './control-api';
import { GameMark, gameMarkForSoftware } from './game-marks';
import { Icon, type IconName } from './icons';
import { Dialog } from './modal';
import {
  clearServerIcon,
  setServerCustomIcon,
  setServerIconPreset,
  type ServerAppearance,
  type ServerIconPreset,
} from './server-appearance-api';

const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
const MAX_ICON_BYTES = 512 * 1024;
const MAX_OUTPUT_EDGE = 512;

const presetOptions: ReadonlyArray<{
  id: ServerIconPreset;
  label: string;
  description: string;
  icon: IconName;
}> = [
  { id: 'grass', label: 'Overworld', description: 'Earth and grass', icon: 'servers' },
  { id: 'portal', label: 'Portal', description: 'Violet energy', icon: 'network' },
  { id: 'crystal', label: 'Crystal', description: 'Cold cyan light', icon: 'storage' },
  { id: 'fortress', label: 'Fortress', description: 'Stone and iron', icon: 'host' },
  { id: 'ember', label: 'Ember', description: 'Warm red glow', icon: 'activity' },
  { id: 'ocean', label: 'Ocean', description: 'Deep blue water', icon: 'network' },
];

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not save the server icon.';
}

function appearancePreset(appearance: ServerAppearance): ServerIconPreset | null {
  return appearance.kind === 'preset' ? appearance.preset : null;
}

export function ServerArtwork({
  server,
  size = 'row',
}: {
  server: ManagedServer;
  size?: 'row' | 'detail';
}) {
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => setImageFailed(false), [server.appearance]);
  const live = serverIsLive(server.status);
  const appearance = server.appearance;
  const preset = appearancePreset(appearance);
  return (
    <span
      class={`server-artwork server-artwork--${size} server-artwork--${live ? 'online' : 'offline'}${preset === null ? '' : ` server-artwork--${preset}`}`}
      aria-hidden="true"
    >
      {appearance.kind === 'custom' && !imageFailed
        ? <img src={appearance.imageUrl} alt="" loading="lazy" decoding="async" onError={() => setImageFailed(true)} />
        : (() => {
            const game = appearance.kind === 'default' ? gameMarkForSoftware(server.software, server.kind) : null;
            if (game !== null) return <GameMark game={game} size={size === 'detail' ? 28 : 22} />;
            return <Icon name={presetOptions.find((option) => option.id === preset)?.icon ?? (live ? 'activity' : 'servers')} size={size === 'detail' ? 24 : 19} />;
          })()}
      <i />
    </span>
  );
}

interface PreparedServerIcon {
  contentType: 'image/png' | 'image/jpeg';
  imageBase64: string;
  previewUrl: string;
  bytes: number;
  width: number;
  height: number;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 32_768) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 32_768));
  }
  return btoa(binary);
}

function canvasBlob(
  canvas: HTMLCanvasElement,
  type: 'image/png' | 'image/jpeg',
  quality?: number,
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob === null) reject(new Error('The browser could not encode this photo.'));
      else resolve(blob);
    }, type, quality);
  });
}

export async function prepareServerIcon(file: File): Promise<PreparedServerIcon> {
  if (file.type !== 'image/png' && file.type !== 'image/jpeg') {
    throw new Error('Choose a PNG or JPEG image. SVG and animated files are not accepted.');
  }
  if (file.size === 0 || file.size > MAX_SOURCE_BYTES) {
    throw new Error('Choose an image smaller than 16 MiB. Helix will optimize it before upload.');
  }
  const bitmap = await createImageBitmap(file);
  try {
    if (bitmap.width < 32 || bitmap.height < 32) {
      throw new Error('Choose an image at least 32 × 32 pixels.');
    }
    if (bitmap.width > 16_384 || bitmap.height > 16_384) {
      throw new Error('This image has unusually large dimensions. Resize it below 16,384 pixels first.');
    }
    const sourceEdge = Math.min(bitmap.width, bitmap.height);
    const sourceX = (bitmap.width - sourceEdge) / 2;
    const sourceY = (bitmap.height - sourceEdge) / 2;
    const outputEdge = Math.min(MAX_OUTPUT_EDGE, sourceEdge);
    const width = outputEdge;
    const height = outputEdge;
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d', { alpha: file.type === 'image/png' });
    if (context === null) throw new Error('The browser could not prepare this image.');
    if (file.type === 'image/jpeg') {
      context.fillStyle = '#111510';
      context.fillRect(0, 0, width, height);
    }
    context.drawImage(bitmap, sourceX, sourceY, sourceEdge, sourceEdge, 0, 0, width, height);

    let contentType: 'image/png' | 'image/jpeg' = file.type;
    let blob = await canvasBlob(canvas, contentType, contentType === 'image/jpeg' ? 0.9 : undefined);
    if (blob.size > MAX_ICON_BYTES) {
      contentType = 'image/jpeg';
      for (const quality of [0.86, 0.76, 0.66, 0.56]) {
        blob = await canvasBlob(canvas, contentType, quality);
        if (blob.size <= MAX_ICON_BYTES) break;
      }
    }
    if (blob.size > MAX_ICON_BYTES) {
      throw new Error('This image could not be reduced below Helix’s 512 KiB icon limit.');
    }
    const bytes = new Uint8Array(await blob.arrayBuffer());
    return {
      contentType,
      imageBase64: bytesToBase64(bytes),
      previewUrl: URL.createObjectURL(blob),
      bytes: bytes.length,
      width,
      height,
    };
  } finally {
    bitmap.close();
  }
}

export function ServerIconDialog({
  server,
  csrfToken,
  onClose,
  onSaved,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  onClose: () => void;
  onSaved: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [selectedPreset, setSelectedPreset] = useState<ServerIconPreset>(appearancePreset(server.appearance) ?? 'grass');
  const [prepared, setPrepared] = useState<PreparedServerIcon | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => () => {
    if (prepared !== null) URL.revokeObjectURL(prepared.previewUrl);
  }, [prepared]);

  const save = async (operation: () => Promise<ServerAppearance>): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await operation();
      await onSaved();
      onClose();
    } catch (requestError) {
      if (requestError instanceof ApiError && requestError.status === 401) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };

  const chooseFile = async (file: File | undefined): Promise<void> => {
    if (file === undefined) return;
    setPreparing(true);
    setError(null);
    try {
      const next = await prepareServerIcon(file);
      setPrepared((current) => {
        if (current !== null) URL.revokeObjectURL(current.previewUrl);
        return next;
      });
    } catch (prepareError) {
      setError(describeError(prepareError));
    } finally {
      setPreparing(false);
    }
  };

  return (
    <Dialog title={`Server icon · ${server.name}`} onClose={onClose} wide flush>
      <div class="server-icon-editor">
        <section>
          <div class="server-icon-editor__head"><div><strong>Presets</strong><span>Fast, lightweight artwork stored as a theme choice.</span></div></div>
          <div class="server-icon-presets" role="radiogroup" aria-label="Server icon presets">
            {presetOptions.map((option) => <button class={`server-icon-preset server-artwork--${option.id}${selectedPreset === option.id ? ' is-selected' : ''}`} type="button" role="radio" aria-checked={selectedPreset === option.id} key={option.id} onClick={() => setSelectedPreset(option.id)}><span><Icon name={option.icon} size={20} /></span><strong>{option.label}</strong><small>{option.description}</small></button>)}
          </div>
          <button class="button button--primary" type="button" disabled={busy || preparing || (server.appearance.kind === 'preset' && server.appearance.preset === selectedPreset)} onClick={() => void save(() => setServerIconPreset(server.id, selectedPreset, server.appearance.revision, csrfToken))}>Use {presetOptions.find((option) => option.id === selectedPreset)?.label}</button>
        </section>
        <section class="server-icon-upload">
          <div class="server-icon-editor__head"><div><strong>Your photo</strong><span>PNG or JPEG. Helix center-crops a square and resizes it locally before upload.</span></div></div>
          <label class={`server-icon-drop${preparing ? ' is-busy' : ''}`}>
            {prepared === null ? <><Icon name="plus" size={25} /><strong>{preparing ? 'Optimizing photo…' : 'Choose a photo'}</strong><span>Up to 16 MiB source · stored icon at most 512 KiB</span></> : <><img src={prepared.previewUrl} alt="Prepared server icon preview" /><strong>{prepared.width} × {prepared.height}</strong><span>{Math.ceil(prepared.bytes / 1_024)} KiB · {prepared.contentType.replace('image/', '').toUpperCase()}</span></>}
            <input type="file" accept="image/png,image/jpeg,.png,.jpg,.jpeg" disabled={busy || preparing} onChange={(event) => void chooseFile(event.currentTarget.files?.[0])} />
          </label>
          <button class="button button--primary" type="button" disabled={busy || preparing || prepared === null} onClick={() => prepared !== null && void save(() => setServerCustomIcon(server.id, prepared.contentType, prepared.imageBase64, server.appearance.revision, csrfToken))}>Use this photo</button>
        </section>
      </div>
      {error !== null && <div class="inline-error" role="alert"><Icon name="warning" size={15} />{error}</div>}
      <div class="dialog-actions server-icon-dialog-actions">
        {server.appearance.kind !== 'default' && <button class="button button--danger-quiet" type="button" disabled={busy || preparing} onClick={() => void save(() => clearServerIcon(server.id, server.appearance.revision, csrfToken))}>Restore default</button>}
        <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button>
      </div>
    </Dialog>
  );
}
