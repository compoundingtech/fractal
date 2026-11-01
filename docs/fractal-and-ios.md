# Using the terminal UI and the iOS app

Both clients show the work in your fleet. Finish [getting started](getting-started.md) on the daemon machine first.

## Use the terminal UI

In a terminal on that machine, run:

```sh
st
```

The terminal UI opens a space with tabs and splits, the last one you used on this machine; `st ui --space NAME` opens a named one. Spaces are the terminal UI's only layout: `st ui --classic` no longer opens the layout from before them, and says to run `st ui` or `st ui --space NAME` instead. **Ctrl+S** shows or hides the sidebar; choose **Agents** or **Missions** there. **Ctrl+K** finds an agent, mission, machine, space, or text in conversations, and opens **Inbox**, **Usage** and the **Fleet** list; type a name and press **Enter** to open it.

- **Inbox** lists your alerts, then updates that ask nothing. An alert is anything waiting on your answer: an ask, a review gate, a launch or revision approval, a harness prompt or login. **Ctrl+H**, when you are not typing, or the alert count in the top bar opens or focuses its tab; the count is hidden when there are no alerts. It sits beside agent tabs: **Tab** switches tabs, **Ctrl+W** closes it, and its tab can move into a split with the tab menu or by dragging. **Esc** does not close Inbox. Each alert also shows in its agent's conversation, above the message box, as one line; click it or press **Ctrl+N** to expand it in place to the same card Inbox shows; a click on its top line, or **Esc** while the card has the keys, folds it back. Opening again focuses the existing Inbox in this space, keeping the list and selected card.

A harness prompt's card reads like the harness's own dialog: what it asks about (the command, edit or plan in full), its question line ("Allow this read outside the working directories?"), and its options numbered in the harness's order and words ("1. Yes, but ask again next time"), with one marked **❯**. The card offers only the answers st can send, so the harness's other choices (keep allowing, auto-accept edits) stay in its terminal, and the card says so. A Claude question shows each question with its options to choose (several for a multi-select) before **Submit answers**; a screen prompt (a new MCP server, workspace trust) and a call Claude's auto mode denied show their own answers. The words come from one shared model for web, tui and ios, checked against the same cases in both languages: `crates/fractal-ui-model/src/prompts.rs` (tui uses it) and `clients/typescript/fractal-views/promptForm.ts` (for web and ios).

Answering a prompt's card:

- **Click** an option to answer it. A refusal (No, exit, quit) is a plain option and asks nothing more.
- **Ctrl+N** in a conversation, or **a** on the card in Inbox, gives the card the keys. **↑/↓** and **Tab** move the mark and answer nothing. A number answers that option, or chooses it for a question; on a card of ten or more options, a digit that starts a longer number waits for the second digit, or for **Enter** to take it alone. **Enter** answers the marked option only when it is a safe one about what is shown in full (allow a call shown whole, approve a plan shown whole, submit your own choices, continue without an MCP server), never a refusal. **Esc** gives the keys back.
- Typing never waits on a card: a letter goes straight to the message box with the keys.
- A call too long for the band is cut there and its allow withheld; click its note to read it whole on the card in Inbox, which offers it. A card whose form does not fit the band at all (several questions on a short terminal) shows only its title and a note: click it to answer in Inbox.

Follow each card's displayed actions; opening a card does not approve it.
- **Agents** opens a seat's conversation, with messages, the harness transcript, and tool output. Scroll to read earlier entries; type in the message box and press **Enter** to reply.
- **Images** in a conversation or on an ask's card show as a line naming the image, its type and its size. Click the line, or press **Ctrl+U** on the focused row, to show it inline; terminals that draw images (kitty, iTerm2, sixel) show the first one on screen by themselves. **open** shows it in your viewer or saves it to your downloads. See [what shows where](st3/attachments.md#what-shows-where).
- **Missions** shows each run's steps, progress, waiting reason, and who is working on it, and under them the documents the mission and its steps name.
- **Documents** open read-only, as their author wrote them, with their version and when it changed. Type a name such as `doc/garden/plan` in **Ctrl+K**, choose **Documents** there for the list (or `doc/garden/` for the documents under it), or click a `doc/…` link in a message, a mission or another document. On a document, clicking a link opens it in place; **b** goes back, **n** and **p** choose a link and **Enter** opens it. A document that cannot be shown says why: not found, not synced to this machine yet (it shows once it arrives), or the host did not answer; **r** reads it again.
- **Usage** shows observed tokens, estimated cost, and account limits. Click **$ usage** in the top bar (widen the terminal if needed) or choose **Usage** with **Ctrl+K**; **b** changes grouping and **p** changes the period.

Submitting a request answer in **Now** immediately removes that card locally and selects the next item (or the preceding item at the end). The footer shows **Sending** while the action and the live Attention stream confirm it; this does not mean the daemon has accepted it yet. An old stream frame cannot bring an in-flight answer back. If sending fails, the card and any submitted words return with an error, without moving focus away from work started on the next item. A connection failure can leave the daemon's outcome unknown: check the restored request before sending it again. Pending answers are local to this running UI, not a durable offline queue.

Opening an agent attaches its live terminal; **Ctrl+\\** opens the leader list, where **c** shows its conversation (**Ctrl+\\** again closes the tab, **t** opens a new one, **q** quits). The **Attach** button opens the agent's terminal again; **Enter** attaches a detached terminal pane. Finish harness login or trust prompts there. An agent without a terminal opens its conversation. To open agents on their conversation instead, choose **Conversation first** in **Ctrl+K**; the choice stays on this device. Alerts for the agent show on top of its terminal: click to see and answer them, while every key still goes to the terminal. If the connection drops, the terminal says it is reconnecting and why, and attaches again by itself. Detach before using the terminal UI's direct navigation keys, or use **Ctrl+\\** then **[** or **]** for place-history back/forward. **Ctrl+Q** quits the terminal UI when detached; the daemon, agents, and their work keep running. See the [terminal client reference](../crates/fractal-tui/README.md) for more controls.

### Connect from another machine

Install Fractal on the client machine. Publish the daemon's **paired-only gateway** with the [gateway setup](st3/client-v0/README.md#tailnet-carrier), using HTTPS or an encrypted tailnet route. On the daemon machine, make a pairing challenge:

```sh
st devices --as person/ada pair --full-control 'Garden laptop'
```

On the client machine, use your gateway URL and the returned pairing ID:

```sh
printf 'Gateway URL: '; read -r st_gateway_url
printf 'Pairing ID: '; read -r st_pairing_id
# Set st_member_fingerprint to the fingerprint obtained through a trusted channel.
st devices complete "$st_gateway_url" "$st_pairing_id" --fingerprint "$st_member_fingerprint"
st ui --client
```

Enter the single-use code privately when prompted. The saved pairing supplies your person identity and lets the terminal UI connect without a local daemon. See [client-only setup](st3/client-only.md) for multiple members and reconnecting.

## Put the iOS app on your phone

There is no App Store or TestFlight build; build it yourself on a Mac with full Xcode, Node.js 24, and CocoaPods. From a new checkout:

```sh
mkdir -p ~/src
git clone https://github.com/compoundingtech/fractal.git ~/src/fractal
cd ~/src/fractal
corepack enable
pnpm install --frozen-lockfile
cd apps/ios
pnpm exec expo prebuild --platform ios --clean --no-install
pnpm run pods
pnpm run export:ios
open ios/fractal.xcworkspace
```

In Xcode, select the **fractal** scheme and your connected iPhone. Under **Signing & Capabilities**, choose your own Apple Development team and automatic provisioning; use a unique bundle identifier if your team cannot register the default. Enable Developer Mode on the phone when asked. From `apps/ios`, [build and install](https://docs.expo.dev/guides/local-app-development/) onto the phone:

```sh
pnpm exec expo run:ios --device --configuration Release
```

Choose your iPhone when prompted. Release embeds the JavaScript so you can use the app without Metro.

On your daemon machine, make a separate challenge for the phone:

```sh
st devices --as person/ada pair --full-control 'Garden iPhone'
```

In the app, enter the reachable gateway URL, pairing ID, and code. Open **Agents**, select your worker, and send a message. To read a document, open **Resources**, then **Documents**: tap one in the list, or type `doc/NAME` and tap **Open**. A `doc/…` link in a message, a mission or a step opens that document; inside it, tap a link to follow it and use back to return. Documents are read-only. Images in a conversation and on an ask's card show as a box naming the image, its type and its size; each loads when it scrolls into view. Tap one to zoom (pinch, **Close**) or **Share** it to save it or open it in another app; see [what shows where](st3/attachments.md#what-shows-where). [Build and run the iOS app](site/guides/ios-app.md) has the Xcode prerequisites, simulator and Debug workflow, signing details, and pairing troubleshooting.
