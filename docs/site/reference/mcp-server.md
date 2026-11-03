---
title: "Fractal MCP server"
description: "Install, pair, and connect an MCP client to Fractal's bounded assistant tools."
order: 6
---

# Fractal MCP server

The Fractal MCP server connects an MCP client to the daemon's paired-client gateway. It exposes a closed set of tools for alerts, missions, agents, conversations, and explicitly scoped writes. It does not expose daemon control or give an assistant person authority.

The Rust package is `fractal-chatgpt`; its executable is `st-chatgpt`. The supported assistant principal is `assistant/chatgpt`, including when a generic MCP client uses the adapter. Pairing requires the configured owning person's explicit consent to disclose assistant reads to OpenAI. Do not use a person credential for this server.

The [external-assistant contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/external-assistants.md) defines the authority and wire behavior. Remote access is a restricted dogfood path, not an authenticated public service.

## Install

Install and configure [Fractal](../getting-started/install.md) first. The daemon must be running, with an owning person configured and its paired-client gateway available. The adapter must run on the daemon's machine because it connects over a Unix socket.

Build the adapter from the repository root with Rust and the repository's build prerequisites:

```sh
cargo build --release -p fractal-chatgpt --bin st-chatgpt
```

The binary is `target/release/st-chatgpt`. Alternatively, obtain it from the repository's Nix package:

```sh
nix build github:compoundingtech/fractal#st-chatgpt
```

That binary is `result/bin/st-chatgpt`. Use a matching Fractal revision. The examples below assume the binary is on `PATH`.

## Pair and prepare a grant

Pairing and grant administration are explicit owner operations, not adapter startup behavior. Run administrative commands on the authoritative grant-issuer machine through its trusted-local Unix API. Replace `person/operator` with the configured owner; replace the uppercase placeholders with your own values. Do not paste pairing codes, profiles, or bearer tokens into chat or client configuration.

1. Record the owner's disclosure-consent decision. Begin a grant with every scope named explicitly. These four minimal read scopes are mandatory; the conversation scope is an explicit addition for the conversation tools:

   ```sh
   fractal devices --as person/operator pair --assistant assistant/chatgpt \
     --scope read.assistant.summary \
     --scope read.assistant.attention \
     --scope read.assistant.missions \
     --scope read.assistant.agents \
     --scope read.assistant.conversation \
     --expires-in 1h \
     --disclose-to-openai \
     --consent-evidence CONSENT_RECORD
   ```

   Nothing is granted implicitly. Omit the conversation scope if conversation access is not intended. Add `--scope write.assistant.answers` or `--scope write.assistant.messages` only when the owner explicitly authorizes that write capability. Read consent is not write authorization.

2. Before the single-use challenge expires (five minutes), complete it on the adapter machine. `MEMBER_ORIGIN` is the issuer gateway's HTTP or HTTPS origin, without a path. The code is read privately from stdin:

   ```sh
   fractal devices complete MEMBER_ORIGIN PAIRING_ID --assistant \
     --profile /PATH/TO/assistant-profile.json
   ```

   Replace `/PATH/TO/assistant-profile.json` with an absolute path in a directory owned by you and not writable by others; relative profile paths are rejected. Completion seals the original scopes, expiry, and owner; it cannot escalate them. The CLI stores an owner-only assistant profile separate from person profiles and reports the grant ID without printing its credential.

3. Prepare a private directory and extract only the new session's bearer into a separate credential file. The profile is JSON; it is **not** the adapter's credential-file format. For the explicitly selected profile above, use `jq` locally (never print the result):

   ```sh
   install -d -m 700 /PATH/TO/mcp-private
   (umask 077; jq -er '.devices[-1].session.credential' \
     /PATH/TO/assistant-profile.json > /PATH/TO/mcp-private/credential)
   chmod 600 /PATH/TO/mcp-private/credential
   ```

   Keep both files private. The adapter requires a regular credential file owned by its effective user, with no group/other permission bits, at most 4096 bytes, containing one nonempty ASCII bearer token. It refuses a final-component symlink. Use a new credential-file destination rather than overwriting an untrusted path.

4. Inspect the resulting grant on the issuer:

   ```sh
   fractal devices --as person/operator grant inspect GRANT_ID
   ```

CLI pairing selects `caller-bound-allow`: reads within the sealed scopes, consent, expiry, and byte budget do not need a separate read-trust or policy call. The proof-gated mode described in the engineering contract is a different mode, not an extra setup step for these CLI grants.

## Run with stdio

Use stdio when the MCP client launches a local process:

```sh
st-chatgpt --transport stdio \
  --socket /PATH/TO/st3-client.sock \
  --credential-file /PATH/TO/mcp-private/credential
```

Use the daemon's **paired-client** socket, whose filename must be `st3-client.sock`, not its control socket. All paths in the examples are placeholders, not defaults.

A generic MCP client configuration looks like this; adapt the outer configuration format to your client. Use absolute paths and keep the bearer out of the configuration:

```json
{
  "mcpServers": {
    "fractal": {
      "command": "/PATH/TO/st-chatgpt",
      "args": [
        "--transport", "stdio",
        "--socket", "/PATH/TO/st3-client.sock",
        "--credential-file", "/PATH/TO/mcp-private/credential"
      ]
    }
  }
}
```

## Run with loopback HTTP

HTTP is the default transport. The default listen address is `127.0.0.1:8787`:

```sh
st-chatgpt --transport http --listen 127.0.0.1:8787 \
  --socket /PATH/TO/st3-client.sock \
  --credential-file /PATH/TO/mcp-private/credential
```

Configure a local Streamable HTTP MCP client with `http://127.0.0.1:8787/mcp`. Only `POST /mcp` is a protocol endpoint; `GET /mcp` returns 405. The adapter rejects non-loopback listen addresses.

### ChatGPT through an operator-owned tunnel

The HTTP adapter has **no inbound authentication**. Its credential authenticates outbound requests to the daemon; it does not authenticate clients connecting to `/mcp`. Anyone who reaches the endpoint can exercise the adapter's grant.

For a deliberately authorized dogfood connection, the operator starts and owns a tunnel forwarding an HTTPS endpoint to `http://127.0.0.1:8787`. Keep the adapter bound to loopback. In a ChatGPT client with developer-mode MCP support, create an app using the tunnel's HTTPS URL with `/mcp` and the no-auth option. Client/plan availability is a prerequisite; this page does not promise support for every ChatGPT account.

The HTTP transport validates loopback host authorities to resist DNS rebinding. Configure the tunnel's upstream request to use a loopback `Host`, such as `127.0.0.1:8787`; forwarding the public hostname unchanged can be rejected. This validation is not client authentication.

No-auth tunnel access is an explicit dogfood exception, not a public-distribution setup. Treat the reachable endpoint as access to the grant, keep its lifetime short, and stop the tunnel and revoke the grant when finished. Do not place the daemon bearer in the tunnel URL or ChatGPT configuration. The adapter does not implement OAuth; [section E of the contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/external-assistants.md#e-remote-access-over-oauth) specifies it but is not an implementation or permission to expose a server.

## Tools

The table summarizes the descriptions published by the adapter. A scope listed here is in addition to the mandatory baseline read scopes. Discovery of a tool does not mean the grant authorizes it.

| Name | Required scope | Access | Purpose |
| --- | --- | --- | --- |
| `fractal_alerts_list` | `read.assistant.attention` | Read | List bounded owner alerts with actionable questions/options, episodes, targets, and evidence when present. |
| `fractal_alert_get` | `read.assistant.attention` | Read | Read one exact owner alert, including its episode and named answer options. |
| `fractal_alert_answer` | `write.assistant.answers` | Write | Execute one typed person-ask answer using the alert's current episode and fence. |
| `fractal_missions_list` | `read.assistant.missions` | Read | List mission metadata. |
| `fractal_agents_list` | `read.assistant.agents` | Read | List agent metadata, not priorities or conversation content. |
| `fractal_conversations_list` | `read.assistant.conversation` | Read | Discover source-bounded conversation metadata and read handles without conversation content. |
| `fractal_conversation_detail` | `read.assistant.conversation` | Read | Read one agent's current conversation with bounded chat excerpts and revision-pinned read handles. |
| `fractal_conversation_timeline` | `read.assistant.conversation` | Read | Page bounded chat excerpts and revision-pinned read handles. |
| `fractal_conversation_search` | `read.assistant.conversation` | Read | Search bounded plain conversation text snippets. |
| `fractal_conversation_read` | `read.assistant.conversation` | Read | Read one bounded revision-pinned conversation content range. |
| `fractal_message_send` | `write.assistant.messages` | Write | Send one exact message to any existing agent; person recipients are refused. |

The corresponding `st_*` tool names are hidden, deprecated aliases for cached clients and are removed in `fractal-chatgpt` 0.2.0. Refresh client tool discovery and use `fractal_*`; aliases do not grant additional authority. This deprecation concerns MCP tool names, not the `st-chatgpt` executable name.

## Limits and grant lifecycle

- **Bounded responses:** `max_response_bytes` covers the entire JSON envelope, not just text and not a token count. The default is 4096 bytes and the maximum is 16384 bytes; configured policy may lower either. A response that cannot fit fails with `response-budget-exceeded` (413), rather than silently fetching an entire body.
- **Paging:** list reads return bounded whole-item pages and `value.page.next_cursor`. Pass the cursor back to the same tool with the same filters; do not edit it. Byte packing can return fewer than `limit` items. Timeline/search pages default to 10 and accept at most 50. Cursors bind the grant, policy revision, and snapshot; restart from the first page on `page-cursor-expired`. An empty alerts page is not proof that no work exists.
- **Content handles:** copy a non-null item's `read` object unchanged into `fractal_conversation_read`. It contains `agent_id`, `session_id`, `reference`, `revision`, `start`, and `end`. A pagination cursor is not a content reference. A detail preview can continue through the timeline tool. Content reads do not stream or follow attachments.
- **Explicit writes:** read scopes never authorize answers or messages. Answers apply only to supported structured owner asks, not authored person-review steps. Writes use current fences and idempotency keys. Message authority covers any existing agent, not people; stored recipient metadata is not an enforced allowlist. A ChatGPT confirmation card is advisory, not a second daemon approval or the receiving agent's owning-person authorization.
- **Expiry:** grants default to one hour, must have a positive duration, and cannot exceed 30 days. There is no automatic renewal. Renew before expiry, preserving existing scopes and recording consent again:

  ```sh
  fractal devices --as person/operator grant renew GRANT_ID \
    --expires-in 1h --disclose-to-openai --consent-evidence CONSENT_RECORD
  ```

- **Policy and revocation:** `grant policy GRANT_ID` replaces scopes, expiry, and consent using the same explicit grant-term flags as pairing. Include the four mandatory reads and every other scope you intend to keep. Policy changes and renewal increment the policy revision, invalidating old policy-bound continuations. Revoke the exact grant on the issuer:

  ```sh
  fractal devices --as person/operator grant revoke GRANT_ID
  ```

  Requests recheck expiry and revocation. Expired or revoked grants cannot be revived; pair a fresh grant and replace the credential file. Restarting the adapter does not restore access. The adapter reads its credential at startup, so restart it after replacing the file.
- **Transport boundary:** HTTP listens only on loopback and has no inbound auth. OAuth is specified, not implemented. The server is not a general-purpose daemon proxy.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| HTTP 400 with a hand-written request | Send a valid `Host` header (ordinary HTTP clients supply it). Also use an MCP client that sends the required MCP protocol and content-negotiation headers; raw JSON without the MCP handshake is not a client configuration. |
| HTTP 405 in a browser | Use a Streamable HTTP MCP client with `POST /mcp`; a browser's GET is not a tool call. |
| Credential permission or parse error | Supply the bearer-only file, not the JSON profile. Check ownership, mode 0600, size, and that the file is not a symlink. Never print the token while diagnosing. |
| Grant expired or revoked | Inspect it through the issuer's local CLI. Pair a fresh grant, replace the private bearer file, and restart the adapter. An expired grant cannot be renewed. |
| Missing-scope denial | Inspect the exact grant. Replace policy explicitly if the owner authorizes the scope; do not substitute a person credential. |
| `response-budget-exceeded` | Request a smaller page or content range, or raise the requested byte budget within the policy maximum. A single oversized item may still fail. |
| `page-cursor-expired` or stale content revision | Restart discovery and use newly returned cursors/read handles. Do not manufacture a cursor or reference. |
| Socket rejected or unavailable | Use the current daemon's `st3-client.sock`, run the adapter on that machine, and check local ownership/access. A control socket is intentionally rejected. |
| Tunnel connects but tools are unavailable | Check the loopback adapter and grant first, then the operator-owned tunnel's forwarding to `/mcp` and the client's MCP support. Do not expose a non-loopback listener to bypass the problem. |

For authority, consent, proof-gated reads, write semantics, and remote-access requirements, read the [external-assistant engineering contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/external-assistants.md).
