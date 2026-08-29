import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from "./api";

export interface SystemPackage {
  name: string;
  installedVersion: string;
  candidateVersion: string | null;
  upgradeAvailable: boolean;
  held: boolean | null;
  downloadSizeBytes: number | null;
  installedSizeBytes: number | null;
  sourcePackage: string | null;
  candidateOrigin: string | null;
  category: string | null;
  description: string;
  securityUpdate: boolean | null;
  restartHint: string;
  restartImpactKnown: boolean;
}

export interface SystemPackageInventory {
  availability: "ready" | "degraded" | "unavailable";
  collectedAtUnixMs: number;
  aptCacheRefreshedAtUnixMs: number | null;
  aptCacheRefreshPerformed: false;
  inventory: {
    installedTotal: number;
    upgradeAvailableTotal: number;
    securityUpdateTotal: number;
    truncated: boolean;
    packages: SystemPackage[];
  };
  simulation: {
    available: boolean;
    upgradeCandidates: number;
    newPackages: number;
    removals: number;
    heldBack: number;
    error: string | null;
    stateCanChangeAfterSimulation: true;
    mutatedPackageState: false;
  };
  hostRestart: {
    rebootRequiredMarkerPresent: boolean;
    packages: string[];
    automaticReboot: false;
  };
  upgradeApply: {
    available: boolean;
    reasonCode: string;
    reason: string;
    wouldRequireExplicitPackageCandidates: boolean;
    wouldRequireDisruptionAcknowledgement: boolean;
    requiredCapability: string;
    rollbackClaimed: false;
    automaticReboot: false;
    aptOrDpkgMutationAvailable: boolean;
    packageListsRefreshAvailable: boolean;
    conffilePolicy: "preserve_existing";
    newPackagesAllowed: false;
    packageRemovalsAllowed: false;
  };
  helixSelfUpdate: {
    available: boolean;
    reasonCode: string;
    reason: string;
    gitPullUsed: false;
    currentVersion: string;
    latestVersion: string | null;
    latestTag: string | null;
    releaseUrl: string | null;
    releaseNotes: string | null;
    updateAvailable: boolean;
    composeDetected: boolean;
    requiredConfirmation: string;
    rollbackClaimed: true;
    automaticReboot: false;
  };
  tools: {
    dpkgQuery: boolean;
    aptCache: boolean;
    aptGet: boolean;
    aptMark: boolean;
  };
  errors: Array<{ component: string; message: string }>;
}

export interface PackageJob {
  id: string;
  kind: string;
  status: "queued" | "running" | "complete" | "failed";
  stage: string;
  progressPercent: number;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
  result: unknown | null;
  error: string | null;
}

const MAX_TEXT = 4_096;

function bool(
  record: Record<string, unknown>,
  key: string,
  context: string,
): boolean {
  const value = record[key];
  if (typeof value !== "boolean")
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function requiredFalse(
  record: Record<string, unknown>,
  key: string,
  context: string,
): false {
  if (bool(record, key, context))
    throw new ApiError(`${context} returned an unsupported ${key} value.`);
  return false;
}

function requiredTrue(
  record: Record<string, unknown>,
  key: string,
  context: string,
): true {
  if (!bool(record, key, context))
    throw new ApiError(`${context} returned an unsupported ${key} value.`);
  return true;
}

function integer(
  record: Record<string, unknown>,
  key: string,
  context: string,
): number {
  return expectNumber(record, key, context, {
    integer: true,
    minimum: 0,
    maximum: Number.MAX_SAFE_INTEGER,
  });
}

function nullableInteger(
  record: Record<string, unknown>,
  key: string,
  context: string,
): number | null {
  return record[key] === null ? null : integer(record, key, context);
}

function nullableBool(
  record: Record<string, unknown>,
  key: string,
  context: string,
): boolean | null {
  return record[key] === null ? null : bool(record, key, context);
}

function text(
  record: Record<string, unknown>,
  key: string,
  context: string,
  allowEmpty = false,
): string {
  const value = record[key];
  if (
    typeof value !== "string" ||
    value.length > MAX_TEXT ||
    (!allowEmpty && value.trim().length === 0) ||
    Array.from(value).some(
      (character) =>
        /\p{Cc}/u.test(character) && character !== "\n" && character !== "\t",
    )
  ) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function nullableText(
  record: Record<string, unknown>,
  key: string,
  context: string,
): string | null {
  return record[key] === null || record[key] === undefined
    ? null
    : text(record, key, context, true);
}

function literal<T extends string>(
  record: Record<string, unknown>,
  key: string,
  context: string,
  values: readonly T[],
): T {
  const value = expectString(record, key, context);
  if (!values.includes(value as T))
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value as T;
}

function parsePackage(value: unknown): SystemPackage {
  const context = "system package";
  const item = expectRecord(value, context);
  return {
    name: text(item, "name", context),
    installedVersion: text(item, "installed_version", context),
    candidateVersion: nullableText(item, "candidate_version", context),
    upgradeAvailable: bool(item, "upgrade_available", context),
    held: nullableBool(item, "held", context),
    downloadSizeBytes: nullableInteger(item, "download_size_bytes", context),
    installedSizeBytes: nullableInteger(item, "installed_size_bytes", context),
    sourcePackage: nullableText(item, "source_package", context),
    candidateOrigin: nullableText(item, "candidate_origin", context),
    category: nullableText(item, "category", context),
    description: text(item, "description", context, true),
    securityUpdate: nullableBool(item, "security_update", context),
    restartHint: text(item, "restart_hint", context),
    restartImpactKnown: bool(item, "restart_impact_known", context),
  };
}

function parseHelixSelfUpdate(value: unknown): SystemPackageInventory["helixSelfUpdate"] {
  const context = "Helix update readiness";
  const selfUpdate = expectRecord(value, context);
  return {
    available: bool(selfUpdate, "available", context),
    reasonCode: text(selfUpdate, "reason_code", context),
    reason: text(selfUpdate, "reason", context),
    gitPullUsed: requiredFalse(selfUpdate, "git_pull_used", context),
    currentVersion: text(selfUpdate, "current_version", context),
    latestVersion: nullableText(selfUpdate, "latest_version", context),
    latestTag: nullableText(selfUpdate, "latest_tag", context),
    releaseUrl: nullableText(selfUpdate, "release_url", context),
    releaseNotes: nullableText(selfUpdate, "release_notes", context),
    updateAvailable: bool(selfUpdate, "update_available", context),
    composeDetected: bool(selfUpdate, "compose_detected", context),
    requiredConfirmation: text(selfUpdate, "required_confirmation", context),
    rollbackClaimed: requiredTrue(selfUpdate, "rollback_claimed", context),
    automaticReboot: requiredFalse(selfUpdate, "automatic_reboot", context),
  };
}

export function parseSystemPackageInventory(
  value: unknown,
): SystemPackageInventory {
  const context = "system package inventory";
  const root = expectRecord(value, context);
  if (integer(root, "schema_version", context) !== 1)
    throw new ApiError(
      "System package inventory returned an unsupported schema.",
    );
  const inventory = expectRecord(root.inventory, "package inventory");
  const simulation = expectRecord(root.simulation, "package simulation");
  const restart = expectRecord(root.host_restart, "host restart state");
  const apply = expectRecord(root.upgrade_apply, "package update readiness");
  const tools = expectRecord(root.tools, "package tools");
  return {
    availability: literal(root, "availability", context, [
      "ready",
      "degraded",
      "unavailable",
    ] as const),
    collectedAtUnixMs: integer(root, "collected_at_unix_ms", context),
    aptCacheRefreshedAtUnixMs: nullableInteger(
      root,
      "apt_cache_refreshed_at_unix_ms",
      context,
    ),
    aptCacheRefreshPerformed: requiredFalse(
      root,
      "apt_cache_refresh_performed",
      context,
    ),
    inventory: {
      installedTotal: integer(
        inventory,
        "installed_total",
        "package inventory",
      ),
      upgradeAvailableTotal: integer(
        inventory,
        "upgrade_available_total",
        "package inventory",
      ),
      securityUpdateTotal: integer(
        inventory,
        "security_update_total",
        "package inventory",
      ),
      truncated: bool(inventory, "truncated", "package inventory"),
      packages: expectArray(
        inventory,
        "packages",
        "package inventory",
        5_000,
      ).map(parsePackage),
    },
    simulation: {
      available: bool(simulation, "available", "package simulation"),
      upgradeCandidates: integer(
        simulation,
        "upgrade_candidates",
        "package simulation",
      ),
      newPackages: integer(simulation, "new_packages", "package simulation"),
      removals: integer(simulation, "removals", "package simulation"),
      heldBack: integer(simulation, "held_back", "package simulation"),
      error: nullableText(simulation, "error", "package simulation"),
      stateCanChangeAfterSimulation: requiredTrue(
        simulation,
        "state_can_change_after_simulation",
        "package simulation",
      ),
      mutatedPackageState: requiredFalse(
        simulation,
        "mutated_package_state",
        "package simulation",
      ),
    },
    hostRestart: {
      rebootRequiredMarkerPresent: bool(
        restart,
        "reboot_required_marker_present",
        "host restart state",
      ),
      packages: expectArray(
        restart,
        "packages",
        "host restart state",
        5_000,
      ).map((entry) => {
        if (
          typeof entry !== "string" ||
          entry.length === 0 ||
          entry.length > 256
        )
          throw new ApiError(
            "Host restart state returned an invalid package name.",
          );
        return entry;
      }),
      automaticReboot: requiredFalse(
        restart,
        "automatic_reboot",
        "host restart state",
      ),
    },
    upgradeApply: {
      available: bool(apply, "available", "package update readiness"),
      reasonCode: text(apply, "reason_code", "package update readiness"),
      reason: text(apply, "reason", "package update readiness"),
      wouldRequireExplicitPackageCandidates: bool(
        apply,
        "would_require_explicit_package_candidates",
        "package update readiness",
      ),
      wouldRequireDisruptionAcknowledgement: bool(
        apply,
        "would_require_disruption_acknowledgement",
        "package update readiness",
      ),
      requiredCapability: text(
        apply,
        "required_capability",
        "package update readiness",
      ),
      rollbackClaimed: requiredFalse(
        apply,
        "rollback_claimed",
        "package update readiness",
      ),
      automaticReboot: requiredFalse(
        apply,
        "automatic_reboot",
        "package update readiness",
      ),
      aptOrDpkgMutationAvailable: bool(
        apply,
        "apt_or_dpkg_mutation_available",
        "package update readiness",
      ),
      packageListsRefreshAvailable: bool(
        apply,
        "package_lists_refresh_available",
        "package update readiness",
      ),
      conffilePolicy: literal(
        apply,
        "conffile_policy",
        "package update readiness",
        ["preserve_existing"] as const,
      ),
      newPackagesAllowed: requiredFalse(
        apply,
        "new_packages_allowed",
        "package update readiness",
      ),
      packageRemovalsAllowed: requiredFalse(
        apply,
        "package_removals_allowed",
        "package update readiness",
      ),
    },
    helixSelfUpdate: parseHelixSelfUpdate(root.helix_self_update),
    tools: {
      dpkgQuery: bool(tools, "dpkg_query", "package tools"),
      aptCache: bool(tools, "apt_cache", "package tools"),
      aptGet: bool(tools, "apt_get", "package tools"),
      aptMark: bool(tools, "apt_mark", "package tools"),
    },
    errors: expectArray(root, "errors", context, 16).map((entry) => {
      const error = expectRecord(entry, "package inventory error");
      return {
        component: text(error, "component", "package inventory error"),
        message: text(error, "message", "package inventory error"),
      };
    }),
  };
}

export function getSystemPackageInventory(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<SystemPackageInventory> {
  return requestJson("/api/v1/system/packages", parseSystemPackageInventory, {
    csrfToken,
    signal,
    timeoutMs: 20_000,
  });
}

function parseJobDispatch(value: unknown): { jobId: string } {
  const root = expectRecord(value, "package job dispatch");
  return { jobId: text(root, "job_id", "package job dispatch") };
}

function parsePackageJob(value: unknown): PackageJob {
  const context = "package job";
  const root = expectRecord(value, context);
  return {
    id: text(root, "id", context),
    kind: text(root, "kind", context),
    status: literal(root, "status", context, [
      "queued",
      "running",
      "complete",
      "failed",
    ] as const),
    stage: text(root, "stage", context),
    progressPercent: integer(root, "progress_percent", context),
    createdAtUnixMs: integer(root, "created_at_unix_ms", context),
    updatedAtUnixMs: integer(root, "updated_at_unix_ms", context),
    result: root.result ?? null,
    error: nullableText(root, "error", context),
  };
}

export function refreshSystemPackageLists(
  csrfToken: string,
): Promise<{ jobId: string }> {
  return requestJson("/api/v1/system/packages/refresh", parseJobDispatch, {
    method: "POST",
    body: {},
    csrfToken,
  });
}

export function applySystemPackageUpdates(
  packages: SystemPackage[],
  confirmation: string,
  disruptionAcknowledged: boolean,
  csrfToken: string,
): Promise<{ jobId: string }> {
  return requestJson("/api/v1/system/packages/apply", parseJobDispatch, {
    method: "POST",
    body: {
      packages: packages.map((item) => ({
        name: item.name,
        installed_version: item.installedVersion,
        candidate_version: item.candidateVersion,
      })),
      confirmation,
      disruption_acknowledged: disruptionAcknowledged,
    },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function applyHelixUpdate(
  targetTag: string,
  confirmation: string,
  disruptionAcknowledged: boolean,
  csrfToken: string,
): Promise<{ jobId: string }> {
  return requestJson("/api/v1/system/helix/apply", parseJobDispatch, {
    method: "POST",
    body: {
      target_tag: targetTag,
      confirmation,
      disruption_acknowledged: disruptionAcknowledged,
    },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function checkHelixUpdate(
  csrfToken: string,
): Promise<SystemPackageInventory["helixSelfUpdate"]> {
  return requestJson("/api/v1/system/helix/check", parseHelixSelfUpdate, {
    method: "POST",
    body: {},
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function getSystemPackageJob(
  jobId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<PackageJob> {
  return requestJson(
    `/api/v1/system/packages/jobs/${encodeURIComponent(jobId)}`,
    parsePackageJob,
    {
      csrfToken,
      signal,
    },
  );
}
