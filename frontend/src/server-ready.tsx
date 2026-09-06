import { CopyButton } from './copy-button';
import { Icon } from './icons';

export function ServerReadySummary({ name, host, port, elapsed, pack, runtime, hostRequested, firewallState, hostError, pumpkin = false, bedrockPort }: {
  name: string; host: string; port: number; elapsed: string; pack: string | null;
  runtime: string; hostRequested: boolean; firewallState: string | null; hostError: string | null;
  pumpkin?: boolean;
  bedrockPort?: number | undefined;
}) {
  const address = host ? `${host.includes(':') ? `[${host}]` : host}:${port}` : null;
  const firewall = !hostRequested ? 'Host firewall unchanged'
    : firewallState === 'helix_rule_verified' ? 'Host firewall rule verified'
    : firewallState === 'ufw_inactive_not_blocking' ? 'UFW is inactive · no UFW rule needed'
    : firewallState === 'ufw_unavailable' ? 'UFW is not installed · check any other host firewall'
    : 'Check host firewall in Network';
  return <section class="server-ready">
    <div class="server-ready__heading"><span class="server-ready__check"><Icon name="check" size={22} /></span>
      <div><h3>{name}</h3><p>Online · ready in {elapsed}</p></div></div>
    <div class="server-ready__address"><div><span>Join on your network</span><strong>{address ?? 'Find this host’s LAN address in Network'}</strong></div>
      {address !== null && <CopyButton text={address} class="button button--quiet" />}</div>
    <div class="server-ready__facts"><div><span>Installation</span><strong>{pack ?? runtime}</strong>{pack !== null && <small>{runtime}</small>}</div>
      <div><span>Host network</span><strong>{firewall}</strong><small>Router settings were not changed.</small></div></div>
    <section class="server-ready__forward"><h4>Inviting players outside your network?</h4>
      <p>Add this port-forward in your router. Skip this for LAN-only play.</p>
      <dl><div><dt>Protocol</dt><dd>{pumpkin ? 'TCP (Java)' : 'TCP'}</dd></div><div><dt>External → internal port</dt><dd>{port} → {port}</dd></div><div><dt>Destination device</dt><dd>{host || 'This server’s LAN IP'}</dd></div></dl>
      {pumpkin && bedrockPort !== undefined && <p>Bedrock NetherNet: TCP + UDP {bedrockPort} → {bedrockPort} on the same device. Behind NAT, set networking.bedrock.nethernet.external_ip in pumpkin.toml to your public IP.</p>}
      <small>This forwards Minecraft only—not your Helix dashboard. Internet access has not been tested.</small>
    </section>
    {hostError !== null && <div class="server-ready__notice"><Icon name="warning" size={16} /><p>Your server is running. Host firewall setup needs attention: {hostError} Open Network to review it.</p></div>}
  </section>;
}
