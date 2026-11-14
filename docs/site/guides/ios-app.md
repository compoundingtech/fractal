---
title: "Build and run the iOS app"
description: "Build a local iPhone or simulator client and pair it to your daemon."
order: 11
---

# Build and run the iOS app

The Fractal iOS app is a client for your fleet. It connects through a paired-only gateway; the phone does not host agents or replicate the graph. There is no App Store or TestFlight installation path documented here: build the app locally.

## Requirements

Use a Mac with full Xcode, selected command-line tools, and an installed iOS Simulator runtime. Open Xcode once to finish setup. Install Node.js 24, Corepack, and CocoaPods, or use the repository's web development shell for JavaScript tooling. Native Xcode and signing still belong to the Mac.

## Prepare a checkout

```sh
git clone https://github.com/compoundingtech/fractal.git
cd fractal
corepack enable
pnpm install --frozen-lockfile
cd apps/ios
pnpm exec expo prebuild --platform ios --clean --no-install
pnpm run pods
```

Prebuild regenerates the ignored native project. Keep lasting native changes in checked-in Expo modules or configuration, not just generated files. Open the generated `.xcworkspace` in Xcode, not the project without CocoaPods.

## Simulator

Select the app scheme and an installed iPhone simulator in Xcode, use Debug, and run. Keep ordinary simulator signing enabled; disabling signing prevents usable Keychain storage and pairing.

Start Metro from `apps/ios` in another terminal:

```sh
pnpm start
```

Keep it running for a Debug build. This is a local native build; no Expo account, EAS service, or App Store upload is required.

## Pair to the daemon

Finish [Fractal setup](../getting-started/install.md). Make its [paired-only gateway](https://github.com/compoundingtech/fractal/blob/main/docs/st3/client-v0/README.md#tailnet-carrier) reachable from the simulator or phone. Use HTTPS or an encrypted tailnet route; plain LAN HTTP does not supply transport encryption. The gateway must carry HTTP requests and WebSockets. Never expose the privileged daemon socket.

```sh
fractal devices --as person/ada pair --full-control 'Garden iPhone'
```

Copy the returned person-root fingerprint separately from the trusted machine. Enter the gateway URL, pairing ID, private code, and fingerprint in the app. Open **Resources**, select your worker and send a message, then confirm a reply. Pairing success alone does not prove the account has the required control scope or that the worker is ready.

## Physical iPhone

Connect the phone, enable Developer Mode, select your Apple Development team and automatic provisioning, and choose a unique bundle identifier if required by your team. Do not commit signing identities, profiles, or pairing credentials.

For a self-contained Release build from `apps/ios`:

```sh
pnpm run export:ios
pnpm exec expo run:ios --device --configuration Release
```

Release embeds JavaScript and runs without Metro, but still needs its gateway for live work. See the [app development guide](https://github.com/compoundingtech/fractal/blob/main/apps/ios/README.md) for detailed scopes and native development.
