---
title: "Terminal and iOS clients"
description: "Follow work, answer alerts, and connect a paired client without a local daemon."
order: 10
---

# Terminal and iOS clients

Both clients show work in your fleet. Finish [installation](../getting-started/install.md) on the daemon machine first.

## Terminal interface

Run `fractal` locally. **Ctrl+K** finds an agent, mission, or view. **Ctrl+H** opens Home / Now; **Ctrl+S** toggles the sidebar. Spaces hold tabs and splits; `fractal ui --space NAME` opens a named layout.

Opening an agent can attach its live terminal. Detach with **Ctrl+\** before using interface shortcuts; the leader list offers conversation and tab actions. Native login and trust prompts belong in the harness terminal. **Ctrl+Q** quits the client while the daemon and agents keep working.

Conversations carry messages; Now carries actionable requests. Read an alert's context and choose its explicit action. Merely opening the card is not approval.

## A client on another machine

Expose only the daemon's **paired-only gateway** through HTTPS or an encrypted tailnet route. Never expose the privileged daemon socket. Follow [gateway setup](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/README.md#tailnet-carrier).

On the daemon machine:

```sh
fractal devices --as person/ada pair --full-control 'Garden laptop'
```

On the client machine, complete pairing with the gateway URL, returned pairing ID, and member fingerprint obtained through a trusted channel:

```sh
fractal devices complete "$GATEWAY_URL" "$PAIRING_ID" --fingerprint "$MEMBER_FINGERPRINT"
fractal ui --client
```

Enter the single-use code privately when prompted. The saved profile supplies your person identity; no local daemon is needed for this client. Full control is broad authority: pair only devices you trust and revoke lost devices.

## iOS

The phone is a paired client, not a graph replica or agent host. It shows work, conversations, documents, and requests. Confirm it is connected before sending actions. An offline screen does not mean your remote agents have stopped.

See [Build and run the iOS app](ios-app.md) for local builds and [Client API](../reference/client-api.md) for scopes and transport details.
