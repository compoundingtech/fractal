// arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
// Never expose identity values in a conflict diagnostic, or select either identity.
export const identityEnv = (preferredName, legacyName, env = process.env) => {
  const preferred = env[preferredName];
  const legacy = env[legacyName];
  if (preferred && legacy !== undefined && preferred !== legacy) {
    throw new Error(`Conflicting identity environment variables: ${preferredName} and ${legacyName}`);
  }
  return preferred ?? legacy;
};
