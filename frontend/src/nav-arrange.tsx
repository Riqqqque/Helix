import {
  addableDashboardSections,
  hideDashboardPage,
  moveVisibleNavigationItem,
  showDashboardPage,
} from './dashboard-page-catalog';
import { visibleDashboardSections, type PrimaryDashboardSectionId } from './dashboard-preferences';
import { Icon, type IconName } from './icons';

export function NavOrderButtons({
  label,
  section,
  visibleIndex,
  visibleCount,
  order,
  hiddenPages,
  serversEnabled,
  onOrderChange,
  onHide,
}: {
  label: string;
  section: PrimaryDashboardSectionId;
  visibleIndex: number;
  visibleCount: number;
  order: readonly PrimaryDashboardSectionId[];
  hiddenPages: readonly PrimaryDashboardSectionId[];
  serversEnabled: boolean;
  onOrderChange: (order: PrimaryDashboardSectionId[]) => void;
  onHide: (section: PrimaryDashboardSectionId) => void;
}) {
  return (
    <div class="nav-item-order">
      <button type="button" disabled={visibleIndex <= 0} onClick={() => onOrderChange(moveVisibleNavigationItem(order, hiddenPages, serversEnabled, section, -1))} aria-label={`Move ${label} up`}><Icon name="chevron" size={12} class="icon--up" /></button>
      <button type="button" disabled={visibleIndex >= visibleCount - 1} onClick={() => onOrderChange(moveVisibleNavigationItem(order, hiddenPages, serversEnabled, section, 1))} aria-label={`Move ${label} down`}><Icon name="chevron" size={12} class="icon--down" /></button>
      <button type="button" onClick={() => onHide(section)} aria-label={`Hide ${label}`}><Icon name="trash" size={12} /></button>
    </div>
  );
}

export function NavAddCatalog({
  order,
  hiddenPages,
  serversEnabled,
  pages,
  onAdd,
}: {
  order: readonly PrimaryDashboardSectionId[];
  hiddenPages: readonly PrimaryDashboardSectionId[];
  serversEnabled: boolean;
  pages: ReadonlyArray<{ id: string; label: string; icon: IconName }>;
  onAdd: (section: PrimaryDashboardSectionId) => void;
}) {
  const addable = addableDashboardSections(order, hiddenPages, serversEnabled);
  if (addable.length === 0) return null;
  return (
    <div class="sidebar-nav__catalog">
      <span>Add a page</span>
      {addable.map((section) => {
        const item = pages.find((entry) => entry.id === section);
        if (item === undefined) return null;
        return (
          <button type="button" key={section} onClick={() => onAdd(section)}>
            <Icon name="plus" size={13} />
            <Icon name={item.icon} size={15} />
            <span>{item.label}</span>
          </button>
        );
      })}
    </div>
  );
}

export function hideArrangedPage(
  hiddenPages: readonly PrimaryDashboardSectionId[],
  section: PrimaryDashboardSectionId,
  serversEnabled: boolean,
  order: readonly PrimaryDashboardSectionId[],
  active: string,
  onHiddenPagesChange: (pages: PrimaryDashboardSectionId[]) => void,
  onServersEnabledChange: (enabled: boolean) => void,
): void {
  const nextHidden = hideDashboardPage(hiddenPages, section);
  const nextServersEnabled = section === 'servers' ? false : serversEnabled;
  if (section === 'servers') onServersEnabledChange(false);
  else onHiddenPagesChange(nextHidden);
  if (active === section) {
    const remaining = visibleDashboardSections(order, nextHidden, nextServersEnabled);
    window.location.hash = `#${remaining[0] ?? 'settings'}`;
  }
}

export function addArrangedPage(
  hiddenPages: readonly PrimaryDashboardSectionId[],
  section: PrimaryDashboardSectionId,
  onHiddenPagesChange: (pages: PrimaryDashboardSectionId[]) => void,
  onServersEnabledChange: (enabled: boolean) => void,
): void {
  if (section === 'servers') onServersEnabledChange(true);
  else onHiddenPagesChange(showDashboardPage(hiddenPages, section));
}

export type NavArrangeApi = {
  NavOrderButtons: typeof NavOrderButtons;
  NavAddCatalog: typeof NavAddCatalog;
  hideArrangedPage: typeof hideArrangedPage;
  addArrangedPage: typeof addArrangedPage;
};
