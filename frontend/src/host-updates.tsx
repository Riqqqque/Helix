import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { ApiError, getHealth } from "./api";
import { InlineError, ProgressBar } from "./dashboard-ui";
import { formatBytes, formatTimestamp } from "./format";
import { HostRebootButton } from "./host-reboot-button";
import { Icon, type IconName } from "./icons";
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
  canPower?: boolean;
  onSessionExpired: () => void;
}

export type PackageFilter = "updates" | "security" | "held" | "all";
const PAGE_SIZE = 75;
const MAX_APPLY_BATCH = 512;
const PACKAGE_JOB_STORAGE_KEY = "helix.package-job";
const MAX_JOB_POLL_FAILURES = 8;

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

function isMissingJobError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    error.status === 400 &&
    /does not exist|invalid job|job id/i.test(error.message)
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
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function restartLabel(item: SystemPackage): string {
  if (item.restartHint === "host_reboot_requested")
    return "Linux asked for a reboot";
  if (item.restartHint === "likely_host_reboot") return "Often needs a reboot";
  if (item.restartHint === "likely_service_restart")
    return "Services may restart";
  if (item.restartImpactKnown) return item.restartHint.replaceAll("_", " ");
  return "Impact unknown";
}

function restartTone(item: SystemPackage): "warning" | "idle" {
  return packageNeedsHostReboot(item) ? "warning" : "idle";
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

function jobUpdatedCount(job: PackageJob): number {
  const updated = asRecord(job.result)?.updated;
  return Array.isArray(updated) ? updated.length : 0;
}

function jobResultNote(job: PackageJob): string | null {
  const note = asRecord(job.result)?.note;
  return typeof note === "string" && note.length > 0 ? note : null;
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

function jobIsActive(job: PackageJob | null): boolean {
  return job !== null && (job.status === "queued" || job.status === "running");
}

interface HeroModel {
  tone: "accent" | "good" | "warning" | "danger";
  icon: IconName;
  title: string;
  detail: string;
}

export function heroModel(
  data: SystemPackageInventory,
  job: PackageJob | null,
  safeCount: number,
  downloadBytes: number,
): HeroModel {
  if (job !== null && (job.status === "queued" || job.status === "running")) {
    return {
      tone: "accent",
      icon: "update",
      title:
        job.kind === "system_package_lists_refresh"
          ? "Checking for updates"
          : job.kind === "helix_release_apply"
            ? "Updating Helix"
            : "Installing Linux updates",
      detail: `${job.stage} · ${job.progressPercent}% · this keeps running if you leave the page`,
    };
  }
  if (job?.status === "failed") {
    return {
      tone: "danger",
      icon: "warning",
      title:
        job.kind === "system_package_lists_refresh"
          ? "The update check did not finish"
          : job.kind === "helix_release_apply"
            ? "The Helix update did not finish"
            : "The update did not finish",
      detail: job.error ?? "The operation did not complete.",
    };
  }
  if (job?.status === "complete" && jobRebootRequired(job)) {
    const names = jobRebootPackages(job);
    return {
      tone: "warning",
      icon: "warning",
      title: "Updates installed — a host reboot is needed",
      detail: `${
        names.length > 0 ? names.join(", ") : "One or more packages"
      } asked Linux for a reboot. Helix never reboots on its own.`,
    };
  }
  if (data.hostRestart.rebootRequiredMarkerPresent) {
    return {
      tone: "warning",
      icon: "warning",
      title: "This host needs a reboot",
      detail: `${
        data.hostRestart.packages.length > 0
          ? data.hostRestart.packages.join(", ")
          : "A previous update"
      } asked Linux for a reboot. Rebooting disconnects Helix, players, and every service until the host is back.`,
    };
  }
  if (job?.status === "complete") {
    return {
      tone: "good",
      icon: "check",
      title:
        job.kind === "system_package_lists_refresh"
          ? "Package lists are up to date"
          : job.kind === "helix_release_apply"
            ? "Helix update staged"
            : `Updates installed${
                jobUpdatedCount(job) > 0
                  ? ` — ${jobUpdatedCount(job)} package${
                      jobUpdatedCount(job) === 1 ? "" : "s"
                    } updated`
                  : ""
              }`,
      detail:
        job.kind === "system_package_lists_refresh"
          ? "Nothing was installed — the signed lists were refreshed."
          : job.kind === "helix_release_apply"
            ? "The dashboard will restart. Refresh after it comes back."
            : (jobResultNote(job) ??
              "Every selected version verified. Linux did not ask for a reboot."),
    };
  }
  if (data.availability !== "ready") {
    return {
      tone: "warning",
      icon: "warning",
      title: "Helix cannot fully read this host's packages",
      detail:
        data.errors[0]?.message ??
        "Some package information is missing. The details are listed below.",
    };
  }
  if (safeCount > 0) {
    return {
      tone: "accent",
      icon: "update",
      title: `${safeCount} update${safeCount === 1 ? "" : "s"} ready to install`,
      detail: `${formatBytes(downloadBytes)} download · ${
        data.inventory.securityUpdateTotal
      } security update${
        data.inventory.securityUpdateTotal === 1 ? "" : "s"
      } · lists from ${formatCacheAge(
        data.aptCacheRefreshedAtUnixMs,
        data.collectedAtUnixMs,
      )}`,
    };
  }
  if (data.inventory.upgradeAvailableTotal > 0) {
    return {
      tone: "warning",
      icon: "warning",
      title: `${data.inventory.upgradeAvailableTotal} update${
        data.inventory.upgradeAvailableTotal === 1 ? "" : "s"
      } need attention`,
      detail:
        "Every pending update is held by APT or missing the details Helix needs to install it safely. They are marked Held in the list.",
    };
  }
  return {
    tone: "good",
    icon: "check",
    title: "Linux is up to date",
    detail: `${data.inventory.installedTotal.toLocaleString()} packages installed · lists from ${formatCacheAge(
      data.aptCacheRefreshedAtUnixMs,
      data.collectedAtUnixMs,
    )}`,
  };
}

export function PackageTable({
  data,
  filter,
  query,
  page,
  onFilter,
  onQuery,
  onPage,
  onRefreshView,
  refreshing,
}: {
  data: SystemPackageInventory;
  filter: PackageFilter;
  query: string;
  page: number;
  onFilter: (filter: PackageFilter) => void;
  onQuery: (query: string) => void;
  onPage: (page: number) => void;
  onRefreshView: () => void;
  refreshing: boolean;
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
  const heldCount = data.inventory.packages.filter(
    (item) => item.held === true,
  ).length;
  return (
    <section class="surface package-inventory">
      <div class="section-title package-inventory-head">
        <div>
          <h2>
            Package list{" "}
            <InfoTip text="Installed versions come from dpkg. Candidate versions, origins, descriptions, and download sizes depend on the current package lists. Held and incomplete packages are excluded from an install run." />
          </h2>
          <p>
            {packages.length.toLocaleString()} matching · collected{" "}
            {formatTimestamp(data.collectedAtUnixMs)}
          </p>
        </div>
        <div class="package-heading-actions">
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
          <button
            class="button button--quiet"
            type="button"
            disabled={refreshing}
            onClick={onRefreshView}
          >
            <Icon name="refresh" size={15} />
            {refreshing ? "Reading…" : "Refresh view"}
          </button>
        </div>
      </div>
      <div class="package-filter-bar" role="group" aria-label="Package filter">
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
              <th>Package</th>
              <th>Update</th>
              <th>Size</th>
              <th>Effect</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((item) => (
              <tr key={item.name}>
                <td>
                  <strong>{item.name}</strong>
                  <small>
                    {item.description || "No package description available."}
                  </small>
                </td>
                <td>
                  {item.upgradeAvailable ? (
                    <>
                      <code>{item.installedVersion}</code>
                      <span aria-hidden="true"> → </span>
                      <code>{item.candidateVersion ?? "unknown"}</code>
                    </>
                  ) : (
                    <code>{item.installedVersion}</code>
                  )}
                  <small>
                    {item.upgradeAvailable
                      ? [item.sourcePackage, item.candidateOrigin]
                          .filter((value): value is string => value !== null)
                          .join(" · ") || "Origin unavailable"
                      : "Installed"}
                  </small>
                </td>
                <td>
                  <strong>
                    {item.downloadSizeBytes === null
                      ? "—"
                      : formatBytes(item.downloadSizeBytes)}
                  </strong>
                  <small>
                    {item.installedSizeBytes === null
                      ? "Installed size unknown"
                      : `${formatBytes(item.installedSizeBytes)} installed`}
                  </small>
                </td>
                <td>
                  <div class="package-signals">
                    {item.securityUpdate === true && (
                      <span class="state-label state-label--warning">
                        Security
                      </span>
                    )}
                    {item.held === true && (
                      <span class="state-label state-label--idle">Held</span>
                    )}
                    {item.upgradeAvailable &&
                      (item.restartImpactKnown ? (
                        <span
                          class={`state-label state-label--${restartTone(item)}`}
                        >
                          {restartLabel(item)}
                        </span>
                      ) : (
                        <small>Impact unknown</small>
                      ))}
                  </div>
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
  );
}

export function HostUpdatesPanel({
  csrfToken,
  canPower = false,
  onSessionExpired,
}: HostUpdatesProps) {
  const [data, setData] = useState<SystemPackageInventory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<PackageFilter>("updates");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [job, setJob] = useState<PackageJob | null>(null);
  const [jobNote, setJobNote] = useState<string | null>(null);
  const [startingMutation, setStartingMutation] = useState(false);
  const [applyOpen, setApplyOpen] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [helixApplyOpen, setHelixApplyOpen] = useState(false);
  const [helixError, setHelixError] = useState<string | null>(null);
  const [waitingForHelix, setWaitingForHelix] = useState(false);
  const [disruptionAcknowledged, setDisruptionAcknowledged] = useState(false);
  const [helixDisruptionAcknowledged, setHelixDisruptionAcknowledged] =
    useState(false);
  const githubChecked = useRef(false);
  const helixRestartFromVersion = useRef<string | null>(null);
  const pollFailures = useRef(0);

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

  const forgetJob = useCallback((): void => {
    pollFailures.current = 0;
    setJob(null);
    try {
      localStorage.removeItem(PACKAGE_JOB_STORAGE_KEY);
    } catch {
      /* Storage is optional. */
    }
  }, []);

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
      .then((restored) => {
        if (controller.signal.aborted) return;
        if (restored.status === "queued" || restored.status === "running") {
          setJob(restored);
        }
      })
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
    if (
      job === null ||
      (job.status !== "queued" && job.status !== "running")
    )
      return;
    const active = job;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void getSystemPackageJob(active.id, csrfToken, controller.signal)
        .then((next) => {
          if (controller.signal.aborted) return;
          pollFailures.current = 0;
          setJob(next);
          if (next.status === "complete" || next.status === "failed") {
            try {
              localStorage.removeItem(PACKAGE_JOB_STORAGE_KEY);
            } catch {
              /* Storage is optional. */
            }
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
          if (isSessionError(requestError)) {
            onSessionExpired();
            return;
          }
          if (isMissingJobError(requestError)) {
            forgetJob();
            setJobNote(
              "Helix lost track of that update job — the service may have restarted while it ran. Check the package list to see what landed before trying again.",
            );
            return;
          }
          pollFailures.current += 1;
          if (pollFailures.current >= MAX_JOB_POLL_FAILURES) {
            forgetJob();
            setJobNote(
              "Helix could not reach the update job. Check the package list to see what landed before trying again.",
            );
          }
        });
    }, 1_500);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [csrfToken, data, job, load, onSessionExpired, forgetJob]);

  const safeUpdates = useMemo(
    () => data?.inventory.packages.filter(selectableUpdate) ?? [],
    [data],
  );
  const applyBatch = useMemo(
    () => safeUpdates.slice(0, MAX_APPLY_BATCH),
    [safeUpdates],
  );
  const downloadBytes = useMemo(
    () =>
      applyBatch.reduce(
        (total, item) => total + (item.downloadSizeBytes ?? 0),
        0,
      ),
    [applyBatch],
  );
  const rebootRiskPackages = useMemo(
    () => applyBatch.filter(packageNeedsHostReboot),
    [applyBatch],
  );
  const excludedCount = Math.max(
    0,
    (data?.inventory.upgradeAvailableTotal ?? 0) - safeUpdates.length,
  );
  const rebootNeeded =
    data?.hostRestart.rebootRequiredMarkerPresent === true ||
    (job !== null && jobRebootRequired(job));
  const mutationBusy =
    startingMutation || waitingForHelix || jobIsActive(job);
  const applyBlocked =
    data === null ||
    !data.upgradeApply.available ||
    !data.simulation.available ||
    applyBatch.length === 0;
  const applyBlockReason =
    data === null
      ? null
      : !data.upgradeApply.available
        ? data.upgradeApply.reason
        : !data.simulation.available
          ? (data.simulation.error ??
            "Helix cannot preview what APT would change.")
          : applyBatch.length === 0
            ? "There is nothing to install."
            : null;

  const rememberJob = (id: string, kind: string, stage: string): void => {
    const now = Date.now();
    pollFailures.current = 0;
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
    if (mutationBusy) return;
    setStartingMutation(true);
    setError(null);
    setJobNote(null);
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
    if (applyBlocked || !disruptionAcknowledged || mutationBusy) return;
    setStartingMutation(true);
    setApplyError(null);
    setJobNote(null);
    try {
      const dispatched = await applySystemPackageUpdates(
        applyBatch,
        expectedConfirmation(applyBatch.length),
        disruptionAcknowledged,
        csrfToken,
      );
      rememberJob(
        dispatched.jobId,
        "system_package_apply",
        `Queued to verify ${applyBatch.length} package version${applyBatch.length === 1 ? "" : "s"}`,
      );
      setApplyOpen(false);
      setDisruptionAcknowledged(false);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setApplyError(describeError(requestError));
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
      !helixDisruptionAcknowledged ||
      mutationBusy
    )
      return;
    setStartingMutation(true);
    setHelixError(null);
    setJobNote(null);
    try {
      const dispatched = await applyHelixUpdate(
        data.helixSelfUpdate.latestTag,
        data.helixSelfUpdate.requiredConfirmation,
        helixDisruptionAcknowledged,
        csrfToken,
      );
      rememberJob(
        dispatched.jobId,
        "helix_release_apply",
        "Queued to download a digest-pinned Helix release",
      );
      setHelixApplyOpen(false);
      setHelixDisruptionAcknowledged(false);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setHelixError(describeError(requestError));
    } finally {
      setStartingMutation(false);
    }
  };

  const hero = data !== null ? heroModel(data, job, applyBatch.length, downloadBytes) : null;
  const cacheStale =
    data !== null &&
    cacheLooksStale(data.aptCacheRefreshedAtUnixMs, data.collectedAtUnixMs);
  const securityCount = applyBatch.filter(
    (item) => item.securityUpdate === true,
  ).length;
  const rebootControl =
    canPower === true ? (
      <HostRebootButton
        csrfToken={csrfToken}
        disabled={mutationBusy}
        onSessionExpired={onSessionExpired}
      />
    ) : null;

  return (
    <div class="infrastructure-panel" aria-busy={loading}>
      <InlineError message={error} />
      {waitingForHelix ? (
        <section
          class="update-hero update-hero--accent"
          aria-live="polite"
        >
          <div class="update-hero-icon">
            <Icon name="update" size={22} />
          </div>
          <div class="update-hero-body">
            <strong>Helix is restarting</strong>
            <span>
              This page reloads when the new dashboard answers. Game containers
              stay running.
            </span>
          </div>
        </section>
      ) : data === null ? (
        <section class="update-hero update-hero--idle">
          <div class="update-hero-icon">
            <Icon name={error === null ? "update" : "warning"} size={22} />
          </div>
          <div class="update-hero-body">
            <strong>
              {error === null
                ? "Reading installed packages and update lists…"
                : "Linux updates are unavailable"}
            </strong>
            <span>
              {error === null
                ? "This usually takes a few seconds."
                : "The package inventory could not be loaded."}
            </span>
          </div>
          {error !== null && (
            <div class="update-hero-actions">
              <button
                class="button button--primary"
                type="button"
                onClick={() => void load()}
              >
                <Icon name="refresh" size={15} />
                Try again
              </button>
            </div>
          )}
        </section>
      ) : (
        hero !== null && (
          <section
            class={`update-hero update-hero--${hero.tone}`}
            aria-live="polite"
          >
            <div class="update-hero-icon">
              <Icon name={hero.icon} size={22} />
            </div>
            <div class="update-hero-body">
              <strong>{hero.title}</strong>
              <span>{hero.detail}</span>
              {job !== null &&
                (job.status === "queued" || job.status === "running") && (
                  <ProgressBar
                    value={Math.max(2, job.progressPercent)}
                    tone="normal"
                  />
                )}
              {job?.status === "failed" && (
                <span class="update-hero-hint">
                  {job.kind === "system_package_lists_refresh"
                    ? "Nothing was installed — the lists could not be refreshed. Try again."
                    : job.kind === "system_package_apply"
                      ? "APT may have changed some packages before it stopped — check the list below to see the current state."
                      : "Check the details and try again."}
                </span>
              )}
              {cacheStale &&
                !jobIsActive(job) &&
                job?.status !== "failed" && (
                  <span class="update-hero-hint">
                    The package lists are more than a day old — check for
                    updates before installing.
                  </span>
                )}
              {applyBatch.length > 0 &&
                applyBlocked &&
                applyBlockReason !== null && (
                  <span class="update-hero-hint">{applyBlockReason}</span>
                )}
            </div>
            {!jobIsActive(job) && (
            <div class="update-hero-actions">
              {job?.status === "complete" || job?.status === "failed" ? (
                <>
                  {job.status === "complete" &&
                    jobRebootRequired(job) &&
                    rebootControl}
                  <button
                    class="button button--quiet"
                    type="button"
                    onClick={forgetJob}
                  >
                    Dismiss
                  </button>
                </>
              ) : (
                <>
                  {rebootNeeded && rebootControl}
                  {applyBatch.length > 0 && (
                    <button
                      class="button button--primary"
                      type="button"
                      disabled={mutationBusy || applyBlocked}
                      title={applyBlockReason ?? undefined}
                      onClick={() => {
                        setDisruptionAcknowledged(false);
                        setApplyError(null);
                        setApplyOpen(true);
                      }}
                    >
                      <Icon name="update" size={15} />
                      Install {applyBatch.length} update
                      {applyBatch.length === 1 ? "" : "s"} ·{" "}
                      {formatBytes(downloadBytes)}
                    </button>
                  )}
                  <button
                    class="button button--quiet"
                    type="button"
                    disabled={
                      mutationBusy ||
                      data.upgradeApply.packageListsRefreshAvailable !== true
                    }
                    title={
                      data.upgradeApply.packageListsRefreshAvailable
                        ? "Refresh the signed package lists"
                        : "Checking for updates is unavailable on this host"
                    }
                    onClick={() => void startRefresh()}
                  >
                    <Icon name="refresh" size={15} />
                    {job?.kind === "system_package_lists_refresh" &&
                    jobIsActive(job)
                      ? "Checking…"
                      : "Check for updates"}
                  </button>
                  {!rebootNeeded && rebootControl}
                </>
              )}
            </div>
            )}
          </section>
        )
      )}
      {jobNote !== null && (
        <div class="package-safety-note package-safety-note--warning">
          <Icon name="warning" size={17} />
          <div>
            <strong>Update job could not be followed</strong>
            <span>{jobNote}</span>
          </div>
          <button
            class="button button--quiet"
            type="button"
            onClick={() => setJobNote(null)}
          >
            Dismiss
          </button>
        </div>
      )}
      {data !== null && (
        <>
          <PackageTable
            data={data}
            filter={filter}
            query={query}
            page={page}
            onFilter={setFilter}
            onQuery={setQuery}
            onPage={setPage}
            refreshing={loading}
            onRefreshView={() => void load()}
          />
          <section class="surface helix-update-line">
            <div class="helix-update-copy">
              <Icon name="update" size={15} />
              <div>
                <strong>
                  {data.helixSelfUpdate.updateAvailable &&
                  data.helixSelfUpdate.latestVersion !== null
                    ? `Helix ${data.helixSelfUpdate.latestVersion} is available`
                    : `Helix ${data.helixSelfUpdate.currentVersion}`}
                </strong>
                <span>{data.helixSelfUpdate.reason}</span>
              </div>
            </div>
            <div class="helix-update-actions">
              {data.helixSelfUpdate.releaseUrl !== null && (
                <a
                  class="button button--quiet"
                  href={data.helixSelfUpdate.releaseUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  <Icon name="external" size={14} />
                  Release notes
                </a>
              )}
              <button
                class="button button--quiet"
                type="button"
                disabled={mutationBusy}
                onClick={() => void startCheckHelix()}
              >
                Check GitHub
              </button>
              {data.helixSelfUpdate.available && (
                <button
                  class="button button--primary"
                  type="button"
                  disabled={mutationBusy}
                  onClick={() => {
                    setHelixDisruptionAcknowledged(false);
                    setHelixError(null);
                    setHelixApplyOpen(true);
                  }}
                >
                  <Icon name="update" size={15} />
                  {data.helixSelfUpdate.updateAvailable &&
                  data.helixSelfUpdate.latestVersion !== null
                    ? `Update to ${data.helixSelfUpdate.latestVersion}`
                    : "Update Helix"}
                </button>
              )}
            </div>
          </section>
          {(data.errors.length > 0 || data.availability !== "ready") && (
            <section class="surface package-diagnostics">
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
              {data.errors.map((item) => (
                <p
                  class="table-note"
                  key={`${item.component}-${item.message}`}
                >
                  <strong>{errorComponentLabel(item.component)}</strong>:{" "}
                  {item.message}
                </p>
              ))}
            </section>
          )}
        </>
      )}
      {applyOpen && data !== null && (
        <Dialog
          title={`Install ${applyBatch.length} update${applyBatch.length === 1 ? "" : "s"}?`}
          onClose={() => !startingMutation && setApplyOpen(false)}
          wide
        >
          <div class="package-apply-dialog">
            <InlineError message={applyError} />
            <div class="package-apply-summary">
              <strong>{formatBytes(downloadBytes)} to download</strong>
              <span>
                Helix installs the exact versions listed below. Versions, held
                packages, disk space, and the change preview are rechecked
                immediately before APT runs — if anything changed, the job
                stops instead of guessing.
              </span>
            </div>
            <div class="package-apply-preview">
              {applyBatch.slice(0, 12).map((item) => (
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
              {applyBatch.length > 12 && (
                <small>+ {applyBatch.length - 12} more packages</small>
              )}
            </div>
            <ul class="package-apply-effects">
              <li>
                {securityCount > 0
                  ? `${securityCount} of ${applyBatch.length} updates come from a security archive.`
                  : "None of these updates are marked as security updates."}
              </li>
              <li>
                Package services can restart while they are configured. Active
                streams, game servers, and other workloads may be interrupted.
              </li>
              <li>
                Existing config files are kept. No new packages are added and
                nothing is removed.
              </li>
              <li>
                Helix never reboots Linux — if a reboot is needed, this page
                says so after the job finishes.
              </li>
              {excludedCount > 0 && (
                <li>
                  {excludedCount} pending update
                  {excludedCount === 1 ? "" : "s"} will not be installed —
                  {excludedCount === 1 ? " it is" : " they are"} held by APT or
                  missing details Helix needs.
                </li>
              )}
              {safeUpdates.length > MAX_APPLY_BATCH && (
                <li>
                  Only the first {MAX_APPLY_BATCH} updates install in this run.
                  Run again afterwards for the rest.
                </li>
              )}
            </ul>
            {rebootRiskPackages.length > 0 && (
              <div class="package-safety-note package-safety-note--warning">
                <Icon name="warning" size={17} />
                <div>
                  <strong>These updates often need a host reboot</strong>
                  <span>
                    {rebootRiskPackages.map((item) => item.name).join(", ")}.
                    Helix will not reboot. If Linux asks for one afterwards,
                    the reboot control appears on this page.
                  </span>
                </div>
              </div>
            )}
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
                  The updates above and their effects look right to me.
                </small>
              </span>
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
              class="button button--primary"
              type="button"
              disabled={startingMutation || !disruptionAcknowledged}
              onClick={() => void startApply()}
            >
              {startingMutation
                ? "Starting verified job…"
                : `Install ${applyBatch.length} update${applyBatch.length === 1 ? "" : "s"}`}
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
            <InlineError message={helixError} />
            <div class="package-apply-summary">
              <strong>
                {data.helixSelfUpdate.currentVersion} →{" "}
                {data.helixSelfUpdate.latestVersion}
              </strong>
              <span>
                Helix downloads the SHA-256-pinned GitHub source archive,
                rebuilds only the dashboard and gateway, replaces helix-privd
                and helix-terminald, health-checks, and restores those if the
                new release does not come up. Game containers, AMP, and Plex
                stay running. This is not git pull.
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
                  Wait for the new version, then refresh. Linux is not
                  rebooted.
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
