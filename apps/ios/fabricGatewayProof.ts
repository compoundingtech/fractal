import { isClientApiVersion, isClientErrorVersion } from './protocolVersions.ts';

// Pairing discovery is public; prove that an actual protected route still refuses this client.
export async function verifyUnpairedFabricGateway(url: string, client: string, fetchImpl: typeof fetch = fetch): Promise<void> {
  const response = await fetchImpl(`${url}/v1/client/agents?limit=1`, { headers: { Accept: 'application/json', 'x-st3-client': client } });
  const payload = await response.json();
  if (![401, 403].includes(response.status) || !isClientApiVersion(payload?.api_version) || !isClientErrorVersion(payload?.error_version)) {
    throw new Error('Test gateway did not refuse an unpaired client on a protected route');
  }
}
