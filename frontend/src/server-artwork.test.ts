import { afterEach, describe, expect, it, vi } from 'vitest';
import { prepareServerIcon } from './server-artwork';

afterEach(() => vi.unstubAllGlobals());

describe('server artwork preparation', () => {
  it('center-crops wide photos to a bounded square before upload', async () => {
    const drawImage = vi.fn();
    const close = vi.fn();
    const canvas = {
      width: 0,
      height: 0,
      getContext: vi.fn(() => ({ drawImage, fillRect: vi.fn(), fillStyle: '' })),
      toBlob: vi.fn((callback: (blob: Blob | null) => void) => callback(new Blob(['icon'], { type: 'image/png' }))),
    };
    vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 1_024, height: 640, close })));
    vi.stubGlobal('document', { createElement: vi.fn(() => canvas) });
    vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:prepared-icon') });
    vi.stubGlobal('btoa', () => 'aWNvbg==');

    const result = await prepareServerIcon({ type: 'image/png', size: 8_192 } as File);

    expect(result).toMatchObject({ width: 512, height: 512, previewUrl: 'blob:prepared-icon' });
    expect(canvas).toMatchObject({ width: 512, height: 512 });
    expect(drawImage).toHaveBeenCalledWith(expect.anything(), 192, 0, 640, 640, 0, 0, 512, 512);
    expect(close).toHaveBeenCalledOnce();
  });
});
