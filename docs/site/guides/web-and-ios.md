---
title: "Web and iOS clients"
description: "Open the daemon's local web app or pair an iOS client to follow your agents."
order: 10
---

# Web and iOS clients

The web app and iOS app show work in your fleet. Finish [installation](../getting-started/install.md) on the daemon machine first. The daemon serves the installed web app locally by default; a separate source checkout or JavaScript server is not needed for ordinary local access.

## Open the local web app

With the daemon running and its web assets installed:

```sh
fractal web
```

The daemon's local browser listener defaults to `http://127.0.0.1:7410`. The launcher reads a protected `web-launch-url` file from the daemon's state directory and opens the session URL in your browser. Do not copy that URL into chat, logs, or Git: it contains a one-time launch token.

The daemon consumes that token, sets a 12-hour HttpOnly SameSite=Strict session cookie, redirects to the app, and rotates the launch token. Restarting the daemon clears its browser sessions; existing WebSockets do not yet enforce session expiry live. `fractal web` opens the daemon's existing session endpoint; it does not start the daemon or a second server. If a browser cannot be opened, it prints the session URL, which must still be treated as a credential.

The web app provides an agent roster and conversations. Select an agent to inspect its observed state and send a message through the composer. Check connection and freshness indicators: retained content during a disconnect is not evidence that an agent is still running. A message send is not proof that its requested work completed.

## Availability and configuration

Run `fractal doctor` if the web app is unavailable. Missing web assets or a missing configured person can disable web access without disabling the daemon API. An occupied listener port is a daemon startup error, not an automatic fallback to another port.

For an explicitly supervised foreground daemon, `fractal up --web-listen off` disables the browser listener; `--web-dist DIR` selects a web asset directory. Do not start another daemon against an existing store merely to change these options. Assets are installed separately from the executable, and changing them requires a daemon restart.

Use `fractal web --state-dir DIR` when opening a daemon with a nondefault state directory. `FRACTAL_WEB_URL` is an explicit URL override; it is not needed for normal local access and does not configure a server.

The local listener is a **same-machine browser boundary**. Do not publish port 7410 through Tailscale Serve or expose it to the network. The local session acts as the daemon's configured person; it is not a multi-user authorization boundary. See the [local daemon web contract](https://github.com/compoundingtech/fractal/blob/main/docs/fractal-web.md#local-daemon-web-app) for exact origin, cookie, and current session limitations.

## Remote web access

Remote browser access uses a separate, opt-in tailnet publisher, not the daemon's local browser listener. That publisher has its own configuration, paired bearer, allowed tailnet logins, and loopback port; the documented example uses port 7411 so it does not collide with the local listener.

Follow the [opt-in tailnet publisher guide](https://github.com/compoundingtech/fractal/blob/main/docs/fractal-web.md#opt-in-tailnet-publisher). It describes the source build, server configuration, and Tailscale Serve HTTPS setup. Never use public Funnel for that publisher: admission depends on the authenticated tailnet login. Every local user who can reach its loopback port must already be trusted with the paired person's authority.

## iOS

The phone is a paired client, not a graph replica or agent host. It shows work, conversations, documents, and requests. Confirm it is connected before sending actions. An offline screen does not mean your remote agents have stopped.

Expose only the daemon's **paired-only gateway** through HTTPS or an encrypted tailnet route for the phone. This is separate from the local browser listener. Never expose the privileged daemon socket. Follow [gateway setup](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/README.md#tailnet-carrier).

On the daemon machine, begin a separate pairing for the phone:

```sh
fractal devices --as person/ada pair --full-control 'Garden iPhone'
```

Use your own person identity. Enter the gateway URL, pairing ID, and single-use code privately in the app. Pair only devices you trust and revoke lost devices. Merely opening a request card is not approval: read its context and choose an explicit available action.

See [Build and run the iOS app](ios-app.md) for local builds and [Client API](../reference/client-api.md) for scopes and transport details.
