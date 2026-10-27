// The carrier's ports on this build's native fabric bridge. Nothing here runs unless the person
// chose fabric and the build links the bridge.
import { FabricRefusal, type CarrierPorts, type FabricPath } from './carrier';
import { nativeRefusalAfter } from './fabricSession';
import { dialFabric, fabricAvailable, fabricIdentity, fabricStats, stopFabric } from './modules/st-fabric';

export { fabricAvailable, fabricIdentity };

function newestAttempt(stats: Record<string, unknown>): number {
  const attempts = Array.isArray(stats.attempts) ? stats.attempts : [];
  return Math.max(0, ...attempts.map(attempt => (attempt && typeof attempt.id === 'number' ? attempt.id : 0)));
}

export function nativeCarrierPorts(): CarrierPorts {
  // Attempts before this dial are old news: only a refusal after it concerns this connection.
  let floor = 0;
  const refusal = async () => {
    const refused = nativeRefusalAfter(await fabricStats(), floor);
    return refused ? new FabricRefusal(refused.code) : null;
  };
  return {
    dial: async target => {
      try { floor = newestAttempt(await fabricStats()); } catch { /* the first dial has no earlier attempts */ }
      const result = await dialFabric(target.node, target.service, target.address, 'auto');
      // The listener answers before the member does. One unauthenticated capabilities read through it
      // shows whether the member admitted this phone, refused it, or cannot be reached; the carrier's
      // dial limit bounds it, and stopping the bridge ends it.
      try {
        const response = await fetch(`${result.url}/v1/client/capabilities`, { headers: { accept: 'application/json' } });
        await response.text().catch(() => '');
      } catch {
        const refused = await refusal().catch(() => null);
        if (refused) throw refused;
        throw new Error('The member did not answer over fabric');
      }
      return { url: result.url };
    },
    stop: stopFabric,
    refusal,
    now: () => Date.now(),
    wait: ms => new Promise(resolve => setTimeout(resolve, ms)),
  };
}

/** The path the newest successful connection took, read on demand for the connection screen. */
export async function fabricPathInUse(): Promise<FabricPath> {
  try {
    const stats = await fabricStats();
    const attempts = Array.isArray(stats.attempts) ? stats.attempts : [];
    for (let index = attempts.length - 1; index >= 0; index--) {
      const attempt = attempts[index];
      if (attempt?.result === 'connected') {
        return attempt.selectedPath === 'direct' ? 'direct' : attempt.selectedPath === 'relay' ? 'relay' : 'unknown';
      }
    }
  } catch { /* no measurement is not a failure */ }
  return 'unknown';
}
