import { useCallback, useEffect, useMemo, useState } from "preact/hooks";
import { ApiError } from "./api";
import { InlineError } from "./dashboard-ui";
import { formatBytes, formatTimestamp } from "./format";
import { Icon } from "./icons";
import { InfoTip } from "./info-tip";
import { Dialog } from "./modal";
import {
  applySystemPackageUpdates,
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
    : "Helix could not read the package inventory.";
}

function isSessionError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.status === 401 || error.code === "csrf_rejected")
  );
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
  if (item.restartHint === "host_reboot_requested")
    return "Host reboot requested";
  if (item.restartImpactKnown) return item.restartHint.replaceAll("_", " ");
  return "Impact unknown";
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
  return (
    <>
      <section class="update-summary-grid" aria-label="Package update summary">
        <article>
          <span>
            Installed{" "}
            <InfoTip text="Packages currently reported by dpkg. A bounded inventory can be truncated on unusually large hosts." />
          </span>
          <strong>{data.inventory.installedTotal.toLocaleString()}</strong>
          <small>
            {data.inventory.truncated
              ? "Inventory truncated"
              : "dpkg installed packages"}
          </small>
        </article>
        <article>
          <span>
            Updates{" "}
            <InfoTip text="Installed and candidate versions differ in the current APT metadata. The candidate can change after the next package-list refresh." />
          </span>
          <strong>{data.inventory.upgradeAvailableTotal}</strong>
          <small>Candidate version differs</small>
        </article>
        <article>
          <span>
            Security{" "}
            <InfoTip text="Best-effort classification from the candidate package origin. Unknown is kept separate from false." />
          </span>
          <strong
            class={
              data.inventory.securityUpdateTotal > 0 ? "update-accent" : ""
            }
          >
            {data.inventory.securityUpdateTotal}
          </strong>
          <small>Security-origin candidates</small>
        </article>
        <article>
          <span>
            APT cache age{" "}
            <InfoTip text="Helix only reads the newest package-list timestamp here. Opening or refreshing this page never runs apt update." />
          </span>
          <strong>{cacheAge}</strong>
          <small>
            {data.aptCacheRefreshedAtUnixMs === null
              ? "No cache timestamp found"
              : formatTimestamp(data.aptCacheRefreshedAtUnixMs)}
          </small>
        </article>
      </section>

      <div
        class={`package-safety-note ${data.hostRestart.rebootRequiredMarkerPresent ? "package-safety-note--warning" : ""}`}
      >
        <Icon
          name={
            data.hostRestart.rebootRequiredMarkerPresent ? "warning" : "info"
          }
          size={17}
        />
        <div>
          <strong>
            {data.hostRestart.rebootRequiredMarkerPresent
              ? "Linux reports a host reboot is pending"
              : "Nothing changes until you confirm it"}
          </strong>
          <span>
            {data.hostRestart.rebootRequiredMarkerPresent
              ? `${data.hostRestart.packages.length > 0 ? data.hostRestart.packages.join(", ") : "One or more prior updates"} requested a reboot. Helix never reboots Linux automatically.`
              : "Reading and filtering this inventory is non-mutating. Check for updates refreshes APT metadata; applying updates uses only the exact packages you select."}
          </span>
        </div>
      </div>

      <section class="surface infrastructure-section package-readiness">
        <div class="section-title">
          <div>
            <h2>
              Update readiness{" "}
              <InfoTip text="A safe update job must revalidate exact package candidates, dpkg locks, disk space, conffile policy, and workload impact immediately before applying anything." />
            </h2>
            <p>Read-only simulation and explicit safety gates</p>
          </div>
          <span
            class={`state-label state-label--${data.simulation.available ? "good" : "warning"}`}
          >
            {data.simulation.available
              ? "Simulation ready"
              : "Simulation unavailable"}
          </span>
        </div>
        <div class="update-readiness-grid">
          <article>
            <span>Simulation</span>
            <strong>
              {data.simulation.upgradeCandidates} upgrades ·{" "}
              {data.simulation.newPackages} new
            </strong>
            <small>
              {data.simulation.removals} removals · {data.simulation.heldBack}{" "}
              held back
            </small>
            {data.simulation.error !== null && <p>{data.simulation.error}</p>}
          </article>
          <article>
            <span>Host packages</span>
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
            <span>Helix itself</span>
            <strong>Self-update unavailable</strong>
            <small>{data.helixSelfUpdate.reason}</small>
            <button class="button" type="button" disabled>
              <Icon name="update" size={15} />
              Update Helix
            </button>
          </article>
        </div>
        <p class="readiness-footnote">
          Package rollback is not claimed. Existing conffiles are kept, held
          packages and removals are refused, exact candidates are rechecked, and
          no update can trigger an automatic reboot.
        </p>
      </section>

      <section class="surface infrastructure-section package-inventory">
        <div class="section-title package-inventory-head">
          <div>
            <h2>
              Package inventory{" "}
              <InfoTip text="Installed versions come from dpkg. Candidate versions, origins, descriptions, and download sizes depend on the available APT metadata and can be unknown." />
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
                      class={`state-label state-label--${item.restartHint === "host_reboot_requested" ? "warning" : "idle"}`}
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
              <h2>Inventory diagnostics</h2>
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
                {tool.replace(/([A-Z])/gu, " $1")}
              </span>
            ))}
          </div>
          {data.errors.map((error) => (
            <p class="table-note" key={`${error.component}-${error.message}`}>
              <strong>{error.component}</strong>: {error.message}
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
  const [confirmation, setConfirmation] = useState("");
  const [disruptionAcknowledged, setDisruptionAcknowledged] = useState(false);

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
            void load();
          }
        })
        .catch((requestError: unknown) => {
          if (controller.signal.aborted) return;
          if (isSessionError(requestError)) onSessionExpired();
          else setError(describeError(requestError));
        });
    }, 1_000);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [csrfToken, job, load, onSessionExpired]);

  const selectedPackages = useMemo(
    () =>
      data?.inventory.packages.filter(
        (item) => selected.has(item.name) && selectableUpdate(item),
      ) ?? [],
    [data, selected],
  );
  const mutationBusy =
    startingMutation || job?.status === "queued" || job?.status === "running";
  const confirmationPhrase = expectedConfirmation(selectedPackages.length);

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
        "Queued to revalidate exact candidates",
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
            System updates{" "}
            <InfoTip text="Helix separates a read-only inventory refresh from APT's package-list refresh and from an explicitly confirmed package apply. Package services may restart; Linux never reboots automatically." />
          </h2>
          <p>
            Exact package candidates, visible safety gates, no surprise reboot
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
      {job !== null && (
        <section
          class={`package-job-banner package-job-banner--${job.status}`}
          aria-live="polite"
        >
          <Icon
            name={
              job.status === "failed"
                ? "warning"
                : job.status === "complete"
                  ? "check"
                  : "update"
            }
            size={18}
          />
          <div>
            <strong>
              {job.status === "complete"
                ? "Package operation complete"
                : job.status === "failed"
                  ? "Package operation stopped safely"
                  : job.stage}
            </strong>
            <span>
              {job.status === "failed"
                ? (job.error ?? "The package operation did not complete.")
                : job.status === "complete"
                  ? job.stage
                  : `${job.progressPercent}% · The broker keeps this running if you leave the page.`}
            </span>
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
              ? "Reading dpkg and APT metadata…"
              : "Package inventory is unavailable."}
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
                packages, new packages, removals, candidate drift, and
                insufficient disk headroom stop the job.
              </span>
            </div>
            <div class="package-apply-preview">
              {selectedPackages.slice(0, 12).map((item) => (
                <span key={item.name}>
                  <strong>{item.name}</strong>
                  <code>
                    {item.installedVersion} → {item.candidateVersion}
                  </code>
                </span>
              ))}
              {selectedPackages.length > 12 && (
                <small>
                  + {selectedPackages.length - 12} more selected packages
                </small>
              )}
            </div>
            <div class="package-safety-note package-safety-note--warning">
              <Icon name="warning" size={17} />
              <div>
                <strong>
                  Services can restart during package configuration
                </strong>
                <span>
                  Active streams, servers, or other workloads may be
                  interrupted. Helix preserves existing conffiles, claims no
                  package rollback, and will not reboot the host.
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
    </div>
  );
}
