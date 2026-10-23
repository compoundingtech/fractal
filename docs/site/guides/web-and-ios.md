---
title: "Web and iOS clients"
description: "Open an existing web app or pair an iOS client to follow your agents."
order: 10
---

# Web and iOS clients

The web app and iOS app show work in your fleet. Finish [installation](../getting-started/install.md) on the daemon machine first. A graphical client is separate from the daemon: starting the daemon does not start a web server.

## Open the web app

If you already have a configured web server, open its URL in your browser. The CLI can open that URL for you:

```sh
FRACTAL_WEB_URL=https://your-configured-web-app.example fractal web
```

Replace the example URL with your actual endpoint. `fractal web` opens a running app; it does **not** start or configure the server. Without `FRACTAL_WEB_URL`, it opens `http://127.0.0.1:5173`, which is useful only when a server is already listening there. If a browser cannot be opened, the command prints the URL.

The web app provides an agent roster and conversations. Select an agent to inspect its observed state and send a message through the composer. Check the connection and freshness indicators: retained content during a disconnect is not evidence that an agent is still running. A message send is not proof that its requested work completed.

## Host the web app

The web app currently requires a source checkout and separate server configuration. There is no automatic web-server startup in `fractal web`.

From the repository root after the frozen workspace install, the build and server commands are:

```sh
pnpm --dir apps/fractal-web build
pnpm --dir apps/fractal-web start
```

The start command requires the hosting configuration described in the [web implementation guide](https://github.com/compoundingtech/fractal/blob/main/docs/fractal-web.md#hosting-the-web-app-on-a-tailnet): an existing paired-only gateway socket, a protected paired bearer file, the exact public origin, and an allowed tailnet login list. Do not run the start command expecting those credentials or settings to be created for you.

The documented deployment binds the server to loopback and publishes it with Tailscale Serve over HTTPS. Never use public Funnel for that deployment: its admission depends on the authenticated tailnet login. Every local user who can reach the loopback server must already be trusted with the paired person's authority. A full-control pairing grants broad control; this is not a multi-user authorization boundary.

## iOS

The phone is a paired client, not a graph replica or agent host. It shows work, conversations, documents, and requests. Confirm it is connected before sending actions. An offline screen does not mean your remote agents have stopped.

Expose only the daemon's **paired-only gateway** through HTTPS or an encrypted tailnet route. Never expose the privileged daemon socket. Follow [gateway setup](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/README.md#tailnet-carrier).

On the daemon machine, begin a separate pairing for the phone:

```sh
fractal devices --as person/ada pair --full-control 'Garden iPhone'
```

Use your own person identity. Enter the gateway URL, pairing ID, and single-use code privately in the app. Pair only devices you trust and revoke lost devices. Merely opening a request card is not approval: read its context and choose an explicit available action.

See [Build and run the iOS app](ios-app.md) for local builds and [Client API](../reference/client-api.md) for scopes and transport details.
