import type { ComponentChildren } from "preact";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "preact/hooks";
import { ApiError } from "./api";
import {
  createMinecraftServer,
  getDirectory,
  getTrashedNativeServers,
  getServerBackups,
  getServerDetail,
  getServerLogs,
  getMinecraftPortPolicy,
  restoreServerBackup,
  restoreTrashedServerBackup,
  restoreTrashedNativeServer,
  runServerAction,
  saveServerSettings,
  sendConsoleCommand,
  saveMinecraftPortPolicy,
  setServerNetworkExposure,
  trashServerBackup,
  trashNativeServer,
  type BrokerJob,
  type DirectoryListing,
  type HostInventory,
  type GamePortPolicy,
  type GamePortRange,
  type ManagedServer,
  type MinecraftSettings,
  type MinecraftSettingField,
  type MinecraftCreateInput,
  type MinecraftSoftware,
  type NativeServerDetail,
  type ServerAction,
  type ServerBackup,
  type ServerBackupTrash,
  type ServerBackupTrashPolicy,
  type ServerLogSnapshot,
  type TrashedNativeServerCatalog,
} from "./control-api";
import type { DashboardData } from "./dashboard-model";
import type { RefreshIntervalMs } from "./dashboard-preferences";
import {
  InlineError,
  Metric,
  PageHead,
  ProgressBar,
  toneForPercent,
} from "./dashboard-ui";
import { FileManager } from "./file-manager";
import {
  formatBytes,
  formatDuration,
  formatPercent,
  formatTimestamp,
} from "./format";
import { Icon, type IconName } from "./icons";
import { InfoTip } from "./info-tip";
import { useJobPolling } from "./job-polling";
import {
  getNetworkInventory,
  type GamePortMapping,
  type NetworkInventory,
} from "./network-api";
import { MarketplaceRoute, preloadMarketplaceRoute } from "./marketplace-route";
import {
  createMinecraftModpack,
  parseMinecraftModpackCreateResult,
  type MinecraftModpackCreateResult,
  type ModpackSelection,
} from "./modpack-api";
import { ModpackRoute, preloadModpackPicker } from "./modpack-route";
import { Dialog } from "./modal";
import { ServerArtwork, ServerIconDialog } from "./server-artwork";
import {
  consoleHistoryEntryKey,
  getServerLogHistory,
  mergeLatestConsoleHistory,
  prependOlderConsoleHistory,
  type ConsoleHistoryEntry,
  type ConsoleHistoryPage,
} from "./server-history-api";
import {
  getServerManagerReadiness,
  type InstallableMinecraftSoftware,
  type ServerManagerReadiness,
} from "./server-manager-api";

function describeError(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Helix could not complete that request.";
}

function isSessionError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.status === 401 || error.code === "csrf_rejected")
  );
}

export const minecraftCreateSoftwareOptions: ReadonlyArray<{
  id: InstallableMinecraftSoftware;
  name: string;
  detail: string;
}> = [
  { id: "paper", name: "Paper", detail: "Fast, plugin-ready, best default" },
  { id: "purpur", name: "Purpur", detail: "Paper with deeper gameplay tuning" },
  {
    id: "folia",
    name: "Folia",
    detail: "Region-threaded Paper for compatible high-concurrency worlds",
  },
  {
    id: "fabric",
    name: "Fabric",
    detail: "Lightweight mod loader for Fabric server mods",
  },
  { id: "vanilla", name: "Vanilla", detail: "Official Mojang server" },
];

function nextMinecraftPort(servers: ManagedServer[]): number {
  const used = new Set(
    servers.flatMap((server) =>
      server.gamePort === null ? [] : [server.gamePort],
    ),
  );
  for (let port = 25_565; port <= 25_599; port += 1)
    if (!used.has(port)) return port;
  return 25_600;
}

function MinecraftSoftwarePicker({
  selected,
  readiness,
  readinessError,
  catalogOpen,
  onSelect,
  onCatalogToggle,
}: {
  selected: MinecraftSoftware;
  readiness: ServerManagerReadiness | null;
  readinessError: string | null;
  catalogOpen: boolean;
  onSelect: (software: MinecraftSoftware) => void;
  onCatalogToggle: () => void;
}) {
  const supported = new Set(
    readiness?.availability === "ready"
      ? readiness.supportedMinecraftSoftware
      : [],
  );
  const catalog =
    readiness?.availability === "ready"
      ? readiness.minecraftSoftwareCatalog
      : [];
  return (
    <>
      <fieldset class="software-picker field--wide">
        <legend>Server software</legend>
        {minecraftCreateSoftwareOptions.map((option) => {
          const available = supported.has(option.id);
          return (
            <label
              class={`${selected === option.id ? "is-selected" : ""}${available ? "" : " is-disabled"}`}
              key={option.id}
            >
              <input
                type="radio"
                name="software"
                value={option.id}
                checked={selected === option.id}
                disabled={!available}
                onChange={() => onSelect(option.id)}
              />
              <span>
                <strong>{option.name}</strong>
                <small>
                  {available
                    ? option.detail
                    : readiness === null
                      ? "Checking host support…"
                      : "Unavailable on this host"}
                </small>
              </span>
              <i />
            </label>
          );
        })}
      </fieldset>
      <div
        class={`software-readiness-note field--wide ${readinessError === null ? "" : "is-error"}`}
        role={readinessError === null ? "status" : "alert"}
      >
        <span>
          {readinessError ??
            (readiness === null
              ? "Checking the native manager before creation…"
              : `${supported.size} one-click software choices are ready on this host.`)}
        </span>
        <button
          type="button"
          disabled={catalog.length === 0}
          onClick={onCatalogToggle}
        >
          {catalogOpen ? "Close software guide" : "Explore server software"}
        </button>
      </div>
      {catalogOpen && catalog.length > 0 && (
        <section class="software-catalog-view field--wide">
          <header>
            <div>
              <strong>Server software guide</strong>
              <span>
                Helix only enables choices with a tested install and update
                path.
              </span>
            </div>
            <small>{catalog.length} explained</small>
          </header>
          <div>
            {catalog.map((entry) => {
              const option = minecraftCreateSoftwareOptions.find(
                (item) => item.id === entry.id,
              );
              const selectable =
                entry.installable &&
                option !== undefined &&
                supported.has(option.id);
              return (
                <article class={selectable ? "is-ready" : ""} key={entry.id}>
                  <div>
                    <strong>
                      {entry.name}
                      {entry.recommended && <small>Recommended</small>}
                    </strong>
                    <span>{entry.kind.replaceAll("_", " ")}</span>
                  </div>
                  <p>{entry.appeal}</p>
                  <small>{entry.note}</small>
                  <footer>
                    <span>{entry.status.replaceAll("_", " ")}</span>
                    <button
                      type="button"
                      disabled={!selectable}
                      onClick={() => {
                        if (option !== undefined) onSelect(option.id);
                      }}
                    >
                      {selectable
                        ? selected === entry.id
                          ? "Selected"
                          : "Choose"
                        : "Not available"}
                    </button>
                  </footer>
                </article>
              );
            })}
          </div>
        </section>
      )}
    </>
  );
}

type MinecraftCreateMode = "software" | "modpack" | "custom";

function CustomJarBrowser({
  csrfToken,
  selectedPath,
  onSelect,
  onClose,
  onSessionExpired,
}: {
  csrfToken: string;
  selectedPath: string;
  onSelect: (path: string) => void;
  onClose: () => void;
  onSessionExpired: () => void;
}) {
  const [path, setPath] = useState("/");
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(0);
  const [cursors, setCursors] = useState<Array<string | null>>([null]);
  const activeLoad = useRef<AbortController | null>(null);

  const load = useCallback(
    async (
      nextPath: string,
      cursor: string | null = null,
      nextPage = 0,
    ): Promise<void> => {
      activeLoad.current?.abort();
      const controller = new AbortController();
      activeLoad.current = controller;
      setLoading(true);
      setError(null);
      try {
        const result = await getDirectory(
          nextPath,
          csrfToken,
          cursor,
          100,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setListing(result);
        setPath(result.path);
        setPage(nextPage);
      } catch (requestError) {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      } finally {
        if (activeLoad.current === controller) {
          activeLoad.current = null;
          setLoading(false);
        }
      }
    },
    [csrfToken, onSessionExpired],
  );

  const navigate = useCallback(
    (nextPath: string): void => {
      setCursors([null]);
      void load(nextPath);
    },
    [load],
  );

  useEffect(() => {
    void load("/");
    return () => activeLoad.current?.abort();
  }, [load]);

  const entries =
    listing?.entries.filter(
      (entry) =>
        entry.kind === "directory" ||
        (entry.kind === "file" && /\.jar$/iu.test(entry.name)),
    ) ?? [];
  const crumbs =
    path === "/" ? ["/"] : ["/", ...path.split("/").filter(Boolean)];
  const showPrevious = page > 0;
  const showNext = listing?.hasMore === true && listing.nextCursor !== null;

  const nextPage = (): void => {
    if (!showNext || listing?.nextCursor === null || listing === null) return;
    const nextCursor = listing.nextCursor;
    const nextPageIndex = page + 1;
    setCursors((current) => [
      ...current.slice(0, nextPageIndex),
      nextCursor,
    ]);
    void load(path, nextCursor, nextPageIndex);
  };

  const previousPage = (): void => {
    if (!showPrevious) return;
    const previousPageIndex = page - 1;
    void load(path, cursors[previousPageIndex] ?? null, previousPageIndex);
  };

  return (
    <section
      class="custom-jar-browser"
      aria-busy={loading}
      aria-label="Choose a server JAR from Storage"
    >
      <header>
        <div>
          <strong>Choose from Storage</strong>
          <span>Folders and runnable JAR files only</span>
        </div>
        <button
          class="icon-button"
          type="button"
          onClick={onClose}
          aria-label="Close Storage browser"
        >
          <Icon name="close" size={16} />
        </button>
      </header>
      <div class="custom-jar-browser__crumbs" aria-label="Current folder">
        {crumbs.map((crumb, index) => {
          const target =
            index === 0
              ? "/"
              : `/${crumbs.slice(1, index + 1).join("/")}`;
          return (
            <span key={`${crumb}-${index}`}>
              <button type="button" onClick={() => navigate(target)}>
                {crumb}
              </button>
              {index < crumbs.length - 1 && (
                <Icon name="chevron" size={11} />
              )}
            </span>
          );
        })}
      </div>
      <InlineError message={error} />
      <div class="custom-jar-browser__list">
        {listing?.parent !== null && listing !== null && (
          <button
            type="button"
            onClick={() =>
              listing.parent !== null && navigate(listing.parent)
            }
          >
            <Icon name="folder" size={17} />
            <span>
              <strong>..</strong>
              <small>Parent folder</small>
            </span>
            <Icon name="chevron" size={13} />
          </button>
        )}
        {entries.map((entry) =>
          entry.kind === "directory" ? (
            <button
              type="button"
              key={entry.path}
              onClick={() => navigate(entry.path)}
            >
              <Icon name="folder" size={17} />
              <span>
                <strong>{entry.name}</strong>
                <small>Folder</small>
              </span>
              <Icon name="chevron" size={13} />
            </button>
          ) : (
            <button
              class={selectedPath === entry.path ? "is-selected" : ""}
              type="button"
              key={entry.path}
              onClick={() => onSelect(entry.path)}
            >
              <Icon name="file" size={17} />
              <span>
                <strong>{entry.name}</strong>
                <small>{formatBytes(entry.sizeBytes)} · Server JAR</small>
              </span>
              <Icon
                name={selectedPath === entry.path ? "check" : "plus"}
                size={13}
              />
            </button>
          ),
        )}
        {loading && (
          <div class="custom-jar-browser__state">
            <Icon name="refresh" class="is-spinning" size={16} />
            Reading {path}…
          </div>
        )}
        {!loading && entries.length === 0 && (
          <div class="custom-jar-browser__state">
            No folders or JAR files on this page.
          </div>
        )}
      </div>
      <footer>
        <span>
          Page {page + 1} · {listing?.totalEntries.toLocaleString() ?? 0} total
          items
        </span>
        <div>
          <button
            class="button button--quiet"
            type="button"
            disabled={loading || !showPrevious}
            onClick={previousPage}
          >
            Previous
          </button>
          <button
            class="button button--quiet"
            type="button"
            disabled={loading || !showNext}
            onClick={nextPage}
          >
            Next
          </button>
        </div>
      </footer>
      <small>
        Helix still verifies the selected path against its configured Storage
        roots before copying anything.
      </small>
    </section>
  );
}

export function serverActionDescription(
  server: ManagedServer,
  action: ServerAction,
): string {
  if (server.manager === "amp_import") {
    if (action === "start")
      return `Helix will ask AMP to start ${server.name} and wait for AMP to report the instance online.`;
    if (action === "restart")
      return `Helix will ask AMP to restart ${server.name} and wait for AMP to report the instance online.`;
    if (action === "stop")
      return `Helix will ask AMP to stop ${server.name}. Connected players will be disconnected.`;
  }
  if (action === "start")
    return "Helix will start Minecraft and wait until it answers a health check.";
  if (action === "stop")
    return "Players will be disconnected after a clean shutdown.";
  if (action === "restart")
    return "The server will stop, start, and pass a Minecraft health check before this finishes.";
  if (action === "update") {
    return server.status === "online"
      ? "Helix will stop the server, back it up, stage and verify the new build, then restart and health-check Minecraft. If validation fails, Helix puts the old build back."
      : "Helix will back up and stage the verified build while keeping this server stopped. Its first later startup is not automatically health-validated or rolled back.";
  }
  return `Helix will stop ${server.name} briefly, archive a consistent copy, then bring it back online.`;
}

export type BackupMutation = "create" | "restore" | "trash" | "undo";

export function canRunBackupMutation(
  mutation: BackupMutation,
  canManageServers: boolean,
  canManageBackups: boolean,
  recoverableTrashAvailable: boolean,
): boolean {
  if (mutation === "create") return canManageServers;
  if (mutation === "restore") return canManageBackups;
  return canManageBackups && recoverableTrashAvailable;
}

function parsePortRanges(input: string): GamePortRange[] {
  const tokens = input.split(/[\s,]+/u).map((value) => value.trim()).filter(Boolean);
  if (tokens.length === 0) throw new Error("Add at least one port range or individual port.");
  return tokens.map((token) => {
    const match = /^(\d{1,5})(?:-(\d{1,5}))?$/u.exec(token);
    if (match === null) throw new Error(`“${token}” is not a port or range.`);
    const start = Number(match[1]);
    const end = Number(match[2] ?? match[1]);
    if (start < 1024 || end > 65535 || end < start) {
      throw new Error(`“${token}” must be an ordered range inside 1024–65535.`);
    }
    return { start, end };
  });
}

function parseIndividualPorts(input: string): number[] {
  if (input.trim().length === 0) return [];
  return input.split(/[\s,]+/u).map((token) => {
    const port = Number(token);
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      throw new Error(`“${token}” is not a port inside 1024–65535.`);
    }
    return port;
  });
}

function PortPoolDialog({
  csrfToken,
  canManageNetwork,
  onClose,
  onSessionExpired,
}: {
  csrfToken: string;
  canManageNetwork: boolean;
  onClose: () => void;
  onSessionExpired: () => void;
}) {
  const [policy, setPolicy] = useState<GamePortPolicy | null>(null);
  const [ranges, setRanges] = useState("");
  const [ports, setPorts] = useState("");
  const [autoForward, setAutoForward] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void getMinecraftPortPolicy(csrfToken, controller.signal)
      .then((value) => {
        setPolicy(value);
        setRanges(value.ranges.map((range) => `${range.start}-${range.end}`).join(", "));
        setPorts(value.ports.join(", "));
        setAutoForward(value.autoForwardOnCreate && canManageNetwork);
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      });
    return () => controller.abort();
  }, [canManageNetwork, csrfToken, onSessionExpired]);

  const save = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const parsedRanges = ranges.trim().length === 0 ? [] : parsePortRanges(ranges);
      const parsedPorts = parseIndividualPorts(ports);
      if (parsedRanges.length === 0 && parsedPorts.length === 0) {
        throw new Error("Add at least one port or port range.");
      }
      const saved = await saveMinecraftPortPolicy(
        { ranges: parsedRanges, ports: parsedPorts, autoForwardOnCreate: autoForward },
        csrfToken,
      );
      setPolicy(saved);
      setRanges(saved.ranges.map((range) => `${range.start}-${range.end}`).join(", "));
      setPorts(saved.ports.join(", "));
      onClose();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog title="Minecraft port pool" onClose={onClose} wide>
      <div class="port-pool-summary">
        <div><strong>{policy?.capacity ?? "—"}</strong><span>configured</span></div>
        <div><strong>{policy?.availableCount ?? "—"}</strong><span>unassigned</span></div>
        <div><strong>{policy?.nextAvailablePort ?? "—"}</strong><span>next port</span></div>
      </div>
      <p class="dialog-intro">
        Automatic server creation takes individual ports first, then walks each range in order. It also skips ports already assigned to Helix or currently bound on the host.
      </p>
      <div class="form-grid">
        <label class="field field--wide">
          <span>Port ranges</span>
          <input
            value={ranges}
            disabled={busy || policy === null}
            onInput={(event) => setRanges(event.currentTarget.value)}
            placeholder="25565-25599, 25610-25619"
          />
          <small>Separate ranges with commas or spaces. A single port is accepted here too.</small>
        </label>
        <label class="field field--wide">
          <span>Priority ports</span>
          <input
            value={ports}
            disabled={busy || policy === null}
            onInput={(event) => setPorts(event.currentTarget.value)}
            placeholder="25565, 25570, 25580"
          />
          <small>Optional. These are tried before the ranges; duplicates are removed safely.</small>
        </label>
      </div>
      <label class={`check-row ${canManageNetwork ? "" : "is-disabled"}`}>
        <input
          class="toggle-input"
          type="checkbox"
          checked={autoForward}
          disabled={busy || policy === null || !canManageNetwork}
          onChange={(event) => setAutoForward(event.currentTarget.checked)}
        />
        <span>
          <strong>Default new Minecraft servers to public setup</strong>
          <small>
            {canManageNetwork
              ? "The creation review still shows this choice. Helix will never enable UFW or overwrite an unowned router mapping."
              : "Requires network.firewall.write permission."}
          </small>
        </span>
      </label>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button>
        <button class="button button--primary" type="button" disabled={busy || policy === null} onClick={() => void save()}>
          {busy ? "Saving…" : "Save port pool"}
        </button>
      </div>
    </Dialog>
  );
}

function CreateServerDialog({
  csrfToken,
  servers,
  onClose,
  onComplete,
  onSessionExpired,
  canManageNetwork,
}: {
  csrfToken: string;
  servers: ManagedServer[];
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
  canManageNetwork: boolean;
}) {
  const [step, setStep] = useState<1 | 2>(1);
  const [mode, setMode] = useState<MinecraftCreateMode>("software");
  const [name, setName] = useState("");
  const [software, setSoftware] = useState<MinecraftSoftware>("paper");
  const [version, setVersion] = useState("latest");
  const [modpack, setModpack] = useState<ModpackSelection | null>(null);
  const [customJarPath, setCustomJarPath] = useState("");
  const [customBrowserOpen, setCustomBrowserOpen] = useState(false);
  const [customJavaVersion, setCustomJavaVersion] = useState<17 | 21 | 25>(21);
  const [memory, setMemory] = useState(4096);
  const [players, setPlayers] = useState(20);
  const [port, setPort] = useState(() => nextMinecraftPort(servers));
  const [portMode, setPortMode] = useState<"automatic" | "manual">("automatic");
  const [portPolicy, setPortPolicy] = useState<GamePortPolicy | null>(null);
  const [publicAccess, setPublicAccess] = useState(false);
  const [startOnBoot, setStartOnBoot] = useState(true);
  const [eula, setEula] = useState(false);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [readiness, setReadiness] = useState<ServerManagerReadiness | null>(
    null,
  );
  const [readinessError, setReadinessError] = useState<string | null>(null);
  const [catalogOpen, setCatalogOpen] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    void getServerManagerReadiness(csrfToken, controller.signal)
      .then((result) => {
        setReadiness(result);
        setReadinessError(
          result.availability === "ready"
            ? null
            : "The native manager is unavailable on this host, so creation is safely disabled.",
        );
      })
      .catch((requestError: unknown) => {
        if (isSessionError(requestError)) onSessionExpired();
        else setReadinessError(describeError(requestError));
      });
    return () => controller.abort();
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    const controller = new AbortController();
    void getMinecraftPortPolicy(csrfToken, controller.signal)
      .then((policy) => {
        setPortPolicy(policy);
        if (policy.nextAvailablePort !== null) setPort(policy.nextAvailablePort);
        setPublicAccess(canManageNetwork && policy.autoForwardOnCreate);
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      });
    return () => controller.abort();
  }, [canManageNetwork, csrfToken, onSessionExpired]);

  const polling = useJobPolling({
    job,
    csrfToken,
    onJob: setJob,
    onComplete,
    onSessionExpired,
  });

  const fabricReady =
    readiness?.availability === "ready" &&
    readiness.supportedMinecraftSoftware.includes("fabric");
  const softwareReady =
    readiness?.availability === "ready" &&
    readiness.supportedMinecraftSoftware.some((id) => id === software);
  const customReady =
    readiness?.availability === "ready" &&
    readiness.supportedMinecraftSoftware.includes("custom");
  const customInputReady =
    customReady &&
    customJarPath.trim().startsWith("/") &&
    /\.jar$/iu.test(customJarPath.trim()) &&
    version.trim().length > 0 &&
    !version.trim().toLowerCase().includes("latest");
  const canReview =
    name.trim().length >= 2 &&
    (mode === "modpack"
      ? fabricReady && modpack !== null
      : mode === "custom"
        ? customInputReady
        : softwareReady && version.trim().length > 0) &&
    (portMode === "automatic"
      ? portPolicy?.nextAvailablePort !== null && portPolicy?.nextAvailablePort !== undefined
      : Number.isInteger(port) && port >= 1024 && port <= 65535);

  const selectMode = (next: MinecraftCreateMode): void => {
    setMode(next);
    setError(null);
    if (next === "modpack") preloadModpackPicker();
  };

  const submit = async (): Promise<void> => {
    if (!canReview || !eula) return;
    setSubmitting(true);
    setError(null);
    try {
      if (mode === "modpack") {
        if (modpack === null || !fabricReady)
          throw new Error(
            "Choose an installable Fabric modpack release first.",
          );
        const result = await createMinecraftModpack(
          {
            name: name.trim(),
            memory_mb: memory,
            max_players: players,
            ...(portMode === "manual" ? { game_port: port } : {}),
            network_exposure: publicAccess ? "public" : "private",
            start_on_boot: startOnBoot,
            eula_accepted: eula,
            project_id: modpack.projectId,
            version_id: modpack.versionId,
          },
          csrfToken,
        );
        setJob({
          id: result.jobId,
          kind: "minecraft_modpack_create",
          status: "queued",
          stage: "Queued",
          progressPercent: 0,
          createdAtUnixMs: Date.now(),
          updatedAtUnixMs: Date.now(),
          result: null,
          error: null,
        });
      } else {
        if (mode === "software" && !softwareReady)
          throw new Error(
            "This server software is not installable on this host.",
          );
        if (mode === "custom" && !customInputReady)
          throw new Error(
            "Choose an existing .jar inside a Helix Storage root, an explicit Minecraft version, and a supported Java runtime.",
          );
        const input: MinecraftCreateInput = {
          name: name.trim(),
          software: mode === "custom" ? "custom" : software,
          version: version.trim(),
          memory_mb: memory,
          max_players: players,
          ...(portMode === "manual" ? { game_port: port } : {}),
          network_exposure: publicAccess ? "public" : "private",
          start_on_boot: startOnBoot,
          eula_accepted: eula,
          ...(mode === "custom" ? {
            custom_jar: {
              source_path: customJarPath.trim(),
              java_version: customJavaVersion,
            },
          } : {}),
        };
        const result = await createMinecraftServer(input, csrfToken);
        setJob({
          id: result.jobId,
          kind: "minecraft_create",
          status: "queued",
          stage: "Queued",
          progressPercent: 0,
          createdAtUnixMs: Date.now(),
          updatedAtUnixMs: Date.now(),
          result: null,
          error: null,
        });
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setSubmitting(false);
    }
  };

  if (job !== null) {
    const active = job.status === "running" || job.status === "queued";
    const canClose = !active || polling.paused;
    let modpackResult: MinecraftModpackCreateResult | null = null;
    if (job.status === "complete" && mode === "modpack") {
      try {
        modpackResult = parseMinecraftModpackCreateResult(job.result);
      } catch {
        modpackResult = null;
      }
    }
    const resultRecord =
      typeof job.result === "object" && job.result !== null
        ? (job.result as Record<string, unknown>)
        : null;
    const networkResult =
      typeof resultRecord?.network_exposure === "object" && resultRecord.network_exposure !== null
        ? (resultRecord.network_exposure as Record<string, unknown>)
        : null;
    const publicJoin =
      typeof networkResult?.public_join_address === "string"
        ? networkResult.public_join_address
        : null;
    const publicSetupError =
      typeof networkResult?.error === "string" ? networkResult.error : null;
    return (
      <Dialog
        title={
          job.status === "complete"
            ? "Server ready"
            : job.status === "failed"
              ? "Creation stopped safely"
              : mode === "modpack"
                ? "Installing the modpack"
                : "Building your server"
        }
        onClose={() => canClose && onClose()}
      >
        <div class="job-progress">
          <div class={`job-icon job-icon--${job.status}`}>
            <Icon
              name={
                job.status === "complete"
                  ? "check"
                  : job.status === "failed"
                    ? "warning"
                    : "servers"
              }
              size={26}
            />
          </div>
          <strong>{job.stage}</strong>
          <span>
            {active
              ? mode === "modpack"
                ? "Helix is validating the Modrinth archive, assembling a server-safe subset, and starting the pinned Fabric runtime."
                : "Helix is downloading a verified build, creating the workload, and starting Minecraft."
              : job.status === "complete"
                ? `${name} is online and ready to join.`
                : (job.error ?? "Helix rolled back the incomplete server.")}
          </span>
          <ProgressBar
            value={job.progressPercent}
            tone={job.status === "failed" ? "danger" : "normal"}
          />
          <small>{job.progressPercent}%</small>
          {modpackResult !== null && (
            <div class="modpack-create-result">
              <strong>
                {modpackResult.projectTitle} {modpackResult.versionNumber}
              </strong>
              <span>
                Minecraft {modpackResult.minecraftVersion} · Fabric Loader{" "}
                {modpackResult.fabricLoaderVersion}
              </span>
              <span>
                {modpackResult.installedServerFiles} required server files
                installed · {modpackResult.excludedServerOptionalFiles} optional
                server files excluded · {modpackResult.excludedClientOnlyFiles}{" "}
                client-only files excluded
              </span>
              <span>
                Modrinth-declared SHA-512 verified. This is a server-safe
                subset, not byte-for-byte full-pack parity.
              </span>
            </div>
          )}
          {job.status === "complete" && publicAccess && (
            <div class={`creation-network-result ${publicJoin === null ? "is-warning" : "is-ready"}`}>
              <Icon name={publicJoin === null ? "warning" : "network"} size={16} />
              <span>
                <strong>{publicJoin === null ? "Server online · public access needs attention" : publicJoin}</strong>
                {publicJoin === null
                  ? (publicSetupError ?? "Open the server’s Join section to retry automatic public setup.")
                  : "Router mapping confirmed. Test this address from a separate external network before sharing it broadly."}
              </span>
            </div>
          )}
        </div>
        <InlineError message={polling.error ?? error} />
        <div class="dialog-actions">
          {polling.paused && (
            <button
              class="button button--quiet"
              type="button"
              onClick={polling.resume}
            >
              Resume status check
            </button>
          )}
          <button
            class="button button--primary"
            type="button"
            disabled={!canClose}
            onClick={onClose}
          >
            {job.status === "complete" ? "View servers" : "Close"}
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title="New Minecraft server" onClose={onClose} wide>
      <div class="wizard-steps">
        <span class={step === 1 ? "is-current" : "is-done"}>
          1 <b>Server</b>
        </span>
        <i />
        <span class={step === 2 ? "is-current" : ""}>
          2 <b>Review</b>
        </span>
      </div>
      {step === 1 && (
        <div
          class="create-mode-switch"
          role="tablist"
          aria-label="Server starting point"
        >
          <button
            class={mode === "software" ? "is-selected" : ""}
            type="button"
            role="tab"
            aria-selected={mode === "software"}
            onClick={() => selectMode("software")}
          >
            <span class="create-mode-icon">
              <Icon name="servers" size={18} />
            </span>
            <span class="create-mode-copy">
              <strong>Choose server software</strong>
              <small>Paper, Purpur, Folia, Fabric, or Vanilla</small>
            </span>
          </button>
          <button
            class={mode === "modpack" ? "is-selected" : ""}
            type="button"
            role="tab"
            aria-selected={mode === "modpack"}
            onClick={() => selectMode("modpack")}
          >
            <span class="create-mode-icon">
              <Icon name="plus" size={18} />
            </span>
            <span class="create-mode-copy">
              <strong>Start with a modpack</strong>
              <small>A server-capable Fabric pack from Modrinth</small>
            </span>
          </button>
          <button
            class={mode === "custom" ? "is-selected" : ""}
            type="button"
            role="tab"
            aria-selected={mode === "custom"}
            onClick={() => selectMode("custom")}
          >
            <span class="create-mode-icon">
              <Icon name="file" size={18} />
            </span>
            <span class="create-mode-copy">
              <strong>Use your own JAR</strong>
              <small>Import a server JAR already available in Storage</small>
            </span>
          </button>
        </div>
      )}
      <InlineError message={error} />
      {step === 1 ? (
        <form
          class="server-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (canReview) setStep(2);
          }}
        >
          <div class="form-grid">
            <label class="field field--wide">
              <span>Server name</span>
              <input
                autofocus
                required
                minlength={2}
                maxlength={64}
                value={name}
                onInput={(event) => setName(event.currentTarget.value)}
                placeholder="Survival"
              />
            </label>
            {mode === "software" ? (
              <>
                <MinecraftSoftwarePicker
                  selected={software}
                  readiness={readiness}
                  readinessError={readinessError}
                  catalogOpen={catalogOpen}
                  onSelect={setSoftware}
                  onCatalogToggle={() => setCatalogOpen((open) => !open)}
                />
                <label class="field">
                  <span>Minecraft version</span>
                  <input
                    required
                    value={version}
                    onInput={(event) => setVersion(event.currentTarget.value)}
                    placeholder="latest"
                  />
                </label>
              </>
            ) : mode === "custom" ? (
              <>
                <div
                  class={`software-readiness-note field--wide ${customReady ? "" : "is-error"}`}
                  role="status"
                >
                  <Icon name={customReady ? "check" : "warning"} size={16} />
                  <span>
                    {readiness === null
                      ? "Checking the protected local import path…"
                      : customReady
                        ? "Custom JAR import is ready. Helix copies the file into a private container workspace; the original stays untouched."
                        : "Custom JAR import is unavailable because the native manager or its Storage roots are not ready."}
                  </span>
                </div>
                <div class="field field--wide">
                  <div class="field-heading">
                    <label for="custom-server-jar">Server JAR path</label>
                    <button
                      class="field-inline-action"
                      type="button"
                      disabled={!customReady}
                      onClick={() => setCustomBrowserOpen((open) => !open)}
                    >
                      <Icon name="folder" size={14} />
                      {customBrowserOpen ? "Close browser" : "Browse Storage"}
                    </button>
                  </div>
                  <input
                    id="custom-server-jar"
                    required
                    value={customJarPath}
                    onInput={(event) =>
                      setCustomJarPath(event.currentTarget.value)
                    }
                    placeholder="/path/visible/in/helix/server.jar"
                    autocomplete="off"
                    autocapitalize="none"
                    spellcheck={false}
                  />
                  <small>
                    Use an absolute <code>.jar</code> path inside a drive or
                    folder shown in Storage. Helix rejects symlinks, paths
                    outside managed roots, and files larger than 768 MiB.
                  </small>
                </div>
                {customBrowserOpen && customReady && (
                  <CustomJarBrowser
                    csrfToken={csrfToken}
                    selectedPath={customJarPath}
                    onSelect={(path) => {
                      setCustomJarPath(path);
                      setCustomBrowserOpen(false);
                    }}
                    onClose={() => setCustomBrowserOpen(false)}
                    onSessionExpired={onSessionExpired}
                  />
                )}
                <label class="field">
                  <span>Minecraft version</span>
                  <input
                    required
                    value={version}
                    onInput={(event) => setVersion(event.currentTarget.value)}
                    placeholder="1.21.8"
                  />
                  <small>Enter the exact version; “latest” is not accepted.</small>
                </label>
                <label class="field">
                  <span>Java runtime</span>
                  <select
                    value={customJavaVersion}
                    onChange={(event) =>
                      setCustomJavaVersion(
                        Number(event.currentTarget.value) as 17 | 21 | 25,
                      )
                    }
                  >
                    <option value="17">Java 17</option>
                    <option value="21">Java 21</option>
                    <option value="25">Java 25</option>
                  </select>
                  <small>Match the requirement published with your server JAR.</small>
                </label>
              </>
            ) : (
              <>
                <div
                  class={`software-readiness-note field--wide ${fabricReady ? "" : "is-error"}`}
                  role="status"
                >
                  <span>
                    {readiness === null
                      ? "Checking Fabric lifecycle readiness…"
                      : fabricReady
                        ? "Fabric modpack creation is ready on this host."
                        : "Fabric is not lifecycle-ready on this host, so modpack creation is disabled."}
                  </span>
                </div>
                <ModpackRoute
                  csrfToken={csrfToken}
                  selection={modpack}
                  onSelectionChange={setModpack}
                  onSessionExpired={onSessionExpired}
                />
              </>
            )}
            <fieldset class="port-choice field--wide">
              <legend>Game port</legend>
              <label class={portMode === "automatic" ? "is-selected" : ""}>
                <input
                  type="radio"
                  name="port-mode"
                  checked={portMode === "automatic"}
                  onChange={() => setPortMode("automatic")}
                />
                <span>
                  <strong>Automatic from Minecraft pool</strong>
                  <small>
                    {portPolicy?.nextAvailablePort === null
                      ? "No free port remains in the current pool."
                      : portPolicy === null
                        ? "Checking the configured pool…"
                        : `Next available: ${portPolicy.nextAvailablePort} · ${portPolicy.availableCount} of ${portPolicy.capacity} unassigned`}
                  </small>
                </span>
              </label>
              <label class={portMode === "manual" ? "is-selected" : ""}>
                <input
                  type="radio"
                  name="port-mode"
                  checked={portMode === "manual"}
                  onChange={() => setPortMode("manual")}
                />
                <span>
                  <strong>Choose a specific port</strong>
                  <small>Helix will reject a port already assigned or bound on the host.</small>
                </span>
                <input
                  aria-label="Specific game port"
                  type="number"
                  min="1024"
                  max="65535"
                  disabled={portMode !== "manual"}
                  required={portMode === "manual"}
                  value={port}
                  onInput={(event) => setPort(event.currentTarget.valueAsNumber)}
                />
              </label>
            </fieldset>
            <label class="field">
              <span>Memory</span>
              <select
                value={memory}
                onChange={(event) =>
                  setMemory(Number(event.currentTarget.value))
                }
              >
                <option value="2048">2 GiB</option>
                <option value="4096">4 GiB</option>
                <option value="6144">6 GiB</option>
                <option value="8192">8 GiB</option>
                <option value="12288">12 GiB</option>
                <option value="16384">16 GiB</option>
              </select>
            </label>
            <label class="field">
              <span>Maximum players</span>
              <input
                type="number"
                min="1"
                max="10000"
                value={players}
                onInput={(event) =>
                  setPlayers(event.currentTarget.valueAsNumber)
                }
              />
            </label>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              class="button button--primary"
              type="submit"
              disabled={!canReview}
            >
              Review server
            </button>
          </div>
        </form>
      ) : (
        <div class="server-review">
          <dl>
            <div>
              <dt>Name</dt>
              <dd>{name.trim()}</dd>
            </div>
            <div>
              <dt>Starting point</dt>
              <dd>
                {mode === "modpack"
                  ? `${modpack?.projectTitle ?? ""} ${modpack?.versionNumber ?? ""}`
                  : mode === "custom"
                    ? `Custom JAR · Minecraft ${version} · Java ${customJavaVersion}`
                    : `${minecraftCreateSoftwareOptions.find((option) => option.id === software)?.name ?? software} ${version}`}
              </dd>
            </div>
            <div>
              <dt>Resources</dt>
              <dd>
                {memory / 1024} GiB · {players} players
              </dd>
            </div>
            <div>
              <dt>Port</dt>
              <dd>
                {portMode === "automatic"
                  ? `${portPolicy?.nextAvailablePort ?? "Next free"} · automatic`
                  : `${port} · specific`}
              </dd>
            </div>
            <div>
              <dt>Player access</dt>
              <dd>{publicAccess ? "LAN + automatic public setup" : "Private / LAN"}</dd>
            </div>
          </dl>
          {mode === "modpack" && modpack !== null && (
            <div class="modpack-compatibility-note">
              <Icon name="info" size={16} />
              <span>
                <strong>Server-safe Fabric subset</strong>Helix will require the
                exact listed release, verify its Modrinth-declared SHA-512,
                strictly validate every path and declared SHA-1/SHA-512, exclude
                server-optional and client-only files, pin Minecraft and Fabric
                Loader, and roll back the entire new server if installation or
                startup fails.
              </span>
            </div>
          )}
          {mode === "custom" && (
            <div class="modpack-compatibility-note">
              <Icon name="info" size={16} />
              <span>
                <strong>Local artifact, isolated runtime</strong>
                Helix verifies the path and size, hashes a private copy, pins
                the chosen Java container, and rolls back a failed first boot.
                The JAR publisher and Minecraft compatibility cannot be
                verified automatically, so future JAR updates stay manual.
              </span>
            </div>
          )}
          <label class="check-row">
            <input
              class="toggle-input"
              type="checkbox"
              checked={startOnBoot}
              onChange={(event) => setStartOnBoot(event.currentTarget.checked)}
            />
            <span>
              <strong>Start with the host</strong>
              <small>
                Helix will restore this workload after Docker or the host
                restarts.
              </small>
            </span>
          </label>
          <label class={`check-row ${canManageNetwork ? "" : "is-disabled"}`}>
            <input
              class="toggle-input"
              type="checkbox"
              checked={publicAccess}
              disabled={!canManageNetwork}
              onChange={(event) => setPublicAccess(event.currentTarget.checked)}
            />
            <span>
              <strong>Set up public player access</strong>
              <small>
                {canManageNetwork
                  ? "After Minecraft is online, Helix will request and verify an exact TCP mapping from a compatible UPnP router. If UFW is active, Helix also creates a matching owned rule."
                  : "Requires network.firewall.write permission. The server can still be created for LAN or private-network access."}
              </small>
            </span>
          </label>
          <label class="check-row">
            <input
              type="checkbox"
              checked={eula}
              onChange={(event) => setEula(event.currentTarget.checked)}
            />
            <span>
              <strong>I accept the Minecraft EULA</strong>
              <small>
                Required to run a Minecraft server.{" "}
                <a
                  href="https://aka.ms/MinecraftEULA"
                  target="_blank"
                  rel="noreferrer"
                >
                  Read the EULA <Icon name="external" size={12} />
                </a>
              </small>
            </span>
          </label>
          <div class="creation-note">
            <Icon name="activity" />
            <span>
              {mode === "modpack"
                ? "Only opaque Modrinth project and version IDs leave the browser. The broker re-resolves all metadata and will never activate Forge, NeoForge, Quilt, client-only content, unsafe archive paths, or an unverified download."
                : mode === "custom"
                  ? "Helix will import a private copy, pin its SHA-256 and Java runtime, isolate it as an unprivileged container, reserve the ports, write the Minecraft configuration, and start it. Your source file is never modified."
                  : "Helix will resolve a supported build and Java runtime, verify the download, isolate the workload, write the configuration, reserve the ports, and start Minecraft."}
            </span>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              onClick={() => setStep(1)}
            >
              Back
            </button>
            <button
              class="button button--primary"
              type="button"
              disabled={!eula || submitting || !canReview}
              onClick={() => void submit()}
            >
              {submitting ? "Starting…" : "Create and start server"}
            </button>
          </div>
        </div>
      )}
    </Dialog>
  );
}

function ServerActionDialog({
  server,
  action,
  csrfToken,
  onClose,
  onComplete,
  onSessionExpired,
}: {
  server: ManagedServer;
  action: ServerAction;
  csrfToken: string;
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const label = action[0]?.toUpperCase() + action.slice(1);
  const polling = useJobPolling({
    job,
    csrfToken,
    baseDelayMs: 900,
    onJob: setJob,
    onComplete,
    onSessionExpired,
  });
  const submit = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const dispatch = await runServerAction(server.id, action, csrfToken);
      if (dispatch.jobId === null) {
        await onComplete();
        onClose();
      } else {
        setJob({
          id: dispatch.jobId,
          kind: `server_${action}`,
          status: "queued",
          stage: "Queued",
          progressPercent: 0,
          createdAtUnixMs: Date.now(),
          updatedAtUnixMs: Date.now(),
          result: null,
          error: null,
        });
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  if (job !== null) {
    const active = job.status === "queued" || job.status === "running";
    const canClose = !active || polling.paused;
    return (
      <Dialog
        title={
          job.status === "failed"
            ? `${label} failed`
            : job.status === "complete"
              ? `${label} complete`
              : `${label} in progress`
        }
        onClose={() => canClose && onClose()}
      >
        <div class="job-progress">
          <div class={`job-icon job-icon--${job.status}`}>
            <Icon
              name={
                job.status === "complete"
                  ? "check"
                  : job.status === "failed"
                    ? "warning"
                    : action === "backup"
                      ? "backup"
                      : "activity"
              }
              size={26}
            />
          </div>
          <strong>{job.stage}</strong>
          <span>
            {active
              ? "This runs in the background. Closing after a status-check problem will not interrupt it."
              : job.status === "complete"
                ? `${server.name} is ready.`
                : (job.error ?? "Helix could not finish the action.")}
          </span>
          <ProgressBar
            value={
              active ? Math.max(job.progressPercent, 12) : job.progressPercent
            }
            tone={job.status === "failed" ? "danger" : "normal"}
          />
          <small>
            {active ? "Working safely…" : `${job.progressPercent}%`}
          </small>
        </div>
        <InlineError message={polling.error ?? error} />
        <div class="dialog-actions">
          {polling.paused && (
            <button
              class="button button--quiet"
              type="button"
              onClick={polling.resume}
            >
              Resume status check
            </button>
          )}
          <button
            class="button button--primary"
            type="button"
            disabled={!canClose}
            onClick={onClose}
          >
            Close
          </button>
        </div>
      </Dialog>
    );
  }
  return (
    <Dialog title={`${label} ${server.name}?`} onClose={onClose}>
      <div class="dialog-copy">
        <p>{serverActionDescription(server, action)}</p>
      </div>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button class="button button--quiet" type="button" onClick={onClose}>
          Cancel
        </button>
        <button
          class={`button ${action === "stop" ? "button--danger" : "button--primary"}`}
          type="button"
          disabled={busy}
          onClick={() => void submit()}
        >
          {busy ? "Queuing…" : label}
        </button>
      </div>
    </Dialog>
  );
}

function ServerRow({
  server,
  csrfToken,
  canManageServers,
  onRefresh,
  onOpen,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  canManageServers: boolean;
  onRefresh: () => Promise<void>;
  onOpen: () => void;
  onSessionExpired: () => void;
}) {
  const [pending, setPending] = useState<ServerAction | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const online = server.status === "online";
  const memoryPercent =
    server.memoryLimitMb > 0
      ? (server.memoryUsedMb / server.memoryLimitMb) * 100
      : 0;
  const confirmAction = (action: ServerAction): void => {
    if (!canManageServers) return;
    setMenuOpen(false);
    setPending(action);
  };
  const manageTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  return (
    <div class="server-row">
      <button
        class="server-identity server-identity--button"
        type="button"
        onClick={onOpen}
      >
        <ServerArtwork server={server} />
        <span>
          <strong>
            {server.name}
            <small class={`manager-badge manager-badge--${server.manager}`}>
              {server.manager === "helix" ? "Helix" : "Imported"}
            </small>
          </strong>
          <span>
            {server.software} {server.version}
          </span>
          <small>{server.instanceName}</small>
        </span>
      </button>
      <div class="server-stat">
        <span>Players</span>
        <strong>
          {online ? `${server.playersOnline} / ${server.maxPlayers}` : "—"}
        </strong>
      </div>
      <div class="server-stat">
        <span>CPU</span>
        <strong>{online ? formatPercent(server.cpuPercent) : "—"}</strong>
      </div>
      <div class="server-stat server-stat--memory">
        <span>Memory</span>
        <strong>
          {online
            ? `${formatBytes(server.memoryUsedMb * 1024 * 1024)} / ${formatBytes(server.memoryLimitMb * 1024 * 1024)}`
            : `${server.memoryLimitMb / 1024} GiB limit`}
        </strong>
        {online && (
          <ProgressBar
            value={memoryPercent}
            tone={toneForPercent(memoryPercent)}
          />
        )}
      </div>
      <div class="server-stat">
        <span>Port</span>
        <strong>{server.gamePort ?? "—"}</strong>
      </div>
      <div class="server-stat">
        <span>TPS</span>
        <strong>{server.tps === null ? "—" : server.tps.toFixed(1)}</strong>
      </div>
      <div class="server-actions">
        {online ? (
          <button
            class="button button--quiet"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => confirmAction("restart")}
          >
            <Icon name="restart" size={15} />
            Restart
          </button>
        ) : (
          <button
            class="button button--primary"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => confirmAction("start")}
          >
            <Icon name="play" size={15} />
            Start
          </button>
        )}
        <div class={`menu-wrap server-more ${menuOpen ? "is-open" : ""}`}>
          <button
            class="icon-button"
            type="button"
            aria-label={`More actions for ${server.name}`}
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <Icon name="more" />
          </button>
          <div class="pop-menu">
            <button type="button" onClick={onOpen}>
              <Icon name="chevron" size={15} />
              Open
            </button>
            <button
              type="button"
              disabled={!canManageServers}
              title={manageTitle}
              onClick={() => {
                setMenuOpen(false);
                setAppearanceOpen(true);
              }}
            >
              <Icon name="edit" size={15} />
              Change icon
            </button>
            {server.manager === "helix" && (
              <button
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => confirmAction("backup")}
              >
                <Icon name="backup" size={15} />
                Backup
              </button>
            )}
            {server.manager === "helix" &&
              server.software.toLowerCase() !== "custom jar" && (
              <button
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => confirmAction("update")}
              >
                <Icon name="update" size={15} />
                Update
              </button>
            )}
            {online && (
              <button
                class="danger-link"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => confirmAction("stop")}
              >
                <Icon name="stop" size={15} />
                Stop
              </button>
            )}
          </div>
        </div>
      </div>
      {pending !== null && (
        <ServerActionDialog
          server={server}
          action={pending}
          csrfToken={csrfToken}
          onClose={() => setPending(null)}
          onComplete={onRefresh}
          onSessionExpired={onSessionExpired}
        />
      )}
      {appearanceOpen && (
        <ServerIconDialog
          server={server}
          csrfToken={csrfToken}
          onClose={() => setAppearanceOpen(false)}
          onSaved={onRefresh}
          onSessionExpired={onSessionExpired}
        />
      )}
    </div>
  );
}

type NativeServerTab =
  | "overview"
  | "console"
  | "settings"
  | "files"
  | "backups"
  | "logs"
  | "performance"
  | "marketplace"
  | "advanced";

const nativeServerTabs: ReadonlyArray<{
  id: NativeServerTab;
  label: string;
  icon: IconName;
}> = [
  { id: "overview", label: "Overview", icon: "overview" },
  { id: "console", label: "Console", icon: "console" },
  { id: "settings", label: "Settings", icon: "settings" },
  { id: "files", label: "Files", icon: "folder" },
  { id: "backups", label: "Backups", icon: "backup" },
  { id: "logs", label: "Logs", icon: "logs" },
  { id: "performance", label: "Performance", icon: "performance" },
  { id: "marketplace", label: "Marketplace", icon: "search" },
  { id: "advanced", label: "Advanced", icon: "advanced" },
];

export function supportsMarketplaceSoftware(software: string): boolean {
  return /^(?:paper|purpur|folia|fabric)$/iu.test(software.trim());
}

function ConsolePanel({
  detail,
  csrfToken,
  canManageServers,
  onSessionExpired,
}: {
  detail: NativeServerDetail;
  csrfToken: string;
  canManageServers: boolean;
  onSessionExpired: () => void;
}) {
  const [command, setCommand] = useState("");
  const [entries, setEntries] = useState<ConsoleHistoryEntry[]>([]);
  const [olderCursor, setOlderCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [retention, setRetention] = useState<ConsoleHistoryPage["retention"]>({
    maximumBytes: detail.consoleHistory.retentionBytes,
    files: detail.consoleHistory.retentionFiles,
    scope: detail.consoleHistory.scope,
  });
  const [loadingHistory, setLoadingHistory] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [live, setLive] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [commandNotice, setCommandNotice] = useState<string | null>(null);
  const latestRequest = useRef<Promise<void> | null>(null);
  const consoleRef = useRef<HTMLDivElement>(null);
  const followLatestRef = useRef(true);
  const restoreScrollRef = useRef<{ height: number; top: number } | null>(null);
  const [atLatest, setAtLatest] = useState(true);

  const loadLatest = useCallback(
    async (initial = false, signal?: AbortSignal): Promise<void> => {
      if (latestRequest.current !== null) await latestRequest.current;
      const request = (async () => {
        try {
          const page = await getServerLogHistory(
            detail.id,
            csrfToken,
            null,
            200,
            signal,
          );
          setEntries((current) =>
            initial
              ? page.entries
              : mergeLatestConsoleHistory(current, page.entries),
          );
          if (initial) {
            setOlderCursor(page.nextCursor);
            setHasMore(page.hasMore);
          }
          setRetention(page.retention);
          setError(null);
        } catch (requestError) {
          if (signal?.aborted !== true) {
            if (isSessionError(requestError)) onSessionExpired();
            else setError(describeError(requestError));
          }
        } finally {
          if (initial) setLoadingHistory(false);
        }
      })();
      latestRequest.current = request;
      await request;
      if (latestRequest.current === request) latestRequest.current = null;
    },
    [csrfToken, detail.id, onSessionExpired],
  );

  useEffect(() => {
    const controller = new AbortController();
    setEntries([]);
    setOlderCursor(null);
    setHasMore(false);
    setLoadingHistory(true);
    void loadLatest(true, controller.signal);
    return () => controller.abort();
  }, [detail.id, loadLatest]);
  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void loadLatest();
    }, 2_000);
    return () => window.clearInterval(timer);
  }, [live, loadLatest]);

  useLayoutEffect(() => {
    const screen = consoleRef.current;
    if (screen === null) return;
    const restore = restoreScrollRef.current;
    if (restore !== null) {
      screen.scrollTop = restore.top + (screen.scrollHeight - restore.height);
      restoreScrollRef.current = null;
      return;
    }
    if (followLatestRef.current) {
      screen.scrollTop = screen.scrollHeight;
      setAtLatest(true);
    }
  }, [entries, loadingHistory]);

  const loadOlder = async (): Promise<void> => {
    if (olderCursor === null || loadingOlder) return;
    setLoadingOlder(true);
    setError(null);
    try {
      const page = await getServerLogHistory(
        detail.id,
        csrfToken,
        olderCursor,
        200,
      );
      const screen = consoleRef.current;
      if (screen !== null)
        restoreScrollRef.current = {
          height: screen.scrollHeight,
          top: screen.scrollTop,
        };
      followLatestRef.current = false;
      setAtLatest(false);
      setEntries((current) =>
        prependOlderConsoleHistory(current, page.entries),
      );
      setOlderCursor(page.nextCursor);
      setHasMore(page.hasMore);
      setRetention(page.retention);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setLoadingOlder(false);
    }
  };

  const submit = async (value = command): Promise<void> => {
    const next = value.trim().replace(/^\//, "");
    if (next.length === 0) return;
    followLatestRef.current = true;
    setAtLatest(true);
    setBusy(true);
    setError(null);
    setCommandNotice(null);
    try {
      const result = await sendConsoleCommand(detail.id, next, csrfToken);
      setCommand("");
      if (!result.historyRecorded)
        setCommandNotice(
          result.response.length === 0
            ? "The command completed, but Helix could not retain it in console history."
            : `The command completed, but history retention failed. Response: ${result.response}`,
        );
      await loadLatest();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  const entryTime = (entry: ConsoleHistoryEntry): string | null => {
    const value =
      entry.timestamp ??
      (entry.timestampUnixMs === null
        ? null
        : new Date(entry.timestampUnixMs).toISOString());
    if (value === null) return null;
    const parsed = Date.parse(value);
    return Number.isFinite(parsed)
      ? new Date(parsed).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
        })
      : value;
  };
  const commandDisabled =
    !canManageServers || detail.status !== "online" || busy;
  const commandTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  return (
    <section class="server-tool server-console">
      <div class="tool-head">
        <div>
          <h2>
            Persistent console{" "}
            <InfoTip text="Helix stores this server’s console output on the host. Closing the dashboard does not stop collection or erase earlier boots." />
          </h2>
          <p>
            Commands use a loopback-only channel; output stays available across
            dashboard sessions.
          </p>
        </div>
        <div class="quick-commands">
          <button
            type="button"
            disabled={commandDisabled}
            title={commandTitle}
            onClick={() => void submit("list")}
          >
            List players
          </button>
          <button
            type="button"
            disabled={commandDisabled}
            title={commandTitle}
            onClick={() => void submit("save-all flush")}
          >
            Save world
          </button>
          <button
            type="button"
            disabled={commandDisabled}
            title={commandTitle}
            onClick={() => void submit("whitelist list")}
          >
            Whitelist
          </button>
        </div>
      </div>
      <InlineError message={error} />
      {commandNotice !== null && (
        <div class="console-retention-warning" role="status">
          <Icon name="warning" size={14} />
          {commandNotice}
        </div>
      )}
      <div class="console-history-toolbar">
        <div>
          <button
            class="button button--quiet"
            type="button"
            disabled={!hasMore || loadingOlder}
            onClick={() => void loadOlder()}
          >
            {loadingOlder
              ? "Loading…"
              : hasMore
                ? "Load older"
                : "Oldest retained entry"}
          </button>
          {!atLatest && (
            <button
              class="button button--quiet"
              type="button"
              onClick={() => {
                const screen = consoleRef.current;
                followLatestRef.current = true;
                setAtLatest(true);
                if (screen !== null) screen.scrollTop = screen.scrollHeight;
              }}
            >
              Jump to latest
            </button>
          )}
          <span>
            {formatBytes(retention.maximumBytes)} across {retention.files}{" "}
            rotated files · per server
          </span>
        </div>
        <label>
          <input
            class="toggle-input"
            type="checkbox"
            checked={live}
            onChange={(event) => setLive(event.currentTarget.checked)}
          />
          <span>Live updates</span>
        </label>
      </div>
      <div
        ref={consoleRef}
        class="console-screen console-screen--history"
        role="log"
        aria-live={live ? "polite" : "off"}
        aria-busy={loadingHistory}
        onScroll={(event) => {
          const screen = event.currentTarget;
          const nextAtLatest =
            screen.scrollHeight - screen.scrollTop - screen.clientHeight <= 48;
          followLatestRef.current = nextAtLatest;
          setAtLatest(nextAtLatest);
        }}
      >
        {loadingHistory ? (
          <div class="console-empty">
            <Icon name="refresh" class="is-spinning" />
            <strong>Loading retained history</strong>
            <span>Reading the newest console page from this server.</span>
          </div>
        ) : entries.length === 0 ? (
          <div class="console-empty">
            <Icon name="console" />
            <strong>No retained output yet</strong>
            <span>
              Start the server or run a command to create the first entry.
            </span>
          </div>
        ) : (
          entries.map((entry, index) => {
            const time = entryTime(entry);
            if (entry.kind === "boot")
              return (
                <div
                  class="console-marker console-marker--boot"
                  key={`${consoleHistoryEntryKey(entry)}-${index}`}
                >
                  <Icon name="restart" size={14} />
                  <span>
                    <strong>Minecraft boot</strong>
                    <small>
                      {entry.bootStartedAt ?? time ?? "Recorded boot boundary"}
                    </small>
                  </span>
                </div>
              );
            if (entry.kind === "command")
              return (
                <div
                  class="console-marker console-marker--command"
                  key={`${consoleHistoryEntryKey(entry)}-${index}`}
                >
                  <span>&gt;</span>
                  <code>{entry.text.replace(/^\[helix \d+\] > \/?/u, "")}</code>
                  {time !== null && <time>{time}</time>}
                </div>
              );
            if (entry.kind === "command_response")
              return (
                <div
                  class="console-marker console-marker--response"
                  key={`${consoleHistoryEntryKey(entry)}-${index}`}
                >
                  <span>↳</span>
                  <pre>{entry.text.replace(/^\[helix \d+\] < ?/u, "")}</pre>
                  {time !== null && <time>{time}</time>}
                </div>
              );
            const outputText =
              entry.timestamp !== null && entry.text.startsWith(entry.timestamp)
                ? entry.text.slice(entry.timestamp.length).trimStart()
                : entry.text;
            return (
              <div
                class="console-output-line"
                key={`${consoleHistoryEntryKey(entry)}-${index}`}
              >
                {time !== null && <time>{time}</time>}
                <pre>{outputText}</pre>
              </div>
            );
          })
        )}
      </div>
      <div class="console-retention-note">
        <Icon name="backup" size={14} />
        <span>
          <strong>History survives closed tabs and dashboard restarts.</strong>
          <small>
            Helix retains up to {formatBytes(retention.maximumBytes)} in{" "}
            {retention.files} files for this server, then removes the oldest
            output first.
          </small>
        </span>
      </div>
      <form
        class="console-command"
        onSubmit={(event) => {
          event.preventDefault();
          if (canManageServers) void submit();
        }}
      >
        <span>&gt;</span>
        <input
          value={command}
          onInput={(event) => setCommand(event.currentTarget.value)}
          disabled={commandDisabled}
          title={commandTitle}
          placeholder={
            !canManageServers
              ? "This account can view history but cannot run commands"
              : detail.status === "online"
                ? "Enter a command without /"
                : "Start the server to use the console"
          }
          maxlength={512}
          autocomplete="off"
          spellcheck={false}
        />
        <button
          class="button button--primary"
          type="submit"
          disabled={commandDisabled || command.trim().length === 0}
          title={commandTitle}
        >
          {busy ? "Sending…" : "Run"}
        </button>
      </form>
    </section>
  );
}

function SettingsPanel({
  detail,
  csrfToken,
  canManageServers,
  restartSuccessRevision,
  onRestart,
  onSessionExpired,
}: {
  detail: NativeServerDetail;
  csrfToken: string;
  canManageServers: boolean;
  restartSuccessRevision: number;
  onRestart: () => void;
  onSessionExpired: () => void;
}) {
  const [settings, setSettings] = useState<MinecraftSettings>(detail.settings);
  const [saved, setSaved] = useState(detail.settings);
  const [busy, setBusy] = useState(false);
  const [restartPending, setRestartPending] = useState(false);
  const [showRestartChoice, setShowRestartChoice] = useState(false);
  const [changedFields, setChangedFields] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const observedRestartSuccess = useRef(restartSuccessRevision);
  const dirty = JSON.stringify(settings) !== JSON.stringify(saved);
  const restartFields = new Set(settings.restartBehavior.restartRequiredFields);
  const update = <K extends keyof MinecraftSettings>(
    key: K,
    value: MinecraftSettings[K],
  ): void => setSettings((current) => ({ ...current, [key]: value }));
  const restartLabel = (field: MinecraftSettingField): ComponentChildren =>
    restartFields.has(field) ? <em class="restart-field">Restart</em> : null;
  const save = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const result = await saveServerSettings(detail.id, settings, csrfToken);
      setSettings(result.settings);
      setSaved(result.settings);
      if (result.changed) {
        setChangedFields(result.changedFields);
        setRestartPending(result.restartRequired);
        setShowRestartChoice(result.restartRequired);
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  const restartNow = (): void => {
    setShowRestartChoice(false);
    onRestart();
  };
  useEffect(() => {
    if (observedRestartSuccess.current === restartSuccessRevision) return;
    observedRestartSuccess.current = restartSuccessRevision;
    setRestartPending(false);
    setShowRestartChoice(false);
    setChangedFields([]);
  }, [restartSuccessRevision]);
  const manageTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  return (
    <section class="server-tool settings-panel">
      <div class="tool-head">
        <div>
          <h2>
            Minecraft settings{" "}
            <InfoTip text={settings.restartBehavior.message} />
          </h2>
          <p>
            Common options are validated before Helix writes{" "}
            <code>server.properties</code>.
          </p>
        </div>
        {restartPending && !showRestartChoice && (
          <button
            class="button button--primary"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => setShowRestartChoice(true)}
          >
            <Icon name="restart" size={15} />
            Restart to apply
          </button>
        )}
      </div>
      <InlineError message={error} />
      {showRestartChoice && (
        <div class="settings-restart-choice" role="status">
          <span class="settings-restart-choice__icon">
            <Icon name="restart" size={20} />
          </span>
          <div>
            <strong>
              Restart to apply{" "}
              {changedFields.length === 1
                ? "this change"
                : `${changedFields.length} changes`}
              ?
            </strong>
            <p>Saved safely. The reminder stays until a restart succeeds.</p>
          </div>
          <div>
            <button
              class="button button--quiet"
              type="button"
              onClick={() => setShowRestartChoice(false)}
            >
              Later
            </button>
            <button
              class="button button--primary"
              type="button"
              disabled={!canManageServers}
              title={manageTitle}
              onClick={restartNow}
            >
              Restart now
            </button>
          </div>
        </div>
      )}
      <div class="settings-grid">
        <label class="field field--wide">
          <span>Message of the day {restartLabel("motd")}</span>
          <input
            maxlength={128}
            value={settings.motd}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) => update("motd", event.currentTarget.value)}
          />
        </label>
        <label class="field">
          <span>Game mode {restartLabel("game_mode")}</span>
          <select
            value={settings.gameMode}
            disabled={!canManageServers}
            title={manageTitle}
            onChange={(event) =>
              update(
                "gameMode",
                event.currentTarget.value as MinecraftSettings["gameMode"],
              )
            }
          >
            <option value="survival">Survival</option>
            <option value="creative">Creative</option>
            <option value="adventure">Adventure</option>
            <option value="spectator">Spectator</option>
          </select>
        </label>
        <label class="field">
          <span>Difficulty {restartLabel("difficulty")}</span>
          <select
            value={settings.difficulty}
            disabled={!canManageServers}
            title={manageTitle}
            onChange={(event) =>
              update(
                "difficulty",
                event.currentTarget.value as MinecraftSettings["difficulty"],
              )
            }
          >
            <option value="peaceful">Peaceful</option>
            <option value="easy">Easy</option>
            <option value="normal">Normal</option>
            <option value="hard">Hard</option>
          </select>
        </label>
        <label class="field">
          <span>Maximum players {restartLabel("max_players")}</span>
          <input
            type="number"
            min={1}
            max={10_000}
            value={settings.maxPlayers}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) =>
              update("maxPlayers", event.currentTarget.valueAsNumber)
            }
          />
        </label>
        <label class="field">
          <span>Idle kick (minutes) {restartLabel("player_idle_timeout")}</span>
          <input
            type="number"
            min={0}
            max={65_535}
            value={settings.playerIdleTimeout}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) =>
              update("playerIdleTimeout", event.currentTarget.valueAsNumber)
            }
          />
        </label>
        <label class="field">
          <span>View distance {restartLabel("view_distance")}</span>
          <input
            type="number"
            min={2}
            max={32}
            value={settings.viewDistance}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) =>
              update("viewDistance", event.currentTarget.valueAsNumber)
            }
          />
        </label>
        <label class="field">
          <span>Simulation distance {restartLabel("simulation_distance")}</span>
          <input
            type="number"
            min={2}
            max={32}
            value={settings.simulationDistance}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) =>
              update("simulationDistance", event.currentTarget.valueAsNumber)
            }
          />
        </label>
        <label class="field">
          <span>Spawn protection {restartLabel("spawn_protection")}</span>
          <input
            type="number"
            min={0}
            max={65_535}
            value={settings.spawnProtection}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) =>
              update("spawnProtection", event.currentTarget.valueAsNumber)
            }
          />
        </label>
      </div>
      <div class="toggle-grid">
        {(
          [
            {
              key: "onlineMode",
              field: "online_mode",
              title: "Online authentication",
              detail:
                "Verify players with Mojang. Keep this on for public or internet-facing servers.",
            },
            {
              key: "pvp",
              field: "pvp",
              title: "Player combat",
              detail: "Allow players to damage one another.",
            },
            {
              key: "allowFlight",
              field: "allow_flight",
              title: "Allow flight",
              detail:
                "Do not kick players when a mod or plugin enables flight.",
            },
            {
              key: "whiteList",
              field: "white_list",
              title: "Whitelist",
              detail: "Only approved players can join.",
            },
            {
              key: "enforceWhiteList",
              field: "enforce_white_list",
              title: "Enforce whitelist",
              detail: "Remove online players when they are no longer approved.",
            },
          ] as const
        ).map((item) => (
          <label class="setting-toggle" key={item.key}>
            <input
              type="checkbox"
              checked={settings[item.key]}
              disabled={!canManageServers}
              title={manageTitle}
              onChange={(event) =>
                update(item.key, event.currentTarget.checked)
              }
            />
            <span>
              <strong>
                {item.title} {restartLabel(item.field)}
              </strong>
              <small>{item.detail}</small>
            </span>
            <i />
          </label>
        ))}
      </div>
      <div class="settings-footer">
        <span>
          {!canManageServers
            ? "View only · games.manage is required to change settings"
            : dirty
              ? "Unsaved changes"
              : restartPending
                ? "Saved · restart when ready"
                : "Settings match the saved file"}
        </span>
        <button
          class="button button--primary"
          type="button"
          disabled={
            !canManageServers ||
            !dirty ||
            busy ||
            settings.motd.trim().length === 0
          }
          title={manageTitle}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save settings"}
        </button>
      </div>
    </section>
  );
}

function LogsPanel({
  detail,
  csrfToken,
}: {
  detail: NativeServerDetail;
  csrfToken: string;
}) {
  const [snapshot, setSnapshot] = useState<ServerLogSnapshot | null>(null);
  const [live, setLive] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async (): Promise<void> => {
    try {
      setSnapshot(await getServerLogs(detail.id, csrfToken));
      setError(null);
    } catch (requestError) {
      setError(describeError(requestError));
    }
  }, [csrfToken, detail.id]);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(() => void load(), 4_000);
    return () => window.clearInterval(timer);
  }, [live, load]);
  return (
    <section class="server-tool logs-panel">
      <div class="tool-head">
        <div>
          <h2>Process logs</h2>
          <p>The most recent 500 lines from the isolated workload.</p>
        </div>
        <div class="log-actions">
          <label>
            <input
              class="toggle-input"
              type="checkbox"
              checked={live}
              onChange={(event) => setLive(event.currentTarget.checked)}
            />{" "}
            Live
          </label>
          <button
            class="button button--quiet"
            type="button"
            onClick={() => void load()}
          >
            <Icon name="refresh" size={15} />
            Refresh
          </button>
        </div>
      </div>
      <InlineError message={error} />
      <pre class="log-screen">
        {snapshot === null
          ? "Loading logs…"
          : snapshot.lines.length === 0
            ? "No log output yet."
            : snapshot.lines.join("\n")}
      </pre>
      <small class="tool-foot">
        {snapshot === null
          ? "Waiting for the first snapshot"
          : `Updated ${formatTimestamp(snapshot.collectedAtUnixMs)}`}
      </small>
    </section>
  );
}

function RestoreBackupDialog({
  server,
  backup,
  csrfToken,
  onClose,
  onComplete,
  onSessionExpired,
}: {
  server: ManagedServer;
  backup: ServerBackup;
  csrfToken: string;
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const polling = useJobPolling({
    job,
    csrfToken,
    onJob: setJob,
    onComplete,
    onSessionExpired,
  });
  const submit = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const dispatch = await restoreServerBackup(
        server.id,
        backup.id,
        csrfToken,
      );
      if (dispatch.jobId === null) {
        await onComplete();
        onClose();
      } else {
        setJob({
          id: dispatch.jobId,
          kind: "server_restore",
          status: "queued",
          stage: "Queued",
          progressPercent: 0,
          createdAtUnixMs: Date.now(),
          updatedAtUnixMs: Date.now(),
          result: null,
          error: null,
        });
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  if (job !== null) {
    const active = job.status === "queued" || job.status === "running";
    const canClose = !active || polling.paused;
    return (
      <Dialog
        title={
          job.status === "failed"
            ? "Restore failed safely"
            : job.status === "complete"
              ? "Restore complete"
              : "Restoring server"
        }
        onClose={() => canClose && onClose()}
      >
        <div class="job-progress">
          <div class={`job-icon job-icon--${job.status}`}>
            <Icon
              name={
                job.status === "complete"
                  ? "check"
                  : job.status === "failed"
                    ? "warning"
                    : "backup"
              }
              size={26}
            />
          </div>
          <strong>{job.stage}</strong>
          <span>
            {active
              ? "Helix is making a safety backup, restoring the selected files, and validating Minecraft."
              : job.status === "complete"
                ? `${server.name} was restored and validated.`
                : (job.error ?? "The old server was put back.")}
          </span>
          <ProgressBar
            value={active ? 18 : job.progressPercent}
            tone={job.status === "failed" ? "danger" : "normal"}
          />
        </div>
        <InlineError message={polling.error ?? error} />
        <div class="dialog-actions">
          {polling.paused && (
            <button
              class="button button--quiet"
              type="button"
              onClick={polling.resume}
            >
              Resume status check
            </button>
          )}
          <button
            class="button button--primary"
            disabled={!canClose}
            type="button"
            onClick={onClose}
          >
            Close
          </button>
        </div>
      </Dialog>
    );
  }
  return (
    <Dialog
      title={`Restore ${server.name}?`}
      onClose={() => !busy && onClose()}
    >
      <div class="dialog-copy">
        <p>
          Helix will stop Minecraft, create a fresh safety backup, restore the
          selected archive, and only keep it active if the server passes its
          health check.
        </p>
        <p>
          <strong>{formatTimestamp(backup.createdAtUnixMs)}</strong> ·{" "}
          {formatBytes(backup.sizeBytes)}
        </p>
      </div>
      <label class="check-row">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>
          <strong>I understand current files will be replaced</strong>
          <small>
            The current version remains recoverable as both a backup and a
            recovery directory.
          </small>
        </span>
      </label>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button
          class="button button--quiet"
          type="button"
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          class="button button--danger"
          type="button"
          disabled={!confirmed || busy}
          onClick={() => void submit()}
        >
          {busy ? "Queuing…" : "Restore backup"}
        </button>
      </div>
    </Dialog>
  );
}

function DeleteBackupDialog({
  server,
  backup,
  csrfToken,
  onClose,
  onDeleted,
  onSessionExpired,
}: {
  server: ManagedServer;
  backup: ServerBackup;
  csrfToken: string;
  onClose: () => void;
  onDeleted: (trashId: string, backup: ServerBackup) => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const result = await trashServerBackup(server.id, backup.id, csrfToken);
      await onDeleted(result.trashId, backup);
      onClose();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog title="Move backup to trash?" onClose={() => !busy && onClose()}>
      <div class="dialog-copy">
        <p>
          <strong>{formatTimestamp(backup.createdAtUnixMs)}</strong> ·{" "}
          {formatBytes(backup.sizeBytes)}
        </p>
        <p>
          Helix moves this backup to protected trash, where Undo can restore it.
        </p>
      </div>
      <label class="check-row">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>
          <strong>Remove this active backup</strong>
          <small>A recoverable copy stays under Deleted backups.</small>
        </span>
      </label>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button
          class="button button--quiet"
          type="button"
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          class="button button--danger"
          type="button"
          disabled={!confirmed || busy}
          onClick={() => void remove()}
        >
          {busy ? "Moving…" : "Move to trash"}
        </button>
      </div>
    </Dialog>
  );
}

function BackupsPanel({
  server,
  csrfToken,
  refreshKey,
  canManageServers,
  canManageBackups,
  canManageTrash,
  onCreate,
  onRefresh,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  refreshKey: number;
  canManageServers: boolean;
  canManageBackups: boolean;
  canManageTrash: boolean;
  onCreate: () => void;
  onRefresh: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [backups, setBackups] = useState<ServerBackup[]>([]);
  const [trash, setTrash] = useState<ServerBackupTrash[]>([]);
  const [trashPolicy, setTrashPolicy] =
    useState<ServerBackupTrashPolicy | null>(null);
  const [restore, setRestore] = useState<ServerBackup | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ServerBackup | null>(null);
  const [immediateUndo, setImmediateUndo] = useState<{
    trashId: string;
    backup: ServerBackup;
  } | null>(null);
  const [undoing, setUndoing] = useState<string | null>(null);
  const [visibleTrash, setVisibleTrash] = useState(25);
  const [error, setError] = useState<string | null>(null);
  const canCreate = canRunBackupMutation(
    "create",
    canManageServers,
    canManageBackups,
    canManageTrash,
  );
  const canRestore = canRunBackupMutation(
    "restore",
    canManageServers,
    canManageBackups,
    canManageTrash,
  );
  const canUseTrash = canRunBackupMutation(
    "trash",
    canManageServers,
    canManageBackups,
    canManageTrash,
  );
  const load = useCallback(async (): Promise<void> => {
    try {
      const catalog = await getServerBackups(server.id, csrfToken);
      setBackups(catalog.backups);
      setTrash(catalog.trash);
      setTrashPolicy(catalog.trashPolicy);
      setError(null);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    }
  }, [csrfToken, onSessionExpired, server.id]);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);
  const undo = async (trashIdValue: string): Promise<void> => {
    if (!canUseTrash || undoing !== null) return;
    setUndoing(trashIdValue);
    setError(null);
    try {
      await restoreTrashedServerBackup(server.id, trashIdValue, csrfToken);
      if (immediateUndo?.trashId === trashIdValue) setImmediateUndo(null);
      await load();
      await onRefresh();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setUndoing(null);
    }
  };
  return (
    <section class="server-tool backups-panel">
      <div class="tool-head">
        <div>
          <h2>
            Backups{" "}
            <InfoTip text="Deleted backups stay recoverable in protected trash." />
          </h2>
          <p>Local archives with the matching Helix server definition.</p>
        </div>
        <button
          class="button button--primary"
          type="button"
          disabled={!canCreate}
          title={canCreate ? undefined : "Requires games.manage permission"}
          onClick={onCreate}
        >
          <Icon name="backup" size={15} />
          Back up now
        </button>
      </div>
      <InlineError message={error} />
      {!canManageBackups && (
        <div class="backup-capability-note">
          <Icon name="info" size={15} />
          <span>
            You can inspect backups, but restoring or deleting them requires
            games.backups.manage permission.
          </span>
        </div>
      )}
      {canManageBackups && !canManageTrash && (
        <div class="backup-capability-note">
          <Icon name="info" size={15} />
          <span>
            Recoverable deletion is unavailable here. Existing restore controls
            still work.
          </span>
        </div>
      )}
      {immediateUndo !== null && (
        <div class="backup-undo-notice" role="status">
          <Icon name="trash" size={17} />
          <div>
            <strong>Backup moved to trash</strong>
            <span>
              {formatTimestamp(immediateUndo.backup.createdAtUnixMs)} remains
              recoverable.
            </span>
          </div>
          <button
            class="button button--primary"
            type="button"
            disabled={!canUseTrash || undoing !== null}
            onClick={() => void undo(immediateUndo.trashId)}
          >
            {undoing === immediateUndo.trashId ? "Restoring…" : "Undo"}
          </button>
          <button
            class="icon-button"
            type="button"
            onClick={() => setImmediateUndo(null)}
            aria-label="Dismiss backup undo"
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
      <div class="backup-list">
        {backups.map((backup) => (
          <div key={backup.id}>
            <span class="backup-icon">
              <Icon name="backup" />
            </span>
            <div>
              <strong>{formatTimestamp(backup.createdAtUnixMs)}</strong>
              <small>
                {formatBytes(backup.sizeBytes)} ·{" "}
                {backup.definitionPresent ? "Restorable" : "Archive only"}
              </small>
            </div>
            <div class="backup-row-actions">
              <button
                class="button button--quiet"
                type="button"
                disabled={!canRestore || !backup.definitionPresent}
                title={
                  canRestore
                    ? undefined
                    : "Requires games.backups.manage permission"
                }
                onClick={() => setRestore(backup)}
              >
                Restore
              </button>
              <button
                class="icon-button icon-button--danger"
                type="button"
                disabled={!canUseTrash}
                onClick={() => setDeleteTarget(backup)}
                aria-label={`Move backup from ${formatTimestamp(backup.createdAtUnixMs)} to trash`}
                title={
                  !canManageBackups
                    ? "Requires games.backups.manage permission"
                    : canManageTrash
                      ? "Move to recoverable trash"
                      : "Recoverable deletion is unavailable"
                }
              >
                <Icon name="trash" size={15} />
              </button>
            </div>
          </div>
        ))}
        {backups.length === 0 && (
          <div class="empty-state">
            <Icon name="backup" size={26} />
            <strong>No active backups</strong>
            <span>
              Create one before changing plugins, worlds, or server versions.
            </span>
          </div>
        )}
      </div>
      {(trash.length > 0 || trashPolicy !== null) && (
        <section class="deleted-backups">
          <div class="deleted-backups__head">
            <div>
              <h3>Deleted backups</h3>
              <p>{trashPolicy?.note}</p>
            </div>
            <span>{trash.length}</span>
          </div>
          <div class="deleted-backup-list">
            {trash.slice(0, visibleTrash).map((item) => (
              <div key={item.trashId}>
                <span class="backup-icon backup-icon--deleted">
                  <Icon name="trash" />
                </span>
                <div>
                  <strong>{formatTimestamp(item.trashedAtUnixMs)}</strong>
                  <small>
                    {formatBytes(item.sizeBytes)} ·{" "}
                    {item.definitionPresent
                      ? "Restorable definition included"
                      : "Archive only"}
                  </small>
                </div>
                <button
                  class="button button--quiet"
                  type="button"
                  disabled={
                    !canUseTrash || !item.undoAvailable || undoing !== null
                  }
                  title={
                    canManageBackups
                      ? undefined
                      : "Requires games.backups.manage permission"
                  }
                  onClick={() => void undo(item.trashId)}
                >
                  {undoing === item.trashId ? "Restoring…" : "Undo"}
                </button>
              </div>
            ))}
          </div>
          {visibleTrash < trash.length && (
            <button
              class="deleted-backups__more"
              type="button"
              onClick={() =>
                setVisibleTrash((count) => Math.min(trash.length, count + 25))
              }
            >
              Show {Math.min(25, trash.length - visibleTrash)} more
            </button>
          )}
        </section>
      )}
      {restore !== null && (
        <RestoreBackupDialog
          server={server}
          backup={restore}
          csrfToken={csrfToken}
          onClose={() => setRestore(null)}
          onComplete={async () => {
            await load();
            await onRefresh();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
      {deleteTarget !== null && (
        <DeleteBackupDialog
          server={server}
          backup={deleteTarget}
          csrfToken={csrfToken}
          onClose={() => setDeleteTarget(null)}
          onDeleted={async (trashIdValue, backup) => {
            setImmediateUndo({ trashId: trashIdValue, backup });
            await load();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
    </section>
  );
}

function PerformancePanel({ detail }: { detail: NativeServerDetail }) {
  const [samples, setSamples] = useState<
    Array<{ cpu: number; memory: number }>
  >([]);
  useEffect(
    () =>
      setSamples((items) => [
        ...items.slice(-23),
        { cpu: detail.cpuPercent, memory: detail.memoryUsedMb },
      ]),
    [detail.cpuPercent, detail.memoryUsedMb],
  );
  const memoryPercent =
    detail.memoryLimitMb === 0
      ? 0
      : (detail.memoryUsedMb / detail.memoryLimitMb) * 100;
  const startedAt =
    typeof detail.containerState.StartedAt === "string"
      ? Date.parse(detail.containerState.StartedAt)
      : Number.NaN;
  const uptime =
    Number.isFinite(startedAt) && detail.status !== "stopped"
      ? Math.max(0, (Date.now() - startedAt) / 1000)
      : null;
  return (
    <section class="server-tool performance-panel">
      <div class="tool-head">
        <div>
          <h2>Live performance</h2>
          <p>
            Runtime use from the Helix-owned container. Samples build while this
            page is open.
          </p>
        </div>
        <span
          class={`state-label state-label--${detail.status === "online" ? "good" : "idle"}`}
        >
          {detail.status}
        </span>
      </div>
      <div class="performance-cards">
        <Metric
          icon="cpu"
          label="CPU"
          value={formatPercent(detail.cpuPercent)}
          detail="Current container use"
          percent={detail.cpuPercent}
        />
        <Metric
          icon="memory"
          label="Memory"
          value={`${formatBytes(detail.memoryUsedMb * 1024 * 1024)} / ${formatBytes(detail.memoryLimitMb * 1024 * 1024)}`}
          detail={`${formatPercent(memoryPercent)} of Java limit`}
          percent={memoryPercent}
        />
        <Metric
          icon="storage"
          label="Server files"
          value={formatBytes(detail.diskBytes)}
          detail={detail.dataPath}
        />
        <Metric
          icon="activity"
          label="Runtime uptime"
          value={uptime === null ? "Stopped" : formatDuration(uptime)}
          detail={`${detail.playersOnline} connected players`}
        />
      </div>
      <div class="sample-chart">
        <div class="chart-head">
          <strong>Recent memory use</strong>
          <span>{samples.length} / 24 samples</span>
        </div>
        <div class="chart-bars">
          {samples.map((sample, index) => (
            <i
              key={index}
              style={{
                height: `${Math.max(3, Math.min(100, (sample.memory / detail.memoryLimitMb) * 100))}%`,
              }}
              title={`${sample.memory} MiB`}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

function RemoveNativeServerDialog({
  server,
  csrfToken,
  onClose,
  onRemoved,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  onClose: () => void;
  onRemoved: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async (): Promise<void> => {
    if (confirmation !== server.name || busy) return;
    setBusy(true);
    setError(null);
    try {
      await trashNativeServer(server.id, confirmation, csrfToken);
      await onRemoved();
      onClose();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog title={`Remove ${server.name}?`} onClose={() => !busy && onClose()}>
      <div class="dialog-copy">
        <p>
          Helix will stop and remove its exact Docker workload, then move the
          server files into protected recovery storage. Backups, console
          history, and the custom icon remain intact.
        </p>
        <p>
          <strong>This does not permanently erase the world.</strong> The
          Removed servers section can restore it in a stopped state.
        </p>
      </div>
      <label class="field">
        <span>
          Type <strong>{server.name}</strong> to confirm
        </span>
        <input
          autofocus
          autocomplete="off"
          value={confirmation}
          disabled={busy}
          onInput={(event) => setConfirmation(event.currentTarget.value)}
        />
      </label>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button
          class="button button--quiet"
          type="button"
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          class="button button--danger"
          type="button"
          disabled={busy || confirmation !== server.name}
          onClick={() => void remove()}
        >
          {busy ? "Removing safely…" : "Remove server"}
        </button>
      </div>
    </Dialog>
  );
}

function formatJoinAddress(host: string, port: number): string {
  return `${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${port}`;
}

function portDiagnostic(port: GamePortMapping | undefined): {
  tone: "good" | "warning";
  label: string;
  detail: string;
} {
  if (port === undefined)
    return {
      tone: "warning",
      label: "Evidence unavailable",
      detail: "Refresh Network before relying on this address.",
    };
  if (!port.dockerPublished)
    return {
      tone: "warning",
      label: "Not published",
      detail: "The Docker port binding is missing.",
    };
  if (!port.listenerBound)
    return {
      tone: "warning",
      label: "Not listening",
      detail: "The workload has not opened this port yet.",
    };
  if (port.firewallInputAllowance.allowed === true)
    return {
      tone: "good",
      label: "Host ready",
      detail: "Docker and the active host firewall report the port available.",
    };
  if (port.firewallInputAllowance.state === "ufw_inactive")
    return {
      tone: "warning",
      label: "Firewall inactive",
      detail:
        "Docker is published, but UFW is inactive; router reachability is still unknown.",
    };
  return {
    tone: "warning",
    label: "Firewall needs attention",
    detail: "The host does not have a verified matching allowance.",
  };
}

function NativeServerPage({
  server,
  csrfToken,
  canManageServers,
  canManageBackups,
  canManageNetwork,
  hostInventory,
  onBack,
  onRefresh,
  onSessionExpired,
  refreshIntervalMs,
}: {
  server: ManagedServer;
  csrfToken: string;
  canManageServers: boolean;
  canManageBackups: boolean;
  canManageNetwork: boolean;
  hostInventory: HostInventory | null;
  onBack: () => void;
  onRefresh: () => Promise<void>;
  onSessionExpired: () => void;
  refreshIntervalMs: RefreshIntervalMs;
}) {
  const [tab, setTab] = useState<NativeServerTab>("overview");
  const [detail, setDetail] = useState<NativeServerDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<ServerAction | null>(null);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [network, setNetwork] = useState<NetworkInventory | null>(null);
  const [networkError, setNetworkError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [restartSuccessRevision, setRestartSuccessRevision] = useState(0);
  const [exposureBusy, setExposureBusy] = useState(false);
  const detailLoad = useRef<Promise<void> | null>(null);
  const detailController = useRef<AbortController | null>(null);
  const load = useCallback((force = false): Promise<void> => {
    if (!force && detailLoad.current !== null) return detailLoad.current;
    if (force) detailController.current?.abort();
    const controller = new AbortController();
    detailController.current = controller;
    const request = (async (): Promise<void> => {
    try {
      setDetail(await getServerDetail(server.id, csrfToken, controller.signal));
      setError(null);
    } catch (requestError) {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      if (detailController.current === controller) {
        detailController.current = null;
        detailLoad.current = null;
      }
    }
    })();
    detailLoad.current = request;
    return request;
  }, [csrfToken, onSessionExpired, server.id]);
  useEffect(() => {
    void load();
    return () => detailController.current?.abort();
  }, [load]);
  useEffect(() => {
    const controller = new AbortController();
    void getNetworkInventory(csrfToken, controller.signal)
      .then((value) => {
        setNetwork(value);
        setNetworkError(null);
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setNetworkError(describeError(requestError));
      });
    return () => controller.abort();
  }, [csrfToken, onSessionExpired, server.id, refreshKey]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void load();
    }, Math.max(refreshIntervalMs, 5_000));
    return () => window.clearInterval(timer);
  }, [load, refreshIntervalMs]);
  const refresh = useCallback(async (): Promise<void> => {
    await Promise.all([load(true), onRefresh()]);
    setRefreshKey((value) => value + 1);
  }, [load, onRefresh]);
  const completePendingAction = useCallback(async (): Promise<void> => {
    if (pending === "restart") setRestartSuccessRevision((value) => value + 1);
    await refresh();
  }, [pending, refresh]);
  if (detail === null)
    return (
      <div class="page page--server-detail">
        <button class="back-link" type="button" onClick={onBack}>
          <Icon name="back" size={16} />
          All servers
        </button>
        <InlineError message={error} />
        <div class="detail-loading">
          <Icon name="servers" size={28} />
          <span>Opening {server.name}…</span>
        </div>
      </div>
    );
  const online = detail.status === "online";
  const tailscaleAddress =
    hostInventory?.interfaces
      .find((item) => item.name.toLowerCase().startsWith("tailscale"))
      ?.addresses.find(
        (address) => address.scope === "global" || address.scope === "universe",
      )?.address ?? null;
  const tcpEvidence = network?.gamePorts.find(
    (item) => item.instanceId === detail.id && item.protocol === "tcp",
  );
  const udpEvidence = network?.gamePorts.find(
    (item) => item.instanceId === detail.id && item.protocol === "udp",
  );
  const joinAddress =
    tcpEvidence?.privateJoinAddress ??
    (network?.addresses.privateIpv4 === null || network?.addresses.privateIpv4 === undefined
      ? "Private address unavailable"
      : formatJoinAddress(network.addresses.privateIpv4, detail.gamePort));
  const publicEvidence = tcpEvidence?.externalReachability;
  const publicConfigured = publicEvidence?.state === "router_mapping_confirmed";
  const publicJoinAddress = publicEvidence?.joinAddress ?? null;
  const updateExposure = async (enabled: boolean): Promise<void> => {
    setExposureBusy(true);
    setNetworkError(null);
    try {
      await setServerNetworkExposure(detail.id, enabled, csrfToken);
      await refresh();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setNetworkError(describeError(requestError));
    } finally {
      setExposureBusy(false);
    }
  };
  const tcpDiagnostic = portDiagnostic(tcpEvidence);
  const udpDiagnostic = portDiagnostic(udpEvidence);
  const manageTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  return (
    <div class="page page--server-detail">
      <button class="back-link" type="button" onClick={onBack}>
        <Icon name="back" size={16} />
        All servers
      </button>
      <header class="server-detail-head">
        <div class="server-title">
          <ServerArtwork server={server} size="detail" />
          <div>
            <span class="eyebrow">
              HELIX MANAGED · {detail.software.toUpperCase()}
            </span>
            <h1>{detail.name}</h1>
            <p>
              {joinAddress} · {detail.minecraftVersion} · Java{" "}
              {detail.javaVersion}
            </p>
          </div>
        </div>
        <div class="server-detail-actions">
          <button
            class="button button--quiet"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => setAppearanceOpen(true)}
          >
            <Icon name="edit" size={15} />
            Icon
          </button>
          {online ? (
            <>
              <button
                class="button button--quiet"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => setPending("restart")}
              >
                <Icon name="restart" size={15} />
                Restart
              </button>
              <button
                class="button button--danger-quiet"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => setPending("stop")}
              >
                <Icon name="stop" size={15} />
                Stop
              </button>
            </>
          ) : (
            <button
              class="button button--primary"
              type="button"
              disabled={!canManageServers}
              title={manageTitle}
              onClick={() => setPending("start")}
            >
              <Icon name="play" size={15} />
              Start server
            </button>
          )}
          <button
            class="icon-button"
            type="button"
            onClick={() => void refresh()}
            aria-label="Refresh server"
          >
            <Icon name="refresh" size={17} />
          </button>
        </div>
      </header>
      <InlineError message={error} />
      <nav class="server-tabs" aria-label="Server tools">
        {nativeServerTabs
          .filter(
            (item) =>
              item.id === "overview" ||
              (item.id === "marketplace"
                ? supportsMarketplaceSoftware(detail.software)
                : detail.capabilities.includes(item.id)),
          )
          .map((item) => (
            <button
              class={tab === item.id ? "is-active" : ""}
              type="button"
              key={item.id}
              onPointerEnter={
                item.id === "marketplace" ? preloadMarketplaceRoute : undefined
              }
              onFocus={
                item.id === "marketplace" ? preloadMarketplaceRoute : undefined
              }
              onClick={() => setTab(item.id)}
            >
              <Icon name={item.icon} size={16} />
              {item.label}
            </button>
          ))}
      </nav>
      <div class="server-tab-content">
        {tab === "overview" && (
          <>
            <section class="join-card join-card--diagnostics">
              <div class="join-address-grid">
                <article>
                  <span>LOCAL / LAN</span>
                  <strong>{joinAddress}</strong>
                  <small>Use this address from the same LAN.</small>
                  <button
                    type="button"
                    onClick={() =>
                      void navigator.clipboard?.writeText(joinAddress)
                    }
                  >
                    Copy
                  </button>
                </article>
                <article>
                  <span>TAILSCALE</span>
                  <strong>
                    {tailscaleAddress === null
                      ? "Not detected"
                      : formatJoinAddress(tailscaleAddress, detail.gamePort)}
                  </strong>
                  <small>
                    {tailscaleAddress === null
                      ? "Install and connect Tailscale from Hooks to add this private address."
                      : "Use from devices on the same tailnet."}
                  </small>
                  {tailscaleAddress !== null && (
                    <button
                      type="button"
                      onClick={() =>
                        void navigator.clipboard?.writeText(
                          formatJoinAddress(tailscaleAddress, detail.gamePort),
                        )
                      }
                    >
                      Copy
                    </button>
                  )}
                </article>
                <article>
                  <span>PUBLIC INTERNET</span>
                  <strong>
                    {publicConfigured && publicJoinAddress !== null
                      ? publicJoinAddress
                      : publicEvidence?.state === "carrier_grade_nat"
                        ? "Blocked by CGNAT"
                        : publicEvidence?.state === "private_or_reserved"
                          ? "Upstream NAT detected"
                          : publicEvidence?.state === "setup_available"
                            ? "Ready to set up"
                            : "Automatic setup unavailable"}
                  </strong>
                  <small>
                    {publicEvidence?.note ?? "Refresh Network to check the router and public address."}
                  </small>
                  {publicConfigured && publicJoinAddress !== null && (
                    <button type="button" onClick={() => void navigator.clipboard?.writeText(publicJoinAddress)}>Copy</button>
                  )}
                  {publicEvidence?.state === "setup_available" && (
                    <button
                      class="button button--small"
                      type="button"
                      disabled={exposureBusy || !canManageNetwork}
                      title={canManageNetwork ? undefined : "Requires network.firewall.write permission"}
                      onClick={() => void updateExposure(true)}
                    >
                      {exposureBusy ? "Setting up…" : "Set up public access"}
                    </button>
                  )}
                  {publicConfigured && (
                    <button
                      class="button button--small button--quiet"
                      type="button"
                      disabled={exposureBusy || !canManageNetwork}
                      onClick={() => void updateExposure(false)}
                    >
                      {exposureBusy ? "Removing…" : "Remove forwarding"}
                    </button>
                  )}
                  {!publicConfigured && publicEvidence?.state !== "setup_available" && <a href="#network">Review Network</a>}
                </article>
              </div>
              <InlineError message={networkError} />
              <div class="join-evidence">
                <span
                  class={`state-label state-label--${tcpDiagnostic.tone}`}
                >
                  TCP · {tcpDiagnostic.label}
                </span>
                <span
                  class={`state-label state-label--${udpDiagnostic.tone}`}
                >
                  UDP · {udpDiagnostic.label}
                </span>
                <small>
                  {networkError ??
                    `${tcpDiagnostic.detail} ${udpDiagnostic.detail}`}
                </small>
              </div>
            </section>
            <div class="server-overview-grid">
              <section class="surface server-health">
                <div class="section-title">
                  <div>
                    <h2>Right now</h2>
                    <p>Live Minecraft and runtime state</p>
                  </div>
                  <span
                    class={`state-label state-label--${online ? "good" : "idle"}`}
                  >
                    {detail.status}
                  </span>
                </div>
                <div class="server-health-stats">
                  <div>
                    <span>Players</span>
                    <strong>
                      {detail.playersOnline} / {detail.maxPlayers}
                    </strong>
                  </div>
                  <div>
                    <span>CPU</span>
                    <strong>{formatPercent(detail.cpuPercent)}</strong>
                  </div>
                  <div>
                    <span>Memory</span>
                    <strong>
                      {formatBytes(detail.memoryUsedMb * 1024 * 1024)}
                    </strong>
                    <small>
                      of {formatBytes(detail.memoryLimitMb * 1024 * 1024)}
                    </small>
                  </div>
                  <div>
                    <span>Files</span>
                    <strong>{formatBytes(detail.diskBytes)}</strong>
                  </div>
                </div>
              </section>
              <section class="surface server-facts">
                <div class="section-title">
                  <div>
                    <h2>Build</h2>
                    <p>Resolved and pinned by Helix</p>
                  </div>
                </div>
                <dl>
                  <div>
                    <dt>Software</dt>
                    <dd>{detail.software}</dd>
                  </div>
                  <div>
                    <dt>Minecraft</dt>
                    <dd>{detail.minecraftVersion}</dd>
                  </div>
                  <div>
                    <dt>Build</dt>
                    <dd>{detail.build}</dd>
                  </div>
                  <div>
                    <dt>Java</dt>
                    <dd>{detail.javaVersion}</dd>
                  </div>
                  <div>
                    <dt>Created</dt>
                    <dd>{formatTimestamp(detail.createdAtUnixMs)}</dd>
                  </div>
                  <div>
                    <dt>Startup</dt>
                    <dd>{detail.startOnBoot ? "With host" : "Manual"}</dd>
                  </div>
                </dl>
              </section>
            </div>
          </>
        )}
        {tab === "console" && (
          <ConsolePanel
            detail={detail}
            csrfToken={csrfToken}
            canManageServers={canManageServers}
            onSessionExpired={onSessionExpired}
          />
        )}
        {tab === "settings" && (
          <SettingsPanel
            detail={detail}
            csrfToken={csrfToken}
            canManageServers={canManageServers}
            restartSuccessRevision={restartSuccessRevision}
            onRestart={() => setPending("restart")}
            onSessionExpired={onSessionExpired}
          />
        )}
        {tab === "files" && (
          <FileManager
            csrfToken={csrfToken}
            onSessionExpired={onSessionExpired}
            initialPath={detail.dataPath}
          />
        )}
        {tab === "backups" && (
          <BackupsPanel
            server={server}
            csrfToken={csrfToken}
            refreshKey={refreshKey}
            canManageServers={canManageServers}
            canManageBackups={canManageBackups}
            canManageTrash={detail.capabilities.includes(
              "recoverable_backup_trash",
            )}
            onCreate={() => setPending("backup")}
            onRefresh={refresh}
            onSessionExpired={onSessionExpired}
          />
        )}
        {tab === "logs" && <LogsPanel detail={detail} csrfToken={csrfToken} />}
        {tab === "performance" && <PerformancePanel detail={detail} />}
        {tab === "marketplace" &&
          supportsMarketplaceSoftware(detail.software) && (
            <MarketplaceRoute
              server={{
                id: detail.id,
                name: detail.name,
                software: detail.software,
                minecraftVersion: detail.minecraftVersion,
                status: detail.status,
              }}
              csrfToken={csrfToken}
              canManageServers={canManageServers}
              onSessionExpired={onSessionExpired}
              onInstalled={refresh}
            />
          )}
        {tab === "advanced" && (
          <section class="server-tool advanced-panel">
            <div class="tool-head">
              <div>
                <h2>Runtime identity</h2>
                <p>
                  Exact facts for troubleshooting and audits. Credentials are
                  never returned here.
                </p>
              </div>
              <div class="advanced-actions">
                {detail.software.toLowerCase() === "custom jar" ? (
                  <span
                    class="advanced-update-note"
                    title="Replace a custom JAR through a reviewed backup and manual migration. Helix cannot verify its publisher or choose a compatible replacement automatically."
                  >
                    <Icon name="info" size={15} />
                    Manual JAR updates
                  </span>
                ) : (
                  <button
                    class="button button--quiet"
                    type="button"
                    disabled={!canManageServers}
                    title={manageTitle}
                    onClick={() => setPending("update")}
                  >
                    <Icon name="update" size={15} />
                    Check for update
                  </button>
                )}
                <button
                  class="button button--danger-quiet"
                  type="button"
                  disabled={!canManageServers}
                  title={manageTitle}
                  onClick={() => setRemoveOpen(true)}
                >
                  <Icon name="trash" size={15} />
                  Remove server
                </button>
              </div>
            </div>
            <dl class="advanced-facts">
              <div>
                <dt>Instance</dt>
                <dd>
                  <code>{detail.instanceName}</code>
                </dd>
              </div>
              <div>
                <dt>Backend</dt>
                <dd>Docker · isolated numeric user</dd>
              </div>
              <div>
                <dt>Runtime image</dt>
                <dd>
                  <code>{detail.runtimeImage}</code>
                </dd>
              </div>
              <div>
                <dt>Server SHA-256</dt>
                <dd>
                  <code>{detail.artifactSha256}</code>
                </dd>
              </div>
              <div>
                <dt>Data path</dt>
                <dd>
                  <code>{detail.dataPath}</code>
                </dd>
              </div>
              <div>
                <dt>Game port</dt>
                <dd>
                  <code>{detail.gamePort}/tcp + udp</code>
                </dd>
              </div>
              <div>
                <dt>Console</dt>
                <dd>Loopback only</dd>
              </div>
              <div>
                <dt>OOM killed</dt>
                <dd>
                  {detail.containerState.OOMKilled === true ? "Yes" : "No"}
                </dd>
              </div>
              <div>
                <dt>Process ID</dt>
                <dd>
                  {typeof detail.containerState.Pid === "number"
                    ? detail.containerState.Pid
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>Exit code</dt>
                <dd>
                  {typeof detail.containerState.ExitCode === "number"
                    ? detail.containerState.ExitCode
                    : "—"}
                </dd>
              </div>
            </dl>
          </section>
        )}
      </div>
      {pending !== null && (
        <ServerActionDialog
          server={server}
          action={pending}
          csrfToken={csrfToken}
          onClose={() => setPending(null)}
          onComplete={completePendingAction}
          onSessionExpired={onSessionExpired}
        />
      )}
      {appearanceOpen && (
        <ServerIconDialog
          server={server}
          csrfToken={csrfToken}
          onClose={() => setAppearanceOpen(false)}
          onSaved={refresh}
          onSessionExpired={onSessionExpired}
        />
      )}
      {removeOpen && (
        <RemoveNativeServerDialog
          server={server}
          csrfToken={csrfToken}
          onClose={() => setRemoveOpen(false)}
          onRemoved={async () => {
            onBack();
            await onRefresh();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
    </div>
  );
}

export function importedServerPanelUrl(
  server: ManagedServer,
  hostname: string,
): string | null {
  if (
    server.manager !== "amp_import" ||
    !Number.isInteger(server.managerPanelPort) ||
    server.managerPanelPort < 1 ||
    server.managerPanelPort > 65_535
  )
    return null;
  const match = /^amp:([0-9a-f]{8})(?:-[0-9a-f-]+)?$/iu.exec(server.id);
  if (
    match === null ||
    hostname.length === 0 ||
    hostname.length > 253 ||
    /[\s/?#@\\]/u.test(hostname)
  )
    return null;
  const address =
    hostname.includes(":") && !hostname.startsWith("[")
      ? `[${hostname}]`
      : hostname;
  return `http://${address}:${server.managerPanelPort}/instances/${match[1]?.toLowerCase()}`;
}

const HIDDEN_IMPORTED_SERVERS_KEY = "helix.servers.hidden-imports";

function readHiddenImportedServers(): string[] {
  try {
    const value = JSON.parse(
      globalThis.localStorage?.getItem(HIDDEN_IMPORTED_SERVERS_KEY) ?? "[]",
    ) as unknown;
    if (!Array.isArray(value)) return [];
    return [
      ...new Set(
        value.filter(
          (item): item is string =>
            typeof item === "string" && /^amp:[0-9a-f-]{8,128}$/iu.test(item),
        ),
      ),
    ].slice(0, 512);
  } catch {
    return [];
  }
}

function saveHiddenImportedServers(value: readonly string[]): void {
  try {
    globalThis.localStorage?.setItem(
      HIDDEN_IMPORTED_SERVERS_KEY,
      JSON.stringify([...new Set(value)].slice(0, 512)),
    );
  } catch {
    // This is a display preference; an unavailable browser store must not affect AMP.
  }
}

function ImportedServerPage({
  server,
  csrfToken,
  canManageServers,
  onBack,
  onRefresh,
  onHide,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  canManageServers: boolean;
  onBack: () => void;
  onRefresh: () => Promise<void>;
  onHide: () => void;
  onSessionExpired: () => void;
}) {
  const [pending, setPending] = useState<ServerAction | null>(null);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const [hideOpen, setHideOpen] = useState(false);
  const online = server.status === "online";
  const panelUrl =
    typeof window === "undefined"
      ? null
      : importedServerPanelUrl(server, window.location.hostname);
  const manageTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  return (
    <div class="page page--server-detail">
      <button class="back-link" type="button" onClick={onBack}>
        <Icon name="back" size={16} />
        All servers
      </button>
      <header class="server-detail-head">
        <div class="server-title">
          <ServerArtwork server={server} size="detail" />
          <div>
            <span class="eyebrow">IMPORTED · AMP REMAINS SEPARATE</span>
            <h1>{server.name}</h1>
            <p>
              {server.software} {server.version} · Port{" "}
              {server.gamePort ?? "not reported"}
            </p>
          </div>
        </div>
        <div class="server-detail-actions">
          <button
            class="button button--quiet"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => setAppearanceOpen(true)}
          >
            <Icon name="edit" size={15} />
            Icon
          </button>
          {online ? (
            <>
              <button
                class="button button--quiet"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => setPending("restart")}
              >
                <Icon name="restart" size={15} />
                Restart
              </button>
              <button
                class="button button--danger-quiet"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => setPending("stop")}
              >
                <Icon name="stop" size={15} />
                Stop
              </button>
            </>
          ) : (
            <button
              class="button button--primary"
              type="button"
              disabled={!canManageServers}
              title={manageTitle}
              onClick={() => setPending("start")}
            >
              <Icon name="play" size={15} />
              Start
            </button>
          )}
          <button
            class="button button--danger-quiet"
            type="button"
            disabled={!canManageServers}
            title={manageTitle}
            onClick={() => setHideOpen(true)}
          >
            <Icon name="trash" size={15} />
            Remove from Helix
          </button>
        </div>
      </header>
      <section class="imported-notice">
        <Icon name="external" />
        <div>
          <strong>This server still belongs to AMP</strong>
          <p>
            Helix reads its status and offers basic lifecycle shortcuts so the
            host is visible in one place. It does not pretend this is a
            Helix-managed server. New servers use Helix’s own manager and
            receive the full toolset.
          </p>
        </div>
        {panelUrl !== null && (
          <a
            class="button button--primary"
            href={panelUrl}
            target="_blank"
            rel="noreferrer"
          >
            Open AMP <Icon name="external" size={14} />
          </a>
        )}
      </section>
      <div class="server-overview-grid">
        <section class="surface server-health">
          <div class="section-title">
            <div>
              <h2>Imported status</h2>
              <p>Compatibility inventory from AMP</p>
            </div>
            <span
              class={`state-label state-label--${online ? "good" : "idle"}`}
            >
              {server.status.replace("_", " ")}
            </span>
          </div>
          <div class="server-health-stats">
            <div>
              <span>Players</span>
              <strong>
                {server.playersOnline} / {server.maxPlayers}
              </strong>
            </div>
            <div>
              <span>CPU</span>
              <strong>{formatPercent(server.cpuPercent)}</strong>
            </div>
            <div>
              <span>Memory</span>
              <strong>{formatBytes(server.memoryUsedMb * 1024 * 1024)}</strong>
            </div>
            <div>
              <span>Panel</span>
              <strong>{server.panelRunning ? "Running" : "Stopped"}</strong>
            </div>
          </div>
        </section>
        <section class="surface server-facts">
          <div class="section-title">
            <div>
              <h2>AMP identity</h2>
              <p>Read-only compatibility facts</p>
            </div>
          </div>
          <dl>
            <div>
              <dt>Instance</dt>
              <dd>{server.instanceName}</dd>
            </div>
            <div>
              <dt>Path</dt>
              <dd>
                <code>{server.path}</code>
              </dd>
            </div>
            <div>
              <dt>Game port</dt>
              <dd>{server.gamePort ?? "—"}</dd>
            </div>
            <div>
              <dt>Startup</dt>
              <dd>{server.startOnBoot ? "Automatic" : "Manual"}</dd>
            </div>
          </dl>
        </section>
      </div>
      {pending !== null && (
        <ServerActionDialog
          server={server}
          action={pending}
          csrfToken={csrfToken}
          onClose={() => setPending(null)}
          onComplete={onRefresh}
          onSessionExpired={onSessionExpired}
        />
      )}
      {appearanceOpen && (
        <ServerIconDialog
          server={server}
          csrfToken={csrfToken}
          onClose={() => setAppearanceOpen(false)}
          onSaved={onRefresh}
          onSessionExpired={onSessionExpired}
        />
      )}
      {hideOpen && (
        <Dialog
          title={`Remove ${server.name} from Helix?`}
          onClose={() => setHideOpen(false)}
        >
          <div class="dialog-copy">
            <p>
              This hides the imported connection in this browser. It does not
              stop or delete anything in AMP.
            </p>
            <p>
              Use <strong>Open AMP</strong> for an upstream deletion. Hidden
              connections can be restored from the Servers page.
            </p>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              onClick={() => setHideOpen(false)}
            >
              Cancel
            </button>
            <button
              class="button button--danger"
              type="button"
              onClick={() => {
                setHideOpen(false);
                onHide();
              }}
            >
              Hide connection
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}

type ServerFilter = "all" | "minecraft" | "imported";

function isMinecraftServer(server: ManagedServer): boolean {
  if (server.manager === "helix") return true;
  return /minecraft|paper|purpur|folia|fabric|forge|spigot|bukkit|velocity|sponge/iu.test(
    `${server.software} ${server.version}`,
  );
}

function NewServerChooser({
  onMinecraft,
  onClose,
}: {
  onMinecraft: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog title="New server" onClose={onClose} wide>
      <div class="game-create-grid">
        <button type="button" onClick={onMinecraft}>
          <span class="game-create-icon game-create-icon--minecraft">
            <Icon name="servers" size={28} />
          </span>
          <span>
            <strong>Minecraft: Java Edition</strong>
            <small>
              Paper, Purpur, Folia, Fabric, Vanilla, and supported Modrinth
              server packs.
            </small>
          </span>
          <em>Ready</em>
        </button>
        <article class="is-pending">
          <span class="game-create-icon game-create-icon--vrising">V</span>
          <span>
            <strong>V Rising</strong>
            <small>
              Publisher-supported server binaries are currently Windows-only.
              Helix will not pretend an unvalidated Wine container is
              production-ready.
            </small>
          </span>
          <em>Not available on Linux</em>
        </article>
      </div>
      <div class="server-platform-note">
        <Icon name="info" size={16} />
        <span>
          <strong>This is not a host setup problem</strong>The official V Rising
          dedicated server is currently Windows-only. A Linux option would
          depend on Wine, two UDP ports, persistent settings and saves,
          SteamCMD updates, and tested rollback. Helix leaves creation disabled
          instead of presenting a fake “fix” for an unverified runtime.
        </span>
      </div>
      <div class="dialog-actions">
        <button class="button button--quiet" type="button" onClick={onClose}>
          Close
        </button>
      </div>
    </Dialog>
  );
}

export function ServersPage({
  data,
  csrfToken,
  canManageServers,
  canManageBackups,
  canManageNetwork,
  onSessionExpired,
}: {
  data: DashboardData;
  csrfToken: string;
  canManageServers: boolean;
  canManageBackups: boolean;
  canManageNetwork: boolean;
  onSessionExpired: () => void;
}) {
  const [chooseGame, setChooseGame] = useState(false);
  const [creatingMinecraft, setCreatingMinecraft] = useState(false);
  const [portPoolOpen, setPortPoolOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState<ServerFilter>("all");
  const [hiddenImported, setHiddenImported] = useState<string[]>(
    readHiddenImportedServers,
  );
  const [removed, setRemoved] = useState<TrashedNativeServerCatalog | null>(
    null,
  );
  const [removedError, setRemovedError] = useState<string | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);
  const allServers = data.servers.data ?? [];
  const servers = allServers.filter(
    (server) => !hiddenImported.includes(server.id),
  );
  const hiddenServers = allServers.filter((server) =>
    hiddenImported.includes(server.id),
  );
  const selected =
    selectedId === null
      ? null
      : (servers.find((server) => server.id === selectedId) ?? null);
  const loadRemoved = useCallback(async (): Promise<void> => {
    try {
      setRemoved(await getTrashedNativeServers(csrfToken));
      setRemovedError(null);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setRemovedError(describeError(requestError));
    }
  }, [csrfToken, onSessionExpired]);
  useEffect(() => {
    void loadRemoved();
  }, [loadRemoved, servers.length]);

  const hideImportedServer = (id: string): void => {
    const next = [...new Set([...hiddenImported, id])];
    setHiddenImported(next);
    saveHiddenImportedServers(next);
    setSelectedId(null);
  };
  const showImportedServer = (id: string): void => {
    const next = hiddenImported.filter((item) => item !== id);
    setHiddenImported(next);
    saveHiddenImportedServers(next);
  };
  const restoreRemoved = async (trashId: string): Promise<void> => {
    if (restoring !== null) return;
    setRestoring(trashId);
    setRemovedError(null);
    try {
      await restoreTrashedNativeServer(trashId, csrfToken);
      await Promise.all([loadRemoved(), data.refresh()]);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setRemovedError(describeError(requestError));
    } finally {
      setRestoring(null);
    }
  };

  if (selected !== null) {
    return selected.manager === "helix" ? (
      <NativeServerPage
        server={selected}
        csrfToken={csrfToken}
        canManageServers={canManageServers}
        canManageBackups={canManageBackups}
        canManageNetwork={canManageNetwork}
        hostInventory={data.inventory.data}
        onBack={() => setSelectedId(null)}
        onRefresh={data.refresh}
        onSessionExpired={onSessionExpired}
        refreshIntervalMs={data.refreshIntervalMs}
      />
    ) : (
      <ImportedServerPage
        server={selected}
        csrfToken={csrfToken}
        canManageServers={canManageServers}
        onBack={() => setSelectedId(null)}
        onRefresh={data.refresh}
        onHide={() => hideImportedServer(selected.id)}
        onSessionExpired={onSessionExpired}
      />
    );
  }

  const visibleServers = servers.filter(
    (server) =>
      filter === "all" ||
      (filter === "minecraft"
        ? isMinecraftServer(server)
        : server.manager !== "helix"),
  );
  const online = servers.filter((server) => server.status === "online").length;
  const helixManaged = servers.filter(
    (server) => server.manager === "helix",
  ).length;
  const imported = servers.length - helixManaged;
  return (
    <div class="page page--servers">
      <PageHead
        title="Servers"
        detail="Native game hosting and clearly separated external connections."
        actions={
          <>
            <button
              class="button button--quiet"
              type="button"
              disabled={!canManageServers}
              title={canManageServers ? undefined : "Requires games.manage permission"}
              onClick={() => setPortPoolOpen(true)}
            >
              <Icon name="network" />
              Port pools
            </button>
            <button
              class="button button--primary button--create"
              type="button"
              disabled={!canManageServers}
              title={
                canManageServers ? undefined : "Requires games.manage permission"
              }
              onClick={() => setChooseGame(true)}
            >
              <Icon name="plus" />
              New server
            </button>
          </>
        }
      />
      <InlineError message={data.servers.error ?? removedError} />
      <section class="server-summary">
        <div>
          <span class="status-dot status-dot--good" />
          <strong>{online}</strong>
          <span>online</span>
        </div>
        <div>
          <strong>{helixManaged}</strong>
          <span>Helix managed</span>
        </div>
        <div>
          <strong>{imported}</strong>
          <span>imported</span>
        </div>
        <div>
          <strong>
            {servers.reduce((total, server) => total + server.playersOnline, 0)}
          </strong>
          <span>players</span>
        </div>
        <p>
          Native workloads stay under Helix; external managers remain separate.
        </p>
      </section>
      <nav class="server-game-filters" aria-label="Filter servers">
        <button
          class={filter === "all" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("all")}
        >
          All <span>{servers.length}</span>
        </button>
        <button
          class={filter === "minecraft" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("minecraft")}
        >
          Minecraft <span>{servers.filter(isMinecraftServer).length}</span>
        </button>
        <button
          class={filter === "imported" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("imported")}
        >
          Connections <span>{imported}</span>
        </button>
      </nav>
      <section class="server-list surface">
        <div class="server-list-head">
          <span>Server</span>
          <span>Players</span>
          <span>CPU</span>
          <span>Memory</span>
          <span>Port</span>
          <span>TPS</span>
          <span>Actions</span>
        </div>
        {visibleServers.map((server) => (
          <ServerRow
            key={server.id}
            server={server}
            csrfToken={csrfToken}
            canManageServers={canManageServers}
            onRefresh={data.refresh}
            onOpen={() => setSelectedId(server.id)}
            onSessionExpired={onSessionExpired}
          />
        ))}
        {data.servers.phase === "loading" && (
          <div class="table-state">Loading server workloads…</div>
        )}
        {data.servers.phase !== "loading" && visibleServers.length === 0 && (
          <div class="empty-state">
            <Icon name="servers" size={28} />
            <strong>
              {servers.length === 0
                ? "No servers yet"
                : "No servers in this view"}
            </strong>
            <span>
              {servers.length === 0
                ? "Create a native Minecraft server with New server. Helix Native stays separate from any AMP import."
                : "Create a native server, change the filter, or restore a hidden connection below."}
            </span>
          </div>
        )}
      </section>
      {((removed?.servers.length ?? 0) > 0 || hiddenServers.length > 0) && (
        <details class="removed-servers surface">
          <summary>
            <span>
              <Icon name="trash" size={16} />
              <strong>Removed and hidden</strong>
            </span>
            <em>{(removed?.servers.length ?? 0) + hiddenServers.length}</em>
          </summary>
          <div class="removed-server-list">
            {removed?.servers.map((item) => (
              <article key={item.trashId}>
                <div>
                  <strong>{item.name}</strong>
                  <span>
                    {item.software} {item.minecraftVersion} · Port{" "}
                    {item.gamePort}
                  </span>
                  <small>
                    Removed {formatTimestamp(item.trashedAtUnixMs)} · data{" "}
                    {item.dataPresent ? "preserved" : "needs attention"} ·
                    backups {item.backupsPreserved ? "preserved" : "none found"}
                  </small>
                </div>
                <button
                  class="button button--quiet"
                  type="button"
                  disabled={
                    !canManageServers || restoring !== null || !item.dataPresent
                  }
                  onClick={() => void restoreRemoved(item.trashId)}
                >
                  {restoring === item.trashId
                    ? "Restoring…"
                    : "Restore stopped"}
                </button>
              </article>
            ))}
            {hiddenServers.map((item) => (
              <article key={item.id}>
                <div>
                  <strong>{item.name}</strong>
                  <span>Hidden AMP connection</span>
                  <small>The upstream server was never changed.</small>
                </div>
                <button
                  class="button button--quiet"
                  type="button"
                  onClick={() => showImportedServer(item.id)}
                >
                  Show again
                </button>
              </article>
            ))}
          </div>
          {removed !== null && <p>{removed.policy.note}</p>}
        </details>
      )}
      {chooseGame && (
        <NewServerChooser
          onClose={() => setChooseGame(false)}
          onMinecraft={() => {
            setChooseGame(false);
            setCreatingMinecraft(true);
          }}
        />
      )}
      {creatingMinecraft && (
        <CreateServerDialog
          csrfToken={csrfToken}
          servers={servers}
          onClose={() => setCreatingMinecraft(false)}
          onComplete={async () => {
            await data.refresh();
            await loadRemoved();
          }}
          onSessionExpired={onSessionExpired}
          canManageNetwork={canManageNetwork}
        />
      )}
      {portPoolOpen && (
        <PortPoolDialog
          csrfToken={csrfToken}
          canManageNetwork={canManageNetwork}
          onClose={() => setPortPoolOpen(false)}
          onSessionExpired={onSessionExpired}
        />
      )}
    </div>
  );
}
