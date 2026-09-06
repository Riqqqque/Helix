import { describe, expect, it } from 'vitest';
import render from 'preact-render-to-string';
import { loadStorageAnalyzer, StorageAnalyzerRoute } from './storage-analyzer-route';

describe('Storage analyzer loader', () => {
  it('keeps the loading modal programmatically focusable', () => {
    const markup = render(<StorageAnalyzerRoute path="/srv" csrfToken="csrf" onClose={() => undefined} onNavigate={() => undefined} onSessionExpired={() => undefined} />);
    expect(markup).toContain('aria-modal="true"');
    expect(markup).toContain('tabindex="-1"');
  });

  it('shares one lazy import and resolves the real analyzer', async () => {
    const first = loadStorageAnalyzer();
    const second = loadStorageAnalyzer();

    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
