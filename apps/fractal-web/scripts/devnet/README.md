# Devnet launchers

Devnet assigns `DEV_EDGE_URL` and `PORT` before starting either launcher. Neither
launcher queries Devnet status: the edge need not be published before startup.

From the app directory:

```sh
devnet up <purpose> -- scripts/devnet/preview-launch.sh
devnet up <purpose> --ready-path / -- scripts/devnet/launch.sh <purpose> <build-dir>
```

- Both require `DEV_EDGE_URL`, `PORT`, and `WF_ST_CREDENTIAL_FILE`. The credential
  file holds the paired gateway's raw bearer token, not an authorization header.
  Keep it readable only by the deployment user. Use an absolute credential path.
- Both export `WF_EDGE_HOST` from the edge URL's hostname. `WF_ST_GATEWAY` defaults
  to `unix:/run/user/$(id -u)/st3-client.sock`; set it for a different gateway.
- Preview exports `WF_ALLOWED_HOSTS=127.0.0.1,localhost,<edge-host>` and reads the
  credential into `WF_ST_AUTHORIZATION` with a `Bearer ` prefix, as required by the
  public Vite gateway plugin. `WF_VITE_CONFIG` defaults to `vite.config.ts`.
  Config paths and the working directory resolve relative to the app, not the
  caller. Preview needs installed app dependencies (Vite), Bash, and Python 3.
- Immutable launch keeps `<purpose> <build-dir>` compatibility; purpose is unused.
  The build directory resolves with `realpath` from the caller's directory. It
  exports `WF_PORT=$PORT` and passes `WF_ST_CREDENTIAL_FILE` to the build's
  `server/server.mjs`, with `<build-dir>/web` as its argument. It needs Bash,
  Python 3, `realpath`, Node.js, and that deployment-provided build. This is not
  the public `server/main.mts` host, which has a separate configuration contract.

Deployments keep credentials and private composition outside the repository.
A private preview composition can select `WF_VITE_CONFIG=private/vite.config.ts`;
these launchers do not supply private configs, servers, relays, or build tools.
