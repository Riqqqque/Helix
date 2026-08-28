import { describe, expect, it } from 'vitest';
import { dashboardIconUrl, shortcutIconUrl, shortcutLetter } from './shortcut-icons';

describe('shortcut icons', () => {
  it('keeps an explicit http(s) icon', () => {
    expect(shortcutIconUrl({
      name: 'Plex',
      url: 'http://192.168.1.10:32400/web',
      icon: 'https://example.test/plex.png',
    })).toBe('https://example.test/plex.png');
  });

  it('maps Homarr icon names onto the dashboard-icons CDN', () => {
    expect(shortcutIconUrl({
      name: 'Radarr',
      url: 'http://192.168.1.10:7878',
      icon: 'radarr',
    })).toBe(dashboardIconUrl('radarr'));
    expect(shortcutIconUrl({
      name: 'Sonarr',
      url: 'http://192.168.1.10:8989',
      icon: '/imgs/sonarr.png',
    })).toBe(dashboardIconUrl('sonarr'));
  });

  it('ignores Homarr user-media filenames and matches the app instead', () => {
    expect(shortcutIconUrl({
      name: 'Radarr',
      url: 'http://192.168.1.10:7878',
      icon: '/api/user-medias/icon.png',
    })).toBe(dashboardIconUrl('radarr'));
  });

  it('matches names, hostnames, and well-known ports', () => {
    expect(shortcutIconUrl({ name: 'Home Assistant', url: 'http://192.168.1.10:18080' }))
      .toBe(dashboardIconUrl('home-assistant'));
    expect(shortcutIconUrl({ name: 'Media', url: 'http://plex.lan/web' }))
      .toBe(dashboardIconUrl('plex'));
    expect(shortcutIconUrl({ name: 'Movies', url: 'http://192.168.1.10:7878' }))
      .toBe(dashboardIconUrl('radarr'));
  });

  it('drops javascript icons and uses a letter when nothing matches', () => {
    expect(shortcutIconUrl({
      name: '!',
      url: 'http://192.168.1.10:3000',
      icon: 'javascript:alert(1)',
    })).toBeNull();
    expect(shortcutLetter('plex')).toBe('P');
    expect(shortcutLetter('')).toBe('?');
  });
});
