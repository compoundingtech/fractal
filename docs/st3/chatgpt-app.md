# ChatGPT app: composition and install/use

Status: design note for [#2172](https://github.com/compoundingtech/smalltalk/issues/2172). It builds on the
assistant principal and grants in [#2290](https://github.com/compoundingtech/smalltalk/pull/2290) and the
proof-gated assistant reads in [#2541](https://github.com/compoundingtech/smalltalk/pull/2541). Neither is on
`main` yet. Their normative text is `docs/st3/external-assistants.md` on those branches.

The note answers three questions before any code is written:

1. Which pieces exist and where each one lives.
2. How a person installs the app and connects it to their own daemon.
3. How someone who is not the original author does the same with no private infrastructure.

The model is the iOS app ([ios-app.md](../ios-app.md), [client-v0](client-v0/README.md#pairing-and-remote-access)).

## Composition

```
ChatGPT (web, developer mode)
   │  MCP over OpenAI Secure MCP Tunnel (outbound only, no public URL)
   ▼
tunnel-client  ── person's own OpenAI Platform org
   │  MCP Streamable HTTP over a private Unix socket (0700 dir)
   ▼
st-chatgpt adapter  ── this repo; holds the assistant profile
   │  typed client-v0 assistant routes, Bearer + per-request proof
   ▼
st3-client.sock (paired-only gateway) ── never st3.sock
   ▼
st daemon  ── authoritative: principal, grant, scopes, fences, receipts
```

| Piece | Lives in | Owns | Must not |
| --- | --- | --- | --- |
| st daemon | `crates/st3` | `assistant/chatgpt` principal, owner-issued grants, allowlisted projections, proof verification, nonce and idempotency receipts | Speak MCP or OAuth; open a public listener |
| Generated client | `crates/st3-client` (codegen from `operations.json`) | Typed assistant operations | Sign, retry, or widen budgets |
| `st-chatgpt` adapter | New crate in this repo, shipped with st releases | MCP server; fixed tool list mapped 1:1 to typed operations; tool annotations; assistant profile | Run commands, scrape terminals, touch `st3.sock`, put credentials in tool output |
| Approval attestor | Pluggable, pinned per grant with `st devices grant read-trust` | Signing the per-read and per-action approval proofs after the owner approves | Share a key with the adapter (#2541 rejects that) |
| ChatGPT app record | Each person's ChatGPT account | Developer-mode MCP app bound to their tunnel | Hold st credentials |
| Downstream deployment (for example dotfiles) | Outside this repo | Service units and attestor wiring for one installation | Fork the adapter or its tool list |

Decisions behind the table:

- The adapter is a separate process, not daemon code. This keeps MCP, ChatGPT quirks and the tunnel out of the
  daemon, as the iOS app keeps Swift and the phone out of it. The adapter is a client like the phone: it reaches the
  daemon only through the paired gateway and authenticates with its own grant.
- The tool list is a closed set in the adapter source. Each tool names one client-v0 operation, its scope and its
  effect. A new tool needs a new typed operation first. There is no passthrough.
- ChatGPT's confirmation card (`readOnlyHint: false`) is advisory. Authorization comes only from the grant and the
  owner's approval proof that st verifies. Read tools are annotated read-only; every action tool is annotated as a
  write so ChatGPT also asks.

## Comparison with the iOS app

| | iOS app | ChatGPT app |
| --- | --- | --- |
| Principal | Paired device of `person/ada` | `assistant/chatgpt`, never a person, no signing chain |
| Pairing | `st devices --as person/ada pair NAME` | `st devices --as person/ada pair --assistant chatgpt --scope … --disclose-to-openai --consent-evidence …` (owner only, trusted-local Unix) |
| Completion | Phone posts code and device key | `st devices complete URL PAIRING_ID --assistant` on the adapter host |
| Credential store | iOS Keychain | `assistant-devices.json`, mode 0600, adapter host only |
| Lifetime | 30 days | 1 hour default, 24 hours maximum, explicit renew |
| Per request | Bearer + scope | Bearer + scope + one-use approval proof |
| Carrier | Tailscale, Serve HTTPS, LAN, Fabric | Secure MCP Tunnel to the adapter; adapter to `st3-client.sock` locally |
| Packaging | Built from source with the person's own Apple team | `st-chatgpt` binary from the st release; app record created by the person |
| Revocation | `st devices revoke` | `st devices grant revoke`; expiry; principal kill (#2541 slice D) |

## Install and use

Prerequisites for any person (written for a hypothetical `person/ada`):

- A running st member whose configured owning person is `person/ada`.
- A ChatGPT plan with developer-mode MCP apps. Pro allows read tools; writes need Business, Enterprise or Edu at
  the time of writing. ChatGPT reaches custom apps on the web only.
- An OpenAI Platform organization in which Ada can create a Secure MCP Tunnel and a key restricted to Tunnels
  Read + Use.
- An approval attestor (see open question 1).

Steps:

1. Install st as usual. The release contains `st-chatgpt`.
2. On the daemon host, Ada issues the grant over the trusted-local API:
   `st devices --as person/ada pair --assistant chatgpt --scope read.assistant.summary --scope read.assistant.attention --disclose-to-openai --consent-evidence <record>`.
3. On the same host, the adapter completes pairing: `st devices complete <gateway URL> <pairing ID> --assistant`
   (code on stdin).
4. Ada pins the attestor: `st devices grant read-trust <grant> --issuer … --key-id … --attestor-public-key …`.
5. Ada starts `st-chatgpt` and `tunnel-client` as user services. Daemon startup never starts them, as it never
   starts Tailscale Serve or Fabric for the phone.
6. In ChatGPT, Ada creates an MCP app with connection type Tunnel and selects her tunnel.
7. Use: "What needs me in st?" calls the alerts tool, ChatGPT shows the confirmation card, the attestor asks Ada to
   approve the read, and the tool returns the bounded projection with its snapshot and freshness.

Revocation and failure behave as with the phone: `st devices grant revoke` or expiry denies the next request;
an offline daemon or adapter returns an explicit unavailable result; a stale fence, used nonce or exceeded budget
returns its typed error and the adapter reports it without retrying.

Nothing in these steps is specific to one person, host or deployment. A downstream repository may package steps 3–5
as declarative services, but the binary, tool list and docs come from this repository.

### Out of scope for this design

- Public directory publication. It needs a stable public HTTPS MCP endpoint, an OAuth 2.1 authorization server
  whose tokens map to grants, domain verification and OpenAI review. The tunnel path cannot be submitted.
- Mobile and voice use; ChatGPT does not offer custom apps there.

## Open questions for review

1. **Attestor for installations without SCG.** #2541 names an SCG executor as the minter of read proofs. Other
   people do not run SCG. Proposal: keep the attestor a pinned role and add an st-native attestor in which the
   owner approves the request from an st alert on a paired signing device (iOS or stui). SCG stays one valid
   attestor.
2. **Answering alerts.** #2290/#2541 deny `prompt.respond` and `work.done` to assistants, and their metadata pages
   omit question text and options. Slice 1 of #2172 needs an authorized typed answer. Proposal: a dormant
   `assistant.alert.answer` action that binds the exact alert, episode, chosen option and fence into the owner's
   approval proof, in the same shape as the approved send in slice C.
3. **Binary placement.** Separate `st-chatgpt` binary (proposed) or an `st assistant serve` subcommand.
4. **Grant lifetime.** A 24-hour maximum means a daily owner renewal. Keep it, or allow a longer lifetime with
   per-request approval unchanged.
