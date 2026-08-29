import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { ApiError, getHealth } from "./api";
import { InlineError } from "./dashboard-ui";
import { formatBytes, formatTimestamp } from "./format";
import { Icon } from "./icons";
import { InfoTip } from "./info-tip";
import { Dialog } from "./modal";
import {
  applyHelixUpdate,
  applySystemPackageUpdates,
  checkHelixUpdate,
  getSystemPackageInventory,
  getSystemPackageJob,
  refreshSystemPackageLists,
  type PackageJob,
  type SystemPackage,
  type SystemPackageInventory,
} from "./package-api";
import "./infrastructure.css";

export interface HostUpdatesProps {
  csrfToken: string;
  onSessionExpired: () => void;
}

export type PackageFilter = "updates" | "security" | "held" | "all";
const PAGE_SIZE = 75;
const MAX_SELECTED_UPDATES = 512;
const PACKAGE_JOB_STORAGE_KEY = "helix.package-job";

function selectableUpdate(item: SystemPackage): boolean {
  return (
    item.upgradeAvailable &&
    item.held !== true &&
    item.candidateVersion !== null &&
    item.downloadSizeBytes !== null
  );
}

function expectedConfirmation(count: number): string {
  return `APPLY ${count} UPDATE${count === 1 ? "" : "S"}`;
}

function describeError(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Helix could not read Linux updates.";
}

function isSessionError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.status === 401 || error.code === "csrf_rejected")
  );
}

async function helixLiveness(signal?: AbortSignal): Promise<boolean> {
  try {
    const init: RequestInit = {
      cache: "no-store",
      credentials: "same-origin",
    };
    if (signal !== undefined) init.signal = signal;
    const response = await fetch("/healthz", init);
    return response.status === 204;
  } catch {
    return false;
  }
}

function formatCacheAge(
  cacheUnixMs: number | null,
  collectedUnixMs: number,
): string {
  if (cacheUnixMs === null) return "Unknown";
  const seconds = Math.max(
    0,
    Math.floor((collectedUnixMs - cacheUnixMs) / 1_000),
  );
  if (seconds < 60) return "Less than a minute";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function restartLabel(item: SystemPackage): string {
  if (item.restartHint === "host_reboot_requested") return "Linux asked for a reboot";
  if (item.restartHint === "likely_host_reboot") return "Often needs a host reboot";
  if (item.restartHint === "likely_service_restart") return "Services may restart";
  if (item.restartImpactKnown) return item.restartHint.replaceAll("_", " ");
  return "Impact unknown";
}

function restartTone(item: SystemPackage): "warning" | "idle" {
  return item.restartHint === "host_reboot_requested" ||
    item.restartHint === "likely_host_reboot"
    ? "warning"
    : "idle";
}

function packageNeedsHostReboot(item: SystemPackage): boolean {
  return (
    item.restartHint === "host_reboot_requested" ||
    item.restartHint === "likely_host_reboot"
  );
}

function cacheLooksStale(
  cacheUnixMs: number | null,
  collectedUnixMs: number,
): boolean {
  if (cacheUnixMs === null) return true;
  return collectedUnixMs - cacheUnixMs > 24 * 60 * 60 * 1_000;
}

function previewLabel(data: SystemPackageInventory): string {
  if (!data.simulation.available) {
    return data.simulation.error ?? "Preview unavailable";
  }
  const parts = [
    `${data.simulation.upgradeCandidates} package${data.simulation.upgradeCandidates === 1 ? "" : "s"} can be upgraded`,
  ];
  if (data.simulation.newPackages === 0 && data.simulation.removals === 0) {
    parts.push("would not add or remove packages");
  } else {
    parts.push(
      `would add ${data.simulation.newPackages} and remove ${data.simulation.removals}`,
    );
  }
  if (data.simulation.heldBack > 0) {
    parts.push(`${data.simulation.heldBack} held back`);
  }
  return parts.join(" · ");
}

function errorComponentLabel(component: string): string {
  if (component === "apt_simulation" || component === "package_preview") {
    return "Package preview";
  }
  if (component === "dpkg_query") return "Installed packages";
  if (component === "apt_cache") return "Package details";
  if (component === "apt_mark") return "Held packages";
  return component.replaceAll("_", " ");
}

function toolLabel(tool: string): string {
  if (tool === "dpkgQuery") return "dpkg";
  if (tool === "aptCache") return "apt-cache";
  if (tool === "aptGet") return "apt-get";
  if (tool === "aptMark") return "apt-mark";
  return tool;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function jobRebootRequired(job: PackageJob): boolean {
  return asRecord(job.result)?.reboot_required === true;
}

function jobRebootPackages(job: PackageJob): string[] {
  const packages = asRecord(job.result)?.reboot_required_packages;
  if (!Array.isArray(packages)) return [];
  return packages.filter(
    (entry): entry is string => typeof entry === "string" && entry.length > 0,
  );
}

function jobBannerCopy(job: PackageJob): { title: string; detail: string } {
  if (job.status === "failed") {
    return {
      title:
        job.kind === "helix_release_apply"
          ? "Helix update stopped"
          : job.kind === "system_package_lists_refresh"
            ? "Could not check for updates"
            : "Linux update stopped",
      detail: job.error ?? "The operation did not complete.",
    };
  }
  if (job.status === "complete") {
    if (job.kind === "helix_release_apply") {
      return {
        title: "Helix update staged",
        detail:
          "The dashboard will restart. Refresh after it comes back. Game containers stay running.",
      };
    }
    if (job.kind === "system_package_lists_refresh") {
      return {
        title: "Package lists updated",
        detail:
          "Nothing was installed. Review the list and apply the exact packages you want.",
      };
    }
    if (jobRebootRequired(job)) {
      const packages = jobRebootPackages(job);
      return {
        title: "Linux updates installed · host reboot needed",
        detail: `${packages.length > 0 ? packages.join(", ") : "One or more packages"} asked for a reboot. Helix did not reboot. Open Settings → Whole-host reboot when you choose.`,
      };
    }
    return {
      title: "Linux updates installed",
      detail:
        "Selected versions verified. Linux did not ask for a reboot. Helix never reboots automatically.",
    };
  }
  return {
    title: job.stage,
    detail: `${job.progressPercent}% · This keeps running if you leave the page.`,
  };
}

function packageMatches(
  item: SystemPackage,
  filter: PackageFilter,
  query: string,
): boolean {
  const filterMatch =
    filter === "all" ||
    (filter === "updates" && item.upgradeAvailable) ||
    (filter === "security" && item.securityUpdate === true) ||
    (filter === "held" && item.held === true);
  if (!filterMatch) return false;
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return true;
  return [
    item.name,
    item.description,
    item.sourcePackage ?? "",
    item.category ?? "",
    item.candidateOrigin ?? "",
  ].some((value) => value.toLowerCase().includes(needle));
}

export function PackageInventoryView({
  data,
  filter,
  query,
  page,
  onFilter,
  onQuery,
  onPage,
  selected,
  onToggleSelected,
  onSelectSafeUpdates,
  onApplySelected,
  onCheckHelix,
  onUpdateHelix,
  mutationBusy,
}: {
  data: SystemPackageInventory;
  filter: PackageFilter;
  query: string;
  page: number;
  onFilter: (filter: PackageFilter) => void;
  onQuery: (query: string) => void;
  onPage: (page: number) => void;
  selected: ReadonlySet<string>;
  onToggleSelected: (item: SystemPackage) => void;
  onSelectSafeUpdates: () => void;
  onApplySelected: () => void;
  onCheckHelix: () => void;
  onUpdateHelix: () => void;
  mutationBusy: boolean;
}) {
  const packages = useMemo(
    () =>
      data.inventory.packages.filter((item) =>
        packageMatches(item, filter, query),
      ),
    [data.inventory.packages, filter, query],
  );
  const pageCount = Math.max(1, Math.ceil(packages.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const rows = packages.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE);
  const cacheAge = formatCacheAge(
    data.aptCacheRefreshedAtUnixMs,
    data.collectedAtUnixMs,
  );
  const heldCount = data.inventory.packages.filter(
    (item) => item.held === true,
  ).length;
  const selectedPackages = data.inventory.packages.filter((item) =>
    selected.has(item.name),
  );
  const selectedBytes = selectedPackages.reduce(
    (total, item) => total + (item.downloadSizeBytes ?? 0),
    0,
  );
  const safeUpdates = data.inventory.packages.filter(selectableUpdate);
  const cacheStale = cacheLooksStale(
    data.aptCacheRefreshedAtUnixMs,
    data.collectedAtUnixMs,
  );
  const rebootNow = data.hostRestart.rebootRequiredMarkerPresent;
  const previewFailed = !data.simulation.available;
  return (
    <>
      <section class="update-summary-grid" aria-label="Linux update summary">
        <article>
          <span>
            Installed{" "}
            <InfoTip text="Packages currently reported by dpkg. A very large host can truncate this list." />
          </span>
          <strong>{data.inventory.installedTotal.toLocaleString()}</strong>
          <small>
            {data.inventory.truncated
              ? "Inventory truncated"
              : "Currently installed"}
          </small>
        </article>
        <article>
          <span>
            Ready to update{" "}
            <InfoTip text="Installed and candidate versions differ in the current package lists. Check for updates if this looks stale." />
          </span>
          <strong>{data.inventory.upgradeAvailableTotal}</strong>
          <small>
            {data.inventory.upgradeAvailableTotal === 0
              ? "No package updates in the current lists"
              : "Exact versions you can select below"}
          </small>
        </article>
        <article>
          <span>
            Security{" "}
            <InfoTip text="Best-effort classification from the candidate origin. Unknown is kept separate from no." />
          </span>
          <strong
            class={
              data.inventory.securityUpdateTotal > 0 ? "update-accent" : ""
            }
          >
            {data.inventory.securityUpdateTotal}
          </strong>
          <small>From a security archive</small>
        </article>
        <article>
          <span>
            List age{" "}
            <InfoTip text="Opening this page does not talk to the mirrors. Check for updates refreshes the signed package lists." />
          </span>
          <strong>{cacheAge}</strong>
          <small>
            {data.aptCacheRefreshedAtUnixMs === null
              ? "No list timestamp found"
              : formatTimestamp(data.aptCacheRefreshedAtUnixMs)}
          </small>
        </article>
      </section>

      {rebootNow && (
        <div class="package-safety-note package-safety-note--warning">
          <Icon name="warning" size={17} />
          <div>
            <strong>Linux needs a host reboot</strong>
            <span>
              {data.hostRestart.packages.length > 0
                ? `${data.hostRestart.packages.join(", ")} asked for a reboot.`
                : "A previous update asked for a reboot."}{" "}
              Helix never reboots Linux for you. Open{" "}
              <a href="#settings">Settings → Whole-host reboot</a> when you are
              ready. That disconnects Helix, players, and every other service.
            </span>
          </div>
        </div>
      )}

      {cacheStale && !previewFailed && (
        <div class="package-safety-note package-safety-note--warning">
          <Icon name="warning" size={17} />
          <div>
            <strong>Package lists look stale</strong>
            <span>
              Use <strong>Check for updates</strong> before applying anything.
              That only refreshes the signed lists. Nothing is installed until
              you apply selected packages.
            </span>
          </div>
        </div>
      )}

      {previewFailed && (
        <div class="package-safety-note package-safety-note--warning">
          <Icon name="warning" size={17} />
          <div>
            <strong>Helix could not preview what APT would change</strong>
            <span>
              {data.simulation.error ??
                "Package tools did not return a usable preview."}{" "}
              Apply stays off until this works. Try Check for updates, or wait
              if another package tool has the APT lock.
            </span>
          </div>
        </div>
      )}

      {!rebootNow && !previewFailed && !cacheStale && (
        <div class="package-safety-note">
          <Icon name="info" size={17} />
          <div>
            <strong>Nothing changes until you confirm it</strong>
            <span>
              Reading this list is safe. Check for updates talks to the
              mirrors. Apply installs only the exact packages you select. Helix
              never reboots Linux automatically.
            </span>
          </div>
        </div>
      )}

      <section class="surface infrastructure-section package-readiness">
        <div class="section-title">
          <div>
            <h2>
              Linux packages{" "}
              <InfoTip text="Helix has no upgrade-everything button. You choose exact versions. Before apply it rechecks versions, holds, disk space, and that the change would not add or remove packages." />
            </h2>
            <p>
              {previewFailed
                ? "Apply is unavailable until the preview works"
                : previewLabel(data)}
            </p>
          </div>
          <span
            class={`state-label state-label--${data.upgradeApply.available ? "good" : "warning"}`}
          >
            {data.upgradeApply.available
              ? data.inventory.upgradeAvailableTotal === 0
                ? "Up to date"
                : "Ready to apply selected"
              : "Cannot apply yet"}
          </span>
        </div>
        <div class="update-readiness-grid">
          <article>
            <span>Apply selected Linux updates</span>
            <strong>
              {data.upgradeApply.available
                ? `${selected.size} selected`
                : "Apply unavailable"}
            </strong>
            <small>{data.upgradeApply.reason}</small>
            <div class="package-selection-actions">
              <button
                class="button button--quiet"
                type="button"
                disabled={
                  mutationBusy ||
                  !data.upgradeApply.available ||
                  safeUpdates.length === 0
                }
                onClick={onSelectSafeUpdates}
              >
                {selected.size ===
                  Math.min(safeUpdates.length, MAX_SELECTED_UPDATES) &&
                selected.size > 0
                  ? "Clear selection"
                  : `Select safe updates (${Math.min(safeUpdates.length, MAX_SELECTED_UPDATES)})`}
              </button>
              <button
                class="button button--primary"
                type="button"
                disabled={
                  mutationBusy ||
                  !data.upgradeApply.available ||
                  selected.size === 0
                }
                onClick={onApplySelected}
              >
                <Icon name="update" size={15} />
                Apply selected · {formatBytes(selectedBytes)}
              </button>
            </div>
          </article>
          <article>
            <span>Helix dashboard</span>
            <strong>
              {data.helixSelfUpdate.updateAvailable &&
              data.helixSelfUpdate.latestVersion !== null
                ? `Helix ${data.helixSelfUpdate.latestVersion} available`
                : `Helix ${data.helixSelfUpdate.currentVersion}`}
            </strong>
            <small>{data.helixSelfUpdate.reason}</small>
            {data.helixSelfUpdate.releaseUrl !== null && (
              <small>
                <a
                  href={data.helixSelfUpdate.releaseUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  GitHub release
                </a>
              </small>
            )}
            <div class="package-selection-actions">
              <button
                class="button button--quiet"
                type="button"
                disabled={mutationBusy}
                onClick={onCheckHelix}
              >
                Check GitHub
              </button>
              <button
                class="button button--primary"
                type="button"
                disabled={mutationBusy || !data.helixSelfUpdate.available}
                onClick={onUpdateHelix}
              >
                <Icon name="update" size={15} />
                {data.helixSelfUpdate.updateAvailable
                  ? `Update to ${data.helixSelfUpdate.latestVersion}`
                  : "Update Helix"}
              </button>
            </div>
          </article>
        </div>
        <p class="readiness-footnote">
          Helix does not claim it can undo a failed package install. Existing
          config files are kept. Held packages, new packages, and removals are
          refused. If an update needs a host reboot, Helix says so and still
          does not reboot for you.
        </p>
      </section>

      <section class="surface infrastructure-section package-inventory">
        <div class="section-title package-inventory-head">
          <div>
            <h2>
              Package list{" "}
              <InfoTip text="Installed versions come from dpkg. Candidate versions, origins, descriptions, and download sizes depend on the current package lists." />
            </h2>
            <p>
              {packages.length.toLocaleString()} matching packages · collected{" "}
              {formatTimestamp(data.collectedAtUnixMs)}
            </p>
          </div>
          <label class="search-box">
            <Icon name="search" size={15} />
            <input
              value={query}
              onInput={(event) => {
                onQuery(event.currentTarget.value);
                onPage(0);
              }}
              placeholder="Package, source, category…"
              aria-label="Filter packages"
            />
          </label>
        </div>
        <div
          class="package-filter-bar"
          role="group"
          aria-label="Package filter"
        >
          {(
            [
              ["updates", `Updates ${data.inventory.upgradeAvailableTotal}`],
              ["security", `Security ${data.inventory.securityUpdateTotal}`],
              ["held", `Held ${heldCount}`],
              ["all", `All ${data.inventory.packages.length}`],
            ] as const
          ).map(([id, label]) => (
            <button
              class={filter === id ? "is-active" : ""}
              type="button"
              key={id}
              onClick={() => {
                onFilter(id);
                onPage(0);
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div class="table-scroll package-table-wrap">
          <table class="data-table package-table">
            <thead>
              <tr>
                <th class="package-select-column">
                  <span class="sr-only">Select</span>
                </th>
                <th>Package</th>
                <th>Installed → candidate</th>
                <th>Size</th>
                <th>Source</th>
                <th>Signals</th>
                <th>Restart</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((item) => (
                <tr
                  key={item.name}
                  class={selected.has(item.name) ? "is-selected" : ""}
                >
                  <td class="package-select-column">
                    <input
                      type="checkbox"
                      aria-label={`Select ${item.name}`}
                      checked={selected.has(item.name)}
                      disabled={
                        mutationBusy ||
                        !data.upgradeApply.available ||
                        !selectableUpdate(item)
                      }
                      title={
                        !item.upgradeAvailable
                          ? "No update is available."
                          : item.held === true
                            ? "APT holds this package."
                            : item.candidateVersion === null
                              ? "Candidate version is unknown."
                              : item.downloadSizeBytes === null
                                ? "Download size is unknown, so the disk-space gate cannot be proven."
                                : "Select this exact candidate."
                      }
                      onChange={() => onToggleSelected(item)}
                    />
                  </td>
                  <td>
                    <strong>{item.name}</strong>
                    <small>
                      {item.description || "No package description available."}
                    </small>
                  </td>
                  <td>
                    <code>{item.installedVersion}</code>
                    <span aria-hidden="true"> → </span>
                    <code>{item.candidateVersion ?? "unknown"}</code>
                  </td>
                  <td>
                    <strong>
                      {item.downloadSizeBytes === null
                        ? "Unknown download"
                        : formatBytes(item.downloadSizeBytes)}
                    </strong>
                    <small>
                      {item.installedSizeBytes === null
                        ? "Installed size unknown"
                        : `${formatBytes(item.installedSizeBytes)} installed`}
                    </small>
                  </td>
                  <td>
                    <strong>{item.sourcePackage ?? "Unknown source"}</strong>
                    <small>
                      {[item.category, item.candidateOrigin]
                        .filter((value): value is string => value !== null)
                        .join(" · ") || "Origin unavailable"}
                    </small>
                  </td>
                  <td>
                    <div class="package-signals">
                      {item.upgradeAvailable && (
                        <span class="state-label state-label--good">
                          Update
                        </span>
                      )}
                      {item.securityUpdate === true && (
                        <span class="state-label state-label--warning">
                          Security
                        </span>
                      )}
                      {item.held === true && (
                        <span class="state-label state-label--idle">Held</span>
                      )}
                      {item.securityUpdate === null && (
                        <small>Security unknown</small>
                      )}
                    </div>
                  </td>
                  <td>
                    <span
                      class={`state-label state-label--${restartTone(item)}`}
                    >
                      {restartLabel(item)}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {rows.length === 0 && (
          <div class="table-state">No packages match this filter.</div>
        )}
        <footer class="package-pagination">
          <span>
            Showing {packages.length === 0 ? 0 : safePage * PAGE_SIZE + 1}–
            {Math.min((safePage + 1) * PAGE_SIZE, packages.length)} of{" "}
            {packages.length}
          </span>
          <div>
            <button
              class="button button--quiet"
              type="button"
              disabled={safePage === 0}
              onClick={() => onPage(Math.max(0, safePage - 1))}
            >
              Previous
            </button>
            <span>
              Page {safePage + 1} of {pageCount}
            </span>
            <button
              class="button button--quiet"
              type="button"
              disabled={safePage >= pageCount - 1}
              onClick={() => onPage(Math.min(pageCount - 1, safePage + 1))}
            >
              Next
            </button>
          </div>
        </footer>
      </section>

      {(data.errors.length > 0 || data.availability !== "ready") && (
        <section class="surface infrastructure-section package-diagnostics">
          <div class="section-title">
            <div>
              <h2>What Helix could not read</h2>
              <p>Missing tools and partial evidence stay visible</p>
            </div>
            <span class="state-label state-label--warning">
              {data.availability}
            </span>
          </div>
          <div class="tool-status-list">
            {Object.entries(data.tools).map(([tool, available]) => (
              <span key={tool}>
                <i
                  class={`status-dot status-dot--${available ? "good" : "idle"}`}
                />
                {toolLabel(tool)}
              </span>
            ))}
          </div>
          {data.errors.map((error) => (
            <p class="table-note" key={`${error.component}-${error.message}`}>
              <strong>{errorComponentLabel(error.component)}</strong>: {error.message}
            </p>
          ))}
        </section>
      )}
    </>
  );
}

export function HostUpdatesPanel({
  csrfToken,
  onSessionExpired,
}: HostUpdatesProps) {
  const [data, setData] = useState<SystemPackageInventory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<PackageFilter>("updates");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [job, setJob] = useState<PackageJob | null>(null);
  const [startingMutation, setStartingMutation] = useState(false);
  const [applyOpen, setApplyOpen] = useState(false);
  const [helixApplyOpen, setHelixApplyOpen] = useState(false);
  const [waitingForHelix, setWaitingForHelix] = useState(false);
  const [confirmation, setConfirmation] = useState("");
  const [helixConfirmation, setHelixConfirmation] = useState("");
  const [disruptionAcknowledged, setDisruptionAcknowledged] = useState(false);
  const [helixDisruptionAcknowledged, setHelixDisruptionAcknowledged] =
    useState(false);
  const githubChecked = useRef(false);
  const helixRestartFromVersion = useRef<string | null>(null);

  const load = useCallback(
    async (signal?: AbortSignal): Promise<void> => {
      setLoading(true);
      try {
        const next = await getSystemPackageInventory(csrfToken, signal);
        setData(next);
        setError(null);
      } catch (requestError) {
        if (signal?.aborted === true) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      } finally {
        if (signal?.aborted !== true) setLoading(false);
      }
    },
    [csrfToken, onSessionExpired],
  );

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  useEffect(() => {
    if (data === null || githubChecked.current || waitingForHelix) return;
    if (
      data.helixSelfUpdate.latestVersion !== null &&
      data.helixSelfUpdate.reasonCode !== "github_release_unavailable"
    ) {
      githubChecked.current = true;
      return;
    }
    githubChecked.current = true;
    void checkHelixUpdate(csrfToken)
      .then((helixSelfUpdate) => {
        setData((current) =>
          current === null ? current : { ...current, helixSelfUpdate },
        );
      })
      .catch((requestError: unknown) => {
        if (isSessionError(requestError)) onSessionExpired();
      });
  }, [csrfToken, data, onSessionExpired, waitingForHelix]);

  useEffect(() => {
    if (!waitingForHelix) return;
    const started = Date.now();
    const controller = new AbortController();
    let sawDown = false;
    const timer = window.setInterval(() => {
      void (async () => {
        const live = await helixLiveness(controller.signal);
        if (controller.signal.aborted) return;
        if (!live) {
          sawDown = true;
          return;
        }
        try {
          const health = await getHealth(csrfToken, controller.signal);
          if (controller.signal.aborted) return;
          const from = helixRestartFromVersion.current;
          if ((from !== null && health.version !== from) || sawDown) {
            window.location.reload();
            return;
          }
        } catch {
          sawDown = true;
        }
        if (Date.now() - started > 12 * 60 * 1_000) {
          setError(
            "Helix is taking longer than expected to come back. Refresh this page.",
          );
          window.clearInterval(timer);
        }
      })();
    }, 1_000);
    return () => {
      window.clearInterval(timer);
      controller.abort();
    };
  }, [csrfToken, waitingForHelix]);

  useEffect(() => {
    let stored: string | null;
    try {
      stored = localStorage.getItem(PACKAGE_JOB_STORAGE_KEY);
    } catch {
      return;
    }
    if (
      stored === null ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(
        stored,
      )
    )
      return;
    const controller = new AbortController();
    void getSystemPackageJob(stored, csrfToken, controller.signal)
      .then(setJob)
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        try {
          localStorage.removeItem(PACKAGE_JOB_STORAGE_KEY);
        } catch {
          /* Storage is optional. */
        }
        if (isSessionError(requestError)) onSessionExpired();
      });
    return () => controller.abort();
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    if (job === null || (job.status !== "queued" && job.status !== "running"))
      return;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void getSystemPackageJob(job.id, csrfToken, controller.signal)
        .then((next) => {
          if (controller.signal.aborted) return;
          setJob(next);
          if (next.status === "complete" || next.status === "failed") {
            try {
              localStorage.removeItem(PACKAGE_JOB_STORAGE_KEY);
            } catch {
              /* Storage is optional. */
            }
            setSelected(new Set());
            if (
              next.kind === "helix_release_apply" &&
              next.status === "complete"
            ) {
              helixRestartFromVersion.current =
                data?.helixSelfUpdate.currentVersion ?? null;
              setWaitingForHelix(true);
            } else {
              void load();
            }
          }
        })
        .catch((requestError: unknown) => {
          if (controller.signal.aborted) return;
          if (isSessionError(requestError)) onSessionExpired();
          else if (job.kind === "helix_release_apply") {
            helixRestartFromVersion.current =
              data?.helixSelfUpdate.currentVersion ?? null;
            setWaitingForHelix(true);
          } else setError(describeError(requestError));
        });
    }, 1_000);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [csrfToken, data, job, load, onSessionExpired]);

  const selectedPackages = useMemo(
    () =>
      data?.inventory.packages.filter(
        (item) => selected.has(item.name) && selectableUpdate(item),
      ) ?? [],
    [data, selected],
  );
  const mutationBusy =
    startingMutation ||
    waitingForHelix ||
    job?.status === "queued" ||
    job?.status === "running";
  const confirmationPhrase = expectedConfirmation(selectedPackages.length);
  const rebootPackages = selectedPackages.filter(packageNeedsHostReboot);

  const rememberJob = (id: string, kind: string, stage: string): void => {
    const now = Date.now();
    setJob({
      id,
      kind,
      status: "queued",
      stage,
      progressPercent: 0,
      createdAtUnixMs: now,
      updatedAtUnixMs: now,
      result: null,
      error: null,
    });
    try {
      localStorage.setItem(PACKAGE_JOB_STORAGE_KEY, id);
    } catch {
      /* The broker still owns the job. */
    }
  };

  const startRefresh = async (): Promise<void> => {
    setStartingMutation(true);
    setError(null);
    try {
      const dispatched = await refreshSystemPackageLists(csrfToken);
      rememberJob(
        dispatched.jobId,
        "system_package_lists_refresh",
        "Queued to check signed package sources",
      );
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setStartingMutation(false);
    }
  };

  const startApply = async (): Promise<void> => {
    if (
      selectedPackages.length === 0 ||
      confirmation !== confirmationPhrase ||
      !disruptionAcknowledged
    )
      return;
    setStartingMutation(true);
    setError(null);
    try {
      const dispatched = await applySystemPackageUpdates(
        selectedPackages,
        confirmation,
        disruptionAcknowledged,
        csrfToken,
      );
      rememberJob(
        dispatched.jobId,
        "system_package_apply",
        "Queued to check the selected versions",
      );
      setApplyOpen(false);
      setConfirmation("");
      setDisruptionAcknowledged(false);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setStartingMutation(false);
    }
  };

  const startCheckHelix = async (): Promise<void> => {
    if (data === null || mutationBusy) return;
    setStartingMutation(true);
    setError(null);
    try {
      const helixSelfUpdate = await checkHelixUpdate(csrfToken);
      setData({ ...data, helixSelfUpdate });
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setStartingMutation(false);
    }
  };

  const startHelixApply = async (): Promise<void> => {
    if (
      data === null ||
      data.helixSelfUpdate.latestTag === null ||
      helixConfirmation !== data.helixSelfUpdate.requiredConfirmation ||
      !helixDisruptionAcknowledged
    )
      return;
    setStartingMutation(true);
    setError(null);
    try {
      const dispatched = await applyHelixUpdate(
        data.helixSelfUpdate.latestTag,
        helixConfirmation,
        helixDisruptionAcknowledged,
        csrfToken,
      );
      rememberJob(
        dispatched.jobId,
        "helix_release_apply",
        "Queued to download a digest-pinned Helix release",
      );
      setHelixApplyOpen(false);
      setHelixConfirmation("");
      setHelixDisruptionAcknowledged(false);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setStartingMutation(false);
    }
  };

  const toggleSelected = (item: SystemPackage): void => {
    if (!selectableUpdate(item) || mutationBusy) return;
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(item.name)) next.delete(item.name);
      else if (next.size < MAX_SELECTED_UPDATES) next.add(item.name);
      return next;
    });
  };

  const selectSafeUpdates = (): void => {
    if (data === null || mutationBusy) return;
    const safe = data.inventory.packages
      .filter(selectableUpdate)
      .slice(0, MAX_SELECTED_UPDATES);
    setSelected((current) =>
      current.size === safe.length &&
      safe.every((item) => current.has(item.name))
        ? new Set()
        : new Set(safe.map((item) => item.name)),
    );
  };

  return (
    <div class="infrastructure-panel" aria-busy={loading}>
      <div class="section-title section-title--spaced">
        <div>
          <h2>
            Linux updates{" "}
            <InfoTip text="Check for updates refreshes signed package lists. Apply installs only the exact packages you select. Package services may restart. If Linux needs a host reboot, Helix says so and still does not reboot for you." />
          </h2>
          <p>
            Pick exact packages. Helix never reboots Linux automatically.
          </p>
        </div>
        <div class="package-heading-actions">
          <button
            class="button button--quiet"
            type="button"
            disabled={loading || mutationBusy}
            onClick={() => void load()}
          >
            <Icon name="refresh" size={15} />
            {loading ? "Reading…" : "Refresh view"}
          </button>
          <button
            class="button button--primary"
            type="button"
            disabled={
              mutationBusy ||
              data?.upgradeApply.packageListsRefreshAvailable !== true
            }
            onClick={() => void startRefresh()}
          >
            <Icon name="update" size={15} />
            {startingMutation ? "Starting…" : "Check for updates"}
          </button>
        </div>
      </div>
      <InlineError message={error} />
      {waitingForHelix && (
        <section
          class="package-job-banner package-job-banner--running"
          aria-live="polite"
        >
          <Icon name="update" size={18} />
          <div>
            <strong>Helix is restarting</strong>
            <span>
              This page reloads when the new dashboard answers. Game containers
              stay running.
            </span>
          </div>
        </section>
      )}
      {job !== null && !waitingForHelix && (
        <section
          class={`package-job-banner package-job-banner--${job.status === "complete" && jobRebootRequired(job) ? "warning" : job.status}`}
          aria-live="polite"
        >
          <Icon
            name={
              job.status === "failed" || jobRebootRequired(job)
                ? "warning"
                : job.status === "complete"
                  ? "check"
                  : "update"
            }
            size={18}
          />
          <div>
            <strong>{jobBannerCopy(job).title}</strong>
            <span>{jobBannerCopy(job).detail}</span>
          </div>
          {(job.status === "complete" || job.status === "failed") && (
            <button
              class="button button--quiet"
              type="button"
              onClick={() => setJob(null)}
            >
              Dismiss
            </button>
          )}
        </section>
      )}
      {data === null ? (
        <div class="detail-loading">
          <Icon name={error === null ? "update" : "warning"} size={28} />
          <span>
            {error === null
              ? "Reading installed packages and update lists…"
              : "Linux updates are unavailable."}
          </span>
        </div>
      ) : (
        <PackageInventoryView
          data={data}
          filter={filter}
          query={query}
          page={page}
          onFilter={setFilter}
          onQuery={setQuery}
          onPage={setPage}
          selected={selected}
          onToggleSelected={toggleSelected}
          onSelectSafeUpdates={selectSafeUpdates}
          onApplySelected={() => {
            setConfirmation("");
            setDisruptionAcknowledged(false);
            setApplyOpen(true);
          }}
          onCheckHelix={() => {
            void startCheckHelix();
          }}
          onUpdateHelix={() => {
            setHelixConfirmation("");
            setHelixDisruptionAcknowledged(false);
            setHelixApplyOpen(true);
          }}
          mutationBusy={mutationBusy}
        />
      )}
      {applyOpen && (
        <Dialog
          title={`Apply ${selectedPackages.length} selected update${selectedPackages.length === 1 ? "" : "s"}?`}
          onClose={() => !startingMutation && setApplyOpen(false)}
          wide
        >
          <div class="package-apply-dialog">
            <div class="package-apply-summary">
              <strong>
                {formatBytes(
                  selectedPackages.reduce(
                    (total, item) => total + (item.downloadSizeBytes ?? 0),
                    0,
                  ),
                )}{" "}
                download
              </strong>
              <span>
                Exact versions are rechecked immediately before APT runs. Held
                packages, new packages, removals, a version that changed since
                you looked, or not enough free disk stop the job. Nothing is
                applied in those cases.
              </span>
            </div>
            <div class="package-apply-preview">
              {selectedPackages.slice(0, 12).map((item) => (
                <span key={item.name}>
                  <strong>{item.name}</strong>
                  <code>
                    {item.installedVersion} → {item.candidateVersion}
                  </code>
                  {packageNeedsHostReboot(item) && (
                    <small>{restartLabel(item)}</small>
                  )}
                </span>
              ))}
              {selectedPackages.length > 12 && (
                <small>
                  + {selectedPackages.length - 12} more selected packages
                </small>
              )}
            </div>
            {rebootPackages.length > 0 && (
              <div class="package-safety-note package-safety-note--warning">
                <Icon name="warning" size={17} />
                <div>
                  <strong>These updates often need a host reboot</strong>
                  <span>
                    {rebootPackages.map((item) => item.name).join(", ")}. Helix
                    will not reboot Linux. After the job finishes, if Linux
                    asked for a reboot, use Settings → Whole-host reboot when
                    you choose. That stops Helix, players, and every other
                    service until the host is back.
                  </span>
                </div>
              </div>
            )}
            <div class="package-safety-note package-safety-note--warning">
              <Icon name="warning" size={17} />
              <div>
                <strong>
                  Services can restart during package configuration
                </strong>
                <span>
                  Active streams, servers, or other workloads may be
                  interrupted. Existing config files are kept. Helix does not
                  claim it can undo a failed package install, and it will not
                  reboot the host.
                </span>
              </div>
            </div>
            <label class="reboot-acknowledgement">
              <input
                type="checkbox"
                checked={disruptionAcknowledged}
                onChange={(event) =>
                  setDisruptionAcknowledged(event.currentTarget.checked)
                }
              />
              <span>
                <strong>I understand affected services can restart.</strong>
                <small>
                  I have checked the selected packages and current workloads.
                </small>
              </span>
            </label>
            <label class="confirmation-input">
              <span>
                Type <strong>{confirmationPhrase}</strong>
              </span>
              <input
                value={confirmation}
                autocomplete="off"
                spellcheck={false}
                onInput={(event) => setConfirmation(event.currentTarget.value)}
              />
            </label>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              disabled={startingMutation}
              onClick={() => setApplyOpen(false)}
            >
              Cancel
            </button>
            <button
              class="button button--danger"
              type="button"
              disabled={
                startingMutation ||
                confirmation !== confirmationPhrase ||
                !disruptionAcknowledged ||
                selectedPackages.length === 0
              }
              onClick={() => void startApply()}
            >
              {startingMutation
                ? "Starting verified job…"
                : "Apply exact updates"}
            </button>
          </div>
        </Dialog>
      )}
      {helixApplyOpen && data !== null && (
        <Dialog
          title={`Update Helix to ${data.helixSelfUpdate.latestVersion ?? data.helixSelfUpdate.latestTag}?`}
          onClose={() => !startingMutation && setHelixApplyOpen(false)}
          wide
        >
          <div class="package-apply-dialog">
            <div class="package-apply-summary">
              <strong>
                {data.helixSelfUpdate.currentVersion} →{" "}
                {data.helixSelfUpdate.latestVersion}
              </strong>
              <span>
                Helix downloads the SHA-256-pinned GitHub source archive, rebuilds
                only the dashboard and gateway, replaces helix-privd and
                helix-terminald, health-checks, and restores those if the new
                release does not come up. Game containers, AMP, and Plex stay
                running. This is not git pull.
              </span>
            </div>
            {data.helixSelfUpdate.releaseNotes !== null &&
              data.helixSelfUpdate.releaseNotes.length > 0 && (
                <div class="package-apply-preview">
                  <span>{data.helixSelfUpdate.releaseNotes}</span>
                </div>
              )}
            <div class="package-safety-note package-safety-note--warning">
              <Icon name="warning" size={17} />
              <div>
                <strong>The dashboard will disconnect and come back</strong>
                <span>
                  Wait for the new version, then refresh. Linux is not rebooted.
                </span>
              </div>
            </div>
            <label class="reboot-acknowledgement">
              <input
                type="checkbox"
                checked={helixDisruptionAcknowledged}
                onChange={(event) =>
                  setHelixDisruptionAcknowledged(event.currentTarget.checked)
                }
              />
              <span>
                <strong>
                  I understand Helix will restart the dashboard, gateway, and
                  broker.
                </strong>
                <small>Game servers are not replaced by this job.</small>
              </span>
            </label>
            <label class="confirmation-input">
              <span>
                Type{" "}
                <strong>{data.helixSelfUpdate.requiredConfirmation}</strong>
              </span>
              <input
                value={helixConfirmation}
                autocomplete="off"
                spellcheck={false}
                autofocus
                onInput={(event) =>
                  setHelixConfirmation(event.currentTarget.value)
                }
              />
            </label>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              disabled={startingMutation}
              onClick={() => setHelixApplyOpen(false)}
            >
              Cancel
            </button>
            <button
              class="button button--danger"
              type="button"
              disabled={
                startingMutation ||
                helixConfirmation !==
                  data.helixSelfUpdate.requiredConfirmation ||
                !helixDisruptionAcknowledged ||
                data.helixSelfUpdate.latestTag === null
              }
              onClick={() => void startHelixApply()}
            >
              {startingMutation ? "Starting Helix update…" : "Update Helix"}
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
