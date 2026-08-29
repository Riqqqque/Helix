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
  createVRisingServer,
  createValheimServer,
  createTerrariaServer,
  getDirectory,
  getMinecraftVersions,
  getTrashedNativeServers,
  getServerBackups,
  getServerDetail,
  getServerLogs,
  restoreServerBackup,
  restoreTrashedServerBackup,
  restoreTrashedNativeServer,
  runServerAction,
  saveServerSettings,
  sendConsoleCommand,
  setNativeStartOnBoot,
  setNativeMemory,
  setServerBackupPolicy,
  pruneServerBackups,
  purgeTrashedServerBackup,
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
  type MinecraftVersionCatalog,
  type NativeServerDetail,
  type ServerAction,
  type ServerBackup,
  type ServerBackupTrash,
  type ServerBackupTrashPolicy,
  type ServerBackupKeepPolicy,
  type ServerLogSnapshot,
  type TrashedNativeServerCatalog,
  serverIsLive,
  serverPlayerHeadline,
  serverPrimaryLifecycleAction,
  serverReportsTps,
  serverShowsRuntimeStats,
  serverStatusLabel,
  serverStatusTone,
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
  droppedTransferFiles,
  MAX_CUSTOM_JAR_UPLOAD_BYTES,
  uploadHostFile,
} from "./file-upload";
import {
  formatBytes,
  formatDuration,
  formatPercent,
  formatTimestamp,
} from "./format";
import { GameMark } from "./game-marks";
import { Icon, type IconName } from "./icons";
import { InfoTip } from "./info-tip";
import { useJobPolling } from "./job-polling";
import {
  getNetworkInventory,
  leftoverAmpForwardConfirmation,
  releaseAmpRouterForward,
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
import {
  getMinecraftPortPolicy,
  getTerrariaPortPolicy,
  getValheimPortPolicy,
  getVRisingPortPolicy,
  saveMinecraftPortPolicy,
  saveTerrariaPortPolicy,
  saveValheimPortPolicy,
  saveVRisingPortPolicy,
} from "./port-policy-api";
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
    id: "leaves",
    name: "Leaves",
    detail: "Paper-compatible fork with extra world and gameplay controls",
  },
  {
    id: "fabric",
    name: "Fabric",
    detail: "Lightweight mod loader for Fabric server mods",
  },
  {
    id: "neoforge",
    name: "NeoForge",
    detail: "Modern Forge-family loader for current modpacks",
  },
  {
    id: "forge",
    name: "Forge",
    detail: "Classic mod loader for 1.17+ dedicated servers",
  },
  {
    id: "quilt",
    name: "Quilt",
    detail: "Fabric-derived loader with its own mod ecosystem",
  },
  {
    id: "pufferfish",
    name: "Pufferfish",
    detail: "Paper-based performance fork from the Pufferfish CI",
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
  onSelect: (software: InstallableMinecraftSoftware) => void;
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
                ((option !== undefined && supported.has(option.id)) ||
                  (entry.id === "custom" && supported.has("custom")));
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
                        if (entry.id === "custom") onSelect("custom");
                        else if (option !== undefined) onSelect(option.id);
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

function MinecraftVersionField({
  version,
  catalog,
  loading,
  error,
  allowLatest,
  onChange,
  onRetry,
}: {
  version: string;
  catalog: MinecraftVersionCatalog | null;
  loading: boolean;
  error: string | null;
  allowLatest: boolean;
  onChange: (value: string) => void;
  onRetry: () => void;
}) {
  const options = catalog?.versions ?? [];
  const known = new Set(options);
  const current = version.trim();
  const isLatest = current.toLowerCase() === "latest";
  const catalogFailed = error !== null && options.length === 0 && !loading;
  const selectValue =
    allowLatest && (isLatest || current.length === 0)
      ? "latest"
      : known.has(current)
        ? current
        : loading
          ? ""
          : allowLatest
            ? "latest"
            : (options[0] ?? "");
  const latestLabel =
    catalog?.latestVersion !== null && catalog?.latestVersion !== undefined
      ? ` (${catalog.latestVersion})`
      : "";
  return (
    <label class="field">
      <span>Minecraft version</span>
      <select
        required
        value={selectValue}
        disabled={loading && options.length === 0}
        onChange={(event) => onChange(event.currentTarget.value)}
      >
        {loading && options.length === 0 && (
          <option value="" disabled>
            Loading published versions…
          </option>
        )}
        {allowLatest && (
          <option value="latest">Latest stable{latestLabel}</option>
        )}
        {options.map((item) => (
          <option value={item} key={item}>
            {item}
          </option>
        ))}
      </select>
      <small>
        {loading
          ? "Loading published versions…"
          : catalogFailed
            ? error
            : allowLatest
              ? "Pick a published release, or keep Latest so Helix uses the current stable build."
              : "Custom JARs need the exact Minecraft version they were built for."}
      </small>
      {catalogFailed && (
        <button class="button button--quiet" type="button" onClick={onRetry}>
          Try loading versions again
        </button>
      )}
    </label>
  );
}

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

export function serverWorkloadIsRunning(server: ManagedServer): boolean {
  if (server.manager === "helix") {
    return server.panelRunning || server.status === "online";
  }
  return server.status === "online";
}

export function joinErrorOffersPortChange(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("amp already has port")
    || lower.includes("leftover amp router mapping")
    || lower.includes("unowned router")
    || lower.includes("already has a mapping")
    || /port \d+ is already/.test(lower)
  );
}

export function parseAmpPortClaim(message: string): { port: number; leftover: boolean } | null {
  const leftover = /^Leftover AMP router mapping on port (\d+)\b/u.exec(message);
  if (leftover) {
    const port = Number(leftover[1]);
    if (Number.isInteger(port) && port >= 1 && port <= 65_535) return { port, leftover: true };
  }
  const live = /^AMP already has port (\d+) claimed/u.exec(message);
  if (live) {
    const port = Number(live[1]);
    if (Number.isInteger(port) && port >= 1 && port <= 65_535) return { port, leftover: false };
  }
  return null;
}

export function ampHelpPanelUrl(
  message: string,
  servers: ManagedServer[],
  hostname: string,
): string | null {
  const named = servers.find((server) => {
    if (server.manager !== "amp_import") return false;
    return (
      (server.instanceName.length > 0 && message.includes(server.instanceName))
      || (server.name.length > 0 && message.includes(server.name))
    );
  });
  if (named !== undefined) {
    const url = importedServerPanelUrl(named, hostname);
    if (url !== null) return url;
  }
  for (const server of servers) {
    const url = importedServerPanelUrl(server, hostname);
    if (url !== null) return url.replace(/\/instances\/[0-9a-f]+$/iu, "");
  }
  return null;
}

function ServerFault({
  message,
  csrfToken,
  servers,
  canManageNetwork,
  onSessionExpired,
}: {
  message: string | null;
  csrfToken: string;
  servers: ManagedServer[];
  canManageNetwork: boolean;
  onSessionExpired: () => void;
}) {
  if (message === null) return null;
  const claim = parseAmpPortClaim(message);
  if (claim === null) return <InlineError message={message} />;
  return (
    <AmpPortClaimHelp
      message={message}
      claim={claim}
      csrfToken={csrfToken}
      servers={servers}
      canManageNetwork={canManageNetwork}
      onSessionExpired={onSessionExpired}
    />
  );
}

function AmpPortClaimHelp({
  message,
  claim,
  csrfToken,
  servers,
  canManageNetwork,
  onSessionExpired,
}: {
  message: string;
  claim: { port: number; leftover: boolean };
  csrfToken: string;
  servers: ManagedServer[];
  canManageNetwork: boolean;
  onSessionExpired: () => void;
}) {
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [released, setReleased] = useState(false);
  const expected = leftoverAmpForwardConfirmation(claim.port);
  const hostname = globalThis.location?.hostname ?? "";
  const panelUrl = ampHelpPanelUrl(message, servers, hostname);
  const release = async (): Promise<void> => {
    setBusy(true);
    setLocalError(null);
    try {
      const result = await releaseAmpRouterForward(claim.port, confirmation.trim(), csrfToken);
      if (result.ampFilesChanged) {
        setLocalError("Helix stopped because AMP files would have changed.");
        return;
      }
      setReleased(true);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setLocalError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  if (released) {
    return (
      <div class="amp-port-help amp-port-help--ready" role="status">
        <Icon name="check" size={15} />
        <div>
          <strong>Leftover AMP forward on {claim.port} is gone.</strong>
          <p>AMP instance files were not changed. Retry create or save.</p>
        </div>
      </div>
    );
  }
  return (
    <div class="amp-port-help" role="alert">
      <Icon name="warning" size={15} />
      <div>
        <strong>
          {claim.leftover
            ? `Leftover AMP router mapping on ${claim.port}`
            : `AMP already has port ${claim.port} claimed`}
        </strong>
        <p>{message}</p>
        <div class="amp-port-help__actions">
          {panelUrl !== null && (
            <a class="button button--quiet" href={panelUrl} target="_blank" rel="noreferrer">
              Open AMP
            </a>
          )}
        </div>
        {claim.leftover && canManageNetwork && (
          <label class="field">
            <span>Type {expected} to delete only the leftover UPnP mapping</span>
            <input
              value={confirmation}
              disabled={busy}
              autoComplete="off"
              spellcheck={false}
              onInput={(event) => setConfirmation(event.currentTarget.value)}
            />
            <small>This does not stop AMP, rewrite instance files, or touch Helix servers.</small>
          </label>
        )}
        {claim.leftover && canManageNetwork && (
          <button
            class="button button--primary"
            type="button"
            disabled={busy || confirmation.trim() !== expected}
            onClick={() => void release()}
          >
            {busy ? "Removing…" : "Remove leftover AMP forward"}
          </button>
        )}
        {claim.leftover && !canManageNetwork && (
          <p>
            Removing that leftover UPnP mapping needs network.firewall.write. You can also delete
            that TCP forward on the router. Do not hand-edit AMP instance files.
          </p>
        )}
        <InlineError message={localError} />
      </div>
    </div>
  );
}

export function memoryBoundsForKind(
  kind: "minecraft" | "vrising" | "valheim" | "terraria",
): { min: number; max: number } {
  switch (kind) {
    case "vrising":
      return { min: 2_048, max: 24_576 };
    case "valheim":
      return { min: 1_024, max: 16_384 };
    case "terraria":
      return { min: 512, max: 8_192 };
    default:
      return { min: 1_024, max: 24_576 };
  }
}

export function allocatedMemoryOptions(
  kind: "minecraft" | "vrising" | "valheim" | "terraria",
  current: number,
): number[] {
  const { min, max } = memoryBoundsForKind(kind);
  const options = [512, 1_024, 2_048, 4_096, 6_144, 8_192, 12_288, 16_384, 24_576].filter(
    (value) => value >= min && value <= max,
  );
  if (Number.isFinite(current) && current >= min && current <= max && !options.includes(current)) {
    options.push(current);
    options.sort((a, b) => a - b);
  }
  return options;
}

export function publicInternetHint(
  kind: "minecraft" | "vrising" | "valheim" | "terraria",
  port: number,
  queryPort: number | null,
): string {
  if (kind === "vrising") {
    return `Helix does not open this on the internet. Forward UDP ${port}${queryPort === null ? "" : ` and ${queryPort}`} on your router if people should join from outside the LAN.`;
  }
  if (kind === "valheim") {
    return `Helix does not open this on the internet. Forward UDP ${port}–${port + 2} on your router if people should join from outside the LAN.`;
  }
  return `Helix does not open this on the internet. Forward TCP ${port} on your router if people should join from outside the LAN.`;
}

function formatMemoryGiB(memoryMb: number): string {
  return Number.isInteger(memoryMb / 1024) ? `${memoryMb / 1024} GiB` : `${memoryMb} MiB`;
}

export function serverActionDescription(
  server: ManagedServer,
  action: ServerAction,
): string {
  if (action === "kill") {
    return server.manager === "amp_import"
      ? "Helix cannot force-kill AMP instances; they remain under AMP. Use Stop, or kill from the AMP panel."
      : "Stop waits up to 45 seconds for a clean shutdown. Kill sends SIGKILL to the container now. Unsaved data can be lost. Use this when Stop is stuck.";
  }
  if (server.manager === "amp_import") {
    if (action === "start")
      return server.status === "idle"
        ? `Helix will ask AMP to wake ${server.name}. Idle means the game is sleeping; AMP's manager is already up.`
        : `Helix will ask AMP to start ${server.name} and wait for AMP to report the instance online.`;
    if (action === "restart")
      return `Helix will ask AMP to restart ${server.name} and wait for AMP to report the instance online.`;
    if (action === "stop")
      return `Helix will ask AMP to stop ${server.name}. Connected players will be disconnected.`;
  }
  if (action === "start")
    return server.kind === "minecraft"
      ? "Helix will start Minecraft and wait until it answers a health check."
      : "Helix will start this dedicated server and wait until its ready marker is present.";
  if (action === "stop")
    return "Players will be disconnected after a clean shutdown.";
  if (action === "restart")
    return server.kind === "minecraft"
      ? "The server will stop, start, and pass a Minecraft health check before this finishes."
      : "The server will stop, start, and wait for its ready marker before this finishes.";
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
): boolean {
  if (mutation === "create") return canManageServers;
  return canManageBackups;
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
  const [game, setGame] = useState<"minecraft" | "vrising" | "valheim" | "terraria">("minecraft");
  const [ranges, setRanges] = useState("");
  const [ports, setPorts] = useState("");
  const [autoForward, setAutoForward] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    setPolicy(null);
    setError(null);
    const load =
      game === "minecraft"
        ? getMinecraftPortPolicy
        : game === "vrising"
          ? getVRisingPortPolicy
          : game === "valheim"
            ? getValheimPortPolicy
            : getTerrariaPortPolicy;
    void load(csrfToken, controller.signal)
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
  }, [canManageNetwork, csrfToken, game, onSessionExpired]);

  const save = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const parsedRanges = ranges.trim().length === 0 ? [] : parsePortRanges(ranges);
      const parsedPorts = parseIndividualPorts(ports);
      if (parsedRanges.length === 0 && parsedPorts.length === 0) {
        throw new Error("Add at least one port or port range.");
      }
      const savePolicy =
        game === "minecraft"
          ? saveMinecraftPortPolicy
          : game === "vrising"
            ? saveVRisingPortPolicy
            : game === "valheim"
              ? saveValheimPortPolicy
              : saveTerrariaPortPolicy;
      const saved = await savePolicy(
        {
          ranges: parsedRanges,
          ports: parsedPorts,
          autoForwardOnCreate: game === "minecraft" ? autoForward : false,
        },
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
    <Dialog title="Port pools" onClose={onClose} wide>
      <div class="port-pool-game-tabs" role="tablist" aria-label="Game port pool">
        <button
          type="button"
          role="tab"
          aria-selected={game === "minecraft"}
          class={game === "minecraft" ? "is-active" : ""}
          disabled={busy}
          onClick={() => setGame("minecraft")}
        >
          Minecraft
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={game === "vrising"}
          class={game === "vrising" ? "is-active" : ""}
          disabled={busy}
          onClick={() => setGame("vrising")}
        >
          V Rising
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={game === "valheim"}
          class={game === "valheim" ? "is-active" : ""}
          disabled={busy}
          onClick={() => setGame("valheim")}
        >
          Valheim
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={game === "terraria"}
          class={game === "terraria" ? "is-active" : ""}
          disabled={busy}
          onClick={() => setGame("terraria")}
        >
          Terraria
        </button>
      </div>
      <div class="port-pool-summary">
        <div><strong>{policy?.capacity ?? "—"}</strong><span>configured</span></div>
        <div><strong>{policy?.availableCount ?? "—"}</strong><span>unassigned</span></div>
        <div><strong>{policy?.nextAvailablePort ?? "—"}</strong><span>next port</span></div>
      </div>
      <p class="dialog-intro">
        Automatic server creation takes individual ports first, then walks each range in order. It also skips ports already assigned to Helix, ports AMP already has claimed, and ports currently bound on the host.
      </p>
      {policy !== null && policy.ampClaimedPorts.length > 0 && (
        <p class="dialog-intro">
          AMP still has {policy.ampClaimedPorts.slice(0, 8).join(", ")}
          {policy.ampClaimedPorts.length > 8 ? ", …" : ""} in this pool. Automatic create skips those. Helix will not steal them; change the port in AMP or pick a different Helix number.
        </p>
      )}
      <div class="form-grid">
        <label class="field field--wide">
          <span>Port ranges</span>
          <input
            value={ranges}
            disabled={busy || policy === null}
            onInput={(event) => setRanges(event.currentTarget.value)}
            placeholder={
              game === "vrising"
                ? "9876-9910"
                : game === "valheim"
                  ? "2456-2490"
                  : game === "terraria"
                    ? "7777-7796"
                    : "25565-25599, 25610-25619"
            }
          />
          <small>Separate ranges with commas or spaces. A single port is accepted here too.</small>
        </label>
        <label class="field field--wide">
          <span>Priority ports</span>
          <input
            value={ports}
            disabled={busy || policy === null}
            onInput={(event) => setPorts(event.currentTarget.value)}
            placeholder={
              game === "vrising"
                ? "9876, 9878"
                : game === "valheim"
                  ? "2456, 2459"
                  : game === "terraria"
                    ? "7777, 7778"
                    : "25565, 25570, 25580"
            }
          />
          <small>Optional. These are tried before the ranges; duplicates are removed safely.</small>
        </label>
      </div>
      {game === "minecraft" ? (
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
      ) : (
        <p class="dialog-intro">
          {game === "terraria"
            ? "Terraria public setup is chosen per server, not from this pool. Automatic create still stays on the private LAN unless you flip that later."
            : `${game === "valheim" ? "Valheim" : "V Rising"} stays private in this Helix release. Helix does not offer UPnP for its UDP game ports.`}
        </p>
      )}
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
  const [versions, setVersions] = useState<MinecraftVersionCatalog | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [versionsRetry, setVersionsRetry] = useState(0);
  const [jarDropActive, setJarDropActive] = useState(false);
  const [jarUploading, setJarUploading] = useState(false);
  const [jarUploadPercent, setJarUploadPercent] = useState(0);
  const jarInput = useRef<HTMLInputElement | null>(null);

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
    if (mode === "modpack") {
      setVersionsLoading(false);
      return;
    }
    const softwareId = mode === "custom" ? "custom" : software;
    const controller = new AbortController();
    setVersions(null);
    setVersionsLoading(true);
    setVersionsError(null);
    void getMinecraftVersions(softwareId, csrfToken, controller.signal)
      .then((catalog) => {
        if (controller.signal.aborted) return;
        setVersions(catalog);
        setVersion((current) => {
          if (mode === "custom") {
            if (
              current.trim().toLowerCase() === "latest" ||
              current.trim().length === 0
            ) {
              return catalog.latestVersion ?? catalog.versions[0] ?? "";
            }
            return current;
          }
          if (current.trim().toLowerCase() === "latest") return "latest";
          if (catalog.versions.includes(current.trim())) return current;
          return "latest";
        });
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setVersionsError(describeError(requestError));
      })
      .finally(() => {
        if (!controller.signal.aborted) setVersionsLoading(false);
      });
    return () => controller.abort();
  }, [csrfToken, mode, onSessionExpired, software, versionsRetry]);

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

  const loaderReady =
    readiness?.availability === "ready" &&
    ["fabric", "forge", "neoforge", "quilt"].some((id) =>
      readiness.supportedMinecraftSoftware.includes(id as InstallableMinecraftSoftware),
    );
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
    version.trim().toLowerCase() !== "latest";
  const canReview =
    name.trim().length >= 2 &&
    (mode === "modpack"
      ? loaderReady && modpack !== null
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
    if (next === "custom") {
      setVersion((current) =>
        current.trim().toLowerCase() === "latest" ? "" : current,
      );
    } else if (next === "software") {
      setVersion((current) =>
        current.trim().length === 0 ? "latest" : current,
      );
    }
  };

  const uploadCustomJar = async (file: File): Promise<void> => {
    setJarUploading(true);
    setJarUploadPercent(0);
    setError(null);
    try {
      const uploaded = await uploadHostFile({
        file,
        purpose: "custom_jar",
        csrfToken,
        onProgress: setJarUploadPercent,
      });
      setCustomJarPath(uploaded.path);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setJarUploading(false);
      setJarUploadPercent(0);
      setJarDropActive(false);
      if (jarInput.current !== null) jarInput.current.value = "";
    }
  };

  const submit = async (): Promise<void> => {
    if (!canReview || !eula) return;
    setSubmitting(true);
    setError(null);
    try {
      if (mode === "modpack") {
        if (modpack === null || !loaderReady)
          throw new Error("Choose an installable modpack release first.");
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
            provider: modpack.provider,
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
                : parseAmpPortClaim(job.error ?? "") !== null
                  ? "That port is still held by AMP."
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
                  ? parseAmpPortClaim(publicSetupError ?? "") !== null
                    ? "Public access stopped on an AMP-held port. Use the steps below; Helix did not overwrite AMP."
                    : (publicSetupError ?? "Open the server’s Join section to retry automatic public setup.")
                  : "Router mapping confirmed. Test this address from a separate external network before sharing it broadly."}
              </span>
            </div>
          )}
        </div>
        <ServerFault
          message={polling.error ?? error ?? (job.status === "failed" ? job.error : null) ?? publicSetupError}
          csrfToken={csrfToken}
          servers={servers}
          canManageNetwork={canManageNetwork}
          onSessionExpired={onSessionExpired}
        />
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
              <small>Paper, Purpur, Leaves, Folia, Fabric, or Vanilla</small>
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
              <small>Drop a JAR here or choose one already in Storage</small>
            </span>
          </button>
        </div>
      )}
      <ServerFault
        message={error}
        csrfToken={csrfToken}
        servers={servers}
        canManageNetwork={canManageNetwork}
        onSessionExpired={onSessionExpired}
      />
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
                  onSelect={(id) => {
                    if (id === "custom") {
                      selectMode("custom");
                      setCatalogOpen(false);
                      return;
                    }
                    setSoftware(id);
                    if (mode !== "software") setMode("software");
                    setCatalogOpen(false);
                  }}
                  onCatalogToggle={() => setCatalogOpen((open) => !open)}
                />
                <MinecraftVersionField
                  key={`software-${software}`}
                  version={version}
                  catalog={versions}
                  loading={versionsLoading}
                  error={versionsError}
                  allowLatest
                  onChange={setVersion}
                  onRetry={() => setVersionsRetry((value) => value + 1)}
                />
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
                        : "Custom JAR import is unavailable because the native manager is not ready."}
                  </span>
                </div>
                <div
                  class={`field field--wide custom-jar-import${jarDropActive ? " is-drop-target" : ""}`}
                  onDragEnter={(event) => {
                    event.preventDefault();
                    if (customReady && !jarUploading) setJarDropActive(true);
                  }}
                  onDragOver={(event) => {
                    event.preventDefault();
                    if (event.dataTransfer !== null) {
                      event.dataTransfer.dropEffect =
                        customReady && !jarUploading ? "copy" : "none";
                    }
                  }}
                  onDragLeave={(event) => {
                    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                    setJarDropActive(false);
                  }}
                  onDrop={(event) => {
                    event.preventDefault();
                    setJarDropActive(false);
                    if (!customReady || jarUploading) return;
                    try {
                      const files = droppedTransferFiles(event.dataTransfer);
                      const jar = files.find((file) => /\.jar$/iu.test(file.name));
                      if (jar === undefined) {
                        throw new Error("Drop a .jar file.");
                      }
                      void uploadCustomJar(jar);
                    } catch (dropError) {
                      setError(describeError(dropError));
                    }
                  }}
                >
                  <div class="field-heading">
                    <label for="custom-server-jar">Server JAR</label>
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
                  <label
                    class={`custom-jar-drop${jarDropActive ? " is-active" : ""}${jarUploading ? " is-busy" : ""}`}
                  >
                    <Icon name="file" size={22} />
                    <strong>
                      {jarUploading
                        ? `Uploading… ${jarUploadPercent}%`
                        : customJarPath.length > 0
                          ? customJarPath.split("/").at(-1)
                          : "Drop a server JAR here"}
                    </strong>
                    <span>
                      {customJarPath.length > 0
                        ? customJarPath
                        : `Up to ${Math.round(MAX_CUSTOM_JAR_UPLOAD_BYTES / (1024 * 1024))} MiB · or browse Storage`}
                    </span>
                    {jarUploading && <ProgressBar value={jarUploadPercent} />}
                    <input
                      ref={jarInput}
                      type="file"
                      accept=".jar,application/java-archive"
                      disabled={!customReady || jarUploading}
                      onChange={(event) => {
                        const file = event.currentTarget.files?.[0];
                        if (file !== undefined) void uploadCustomJar(file);
                      }}
                    />
                  </label>
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
                    Drop a <code>.jar</code> from this computer or use an
                    absolute path inside Storage. Helix copies the file into a
                    private workspace, rejects folders/symlinks, and caps
                    imports at 768 MiB.
                  </small>
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
                </div>
                <MinecraftVersionField
                  key="custom-jar"
                  version={version}
                  catalog={versions}
                  loading={versionsLoading}
                  error={versionsError}
                  allowLatest={false}
                  onChange={setVersion}
                  onRetry={() => setVersionsRetry((value) => value + 1)}
                />
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
                  class={`software-readiness-note field--wide ${loaderReady ? "" : "is-error"}`}
                  role="status"
                >
                  <span>
                    {readiness === null
                      ? "Checking Fabric lifecycle readiness…"
                      : loaderReady
                        ? "Modpack creation is ready on this host."
                        : "No installable mod loader is ready on this host, so modpack creation is disabled."}
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
                  <small>Helix will reject a port already assigned to Helix, claimed by AMP, or bound on the host.</small>
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
                ? "Only opaque catalog project and version IDs leave the browser. The broker re-resolves Modrinth or CurseForge metadata, pins a supported loader, and refuses client-only files, unsafe archive paths, and unverified downloads."
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

function actionLabel(action: ServerAction): string {
  return `${action.charAt(0).toUpperCase()}${action.slice(1)}`;
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
  const [effectiveAction, setEffectiveAction] = useState<ServerAction>(action);
  const [killConfirm, setKillConfirm] = useState(false);
  const label = actionLabel(effectiveAction);
  const destructive = effectiveAction === "stop" || effectiveAction === "kill";
  const polling = useJobPolling({
    job,
    csrfToken,
    baseDelayMs: 900,
    onJob: setJob,
    onComplete,
    onSessionExpired,
  });
  const queueAction = async (next: ServerAction): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const dispatch = await runServerAction(server.id, next, csrfToken);
      setEffectiveAction(next);
      setKillConfirm(false);
      if (dispatch.jobId === null) {
        await onComplete();
        onClose();
      } else {
        setJob({
          id: dispatch.jobId,
          kind: `server_${next}`,
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
    const canEscapeWithKill =
      server.manager === "helix" &&
      active &&
      (effectiveAction === "stop" || effectiveAction === "restart");
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
                    : effectiveAction === "backup"
                      ? "backup"
                      : effectiveAction === "kill"
                        ? "kill"
                        : "activity"
              }
              size={26}
            />
          </div>
          <strong>{job.stage}</strong>
          <span>
            {active
              ? effectiveAction === "kill"
                ? "SIGKILL is in flight. Closing after a status-check problem will not interrupt it."
                : "This runs in the background. Closing after a status-check problem will not interrupt it."
              : job.status === "complete"
                ? `${server.name} is ready.`
                : (job.error ?? "Helix could not finish the action.")}
          </span>
          <ProgressBar
            value={
              active ? Math.max(job.progressPercent, 12) : job.progressPercent
            }
            tone={
              job.status === "failed" || effectiveAction === "kill"
                ? "danger"
                : "normal"
            }
          />
          <small>
            {active
              ? effectiveAction === "kill"
                ? "Sending SIGKILL…"
                : "Working safely…"
              : `${job.progressPercent}%`}
          </small>
        </div>
        {canEscapeWithKill && killConfirm && (
          <div class="dialog-copy">
            <p>{serverActionDescription(server, "kill")}</p>
          </div>
        )}
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
          {canEscapeWithKill &&
            (killConfirm ? (
              <>
                <button
                  class="button button--quiet"
                  type="button"
                  disabled={busy}
                  onClick={() => setKillConfirm(false)}
                >
                  Keep waiting
                </button>
                <button
                  class="button button--danger"
                  type="button"
                  disabled={busy}
                  onClick={() => void queueAction("kill")}
                >
                  {busy ? "Queuing…" : "Kill now"}
                </button>
              </>
            ) : (
              <button
                class="button button--danger-quiet"
                type="button"
                onClick={() => setKillConfirm(true)}
              >
                Kill instead
              </button>
            ))}
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
          class={`button ${destructive ? "button--danger" : "button--primary"}`}
          type="button"
          disabled={busy}
          onClick={() => void queueAction(action)}
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
  const live = serverIsLive(server.status);
  const showStats = serverShowsRuntimeStats(server.status);
  const running = serverWorkloadIsRunning(server);
  const primaryAction = serverPrimaryLifecycleAction(server);
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
          <small>{serverStatusLabel(server.status)} · {server.instanceName}</small>
        </span>
      </button>
      <div class="server-stat">
        <span>Players</span>
        <strong>
          {serverPlayerHeadline(server)}
        </strong>
      </div>
      <div class="server-stat">
        <span>CPU</span>
        <strong>{live ? formatPercent(server.cpuPercent) : "—"}</strong>
      </div>
      <div class="server-stat server-stat--memory">
        <span>Memory</span>
        <strong>
          {showStats
            ? `${formatBytes(server.memoryUsedMb * 1024 * 1024)} / ${formatBytes(server.memoryLimitMb * 1024 * 1024)}`
            : `${server.memoryLimitMb / 1024} GiB limit`}
        </strong>
        {live && (
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
        <strong>{!serverReportsTps(server) || !live || server.tps === null ? "—" : server.tps.toFixed(1)}</strong>
      </div>
      <div class="server-actions">
        {primaryAction === "restart" ? (
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
        ) : primaryAction === "start" ? (
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
        ) : null}
        <button
          class="button button--quiet"
          type="button"
          onClick={onOpen}
        >
          <Icon name="chevron" size={15} />
          Open
        </button>
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
            {(running || (server.manager === "amp_import" && server.panelRunning)) && (
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
            {running && server.manager === "helix" && (
              <button
                class="danger-link"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => confirmAction("kill")}
              >
                <Icon name="kill" size={15} />
                Kill
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
  return /^(?:paper|purpur|folia|leaves|fabric|forge|neoforge|quilt|pufferfish)$/iu.test(software.trim());
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
            {detail.kind !== "minecraft"
              ? "This dedicated server has no RCON command channel. This view follows the container log."
              : "Commands go over RCON on 127.0.0.1 only — that is loopback, the host talking to itself. Players never use this port. Output stays available across dashboard sessions."}
          </p>
        </div>
        {detail.kind === "minecraft" && (
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
        )}
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
      {detail.kind !== "minecraft" ? (
        <p class="console-retention-note">
          <Icon name="info" size={14} />
          <span>
            <strong>No command console for this game.</strong>
            <small>
              Use Files for host settings and restart the server after edits.
            </small>
          </span>
        </p>
      ) : (
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
      )}
    </section>
  );
}

function SettingsPanel({
  detail,
  csrfToken,
  servers,
  canManageServers,
  canManageNetwork,
  restartSuccessRevision,
  onRestart,
  onSaved,
  onSessionExpired,
}: {
  detail: NativeServerDetail & { settings: MinecraftSettings };
  csrfToken: string;
  servers: ManagedServer[];
  canManageServers: boolean;
  canManageNetwork: boolean;
  restartSuccessRevision: number;
  onRestart: () => void;
  onSaved: () => Promise<void> | void;
  onSessionExpired: () => void;
}) {
  const [settings, setSettings] = useState<MinecraftSettings>(detail.settings);
  const [saved, setSaved] = useState(detail.settings);
  const [busy, setBusy] = useState(false);
  const [restartPending, setRestartPending] = useState(false);
  const [showRestartChoice, setShowRestartChoice] = useState(false);
  const [changedFields, setChangedFields] = useState<string[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
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
    setNotice(null);
    try {
      const result = await saveServerSettings(detail.id, settings, csrfToken);
      setSettings(result.settings);
      setSaved(result.settings);
      if (result.changed) {
        setChangedFields(result.changedFields);
        setRestartPending(result.restartRequired);
        setShowRestartChoice(result.restartRequired);
        const notes = [result.exposureNote, result.exposureWarning].filter(
          (value): value is string => value !== null && value.length > 0,
        );
        if (result.containerRepublished) {
          if (result.changedFields.includes("memory_mb")) {
            notes.unshift(
              `Allocated memory is now ${formatMemoryGiB(result.settings.memoryMb)}. The container was rebound with that limit.`,
            );
          }
          if (result.changedFields.includes("game_port")) {
            notes.unshift(
              `Published port is now ${result.settings.gamePort}. The container was rebound to that port.`,
            );
          }
        }
        setNotice(notes.length > 0 ? notes.join(" ") : null);
        await onSaved();
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
      <ServerFault
        message={error}
        csrfToken={csrfToken}
        servers={servers}
        canManageNetwork={canManageNetwork}
        onSessionExpired={onSessionExpired}
      />
      {notice !== null && (
        <p class="settings-port-note" role="status">
          {notice}
        </p>
      )}
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
          <span>
            Game port {restartLabel("game_port")}{" "}
            <InfoTip text="Helix rebinds the published container when this changes. Public access on the old port is removed. AMP-claimed ports stay with AMP." />
          </span>
          <input
            type="number"
            min={1024}
            max={65_535}
            value={settings.gamePort}
            disabled={!canManageServers}
            title={manageTitle}
            onInput={(event) => {
              const value = event.currentTarget.valueAsNumber;
              if (Number.isFinite(value)) update("gamePort", value);
            }}
          />
        </label>
        <label class="field">
          <span>
            Memory {restartLabel("memory_mb")}{" "}
            <InfoTip text="Helix rebinds the published container when this changes so Java and Docker both get the new limit." />
          </span>
          <select
            value={settings.memoryMb}
            disabled={!canManageServers}
            title={manageTitle}
            onChange={(event) =>
              update("memoryMb", Number(event.currentTarget.value))
            }
          >
            {allocatedMemoryOptions("minecraft", settings.memoryMb).map((value) => (
              <option key={value} value={value}>
                {formatMemoryGiB(value)}
              </option>
            ))}
          </select>
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
            settings.motd.trim().length === 0 ||
            !Number.isFinite(settings.gamePort) ||
            settings.gamePort < 1024 ||
            !Number.isFinite(settings.memoryMb)
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

function AllocatedMemoryEditor({
  detail,
  csrfToken,
  canManageServers,
  onSaved,
  onSessionExpired,
}: {
  detail: NativeServerDetail;
  csrfToken: string;
  canManageServers: boolean;
  onSaved: () => Promise<void> | void;
  onSessionExpired: () => void;
}) {
  const [memoryMb, setMemoryMb] = useState(detail.memoryLimitMb);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    setMemoryMb(detail.memoryLimitMb);
  }, [detail.memoryLimitMb]);
  const dirty = memoryMb !== detail.memoryLimitMb;
  const manageTitle = canManageServers
    ? undefined
    : "Requires games.manage permission";
  const save = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await setNativeMemory(detail.id, memoryMb, csrfToken);
      setMemoryMb(result.memoryMb);
      if (result.changed) {
        setNotice(
          `Allocated memory is now ${formatMemoryGiB(result.memoryMb)}. The container was rebound with that limit.`,
        );
        await onSaved();
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="allocated-memory-editor">
      <label class="field">
        <span>
          Allocated memory{" "}
          <InfoTip text="Helix rebinds the published container when this changes so the new limit is actually used." />
        </span>
        <select
          value={memoryMb}
          disabled={!canManageServers || busy}
          title={manageTitle}
          onChange={(event) => setMemoryMb(Number(event.currentTarget.value))}
        >
          {allocatedMemoryOptions(detail.kind, memoryMb).map((value) => (
            <option key={value} value={value}>
              {formatMemoryGiB(value)}
            </option>
          ))}
        </select>
      </label>
      <button
        class="button button--primary"
        type="button"
        disabled={!canManageServers || !dirty || busy}
        title={manageTitle}
        onClick={() => void save()}
      >
        {busy ? "Saving…" : "Save memory"}
      </button>
      <InlineError message={error} />
      {notice !== null && (
        <p class="settings-port-note" role="status">
          {notice}
        </p>
      )}
    </div>
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
  onDeleted: (trashId: string, backup: ServerBackup, purged: boolean) => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const [forever, setForever] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const result = await trashServerBackup(server.id, backup.id, csrfToken);
      if (forever) {
        await purgeTrashedServerBackup(server.id, result.trashId, csrfToken);
      }
      await onDeleted(result.trashId, backup, forever);
      onClose();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog title={forever ? "Delete this backup forever?" : "Move backup to trash?"} onClose={() => !busy && onClose()}>
      <div class="dialog-copy">
        <p>
          <strong>{formatTimestamp(backup.createdAtUnixMs)}</strong> ·{" "}
          {formatBytes(backup.sizeBytes)}
        </p>
        <p>
          {forever
            ? "Helix deletes this copy now. There is no Undo after this."
            : "Helix moves this backup to protected trash, where Undo can restore it. You can still delete it forever from Deleted backups."}
        </p>
      </div>
      <label class="check-row">
        <input
          type="checkbox"
          checked={forever}
          disabled={busy}
          onChange={(event) => setForever(event.currentTarget.checked)}
        />
        <span>
          <strong>Delete forever</strong>
          <small>Skip trash. This cannot be undone.</small>
        </span>
      </label>
      <label class="check-row">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>
          <strong>{forever ? "I want this backup gone" : "Remove this active backup"}</strong>
          <small>{forever ? "The archive is destroyed on this host." : "A recoverable copy stays under Deleted backups."}</small>
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
          {busy ? (forever ? "Deleting…" : "Moving…") : forever ? "Delete forever" : "Move to trash"}
        </button>
      </div>
    </Dialog>
  );
}

function PurgeBackupDialog({
  server,
  item,
  csrfToken,
  onClose,
  onPurged,
  onSessionExpired,
}: {
  server: ManagedServer;
  item: ServerBackupTrash;
  csrfToken: string;
  onClose: () => void;
  onPurged: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const purge = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await purgeTrashedServerBackup(server.id, item.trashId, csrfToken);
      await onPurged();
      onClose();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog title="Delete this backup forever?" onClose={() => !busy && onClose()}>
      <div class="dialog-copy">
        <p>
          <strong>{formatTimestamp(item.trashedAtUnixMs)}</strong> ·{" "}
          {formatBytes(item.sizeBytes)}
        </p>
        <p>This removes the trashed archive from disk. Undo will no longer work for this copy.</p>
      </div>
      <label class="check-row">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.currentTarget.checked)}
        />
        <span>
          <strong>Delete forever</strong>
          <small>I understand this cannot be undone.</small>
        </span>
      </label>
      <InlineError message={error} />
      <div class="dialog-actions">
        <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button class="button button--danger" type="button" disabled={!confirmed || busy} onClick={() => void purge()}>
          {busy ? "Deleting…" : "Delete forever"}
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
  onCreate,
  onRefresh,
  onSessionExpired,
}: {
  server: ManagedServer;
  csrfToken: string;
  refreshKey: number;
  canManageServers: boolean;
  canManageBackups: boolean;
  onCreate: () => void;
  onRefresh: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [backups, setBackups] = useState<ServerBackup[]>([]);
  const [trash, setTrash] = useState<ServerBackupTrash[]>([]);
  const [trashPolicy, setTrashPolicy] =
    useState<ServerBackupTrashPolicy | null>(null);
  const [keepPolicy, setKeepPolicy] = useState<ServerBackupKeepPolicy | null>(
    null,
  );
  const [keepCountDraft, setKeepCountDraft] = useState("0");
  const [keepDaysDraft, setKeepDaysDraft] = useState("0");
  const [policyBusy, setPolicyBusy] = useState(false);
  const [restore, setRestore] = useState<ServerBackup | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ServerBackup | null>(null);
  const [purgeTarget, setPurgeTarget] = useState<ServerBackupTrash | null>(null);
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
  );
  const canRestore = canRunBackupMutation(
    "restore",
    canManageServers,
    canManageBackups,
  );
  const canUseTrash = canRunBackupMutation(
    "trash",
    canManageServers,
    canManageBackups,
  );
  const load = useCallback(async (): Promise<void> => {
    try {
      const catalog = await getServerBackups(server.id, csrfToken);
      setBackups(catalog.backups);
      setTrash(catalog.trash);
      setTrashPolicy(catalog.trashPolicy);
      setKeepPolicy(catalog.policy);
      setKeepCountDraft(String(catalog.policy.keepCount));
      setKeepDaysDraft(String(catalog.policy.keepDays));
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
            <InfoTip text="Helix stops a running server, archives its data folder, then starts it again. Deleted copies stay in protected trash until you restore them or delete them forever. Count and age limits move the oldest extras to trash after a backup." />
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
      {canManageBackups && keepPolicy !== null && (
        <form
          class="backup-policy"
          onSubmit={(event) => {
            event.preventDefault();
            if (policyBusy) return;
            const keepCount = Number(keepCountDraft);
            const keepDays = Number(keepDaysDraft);
            if (
              !Number.isInteger(keepCount) ||
              keepCount < 0 ||
              keepCount > 50 ||
              !Number.isInteger(keepDays) ||
              keepDays < 0 ||
              keepDays > 365
            ) {
              setError("Keep count is 0–50. Keep days is 0–365. Zero means no limit.");
              return;
            }
            setPolicyBusy(true);
            setError(null);
            void setServerBackupPolicy(server.id, keepCount, keepDays, csrfToken)
              .then((catalog) => {
                setBackups(catalog.backups);
                setTrash(catalog.trash);
                setTrashPolicy(catalog.trashPolicy);
                setKeepPolicy(catalog.policy);
                setKeepCountDraft(String(catalog.policy.keepCount));
                setKeepDaysDraft(String(catalog.policy.keepDays));
              })
              .catch((requestError: unknown) => {
                if (isSessionError(requestError)) onSessionExpired();
                else setError(describeError(requestError));
              })
              .finally(() => setPolicyBusy(false));
          }}
        >
          <div>
            <label>
              Keep this many
              <InfoTip text="0 means keep every backup. Any extra oldest copies move to trash after the next backup or when you save this rule." />
              <input
                type="number"
                min="0"
                max="50"
                value={keepCountDraft}
                disabled={policyBusy}
                onInput={(event) => setKeepCountDraft(event.currentTarget.value)}
              />
            </label>
            <label>
              Keep this many days
              <InfoTip text="0 means no age limit. Backups older than this move to trash after the next backup or when you save this rule." />
              <input
                type="number"
                min="0"
                max="365"
                value={keepDaysDraft}
                disabled={policyBusy}
                onInput={(event) => setKeepDaysDraft(event.currentTarget.value)}
              />
            </label>
          </div>
          <p>{keepPolicy.note}</p>
          <div class="backup-policy__actions">
            <button class="button button--quiet" type="submit" disabled={policyBusy}>
              {policyBusy ? "Saving…" : "Save keep rules"}
            </button>
            <button
              class="button button--quiet"
              type="button"
              disabled={policyBusy || (keepPolicy.keepCount === 0 && keepPolicy.keepDays === 0)}
              onClick={() => {
                setPolicyBusy(true);
                setError(null);
                void pruneServerBackups(server.id, csrfToken)
                  .then((catalog) => {
                    setBackups(catalog.backups);
                    setTrash(catalog.trash);
                    setTrashPolicy(catalog.trashPolicy);
                    setKeepPolicy(catalog.policy);
                  })
                  .catch((requestError: unknown) => {
                    if (isSessionError(requestError)) onSessionExpired();
                    else setError(describeError(requestError));
                  })
                  .finally(() => setPolicyBusy(false));
              }}
            >
              Apply rules now
            </button>
          </div>
        </form>
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
                    : "Move to recoverable trash"
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
                <button
                  class="icon-button icon-button--danger"
                  type="button"
                  disabled={!canUseTrash || undoing !== null}
                  aria-label="Delete this backup forever"
                  title="Delete forever. This cannot be undone."
                  onClick={() => setPurgeTarget(item)}
                >
                  <Icon name="trash" size={15} />
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
          onDeleted={async (trashIdValue, backup, purged) => {
            if (!purged) setImmediateUndo({ trashId: trashIdValue, backup });
            else if (immediateUndo?.trashId === trashIdValue) setImmediateUndo(null);
            await load();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
      {purgeTarget !== null && (
        <PurgeBackupDialog
          server={server}
          item={purgeTarget}
          csrfToken={csrfToken}
          onClose={() => setPurgeTarget(null)}
          onPurged={async () => {
            if (immediateUndo?.trashId === purgeTarget.trashId) setImmediateUndo(null);
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
          <h2>
            Live performance{" "}
            <InfoTip text="CPU and memory are from this container right now. TPS is ticks per second from a local /tps sample over RCON. 20 is a healthy Minecraft tick rate. An em dash means that software does not answer /tps." />
          </h2>
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
      <div class={`performance-cards${detail.kind === "minecraft" ? " performance-cards--with-tps" : ""}`}>
        {detail.kind === "minecraft" && (
          <Metric
            icon="activity"
            label="TPS"
            value={detail.tps === null ? "—" : detail.tps.toFixed(1)}
            detail={
              detail.tps === null
                ? "Shown when the server answers /tps"
                : "1-minute sample from the local console"
            }
          />
        )}
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
  servers,
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
  servers: ManagedServer[];
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
  const [refreshKey, setRefreshKey] = useState(0);
  const [restartSuccessRevision, setRestartSuccessRevision] = useState(0);
  const [bootBusy, setBootBusy] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
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
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setNetwork(null);
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
  const containerUp = detail.status === "online" || detail.status === "starting";
  const isReadyMarkerGame = detail.kind !== "minecraft";
  const usesUdpJoin = detail.kind === "vrising" || detail.kind === "valheim";
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
    (usesUdpJoin ? udpEvidence?.privateJoinAddress : tcpEvidence?.privateJoinAddress) ??
    (network?.addresses.privateIpv4 === null || network?.addresses.privateIpv4 === undefined
      ? "Private address unavailable"
      : formatJoinAddress(network.addresses.privateIpv4, detail.gamePort));
  const publicIp =
    network?.router.externalIpv4
    ?? tcpEvidence?.externalReachability.externalIp
    ?? udpEvidence?.externalReachability.externalIp
    ?? null;
  const publicJoinAddress =
    publicIp === null ? null : formatJoinAddress(publicIp, detail.gamePort);
  const publicInternetNote = publicInternetHint(
    detail.kind,
    detail.gamePort,
    detail.queryPort,
  );
  const updateStartOnBoot = async (enabled: boolean): Promise<void> => {
    setBootBusy(true);
    setBootError(null);
    try {
      await setNativeStartOnBoot(detail.id, enabled, csrfToken);
      await refresh();
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setBootError(describeError(requestError));
    } finally {
      setBootBusy(false);
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
              {isReadyMarkerGame
                ? `${joinAddress} · isolated runtime · UDP ${detail.gamePort}${detail.queryPort === null ? "" : ` / ${detail.queryPort}`}`
                : `${joinAddress} · ${detail.minecraftVersion} · Java ${detail.javaVersion}`}
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
          {containerUp ? (
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
              <button
                class="button button--danger-quiet"
                type="button"
                disabled={!canManageServers}
                title={manageTitle}
                onClick={() => setPending("kill")}
              >
                <Icon name="kill" size={15} />
                Kill
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
          .filter((item) => {
            if (item.id === "overview") return true;
            if (item.id === "marketplace") return supportsMarketplaceSoftware(detail.software);
            if (item.id === "settings") return detail.settings !== null && detail.capabilities.includes("settings");
            return detail.capabilities.includes(item.id);
          })
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
                    {publicJoinAddress ?? "Not detected"}
                  </strong>
                  <small>{publicInternetNote}</small>
                  {publicJoinAddress !== null && (
                    <button
                      type="button"
                      onClick={() =>
                        void navigator.clipboard?.writeText(publicJoinAddress)
                      }
                    >
                      Copy
                    </button>
                  )}
                </article>
              </div>
              <div class="join-evidence">
                {!isReadyMarkerGame && (
                <span
                  class={`state-label state-label--${tcpDiagnostic.tone}`}
                >
                  TCP · {tcpDiagnostic.label}
                </span>
                )}
                <span
                  class={`state-label state-label--${udpDiagnostic.tone}`}
                >
                  UDP · {udpDiagnostic.label}
                </span>
                <small>
                  {usesUdpJoin
                    ? udpDiagnostic.detail
                    : `${tcpDiagnostic.detail} ${udpDiagnostic.detail}`}
                </small>
              </div>
            </section>
            <div class="server-overview-grid">
              <section class="surface server-health">
                <div class="section-title">
                  <div>
                    <h2>Right now</h2>
                    <p>{isReadyMarkerGame ? "Live dedicated-server runtime" : "Live Minecraft and runtime state"}</p>
                  </div>
                  <span
                    class={`state-label state-label--${online ? "good" : "idle"}`}
                  >
                    {detail.status}
                  </span>
                </div>
                <div class={`server-health-stats${isReadyMarkerGame ? "" : " server-health-stats--with-tps"}`}>
                  <div>
                    <span>Players</span>
                    <strong>
                      {isReadyMarkerGame
                        ? `— / ${detail.maxPlayers}`
                        : `${detail.playersOnline} / ${detail.maxPlayers}`}
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
                  {!isReadyMarkerGame && (
                    <div>
                      <span>TPS</span>
                      <strong>
                        {detail.tps === null ? "—" : detail.tps.toFixed(1)}
                      </strong>
                    </div>
                  )}
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
                    <p>{isReadyMarkerGame ? "Isolated Helix runtime" : "Resolved and pinned by Helix"}</p>
                  </div>
                </div>
                <dl>
                  <div>
                    <dt>Software</dt>
                    <dd>{detail.software}</dd>
                  </div>
                  {isReadyMarkerGame ? (
                    <>
                      <div>
                        <dt>Steam app</dt>
                        <dd>{detail.build}</dd>
                      </div>
                      <div>
                        <dt>Runtime</dt>
                        <dd>Isolated dedicated-server container</dd>
                      </div>
                      <div>
                        <dt>Query UDP</dt>
                        <dd>{detail.queryPort ?? "—"}</dd>
                      </div>
                    </>
                  ) : (
                    <>
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
                    </>
                  )}
                  <div>
                    <dt>Created</dt>
                    <dd>{formatTimestamp(detail.createdAtUnixMs)}</dd>
                  </div>
                </dl>
                <label class="check-row startup-toggle">
                  <input
                    class="toggle-input"
                    type="checkbox"
                    checked={detail.startOnBoot}
                    disabled={bootBusy || !canManageServers}
                    title={manageTitle}
                    onChange={(event) => void updateStartOnBoot(event.currentTarget.checked)}
                  />
                  <span>
                    <strong>Start with the host</strong>
                    <small>
                      Docker restart policy unless-stopped. Survives host reboot without starting or stopping the server now.
                    </small>
                  </span>
                </label>
                <InlineError message={bootError} />
                <AllocatedMemoryEditor
                  detail={detail}
                  csrfToken={csrfToken}
                  canManageServers={canManageServers}
                  onSaved={refresh}
                  onSessionExpired={onSessionExpired}
                />
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
        {tab === "settings" && detail.settings !== null && (
          <SettingsPanel
            detail={{ ...detail, settings: detail.settings }}
            csrfToken={csrfToken}
            servers={servers}
            canManageServers={canManageServers}
            canManageNetwork={canManageNetwork}
            restartSuccessRevision={restartSuccessRevision}
            onRestart={() => setPending("restart")}
            onSaved={refresh}
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
            {isReadyMarkerGame && (
              <AllocatedMemoryEditor
                detail={detail}
                csrfToken={csrfToken}
                canManageServers={canManageServers}
                onSaved={refresh}
                onSessionExpired={onSessionExpired}
              />
            )}
            <dl class="advanced-facts">
              <div>
                <dt>
                  Instance{" "}
                  <InfoTip text="Helix’s internal name for this container. It is not the world name players see." />
                </dt>
                <dd>
                  <code>{detail.instanceName}</code>
                </dd>
              </div>
              <div>
                <dt>
                  Backend{" "}
                  <InfoTip text="The game runs in a Docker container as a dedicated numeric Linux user, not as root and not as your login account." />
                </dt>
                <dd>Docker · isolated numeric user</dd>
              </div>
              <div>
                <dt>
                  Runtime image{" "}
                  <InfoTip text="The exact container image Helix starts. A digest pin means Helix will not silently float to a different build." />
                </dt>
                <dd>
                  <code>{detail.runtimeImage}</code>
                </dd>
              </div>
              <div>
                <dt>
                  Server SHA-256{" "}
                  <InfoTip text="A fingerprint of the server JAR Helix installed. If this changes, the file on disk is not the same bytes Helix verified." />
                </dt>
                <dd>
                  <code>{detail.artifactSha256}</code>
                </dd>
              </div>
              <div>
                <dt>
                  Data path{" "}
                  <InfoTip text="The host folder mounted into the container. Worlds, mods, plugins, and configs live here." />
                </dt>
                <dd>
                  <code>{detail.dataPath}</code>
                </dd>
              </div>
              <div>
                <dt>
                  Game port{" "}
                  <InfoTip text="The port players connect to. Helix publishes TCP and UDP for Minecraft. This is not the RCON console port." />
                </dt>
                <dd>
                  <code>{detail.gamePort}/tcp + udp</code>
                </dd>
              </div>
              <div>
                <dt>
                  Allocated memory{" "}
                  <InfoTip text="Docker and Java are both capped at this amount. Raising it republishes the container." />
                </dt>
                <dd>{formatMemoryGiB(detail.memoryLimitMb)}</dd>
              </div>
              <div>
                <dt>
                  Console{" "}
                  <InfoTip text="Loopback only means RCON listens on 127.0.0.1 on this host. The dashboard talks to Minecraft locally. That port is not opened to the LAN or internet, and it is not the game port players join." />
                </dt>
                <dd>Loopback only</dd>
              </div>
              <div>
                <dt>
                  OOM killed{" "}
                  <InfoTip text="The Linux kernel killed the process because it ran out of memory. Raise allocated memory or reduce plugins/mods if this is Yes after a crash." />
                </dt>
                <dd>
                  {detail.containerState.OOMKilled === true ? "Yes" : "No"}
                </dd>
              </div>
              <div>
                <dt>
                  Process ID{" "}
                  <InfoTip text="The current Linux PID inside the container while it is running. Blank when the container is stopped." />
                </dt>
                <dd>
                  {typeof detail.containerState.Pid === "number"
                    ? detail.containerState.Pid
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>
                  Exit code{" "}
                  <InfoTip text="What the process returned last time it stopped. 0 is a clean exit. Anything else is a crash or an explicit non-zero stop." />
                </dt>
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
  const live = serverIsLive(server.status);
  const primaryAction = serverPrimaryLifecycleAction(server);
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
          {primaryAction === "restart" ? (
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
              {panelUrl !== null && (
                <a
                  class="button button--quiet"
                  href={panelUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  Open AMP
                  <Icon name="external" size={14} />
                </a>
              )}
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
            <>
              {primaryAction === "start" && (
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
              {panelUrl !== null && (
                <a
                  class="button button--quiet"
                  href={panelUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  Open AMP
                  <Icon name="external" size={14} />
                </a>
              )}
              {server.panelRunning && (
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
              )}
            </>
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
            Helix reads AMP's real instance state, including idle/sleep, and
            offers basic lifecycle shortcuts so the host is visible in one
            place. Idle means the game is sleeping; AMP's manager is often still
            running. It does not pretend this is a Helix-managed server. New
            servers use Helix’s own manager and receive the full toolset.
          </p>
        </div>
      </section>
      <div class="server-overview-grid">
        <section class="surface server-health">
          <div class="section-title">
            <div>
              <h2>Imported status</h2>
              <p>Compatibility inventory from AMP</p>
            </div>
            <span
              class={`state-label state-label--${serverStatusTone(server.status)}`}
            >
              {serverStatusLabel(server.status)}
            </span>
          </div>
          <div class="server-health-stats">
            <div>
              <span>Players</span>
              <strong>
                {serverPlayerHeadline(server)}
              </strong>
            </div>
            <div>
              <span>CPU</span>
              <strong>{live ? formatPercent(server.cpuPercent) : "—"}</strong>
            </div>
            <div>
              <span>Memory</span>
              <strong>{formatBytes(server.memoryUsedMb * 1024 * 1024)}</strong>
              <small>of {formatBytes(server.memoryLimitMb * 1024 * 1024)}</small>
            </div>
            <div>
              <span>AMP manager</span>
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

type ServerFilter = "all" | "helix" | "minecraft" | "vrising" | "valheim" | "terraria" | "imported";

function isMinecraftServer(server: ManagedServer): boolean {
  if (server.kind === "vrising" || server.kind === "valheim" || server.kind === "terraria") return false;
  if (server.kind === "minecraft") return true;
  return /minecraft|paper|purpur|folia|leaves|fabric|forge|spigot|bukkit|velocity|sponge|quilt|pufferfish|neoforge/iu.test(
    `${server.software} ${server.version}`,
  );
}

function isVRisingServer(server: ManagedServer): boolean {
  return server.kind === "vrising" || /v\s*rising/iu.test(server.software);
}

function isValheimServer(server: ManagedServer): boolean {
  return server.kind === "valheim" || /valheim/iu.test(server.software);
}

function isTerrariaServer(server: ManagedServer): boolean {
  return server.kind === "terraria" || /terraria|tmodloader/iu.test(server.software);
}

export function NewServerChooser({
  onMinecraft,
  onVRising,
  onValheim,
  onTerraria,
  onClose,
}: {
  onMinecraft: () => void;
  onVRising: () => void;
  onValheim: () => void;
  onTerraria: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog title="New server" onClose={onClose} wide>
      <div class="game-create-grid">
        <button type="button" onClick={onMinecraft}>
          <span class="game-create-icon game-create-icon--minecraft">
            <GameMark game="minecraft" size={32} />
          </span>
          <span>
            <strong>Minecraft: Java Edition</strong>
            <small>
              Paper, Fabric, Forge, NeoForge, Quilt, Pufferfish, and Modrinth or CurseForge packs.
            </small>
          </span>
          <em>Ready</em>
        </button>
        <button type="button" onClick={onVRising}>
          <span class="game-create-icon game-create-icon--vrising">
            <GameMark game="vrising" size={32} />
          </span>
          <span>
            <strong>V Rising</strong>
            <small>
              One click installs the dedicated server in an isolated container.
            </small>
          </span>
          <em>Click to install</em>
        </button>
        <button type="button" onClick={onValheim}>
          <span class="game-create-icon game-create-icon--valheim">
            <GameMark game="valheim" size={32} />
          </span>
          <span>
            <strong>Valheim</strong>
            <small>
              Linux dedicated server plus optional BepInEx plugins from Files.
            </small>
          </span>
          <em>Click to install</em>
        </button>
        <button type="button" onClick={onTerraria}>
          <span class="game-create-icon game-create-icon--terraria">
            <GameMark game="terraria" size={32} />
          </span>
          <span>
            <strong>Terraria</strong>
            <small>
              Vanilla or tModLoader. Drop `.tmod` files in mods and restart.
            </small>
          </span>
          <em>Click to install</em>
        </button>
      </div>
      <div class="server-platform-note">
        <Icon name="info" size={16} />
        <span>
          <strong>Nothing is installed on the host OS</strong>
          Helix downloads each dedicated server into a private container. Backups, start-on-boot, files, and logs work the same way across games.
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

function CreateVRisingDialog({
  csrfToken,
  servers,
  canManageNetwork,
  onClose,
  onComplete,
  onSessionExpired,
}: {
  csrfToken: string;
  servers: ManagedServer[];
  canManageNetwork: boolean;
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [name, setName] = useState("");
  const [memory, setMemory] = useState(4096);
  const [players, setPlayers] = useState(40);
  const [portMode, setPortMode] = useState<"automatic" | "manual">("automatic");
  const [gamePort, setGamePort] = useState(9876);
  const [queryPort, setQueryPort] = useState(9877);
  const [startOnBoot, setStartOnBoot] = useState(true);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [portPolicy, setPortPolicy] = useState<GamePortPolicy | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    void getVRisingPortPolicy(csrfToken, controller.signal)
      .then((policy) => {
        setPortPolicy(policy);
        if (policy.nextAvailablePort !== null) {
          setGamePort(policy.nextAvailablePort);
          setQueryPort(policy.nextAvailablePort + 1);
        }
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      });
    return () => controller.abort();
  }, [csrfToken, onSessionExpired]);

  const polling = useJobPolling({
    job,
    csrfToken,
    onJob: setJob,
    onComplete: async () => {
      await onComplete();
      onClose();
    },
    onSessionExpired,
  });

  const submit = async (): Promise<void> => {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const payload: Parameters<typeof createVRisingServer>[0] = {
        name: name.trim(),
        memory_mb: memory,
        max_players: players,
        start_on_boot: startOnBoot,
        wine_runtime_acknowledged: true,
      };
      if (portMode === "manual") {
        payload.game_port = gamePort;
        payload.query_port = queryPort;
      }
      const result = await createVRisingServer(payload, csrfToken);
      setJob({
        id: result.jobId,
        kind: "vrising_create",
        status: "queued",
        stage: "Queued",
        progressPercent: 0,
        createdAtUnixMs: Date.now(),
        updatedAtUnixMs: Date.now(),
        result: null,
        error: null,
      });
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setSubmitting(false);
    }
  };

  const busy = submitting || (job !== null && job.status !== "failed");
  return (
    <Dialog title="New V Rising server" onClose={onClose} wide>
      {job !== null && job.status !== "failed" ? (
        <div class="create-progress">
          <strong>{job.stage}</strong>
          <ProgressBar value={job.progressPercent} />
          <small>
            First create downloads the dedicated server into an isolated container. Leave this open.
          </small>
          {polling.error !== null && (
            <ServerFault
              message={polling.error}
              csrfToken={csrfToken}
              servers={servers}
              canManageNetwork={canManageNetwork}
              onSessionExpired={onSessionExpired}
            />
          )}
        </div>
      ) : (
        <>
          <p class="dialog-intro">
            Helix installs everything the dedicated server needs in an isolated container. Click create. When you uninstall the last V Rising server, that runtime is removed so the host looks like it was never there.
          </p>
          <div class="form-grid">
            <label class="field field--wide">
              <span>Server name</span>
              <input value={name} disabled={busy} onInput={(event) => setName(event.currentTarget.value)} maxlength={80} />
            </label>
            <label class="field">
              <span>Memory (MiB)</span>
              <input type="number" min={2048} max={24576} step={256} value={memory} disabled={busy} onInput={(event) => setMemory(Number(event.currentTarget.value))} />
            </label>
            <label class="field">
              <span>Player limit</span>
              <input type="number" min={1} max={128} value={players} disabled={busy} onInput={(event) => setPlayers(Number(event.currentTarget.value))} />
            </label>
            <label class="field field--wide">
              <span>Ports</span>
              <select value={portMode} disabled={busy} onChange={(event) => setPortMode(event.currentTarget.value as "automatic" | "manual")}>
                <option value="automatic">Automatic from the V Rising pool{portPolicy?.nextAvailablePort ? ` (next ${portPolicy.nextAvailablePort})` : ""}</option>
                <option value="manual">Specific UDP ports</option>
              </select>
            </label>
            {portMode === "manual" && (
              <>
                <label class="field">
                  <span>Game UDP</span>
                  <input type="number" min={1024} max={65535} value={gamePort} disabled={busy} onInput={(event) => setGamePort(Number(event.currentTarget.value))} />
                </label>
                <label class="field">
                  <span>Query UDP</span>
                  <input type="number" min={1024} max={65535} value={queryPort} disabled={busy} onInput={(event) => setQueryPort(Number(event.currentTarget.value))} />
                </label>
              </>
            )}
          </div>
          <label class="check-row">
            <input class="toggle-input" type="checkbox" checked={startOnBoot} disabled={busy} onChange={(event) => setStartOnBoot(event.currentTarget.checked)} />
            <span>
              <strong>Start with the host</strong>
              <small>Docker restart policy unless-stopped. Survives host reboot without starting the server now.</small>
            </span>
          </label>
          <ServerFault
            message={error ?? (job?.error ?? null)}
            csrfToken={csrfToken}
            servers={servers}
            canManageNetwork={canManageNetwork}
            onSessionExpired={onSessionExpired}
          />
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button>
            <button class="button button--primary" type="button" disabled={busy || name.trim().length === 0} onClick={() => void submit()}>
              {submitting ? "Starting…" : "Create V Rising server"}
            </button>
          </div>
        </>
      )}
    </Dialog>
  );
}

function CreateValheimDialog({
  csrfToken,
  servers,
  canManageNetwork,
  onClose,
  onComplete,
  onSessionExpired,
}: {
  csrfToken: string;
  servers: ManagedServer[];
  canManageNetwork: boolean;
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [name, setName] = useState("");
  const [memory, setMemory] = useState(4096);
  const [players, setPlayers] = useState(10);
  const [portMode, setPortMode] = useState<"automatic" | "manual">("automatic");
  const [gamePort, setGamePort] = useState(2456);
  const [startOnBoot, setStartOnBoot] = useState(true);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const polling = useJobPolling({
    job,
    csrfToken,
    onJob: setJob,
    onComplete: async () => {
      await onComplete();
      onClose();
    },
    onSessionExpired,
  });

  const submit = async (): Promise<void> => {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const result = await createValheimServer({
        name: name.trim(),
        memory_mb: memory,
        max_players: players,
        start_on_boot: startOnBoot,
        ...(portMode === "manual" ? { game_port: gamePort } : {}),
      }, csrfToken);
      setJob({
        id: result.jobId,
        kind: "valheim_create",
        status: "queued",
        stage: "Queued",
        progressPercent: 0,
        createdAtUnixMs: Date.now(),
        updatedAtUnixMs: Date.now(),
        result: null,
        error: null,
      });
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setSubmitting(false);
    }
  };

  const busy = submitting || (job !== null && job.status !== "failed");
  return (
    <Dialog title="New Valheim server" onClose={onClose} wide>
      {job !== null && job.status !== "failed" ? (
        <div class="create-progress">
          <strong>{job.stage}</strong>
          <ProgressBar value={job.progressPercent} />
          <small>First create downloads the dedicated server through SteamCMD. Drop a BepInEx pack zip at `/data/bepinex-pack.zip` and plugin DLLs in `/data/plugins` for one-restart mods.</small>
          {polling.error !== null && (
            <ServerFault
              message={polling.error}
              csrfToken={csrfToken}
              servers={servers}
              canManageNetwork={canManageNetwork}
              onSessionExpired={onSessionExpired}
            />
          )}
        </div>
      ) : (
        <>
          <p class="dialog-intro">Helix installs the Linux dedicated server in an isolated container. Public UPnP is not offered yet. Mods: put a BepInEx pack zip and plugin files in the server Files tab, then restart.</p>
          <div class="form-grid">
            <label class="field field--wide"><span>Server name</span><input value={name} disabled={busy} onInput={(event) => setName(event.currentTarget.value)} maxlength={80} /></label>
            <label class="field"><span>Memory (MiB)</span><input type="number" min={1024} max={16384} step={256} value={memory} disabled={busy} onInput={(event) => setMemory(Number(event.currentTarget.value))} /></label>
            <label class="field"><span>Player limit</span><input type="number" min={1} max={64} value={players} disabled={busy} onInput={(event) => setPlayers(Number(event.currentTarget.value))} /></label>
            <label class="field field--wide"><span>Ports</span><select value={portMode} disabled={busy} onChange={(event) => setPortMode(event.currentTarget.value as "automatic" | "manual")}><option value="automatic">Automatic from the Valheim pool</option><option value="manual">Specific UDP game port (uses +1 and +2 too)</option></select></label>
            {portMode === "manual" && <label class="field"><span>Game UDP</span><input type="number" min={1024} max={65535} value={gamePort} disabled={busy} onInput={(event) => setGamePort(Number(event.currentTarget.value))} /></label>}
          </div>
          <label class="check-row"><input class="toggle-input" type="checkbox" checked={startOnBoot} disabled={busy} onChange={(event) => setStartOnBoot(event.currentTarget.checked)} /><span><strong>Start with the host</strong><small>Docker restart policy unless-stopped.</small></span></label>
          <ServerFault
            message={error ?? (job?.error ?? null)}
            csrfToken={csrfToken}
            servers={servers}
            canManageNetwork={canManageNetwork}
            onSessionExpired={onSessionExpired}
          />
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button>
            <button class="button button--primary" type="button" disabled={busy || name.trim().length === 0} onClick={() => void submit()}>{submitting ? "Starting…" : "Create Valheim server"}</button>
          </div>
        </>
      )}
    </Dialog>
  );
}

function CreateTerrariaDialog({
  csrfToken,
  servers,
  canManageNetwork,
  onClose,
  onComplete,
  onSessionExpired,
}: {
  csrfToken: string;
  servers: ManagedServer[];
  canManageNetwork: boolean;
  onClose: () => void;
  onComplete: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [name, setName] = useState("");
  const [software, setSoftware] = useState<"vanilla" | "tmodloader">("vanilla");
  const [memory, setMemory] = useState(2048);
  const [players, setPlayers] = useState(8);
  const [portMode, setPortMode] = useState<"automatic" | "manual">("automatic");
  const [gamePort, setGamePort] = useState(7777);
  const [startOnBoot, setStartOnBoot] = useState(true);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const polling = useJobPolling({
    job,
    csrfToken,
    onJob: setJob,
    onComplete: async () => {
      await onComplete();
      onClose();
    },
    onSessionExpired,
  });

  const submit = async (): Promise<void> => {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const result = await createTerrariaServer({
        name: name.trim(),
        software,
        memory_mb: memory,
        max_players: players,
        start_on_boot: startOnBoot,
        network_exposure: "private",
        ...(portMode === "manual" ? { game_port: gamePort } : {}),
      }, csrfToken);
      setJob({
        id: result.jobId,
        kind: "terraria_create",
        status: "queued",
        stage: "Queued",
        progressPercent: 0,
        createdAtUnixMs: Date.now(),
        updatedAtUnixMs: Date.now(),
        result: null,
        error: null,
      });
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setSubmitting(false);
    }
  };

  const busy = submitting || (job !== null && job.status !== "failed");
  return (
    <Dialog title="New Terraria server" onClose={onClose} wide>
      {job !== null && job.status !== "failed" ? (
        <div class="create-progress">
          <strong>{job.stage}</strong>
          <ProgressBar value={job.progressPercent} />
          <small>Vanilla downloads the publisher zip. tModLoader uses SteamCMD. Drop `.tmod` files in `/data/mods` and restart for one-click mods.</small>
          {polling.error !== null && (
            <ServerFault
              message={polling.error}
              csrfToken={csrfToken}
              servers={servers}
              canManageNetwork={canManageNetwork}
              onSessionExpired={onSessionExpired}
            />
          )}
        </div>
      ) : (
        <>
          <p class="dialog-intro">Vanilla uses the official dedicated zip. tModLoader installs from Steam. Edit `serverconfig.txt` in Files. Place `.tmod` files in the mods folder, then restart.</p>
          <div class="form-grid">
            <label class="field field--wide"><span>Server name</span><input value={name} disabled={busy} onInput={(event) => setName(event.currentTarget.value)} maxlength={80} /></label>
            <label class="field field--wide"><span>Software</span><select value={software} disabled={busy} onChange={(event) => setSoftware(event.currentTarget.value as "vanilla" | "tmodloader")}><option value="vanilla">Vanilla dedicated</option><option value="tmodloader">tModLoader</option></select></label>
            <label class="field"><span>Memory (MiB)</span><input type="number" min={512} max={8192} step={256} value={memory} disabled={busy} onInput={(event) => setMemory(Number(event.currentTarget.value))} /></label>
            <label class="field"><span>Player limit</span><input type="number" min={1} max={255} value={players} disabled={busy} onInput={(event) => setPlayers(Number(event.currentTarget.value))} /></label>
            <label class="field field--wide"><span>Port</span><select value={portMode} disabled={busy} onChange={(event) => setPortMode(event.currentTarget.value as "automatic" | "manual")}><option value="automatic">Automatic from the Terraria pool</option><option value="manual">Specific TCP port</option></select></label>
            {portMode === "manual" && <label class="field"><span>Game TCP</span><input type="number" min={1024} max={65535} value={gamePort} disabled={busy} onInput={(event) => setGamePort(Number(event.currentTarget.value))} /></label>}
          </div>
          <label class="check-row"><input class="toggle-input" type="checkbox" checked={startOnBoot} disabled={busy} onChange={(event) => setStartOnBoot(event.currentTarget.checked)} /><span><strong>Start with the host</strong><small>Docker restart policy unless-stopped.</small></span></label>
          <ServerFault
            message={error ?? (job?.error ?? null)}
            csrfToken={csrfToken}
            servers={servers}
            canManageNetwork={canManageNetwork}
            onSessionExpired={onSessionExpired}
          />
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button>
            <button class="button button--primary" type="button" disabled={busy || name.trim().length === 0} onClick={() => void submit()}>{submitting ? "Starting…" : "Create Terraria server"}</button>
          </div>
        </>
      )}
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
  const [creatingVRising, setCreatingVRising] = useState(false);
  const [creatingValheim, setCreatingValheim] = useState(false);
  const [creatingTerraria, setCreatingTerraria] = useState(false);
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
        servers={servers}
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
      (filter === "helix"
        ? server.manager === "helix"
        : filter === "minecraft"
          ? isMinecraftServer(server)
          : filter === "vrising"
            ? isVRisingServer(server)
            : filter === "valheim"
              ? isValheimServer(server)
              : filter === "terraria"
                ? isTerrariaServer(server)
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
            {servers.reduce((total, server) => total + (server.playerCountVerified ? server.playersOnline : 0), 0)}
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
          class={filter === "helix" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("helix")}
        >
          Helix <span>{helixManaged}</span>
        </button>
        <button
          class={filter === "minecraft" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("minecraft")}
        >
          Minecraft <span>{servers.filter(isMinecraftServer).length}</span>
        </button>
        <button
          class={filter === "vrising" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("vrising")}
        >
          V Rising <span>{servers.filter(isVRisingServer).length}</span>
        </button>
        <button
          class={filter === "valheim" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("valheim")}
        >
          Valheim <span>{servers.filter(isValheimServer).length}</span>
        </button>
        <button
          class={filter === "terraria" ? "is-active" : ""}
          type="button"
          onClick={() => setFilter("terraria")}
        >
          Terraria <span>{servers.filter(isTerrariaServer).length}</span>
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
          <span>
            TPS{" "}
            <InfoTip text="Ticks per second. 20 is a healthy Minecraft server. Helix asks over the local RCON console. Vanilla and most Fabric/Forge/Quilt servers do not answer, so this stays —." />
          </span>
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
                ? "Create a native Minecraft, V Rising, Valheim, or Terraria server with New server. Helix Native stays separate from any AMP import."
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
          onVRising={() => {
            setChooseGame(false);
            setCreatingVRising(true);
          }}
          onValheim={() => {
            setChooseGame(false);
            setCreatingValheim(true);
          }}
          onTerraria={() => {
            setChooseGame(false);
            setCreatingTerraria(true);
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
      {creatingVRising && (
        <CreateVRisingDialog
          csrfToken={csrfToken}
          servers={servers}
          canManageNetwork={canManageNetwork}
          onClose={() => setCreatingVRising(false)}
          onComplete={async () => {
            await data.refresh();
            await loadRemoved();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
      {creatingValheim && (
        <CreateValheimDialog
          csrfToken={csrfToken}
          servers={servers}
          canManageNetwork={canManageNetwork}
          onClose={() => setCreatingValheim(false)}
          onComplete={async () => {
            await data.refresh();
            await loadRemoved();
          }}
          onSessionExpired={onSessionExpired}
        />
      )}
      {creatingTerraria && (
        <CreateTerrariaDialog
          csrfToken={csrfToken}
          servers={servers}
          canManageNetwork={canManageNetwork}
          onClose={() => setCreatingTerraria(false)}
          onComplete={async () => {
            await data.refresh();
            await loadRemoved();
          }}
          onSessionExpired={onSessionExpired}
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
