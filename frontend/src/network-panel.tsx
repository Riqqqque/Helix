import type { JSX } from "preact";
import { useCallback, useEffect, useMemo, useState } from "preact/hooks";
import { ApiError } from "./api";
import { InlineError } from "./dashboard-ui";
import { formatTimestamp } from "./format";
import { Icon } from "./icons";
import { InfoTip } from "./info-tip";
import { Dialog } from "./modal";
import {
  createFirewallRule,
  deleteFirewallRule,
  enableFirewall,
  getNetworkInventory,
  restoreFirewallRule,
  validateFirewallRuleSpec,
  type FirewallAllowanceState,
  type FirewallRuleSpec,
  type GamePortMapping,
  type ManagedFirewallRule,
  type NetworkInventory,
} from "./network-api";
import "./infrastructure.css";

export interface NetworkOperationsProps {
  csrfToken: string;
  canManageFirewall: boolean;
  onSessionExpired: () => void;
}

function describeError(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Helix could not read the network evidence.";
}

function isSessionError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.status === 401 || error.code === "csrf_rejected")
  );
}

function evidenceTone(value: boolean | null): "good" | "warning" | "idle" {
  if (value === true) return "good";
  if (value === false) return "warning";
  return "idle";
}

function firewallAllowanceLabel(state: FirewallAllowanceState): string {
  switch (state) {
    case "allowed":
      return "Matching allow rule";
    case "not_allowed_by_matching_rule":
      return "No matching allow rule";
    case "ufw_inactive":
      return "UFW inactive";
    case "ufw_unavailable":
      return "UFW unavailable";
    case "ufw_state_unverified":
      return "UFW state unknown";
  }
}

function portRange(
  rule: Pick<ManagedFirewallRule, "portStart" | "portEnd" | "protocol">,
): string {
  const ports =
    rule.portStart === rule.portEnd
      ? `${rule.portStart}`
      : `${rule.portStart}–${rule.portEnd}`;
  return `${ports}/${rule.protocol}`;
}

function GamePortRow({ port }: { port: GamePortMapping }) {
  return (
    <tr>
      <td>
        <strong>{port.name}</strong>
        <small>
          {port.manager.replaceAll("_", " ")} ·{" "}
          {port.serverReportedRunning ? "server running" : "server stopped"}
        </small>
      </td>
      <td>
        <code>
          {port.port}/{port.protocol}
        </code>
      </td>
      <td>
        <span
          class={`state-label state-label--${evidenceTone(port.listenerBound)}`}
        >
          {port.listenerBound ? "Bound" : "Not bound"}
        </span>
      </td>
      <td>
        <span
          class={`state-label state-label--${evidenceTone(port.dockerPublished)}`}
        >
          {port.dockerPublished ? "Published" : "Not published"}
        </span>
        {port.dockerPublications[0] !== undefined && (
          <small>
            {port.dockerPublications[0].hostAddress}:
            {port.dockerPublications[0].hostPort}
          </small>
        )}
      </td>
      <td>
        <span
          class={`state-label state-label--${evidenceTone(port.firewallInputAllowance.allowed)}`}
        >
          {firewallAllowanceLabel(port.firewallInputAllowance.state)}
        </span>
      </td>
      <td>
        <span class="state-label state-label--idle">Unknown</span>
        <small>Not tested externally</small>
      </td>
    </tr>
  );
}

export function NetworkEvidenceView({
  inventory,
  canManageFirewall,
  busyRuleId,
  pendingDelete,
  onPendingDelete,
  onDelete,
  onRestore,
}: {
  inventory: NetworkInventory;
  canManageFirewall: boolean;
  busyRuleId: string | null;
  pendingDelete: string | null;
  onPendingDelete: (ruleId: string | null) => void;
  onDelete: (ruleId: string) => void;
  onRestore: (ruleId: string) => void;
}) {
  const firewall = inventory.firewall;
  const listenerRows = inventory.listeners.items.slice(0, 200);
  const publicationRows = inventory.docker.publications.slice(0, 200);
  const activeManaged = firewall.helixManagedRuleState.filter(
    (rule) => rule.state !== "trashed",
  );
  const trashedManaged = firewall.helixManagedRuleState.filter(
    (rule) => rule.state === "trashed",
  );
  return (
    <>
      <section
        class="network-evidence-grid"
        aria-label="Network evidence summary"
      >
        <article class="network-evidence-card">
          <span>
            Local sockets{" "}
            <InfoTip text="Linux reports these TCP and UDP sockets on this host. A listener alone does not mean the firewall or router permits traffic." />
          </span>
          <strong>{inventory.listeners.items.length}</strong>
          <small>
            {inventory.listeners.truncated
              ? "Bounded result · more exist"
              : "Read from Linux socket tables"}
          </small>
        </article>
        <article class="network-evidence-card">
          <span>
            Docker publications{" "}
            <InfoTip text="A Docker publication maps a container port to a host address. It is separate from both a local listener and UFW policy." />
          </span>
          <strong>{inventory.docker.publications.length}</strong>
          <small>
            {inventory.docker.installed
              ? "Exact host bindings shown below"
              : "Docker unavailable"}
          </small>
        </article>
        <article class="network-evidence-card">
          <span>
            UFW firewall{" "}
            <InfoTip text="Helix reads UFW separately. An inactive firewall can be enabled only through the confirmed SSH-safe flow; Helix never resets UFW or changes its default policies." />
          </span>
          <strong class={firewall.active ? "text-good" : ""}>
            {!firewall.installed
              ? "Unavailable"
              : firewall.active
                ? "Active"
                : "Inactive"}
          </strong>
          <small>
            {firewall.error ??
              `Incoming default: ${firewall.defaultPolicy.incoming ?? "unknown"}`}
          </small>
        </article>
        <article class="network-evidence-card network-evidence-card--unknown">
          <span>
            Outside reachability{" "}
            <InfoTip text="Helix has not tested these ports from another network. Router forwarding, upstream firewalls, CGNAT, and ISP policy are outside this evidence." />
          </span>
          <strong>Unknown</strong>
          <small>No external probe was run</small>
        </article>
      </section>

      <div class="network-truth-note">
        <Icon name="info" size={17} />
        <div>
          <strong>Four different facts, shown separately</strong>
          <span>
            A bound listener, Docker publication, and UFW allowance do not prove
            a port works from the internet. Docker DNAT can also bypass the
            normal UFW INPUT path on some configurations.
          </span>
        </div>
      </div>

      <section class="surface infrastructure-section">
        <div class="section-title">
          <div>
            <h2>
              Game port evidence{" "}
              <InfoTip text="Each column answers one narrow question: is the server listening, did Docker publish it, does active UFW show a matching allow rule, and has outside reachability been verified?" />
            </h2>
            <p>
              {inventory.gamePorts.length} mappings reported by Helix and AMP
            </p>
          </div>
        </div>
        <div class="table-scroll">
          <table class="data-table network-game-table">
            <thead>
              <tr>
                <th>Server</th>
                <th>Port</th>
                <th>Local listener</th>
                <th>Docker binding</th>
                <th>UFW INPUT</th>
                <th>Outside host</th>
              </tr>
            </thead>
            <tbody>
              {inventory.gamePorts.map((port) => (
                <GamePortRow
                  key={`${port.instanceId}-${port.port}-${port.protocol}`}
                  port={port}
                />
              ))}
            </tbody>
          </table>
        </div>
        {inventory.gamePorts.length === 0 && (
          <div class="table-state">No managed game ports were reported.</div>
        )}
        {inventory.gamePortInventoryErrors.map((error) => (
          <p class="table-note" key={`${error.manager}-${error.message}`}>
            {error.manager}: {error.message}
          </p>
        ))}
      </section>

      <div class="network-detail-grid">
        <section class="surface infrastructure-section">
          <div class="section-title">
            <div>
              <h2>
                Local listeners{" "}
                <InfoTip text="Process ownership is best-effort because Linux may restrict access to another process’s file descriptors." />
              </h2>
              <p>TCP and UDP sockets from /proc/net</p>
            </div>
            <span class="section-count">
              {inventory.listeners.items.length}
            </span>
          </div>
          <div class="table-scroll infrastructure-scroll">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Protocol</th>
                  <th>Bind</th>
                  <th>Port</th>
                  <th>Process</th>
                </tr>
              </thead>
              <tbody>
                {listenerRows.map((listener) => (
                  <tr
                    key={`${listener.protocol}-${listener.family}-${listener.address}-${listener.port}-${listener.inode}`}
                  >
                    <td>
                      {listener.protocol.toUpperCase()} · {listener.family}
                    </td>
                    <td>
                      <code>{listener.address}</code>
                      {listener.wildcard && (
                        <small>All matching interfaces</small>
                      )}
                    </td>
                    <td>
                      <code>{listener.port}</code>
                    </td>
                    <td>
                      {listener.process === null ? (
                        "Unavailable"
                      ) : (
                        <>
                          <strong>{listener.process.name}</strong>
                          <small>PID {listener.process.pid}</small>
                        </>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {listenerRows.length === 0 && (
            <div class="table-state">No bound sockets were reported.</div>
          )}
          {inventory.listeners.items.length > listenerRows.length && (
            <p class="table-note">
              Showing the first {listenerRows.length} sockets to keep this page
              responsive.
            </p>
          )}
        </section>

        <section class="surface infrastructure-section">
          <div class="section-title">
            <div>
              <h2>
                Docker publications{" "}
                <InfoTip text="The host bind address matters: 127.0.0.1 is local-only, while 0.0.0.0 or :: can bind all matching host interfaces." />
              </h2>
              <p>Container ports mapped onto this host</p>
            </div>
            <span class="section-count">
              {inventory.docker.publications.length}
            </span>
          </div>
          <div class="table-scroll infrastructure-scroll">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Container</th>
                  <th>Host bind</th>
                  <th>Container port</th>
                </tr>
              </thead>
              <tbody>
                {publicationRows.map((publication) => (
                  <tr
                    key={`${publication.containerId}-${publication.protocol}-${publication.hostAddress}-${publication.hostPort}`}
                  >
                    <td>
                      <strong>{publication.containerName}</strong>
                      <small>
                        {publication.composeService ??
                          publication.containerId.slice(0, 12)}
                      </small>
                    </td>
                    <td>
                      <code>
                        {publication.hostAddress}:{publication.hostPort}/
                        {publication.protocol}
                      </code>
                    </td>
                    <td>
                      <code>{publication.containerPort}</code>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {publicationRows.length === 0 && (
            <div class="table-state">
              {inventory.docker.error ??
                "No Docker port publications were reported."}
            </div>
          )}
          <p class="table-note table-note--neutral">{inventory.docker.note}</p>
        </section>
      </div>

      <section class="surface infrastructure-section firewall-section">
        <div class="section-title">
          <div>
            <h2>
              Helix-managed UFW rules{" "}
              <InfoTip text="Only allow rules carrying an exact Helix UUID and a matching protected record can be removed or restored here. Other UFW rules stay read-only." />
            </h2>
            <p>{firewall.mutationScope}</p>
          </div>
          <span
            class={`state-label state-label--${firewall.active ? "good" : "warning"}`}
          >
            {firewall.status}
          </span>
        </div>
        {(firewall.inactiveNote !== null || firewall.error !== null) && (
          <div class="firewall-unavailable">
            <Icon name="warning" size={17} />
            <span>{firewall.error ?? firewall.inactiveNote}</span>
          </div>
        )}
        <div class="managed-rule-list">
          {[...activeManaged, ...trashedManaged].map((rule) => {
            const pending = pendingDelete === rule.ruleId;
            const busy = busyRuleId === rule.ruleId;
            return (
              <article
                key={rule.ruleId}
                class={
                  rule.state === "trashed"
                    ? "managed-rule managed-rule--trashed"
                    : "managed-rule"
                }
              >
                <div>
                  <strong>{rule.name}</strong>
                  <span>
                    <code>{portRange(rule)}</code> ·{" "}
                    {rule.state.replaceAll("_", " ")}
                  </span>
                  {rule.description.length > 0 && (
                    <small>{rule.description}</small>
                  )}
                </div>
                <div class="managed-rule-evidence">
                  <span
                    class={`state-label state-label--${rule.exactBodyVerified ? "good" : "warning"}`}
                  >
                    {rule.exactBodyVerified
                      ? "Exact match"
                      : rule.state === "trashed"
                        ? "Removed"
                        : "Needs review"}
                  </span>
                  {rule.state === "trashed" &&
                    rule.undoExpiresAtUnixMs !== null && (
                      <small>
                        Undo until {formatTimestamp(rule.undoExpiresAtUnixMs)}
                      </small>
                    )}
                </div>
                <div class="managed-rule-actions">
                  {rule.state === "trashed" ? (
                    <button
                      class="button button--quiet"
                      type="button"
                      disabled={
                        !canManageFirewall || !rule.undoAvailable || busy
                      }
                      onClick={() => onRestore(rule.ruleId)}
                    >
                      <Icon name="restart" size={14} />
                      {busy
                        ? "Restoring…"
                        : rule.undoAvailable
                          ? "Undo"
                          : "Undo expired"}
                    </button>
                  ) : pending ? (
                    <>
                      <button
                        class="button button--danger"
                        type="button"
                        disabled={busy}
                        onClick={() => onDelete(rule.ruleId)}
                      >
                        {busy ? "Removing…" : "Confirm remove"}
                      </button>
                      <button
                        class="button button--quiet"
                        type="button"
                        disabled={busy}
                        onClick={() => onPendingDelete(null)}
                      >
                        Cancel
                      </button>
                    </>
                  ) : (
                    <button
                      class="button button--danger-quiet"
                      type="button"
                      disabled={
                        !canManageFirewall ||
                        !firewall.mutationsSupported ||
                        busy ||
                        rule.state !== "active"
                      }
                      onClick={() => onPendingDelete(rule.ruleId)}
                    >
                      <Icon name="trash" size={14} />
                      Remove
                    </button>
                  )}
                </div>
              </article>
            );
          })}
          {firewall.helixManagedRuleState.length === 0 && (
            <div class="table-state">
              No firewall rules have been created by Helix.
            </div>
          )}
        </div>
        {firewall.rules.some((rule) => !rule.managed) && (
          <details class="unmanaged-rules">
            <summary>
              View {firewall.rules.filter((rule) => !rule.managed).length}{" "}
              read-only UFW rules
            </summary>
            <div class="table-scroll">
              <table class="data-table">
                <thead>
                  <tr>
                    <th>Rule</th>
                    <th>Action</th>
                    <th>Source</th>
                  </tr>
                </thead>
                <tbody>
                  {firewall.rules
                    .filter((rule) => !rule.managed)
                    .slice(0, 200)
                    .map((rule) => (
                      <tr key={`${rule.number}-${rule.display}`}>
                        <td>{rule.display}</td>
                        <td>{rule.action}</td>
                        <td>{rule.source}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          </details>
        )}
      </section>
    </>
  );
}

export function NetworkOperationsPanel({
  csrfToken,
  canManageFirewall,
  onSessionExpired,
}: NetworkOperationsProps) {
  const [inventory, setInventory] = useState<NetworkInventory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyRuleId, setBusyRuleId] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [enableOpen, setEnableOpen] = useState(false);
  const [sshPort, setSshPort] = useState("22");
  const [enableConfirmation, setEnableConfirmation] = useState("");
  const [form, setForm] = useState<{
    name: string;
    description: string;
    protocol: "tcp" | "udp";
    portStart: string;
    portEnd: string;
  }>({
    name: "",
    description: "",
    protocol: "tcp",
    portStart: "25565",
    portEnd: "25565",
  });

  const load = useCallback(
    async (signal?: AbortSignal): Promise<void> => {
      setLoading(true);
      try {
        const next = await getNetworkInventory(csrfToken, signal);
        setInventory(next);
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

  const runMutation = useCallback(
    async (
      ruleId: string,
      operation: () => Promise<unknown>,
      success: string,
    ): Promise<boolean> => {
      setBusyRuleId(ruleId);
      setError(null);
      try {
        await operation();
        setNotice(success);
        setPendingDelete(null);
        await load();
        return true;
      } catch (requestError) {
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
        return false;
      } finally {
        setBusyRuleId(null);
      }
    },
    [load, onSessionExpired],
  );

  const submit = async (
    event: JSX.TargetedSubmitEvent<HTMLFormElement>,
  ): Promise<void> => {
    event.preventDefault();
    let spec: FirewallRuleSpec;
    try {
      spec = validateFirewallRuleSpec({
        name: form.name,
        description: form.description,
        protocol: form.protocol,
        portStart: Number(form.portStart),
        portEnd: Number(form.portEnd),
      });
    } catch (validationError) {
      setError(describeError(validationError));
      return;
    }
    const created = await runMutation(
      "create",
      () => createFirewallRule(spec, csrfToken),
      `${spec.name} was added to active UFW and verified.`,
    );
    if (created)
      setForm((current) => ({ ...current, name: "", description: "" }));
  };

  const canCreate =
    canManageFirewall && inventory?.firewall.mutationsSupported === true;
  const detectedSshPorts = useMemo(() => {
    if (inventory === null) return [];
    const candidates = inventory.listeners.items
      .filter(
        (listener) =>
          listener.protocol === "tcp" &&
          (listener.process?.name.toLowerCase().includes("sshd") === true ||
            listener.port === 22),
      )
      .map((listener) => listener.port);
    return [...new Set(candidates)].sort((left, right) => left - right);
  }, [inventory]);
  const canOfferEnable =
    canManageFirewall &&
    inventory?.firewall.installed === true &&
    inventory.firewall.active === false &&
    inventory.firewall.error === null &&
    detectedSshPorts.length > 0;
  const unavailableReason = useMemo(() => {
    if (!canManageFirewall)
      return "Your account does not have network.firewall.write.";
    if (inventory === null) return "Waiting for verified UFW state.";
    if (!inventory.firewall.installed)
      return "UFW is unavailable. Helix will not pretend a rule can be added.";
    if (!inventory.firewall.active)
      return "UFW is inactive. Use the SSH-safe enable flow above before adding rules.";
    return inventory.firewall.error ?? null;
  }, [canManageFirewall, inventory]);

  return (
    <div class="infrastructure-panel" aria-busy={loading}>
      <div class="section-title section-title--spaced">
        <div>
          <h2>
            Ports and firewall{" "}
            <InfoTip text="This inventory keeps socket, container, firewall, and outside-network evidence separate so a partial configuration is never presented as an open internet port." />
          </h2>
          <p>Live host evidence with narrowly scoped UFW controls</p>
        </div>
        <button
          class="button button--quiet"
          type="button"
          disabled={loading || busyRuleId !== null}
          onClick={() => void load()}
        >
          <Icon name="refresh" size={15} />
          {loading ? "Reading…" : "Refresh evidence"}
        </button>
      </div>
      <InlineError message={error} />
      {notice !== null && (
        <div class="infrastructure-notice" role="status">
          <Icon name="check" size={16} />
          <span>{notice}</span>
          <button
            class="icon-button"
            type="button"
            aria-label="Dismiss message"
            onClick={() => setNotice(null)}
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
      {inventory?.firewall.installed === true &&
        !inventory.firewall.active && (
          <div class="firewall-enable-callout">
            <Icon name="warning" size={18} />
            <div>
              <strong>UFW is installed but inactive</strong>
              <span>
                Add and remove controls stay disabled until the host firewall is
                active. Helix can preserve a currently listening SSH port, enable
                UFW, and verify both outcomes in one confirmed operation.
              </span>
              {detectedSshPorts.length === 0 && (
                <small>
                  No listening SSH port was detected, so enabling stays blocked to
                  avoid locking you out.
                </small>
              )}
            </div>
            <button
              class="button button--primary"
              type="button"
              disabled={!canOfferEnable || busyRuleId !== null}
              onClick={() => {
                setSshPort(String(detectedSshPorts[0] ?? 22));
                setEnableConfirmation("");
                setEnableOpen(true);
              }}
            >
              Enable safely
            </button>
          </div>
        )}
      {inventory === null ? (
        <div class="detail-loading">
          <Icon name={error === null ? "network" : "warning"} size={28} />
          <span>
            {error === null
              ? "Reading listeners, Docker bindings, and UFW…"
              : "Network evidence is unavailable."}
          </span>
        </div>
      ) : (
        <NetworkEvidenceView
          inventory={inventory}
          canManageFirewall={canManageFirewall}
          busyRuleId={busyRuleId}
          pendingDelete={pendingDelete}
          onPendingDelete={setPendingDelete}
          onDelete={(ruleId) =>
            void runMutation(
              ruleId,
              () => deleteFirewallRule(ruleId, csrfToken),
              "The rule was removed from UFW. Undo remains available for 15 minutes.",
            )
          }
          onRestore={(ruleId) =>
            void runMutation(
              ruleId,
              () => restoreFirewallRule(ruleId, csrfToken),
              "The exact rule was restored and verified in UFW.",
            )
          }
        />
      )}

      <section class="surface infrastructure-section firewall-create">
        <div class="section-title">
          <div>
            <h2>
              Add a host firewall rule{" "}
              <InfoTip text="This creates a UFW allow rule on this Linux host only. It does not enable UFW, configure a router, bypass CGNAT, or prove outside reachability." />
            </h2>
            <p>Named, recoverable TCP or UDP allowance</p>
          </div>
          <span
            class={`state-label state-label--${canCreate ? "good" : "warning"}`}
          >
            {canCreate ? "Available" : "Unavailable"}
          </span>
        </div>
        <form
          class="firewall-rule-form"
          onSubmit={(event) => void submit(event)}
        >
          <label class="field">
            <span>Name</span>
            <input
              required
              maxLength={80}
              value={form.name}
              onInput={(event) =>
                setForm((current) => ({
                  ...current,
                  name: event.currentTarget.value,
                }))
              }
              placeholder="Plex web"
              disabled={!canCreate || busyRuleId !== null}
            />
          </label>
          <label class="field field--wide">
            <span>Description</span>
            <input
              maxLength={300}
              value={form.description}
              onInput={(event) =>
                setForm((current) => ({
                  ...current,
                  description: event.currentTarget.value,
                }))
              }
              placeholder="Why this host rule exists"
              disabled={!canCreate || busyRuleId !== null}
            />
          </label>
          <label class="field">
            <span>Protocol</span>
            <select
              value={form.protocol}
              onChange={(event) =>
                setForm((current) => ({
                  ...current,
                  protocol: event.currentTarget.value === "udp" ? "udp" : "tcp",
                }))
              }
              disabled={!canCreate || busyRuleId !== null}
            >
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
            </select>
          </label>
          <label class="field">
            <span>Start port</span>
            <input
              type="number"
              min="1"
              max="65535"
              required
              value={form.portStart}
              onInput={(event) =>
                setForm((current) => ({
                  ...current,
                  portStart: event.currentTarget.value,
                }))
              }
              disabled={!canCreate || busyRuleId !== null}
            />
          </label>
          <label class="field">
            <span>End port</span>
            <input
              type="number"
              min="1"
              max="65535"
              required
              value={form.portEnd}
              onInput={(event) =>
                setForm((current) => ({
                  ...current,
                  portEnd: event.currentTarget.value,
                }))
              }
              disabled={!canCreate || busyRuleId !== null}
            />
          </label>
          <div class="firewall-form-action">
            <button
              class="button button--primary"
              type="submit"
              disabled={!canCreate || busyRuleId !== null}
            >
              <Icon name="plus" size={15} />
              {busyRuleId === "create"
                ? "Adding and verifying…"
                : "Add UFW rule"}
            </button>
            <small>
              {unavailableReason ??
                "Range is limited to 1024 ports. The new rule is verified after UFW writes it."}
            </small>
          </div>
        </form>
      </section>
      {enableOpen && (
        <Dialog title="Enable UFW safely?" onClose={() => setEnableOpen(false)}>
          <div class="dialog-copy">
            <p>
              Helix first adds an exact allow rule for the selected, currently
              listening SSH port. It then enables UFW and reads the firewall back.
              If either result cannot be verified, it attempts to restore the
              inactive state.
            </p>
            <p>
              This does not configure your router or prove internet reachability.
              Existing UFW defaults and rules will begin applying to the host.
            </p>
          </div>
          <div class="firewall-enable-form">
            <label class="field">
              <span>Listening SSH port</span>
              <select
                value={sshPort}
                onChange={(event) => setSshPort(event.currentTarget.value)}
              >
                {detectedSshPorts.map((port) => (
                  <option value={port} key={port}>
                    {port}/tcp
                  </option>
                ))}
              </select>
            </label>
            <label class="field">
              <span>
                Type <strong>ENABLE UFW</strong> to confirm
              </span>
              <input
                autocomplete="off"
                value={enableConfirmation}
                onInput={(event) =>
                  setEnableConfirmation(event.currentTarget.value)
                }
              />
            </label>
          </div>
          <div class="dialog-actions">
            <button
              class="button button--quiet"
              type="button"
              disabled={busyRuleId === "enable"}
              onClick={() => setEnableOpen(false)}
            >
              Cancel
            </button>
            <button
              class="button button--danger"
              type="button"
              disabled={
                enableConfirmation !== "ENABLE UFW" ||
                busyRuleId !== null ||
                !detectedSshPorts.includes(Number(sshPort))
              }
              onClick={() =>
                void runMutation(
                  "enable",
                  () =>
                    enableFirewall(
                      Number(sshPort),
                      enableConfirmation,
                      csrfToken,
                    ),
                  `UFW is active and SSH port ${sshPort}/tcp is preserved by a verified Helix rule.`,
                ).then((changed) => {
                  if (changed) setEnableOpen(false);
                })
              }
            >
              {busyRuleId === "enable" ? "Enabling and verifying…" : "Enable UFW"}
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
