import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from "./api";

export type NetworkProtocol = "tcp" | "udp";
export type FirewallManagementState =
  | "active"
  | "trashed"
  | "delete_pending"
  | "restore_pending";

export interface ListenerProcess {
  pid: number;
  name: string;
}

export interface NetworkListener {
  protocol: NetworkProtocol;
  family: "ipv4" | "ipv6";
  address: string;
  port: number;
  wildcard: boolean;
  uid: number;
  inode: number;
  process: ListenerProcess | null;
}

export interface DockerPublication {
  containerId: string;
  containerName: string;
  composeService: string | null;
  protocol: string;
  containerPort: number;
  hostAddress: string;
  hostPort: number;
}

export interface FirewallRule {
  number: number;
  display: string;
  action: string;
  source: string;
  protocol: NetworkProtocol | null;
  portStart: number | null;
  portEnd: number | null;
  comment: string | null;
  helixOwned: boolean;
  ruleId: string | null;
  managed: boolean;
  managementState: FirewallManagementState | null;
  name: string | null;
  description: string | null;
}

export interface ManagedFirewallRule {
  ruleId: string;
  name: string;
  description: string;
  protocol: NetworkProtocol;
  portStart: number;
  portEnd: number;
  state: FirewallManagementState;
  createdAtUnixMs: number;
  trashedAtUnixMs: number | null;
  undoAvailable: boolean;
  undoExpiresAtUnixMs: number | null;
  observedInUfw: boolean;
  exactBodyVerified: boolean;
}

export type FirewallAllowanceState =
  | "ufw_unavailable"
  | "ufw_state_unverified"
  | "ufw_inactive"
  | "allowed"
  | "not_allowed_by_matching_rule";

export interface GamePortMapping {
  instanceId: string;
  name: string;
  manager: string;
  port: number;
  protocol: NetworkProtocol;
  serverReportedRunning: boolean;
  listenerBound: boolean;
  dockerPublished: boolean;
  dockerPublications: DockerPublication[];
  firewallInputAllowance: {
    applicable: boolean;
    allowed: boolean | null;
    state: FirewallAllowanceState;
  };
  externalReachability: {
    state: "unverified";
    reachable: null;
    note: string;
  };
}

export interface NetworkInventoryError {
  manager: string;
  message: string;
}

export interface NetworkInventory {
  collectedAtUnixMs: number;
  listeners: {
    source: "linux_proc_net";
    items: NetworkListener[];
    truncated: boolean;
    ownerProcessBestEffort: boolean;
  };
  docker: {
    installed: boolean;
    publications: DockerPublication[];
    containersTruncated: boolean;
    error: string | null;
    note: string;
  };
  firewall: {
    backend: "ufw";
    installed: boolean;
    active: boolean;
    status: string;
    defaultPolicy: {
      incoming: string | null;
      outgoing: string | null;
      routed: string | null;
    };
    rules: FirewallRule[];
    rulesTruncated: boolean;
    helixManagedRuleState: ManagedFirewallRule[];
    error: string | null;
    mutationsSupported: boolean;
    mutationScope: string;
    inactiveNote: string | null;
  };
  gamePorts: GamePortMapping[];
  gamePortInventoryErrors: NetworkInventoryError[];
  externalReachability: {
    state: "unverified";
    testedFromExternalNetwork: false;
  };
}

export interface FirewallRuleSpec {
  name: string;
  description: string;
  protocol: NetworkProtocol;
  portStart: number;
  portEnd: number;
}

export interface FirewallMutationResult {
  ruleId: string;
  state: "active" | "trashed";
  rule: FirewallRuleSpec;
  verified: true;
  undoAvailable: boolean;
  undoExpiresAtUnixMs: number | null;
  beforeEvidence: FirewallEvidence;
  afterEvidence: FirewallEvidence;
}

export interface FirewallEvidence {
  installed: boolean;
  active: boolean;
  status: string;
  ruleCount: number;
  capturedAtUnixMs: number;
}

const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
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

function integer(
  record: Record<string, unknown>,
  key: string,
  context: string,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  return expectNumber(record, key, context, {
    integer: true,
    minimum: 0,
    maximum,
  });
}

function nullableInteger(
  record: Record<string, unknown>,
  key: string,
  context: string,
  maximum = Number.MAX_SAFE_INTEGER,
): number | null {
  return record[key] === null ? null : integer(record, key, context, maximum);
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

function ruleId(
  record: Record<string, unknown>,
  key: string,
  context: string,
): string {
  const value = expectString(record, key, context);
  if (!UUID.test(value))
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function nullableRuleId(
  record: Record<string, unknown>,
  key: string,
  context: string,
): string | null {
  return record[key] === null ? null : ruleId(record, key, context);
}

function parseDockerPublication(value: unknown): DockerPublication {
  const context = "Docker publication";
  const item = expectRecord(value, context);
  return {
    containerId: text(item, "container_id", context),
    containerName: text(item, "container_name", context),
    composeService: nullableText(item, "compose_service", context),
    protocol: text(item, "protocol", context),
    containerPort: integer(item, "container_port", context, 65_535),
    hostAddress: text(item, "host_address", context),
    hostPort: integer(item, "host_port", context, 65_535),
  };
}

function parseListener(value: unknown): NetworkListener {
  const context = "network listener";
  const item = expectRecord(value, context);
  const processValue = item.process;
  let process: ListenerProcess | null = null;
  if (processValue !== null) {
    const owner = expectRecord(processValue, "listener process");
    process = {
      pid: integer(owner, "pid", "listener process", 4_294_967_295),
      name: text(owner, "name", "listener process"),
    };
  }
  return {
    protocol: literal(item, "protocol", context, ["tcp", "udp"] as const),
    family: literal(item, "family", context, ["ipv4", "ipv6"] as const),
    address: text(item, "address", context),
    port: integer(item, "port", context, 65_535),
    wildcard: bool(item, "wildcard", context),
    uid: integer(item, "uid", context, 4_294_967_295),
    inode: integer(item, "inode", context),
    process,
  };
}

function parseFirewallRule(value: unknown): FirewallRule {
  const context = "firewall rule";
  const item = expectRecord(value, context);
  const protocol =
    item.protocol === null
      ? null
      : literal(item, "protocol", context, ["tcp", "udp"] as const);
  const managementState =
    item.management_state === null
      ? null
      : literal(item, "management_state", context, [
          "active",
          "trashed",
          "delete_pending",
          "restore_pending",
        ] as const);
  return {
    number: integer(item, "number", context, 100_000),
    display: text(item, "display", context),
    action: text(item, "action", context),
    source: text(item, "source", context),
    protocol,
    portStart: nullableInteger(item, "port_start", context, 65_535),
    portEnd: nullableInteger(item, "port_end", context, 65_535),
    comment: nullableText(item, "comment", context),
    helixOwned: bool(item, "helix_owned", context),
    ruleId: nullableRuleId(item, "rule_id", context),
    managed: bool(item, "managed", context),
    managementState,
    name: nullableText(item, "name", context),
    description: nullableText(item, "description", context),
  };
}

function parseManagedRule(value: unknown): ManagedFirewallRule {
  const context = "managed firewall rule";
  const item = expectRecord(value, context);
  return {
    ruleId: ruleId(item, "rule_id", context),
    name: text(item, "name", context),
    description: text(item, "description", context, true),
    protocol: literal(item, "protocol", context, ["tcp", "udp"] as const),
    portStart: integer(item, "port_start", context, 65_535),
    portEnd: integer(item, "port_end", context, 65_535),
    state: literal(item, "state", context, [
      "active",
      "trashed",
      "delete_pending",
      "restore_pending",
    ] as const),
    createdAtUnixMs: integer(item, "created_at_unix_ms", context),
    trashedAtUnixMs: nullableInteger(item, "trashed_at_unix_ms", context),
    undoAvailable: bool(item, "undo_available", context),
    undoExpiresAtUnixMs: nullableInteger(
      item,
      "undo_expires_at_unix_ms",
      context,
    ),
    observedInUfw: bool(item, "observed_in_ufw", context),
    exactBodyVerified: bool(item, "exact_body_verified", context),
  };
}

function parseGamePort(value: unknown): GamePortMapping {
  const context = "game port mapping";
  const item = expectRecord(value, context);
  const allowance = expectRecord(
    item.firewall_input_allowance,
    "game port firewall allowance",
  );
  const outside = expectRecord(
    item.external_reachability,
    "game port external reachability",
  );
  if (outside.reachable !== null)
    throw new ApiError(
      "Network inventory returned an invalid external reachability value.",
    );
  return {
    instanceId: text(item, "instance_id", context),
    name: text(item, "name", context),
    manager: text(item, "manager", context),
    port: integer(item, "port", context, 65_535),
    protocol: literal(item, "protocol", context, ["tcp", "udp"] as const),
    serverReportedRunning: bool(item, "server_reported_running", context),
    listenerBound: bool(item, "listener_bound", context),
    dockerPublished: bool(item, "docker_published", context),
    dockerPublications: expectArray(
      item,
      "docker_publications",
      context,
      512,
    ).map(parseDockerPublication),
    firewallInputAllowance: {
      applicable: bool(allowance, "applicable", "game port firewall allowance"),
      allowed: nullableBool(
        allowance,
        "allowed",
        "game port firewall allowance",
      ),
      state: literal(allowance, "state", "game port firewall allowance", [
        "ufw_unavailable",
        "ufw_state_unverified",
        "ufw_inactive",
        "allowed",
        "not_allowed_by_matching_rule",
      ] as const),
    },
    externalReachability: {
      state: literal(outside, "state", "game port external reachability", [
        "unverified",
      ] as const),
      reachable: null,
      note: text(outside, "note", "game port external reachability"),
    },
  };
}

export function parseNetworkInventory(value: unknown): NetworkInventory {
  const context = "network inventory";
  const root = expectRecord(value, context);
  if (integer(root, "schema_version", context) !== 1)
    throw new ApiError("Network inventory returned an unsupported schema.");
  const listeners = expectRecord(root.listeners, "network listeners");
  const docker = expectRecord(root.docker, "Docker network inventory");
  const firewall = expectRecord(root.firewall, "firewall inventory");
  const defaults = expectRecord(
    firewall.default_policy,
    "firewall default policy",
  );
  const outside = expectRecord(
    root.external_reachability,
    "external reachability",
  );
  if (outside.tested_from_external_network !== false) {
    throw new ApiError(
      "Network inventory returned an invalid external reachability claim.",
    );
  }
  const errors =
    root.game_port_inventory_errors === undefined
      ? []
      : expectArray(root, "game_port_inventory_errors", context, 32).map(
          (value) => {
            const item = expectRecord(value, "game port inventory error");
            return {
              manager: text(item, "manager", "game port inventory error"),
              message: text(item, "message", "game port inventory error"),
            };
          },
        );
  return {
    collectedAtUnixMs: integer(root, "collected_at_unix_ms", context),
    listeners: {
      source: literal(listeners, "source", "network listeners", [
        "linux_proc_net",
      ] as const),
      items: expectArray(listeners, "items", "network listeners", 4_096).map(
        parseListener,
      ),
      truncated: bool(listeners, "truncated", "network listeners"),
      ownerProcessBestEffort: bool(
        listeners,
        "owner_process_best_effort",
        "network listeners",
      ),
    },
    docker: {
      installed: bool(docker, "installed", "Docker network inventory"),
      publications: expectArray(
        docker,
        "publications",
        "Docker network inventory",
        4_096,
      ).map(parseDockerPublication),
      containersTruncated: bool(
        docker,
        "containers_truncated",
        "Docker network inventory",
      ),
      error: nullableText(docker, "error", "Docker network inventory"),
      note: text(docker, "note", "Docker network inventory"),
    },
    firewall: {
      backend: literal(firewall, "backend", "firewall inventory", [
        "ufw",
      ] as const),
      installed: bool(firewall, "installed", "firewall inventory"),
      active: bool(firewall, "active", "firewall inventory"),
      status: text(firewall, "status", "firewall inventory"),
      defaultPolicy: {
        incoming: nullableText(defaults, "incoming", "firewall default policy"),
        outgoing: nullableText(defaults, "outgoing", "firewall default policy"),
        routed: nullableText(defaults, "routed", "firewall default policy"),
      },
      rules: expectArray(firewall, "rules", "firewall inventory", 2_048).map(
        parseFirewallRule,
      ),
      rulesTruncated: bool(firewall, "rules_truncated", "firewall inventory"),
      helixManagedRuleState: expectArray(
        firewall,
        "helix_managed_rule_state",
        "firewall inventory",
        2_048,
      ).map(parseManagedRule),
      error: nullableText(firewall, "error", "firewall inventory"),
      mutationsSupported: bool(
        firewall,
        "mutations_supported",
        "firewall inventory",
      ),
      mutationScope: text(firewall, "mutation_scope", "firewall inventory"),
      inactiveNote: nullableText(
        firewall,
        "inactive_note",
        "firewall inventory",
      ),
    },
    gamePorts: expectArray(root, "game_ports", context, 2_048).map(
      parseGamePort,
    ),
    gamePortInventoryErrors: errors,
    externalReachability: {
      state: literal(outside, "state", "external reachability", [
        "unverified",
      ] as const),
      testedFromExternalNetwork: false,
    },
  };
}

export function validateFirewallRuleSpec(
  input: FirewallRuleSpec,
): FirewallRuleSpec {
  const name = input.name.trim();
  const description = input.description.trim();
  if (name.length < 1 || name.length > 80 || /\p{Cc}/u.test(name))
    throw new Error("Name must be 1–80 characters without control characters.");
  if (description.length > 300 || /\p{Cc}/u.test(description))
    throw new Error(
      "Description must be at most 300 characters without control characters.",
    );
  if (input.protocol !== "tcp" && input.protocol !== "udp")
    throw new Error("Protocol must be TCP or UDP.");
  if (
    !Number.isInteger(input.portStart) ||
    input.portStart < 1 ||
    input.portStart > 65_535
  )
    throw new Error("Start port must be between 1 and 65535.");
  if (
    !Number.isInteger(input.portEnd) ||
    input.portEnd < input.portStart ||
    input.portEnd > 65_535
  )
    throw new Error("End port must be between the start port and 65535.");
  if (input.portEnd - input.portStart + 1 > 1_024)
    throw new Error("A rule can cover at most 1024 ports.");
  return {
    name,
    description,
    protocol: input.protocol,
    portStart: input.portStart,
    portEnd: input.portEnd,
  };
}

function parseMutation(value: unknown): FirewallMutationResult {
  const context = "firewall mutation";
  const root = expectRecord(value, context);
  const state = literal(root, "state", context, ["active", "trashed"] as const);
  const spec = parseRuleSpec(
    root[state === "trashed" ? "original_rule" : "rule"],
    context,
  );
  const beforeEvidence = parseEvidence(
    root.before_evidence,
    "firewall evidence before mutation",
  );
  const afterEvidence = parseEvidence(
    root.after_evidence,
    "firewall evidence after mutation",
  );
  if (!bool(root, "verified", context))
    throw new ApiError("Firewall mutation did not return verified evidence.");
  if (state === "trashed") {
    integer(root, "trashed_at_unix_ms", context);
  } else if (root.created_at_unix_ms !== undefined) {
    integer(root, "created_at_unix_ms", context);
  } else {
    integer(root, "restored_at_unix_ms", context);
  }
  return {
    ruleId: ruleId(root, "rule_id", context),
    state,
    rule: spec,
    verified: true,
    undoAvailable: bool(root, "undo_available", context),
    undoExpiresAtUnixMs:
      root.undo_expires_at_unix_ms === undefined
        ? null
        : nullableInteger(root, "undo_expires_at_unix_ms", context),
    beforeEvidence,
    afterEvidence,
  };
}

function parseRuleSpec(value: unknown, context: string): FirewallRuleSpec {
  const item = expectRecord(value, context);
  return validateFirewallRuleSpec({
    name: text(item, "name", context),
    description: text(item, "description", context, true),
    protocol: literal(item, "protocol", context, ["tcp", "udp"] as const),
    portStart: integer(item, "port_start", context, 65_535),
    portEnd: integer(item, "port_end", context, 65_535),
  });
}

function parseEvidence(value: unknown, context: string): FirewallEvidence {
  const item = expectRecord(value, context);
  return {
    installed: bool(item, "installed", context),
    active: bool(item, "active", context),
    status: text(item, "status", context),
    ruleCount: integer(item, "rule_count", context, 100_000),
    capturedAtUnixMs: integer(item, "captured_at_unix_ms", context),
  };
}

export function getNetworkInventory(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<NetworkInventory> {
  return requestJson("/api/v1/network/inventory", parseNetworkInventory, {
    csrfToken,
    signal,
    timeoutMs: 15_000,
  });
}

export function createFirewallRule(
  input: FirewallRuleSpec,
  csrfToken: string,
): Promise<FirewallMutationResult> {
  const rule = validateFirewallRuleSpec(input);
  return requestJson("/api/v1/network/firewall/rules", parseMutation, {
    method: "POST",
    csrfToken,
    body: {
      name: rule.name,
      description: rule.description,
      protocol: rule.protocol,
      port_start: rule.portStart,
      port_end: rule.portEnd,
    },
  });
}

export function deleteFirewallRule(
  ruleIdValue: string,
  csrfToken: string,
): Promise<FirewallMutationResult> {
  if (!UUID.test(ruleIdValue))
    return Promise.reject(new Error("Invalid firewall rule identity."));
  return requestJson(
    `/api/v1/network/firewall/rules/${ruleIdValue}`,
    parseMutation,
    {
      method: "DELETE",
      csrfToken,
      body: {},
    },
  );
}

export function restoreFirewallRule(
  ruleIdValue: string,
  csrfToken: string,
): Promise<FirewallMutationResult> {
  if (!UUID.test(ruleIdValue))
    return Promise.reject(new Error("Invalid firewall rule identity."));
  return requestJson(
    `/api/v1/network/firewall/rules/${ruleIdValue}/restore`,
    parseMutation,
    {
      method: "POST",
      csrfToken,
      body: {},
    },
  );
}

export function enableFirewall(
  sshPort: number,
  confirmation: string,
  csrfToken: string,
): Promise<{ enabled: true; sshPort: number }> {
  return requestJson(
    "/api/v1/network/firewall/enable",
    (value) => {
      const root = expectRecord(value, "firewall enable result");
      if (
        !bool(root, "enabled", "firewall enable result") ||
        !bool(root, "verified", "firewall enable result")
      ) {
        throw new ApiError("The firewall enable outcome was not verified.");
      }
      return {
        enabled: true,
        sshPort: integer(root, "ssh_port", "firewall enable result", 65_535),
      };
    },
    {
      method: "POST",
      body: { ssh_port: sshPort, confirmation },
      csrfToken,
      timeoutMs: 30_000,
    },
  );
}
