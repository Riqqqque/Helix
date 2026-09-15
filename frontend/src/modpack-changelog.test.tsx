import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ModpackChangelogButton, ModpackChangelogDialog, parseModpackChangelog } from './modpack-changelog';
import { renderMarketplaceBody } from './marketplace-markdown';
import type { ModpackVersion } from './modpack-api';
import type { NativeInstalledModpack } from './control-api';

const pack: NativeInstalledModpack = {provider:'curseforge',projectId:'123',projectTitle:'Test pack',versionId:'1',versionName:'Old',versionNumber:'1.0',minecraftVersion:'1.21.1',loader:'neoforge',loaderVersion:'21.1'};
const version: ModpackVersion = {id:'2',name:'New',versionNumber:'1.1',versionType:'release',status:null,datePublished:'2026-09-09T00:00:00Z',downloads:0,gameVersions:['1.21.1'],loaders:['neoforge'],installable:true,compatibilityReason:'',mrpackFile:null};
const props = {pack,version,csrfToken:'test',onSessionExpired:()=>{}};
describe('modpack changelog', () => {
  it('keeps notes closed until requested', () => {
    const html = render(<ModpackChangelogButton {...props} />);
    expect(html).toContain('Changelog');
    expect(html).not.toContain('role="dialog"');
  });
  it('shows the exact version transition and a loading state', () => {
    const html = render(<ModpackChangelogDialog {...props} onClose={()=>{}} />);
    expect(html).toContain('1.0 → 1.1');
    expect(html).toContain('Loading release notes');
    expect(html).toContain('CurseForge');
  });
  it('accepts empty notes but rejects invalid formats and oversized data', () => {
    expect(parseModpackChangelog({body:'',format:'html',truncated:false}).body).toBe('');
    for (const value of [{body:[],format:'html',truncated:false},{body:'x',format:'script',truncated:false},{body:'x'.repeat(400001),format:'html',truncated:false}]) expect(()=>parseModpackChangelog(value)).toThrow();
  });
  it('renders catalog formatting without running raw HTML', () => {
    const html = render(renderMarketplaceBody('<h2>Fixes</h2><ul><li>World save</li></ul><script>alert(1)</script><img src=x onerror=alert(1)>','html'));
    expect(html).toContain('World save');
    expect(html).not.toContain('<script');
    expect(html).not.toContain('onerror');
  });
});
