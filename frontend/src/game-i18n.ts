type Message = string | ((...args: never[]) => string);

const englishMessages = {
  'games.error.fallback': 'Helix could not load game-hosting readiness.',
  'games.hero.eyebrow': 'Game hosting',
  'games.hero.title': 'Every server, without the guesswork.',
  'games.hero.detail':
    'See lifecycle, players, resources, updates, backups, and warnings in one bounded workspace.',
  'games.action.newServer': 'New server',
  'games.action.newServerUnavailable':
    'Server creation stays locked until verified restore and native execution are available.',
  'games.action.refresh': 'Check readiness',
  'games.action.refreshing': 'Checking…',
  'games.action.retry': 'Try again',
  'games.action.unavailable': 'This action is not connected to a verified host operation yet.',
  'games.summary.registered': 'Registered',
  'games.summary.online': 'Online',
  'games.summary.players': 'Players',
  'games.summary.attention': 'Needs attention',
  'games.summary.unavailable': '—',
  'games.readiness.eyebrow': 'Host readiness',
  'games.readiness.ready': 'Ready for managed games',
  'games.readiness.degraded': 'Game hosting needs attention',
  'games.readiness.unavailable': 'Game hosting is safely locked',
  'games.readiness.readyDetail':
    'The host reported every required game-management boundary as available.',
  'games.readiness.degradedDetail':
    'Existing servers remain visible, but one or more management paths are unavailable.',
  'games.readiness.unavailableDetail':
    'Helix will not expose working-looking controls before restore, privilege, and process boundaries are proven.',
  'games.readiness.loading': 'Checking what this host can safely manage.',
  'games.readiness.failed': 'Readiness could not be verified.',
  'games.readiness.checked': (timestamp: string) => `Checked ${timestamp}`,
  'games.blocker.verified_restore.title': 'Verified restore',
  'games.blocker.verified_restore.detail':
    'A clean restore must succeed before Helix owns real game data.',
  'games.blocker.privileged_broker.title': 'Typed privilege broker',
  'games.blocker.privileged_broker.detail':
    'Root-required operations need a narrow audited systemd boundary.',
  'games.blocker.native_execution.title': 'Native game runner',
  'games.blocker.native_execution.detail':
    'Independent systemd services, cgroups, ports, and crash handling need live Linux proof.',
  'games.blocker.unknown.title': 'Additional host requirement',
  'games.blocker.unknown.detail':
    'This Helix build reported another requirement before game management can be enabled.',
  'games.blocker.status.required': 'Required',
  'games.blocker.status.in_progress': 'In progress',
  'games.blocker.status.ready': 'Ready',
  'games.registry.eyebrow': 'Server registry',
  'games.registry.lockedTitle': 'Your servers will appear here.',
  'games.registry.lockedDetail':
    'Connect Helix to a validated Linux game host to create or adopt servers. Lifecycle controls unlock only after the host passes every safety gate.',
  'games.registry.readyTitle': 'Your servers',
  'games.registry.count': (shown: number, total: number) =>
    `${shown} of ${total} ${total === 1 ? 'server' : 'servers'} shown`,
  'games.filter.search': 'Search servers',
  'games.filter.searchPlaceholder': 'Name, game, version, or address',
  'games.filter.status': 'Status',
  'games.filter.all': 'All servers',
  'games.filter.online': 'Online',
  'games.filter.offline': 'Offline',
  'games.filter.attention': 'Needs attention',
  'games.view.cards': 'Card view',
  'games.view.list': 'Compact view',
  'games.view.label': 'Server view',
  'games.empty.title': 'No game servers yet',
  'games.empty.detail':
    'Open Servers and choose New server. Helix Native stays separate from any AMP import.',
  'games.empty.filteredTitle': 'No servers match these filters',
  'games.empty.filteredDetail': 'Try a different search or status filter.',
  'games.instance.players': 'Players',
  'games.instance.cpu': 'CPU',
  'games.instance.memory': 'Memory',
  'games.instance.uptime': 'Uptime',
  'games.instance.address': 'Address',
  'games.instance.update': 'Update',
  'games.instance.backup': 'Backup',
  'games.instance.warnings': (count: number) =>
    `${count} ${count === 1 ? 'warning' : 'warnings'}`,
  'games.instance.noWarnings': 'No warnings',
  'games.instance.open': (name: string) => `Open ${name}`,
  'games.instance.unlimited': 'No limit reported',
  'games.instance.notReported': 'Not reported',
  'games.status.online': 'Online',
  'games.status.starting': 'Starting',
  'games.status.stopping': 'Stopping',
  'games.status.offline': 'Offline',
  'games.status.installing': 'Installing',
  'games.status.updating': 'Updating',
  'games.status.backing_up': 'Backing up',
  'games.status.restoring': 'Restoring',
  'games.status.degraded': 'Degraded',
  'games.status.failed': 'Failed',
  'games.status.unknown': 'Unknown',
  'games.update.current': 'Current',
  'games.update.available': 'Available',
  'games.update.pinned': 'Pinned',
  'games.update.checking': 'Checking',
  'games.update.unknown': 'Unknown',
  'games.backup.healthy': 'Healthy',
  'games.backup.stale': 'Stale',
  'games.backup.failed': 'Failed',
  'games.backup.unconfigured': 'Not configured',
  'games.backup.unknown': 'Unknown',
  'games.detail.back': 'All servers',
  'games.detail.server': 'Game server',
  'games.detail.start': 'Start',
  'games.detail.stop': 'Stop',
  'games.detail.kill': 'Kill',
  'games.detail.restart': 'Restart',
  'games.detail.backup': 'Back up now',
  'games.detail.overview': 'Overview',
  'games.detail.runtime': 'Runtime',
  'games.detail.safety': 'Safety',
  'games.detail.currentState': 'Current state',
  'games.detail.software': 'Software',
  'games.detail.version': 'Version',
  'games.detail.none': 'None',
  'games.tab.overview': 'Overview',
  'games.tab.console': 'Console',
  'games.tab.players': 'Players',
  'games.tab.settings': 'Settings',
  'games.tab.mods_plugins': 'Mods / Plugins',
  'games.tab.worlds_saves': 'Worlds / Saves',
  'games.tab.files': 'Files',
  'games.tab.networking': 'Networking',
  'games.tab.backups': 'Backups',
  'games.tab.automation': 'Automation',
  'games.tab.logs': 'Logs',
  'games.tab.performance': 'Performance',
  'games.tab.advanced': 'Advanced',
} as const;

export type TranslationId = keyof typeof englishMessages;

type TranslationArgs<Id extends TranslationId> =
  (typeof englishMessages)[Id] extends (...args: infer Args) => string ? Args : [];

export function t<Id extends TranslationId>(
  id: Id,
  ...args: TranslationArgs<Id>
): string {
  const message: Message = englishMessages[id] as Message;
  if (typeof message === 'function') {
    return (message as (...values: TranslationArgs<Id>) => string)(...args);
  }
  return message;
}
