//! Native `st3-client-v0` transport through the local Fabric daemon.

use std::{fmt, path::PathBuf, time::Duration};

use smallclaims::fleet::transport::Fabric;

use crate::ClientError;

const PROTOCOL: &str = "st3-client-v0";
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// A peer's native client protocol, never a proxy URL or arbitrary Fabric protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FabricEndpoint {
    target: String,
    #[cfg(test)]
    program: Option<PathBuf>,
}

impl fmt::Display for FabricEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fabric://{}/{PROTOCOL}", self.target)
    }
}

impl FabricEndpoint {
    pub fn parse(value: &str) -> Result<Self, ClientError> {
        let invalid = || ClientError::Protocol(format!("invalid Fabric client endpoint; expected fabric://<NodeID-or-name>/{PROTOCOL}"));
        let rest = value.strip_prefix("fabric://").ok_or_else(invalid)?;
        let (target, protocol) = rest.split_once('/').ok_or_else(invalid)?;
        if protocol != PROTOCOL
            || target.is_empty()
            || !target.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
            })
            || target == "."
            || target == ".."
        {
            return Err(invalid());
        }
        Ok(Self {
            target: target.to_owned(),
            #[cfg(test)]
            program: None,
        })
    }

    /// Restrict every request (including upgrades) before dialing the peer.
    pub fn validate_path(path: &str) -> Result<(), ClientError> {
        let invalid = || ClientError::Protocol(format!("path is not allowed over Fabric: {path}"));
        if path
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\' || byte == b'#')
        {
            return Err(invalid());
        }
        let route = path.split_once('?').map_or(path, |(route, _)| route);
        if !(route.starts_with("/v1/client/") || route == "/v1/health") {
            return Err(invalid());
        }
        // Decode exactly once. Encoded percent would allow a second decoder to
        // introduce traversal; ordinary encoded subject IDs and filenames are valid.
        let bytes = route.as_bytes();
        let decoded;
        let normalized = if bytes.contains(&b'%') {
            let mut buffer = Vec::with_capacity(bytes.len());
            let mut index = 0;
            while index < bytes.len() {
                let byte = if bytes[index] == b'%' {
                    let hex = bytes.get(index + 1..index + 3).ok_or_else(invalid)?;
                    let hex = std::str::from_utf8(hex).map_err(|_| invalid())?;
                    let byte = u8::from_str_radix(hex, 16).map_err(|_| invalid())?;
                    if byte.is_ascii_control() || matches!(byte, b'\\' | b'%' | b'?' | b'#') {
                        return Err(invalid());
                    }
                    index += 3;
                    byte
                } else {
                    let byte = bytes[index];
                    index += 1;
                    byte
                };
                buffer.push(byte);
            }
            decoded = buffer;
            decoded.as_slice()
        } else {
            bytes
        };
        if !(normalized.starts_with(b"/v1/client/") || normalized == b"/v1/health")
            || normalized
                .split(|byte| *byte == b'/')
                .any(|part| part == b"." || part == b"..")
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub async fn dial(&self) -> Result<PathBuf, ClientError> {
        #[cfg(test)]
        if let Some(program) = &self.program {
            return self.dial_with(&Fabric::new(program.clone())).await;
        }
        self.dial_with(&Fabric::new(PathBuf::from("fabric"))).await
    }

    async fn dial_with(&self, fabric: &Fabric) -> Result<PathBuf, ClientError> {
        tokio::time::timeout(DIAL_TIMEOUT, async {
            let node = if let Ok(node) = self.target.parse::<iroh_base::EndpointId>() {
                node.to_string()
            } else {
                let peers = fabric.peers().await.map_err(|error| {
                    ClientError::Transport(format!(
                        "resolve Fabric peer {}: {error:#}",
                        self.target
                    ))
                })?;
                resolve_peer(&self.target, &peers)?.to_owned()
            };
            fabric
                .dial_socket(&node, PROTOCOL)
                .await
                .map_err(|error| ClientError::Transport(format!("dial {self}: {error:#}")))
        })
        .await
        .map_err(|_| ClientError::Transport(format!("dial {self}: Fabric deadline exceeded")))?
    }
}

fn resolve_peer<'a>(target: &str, peers: &'a [(String, String)]) -> Result<&'a str, ClientError> {
    if let Some((node, _)) = peers.iter().find(|(node, _)| node == target) {
        return Ok(node);
    }
    let mut matching = peers
        .iter()
        .filter(|(_, name)| name.eq_ignore_ascii_case(target));
    match (matching.next(), matching.next()) {
        (Some((node, _)), None) => Ok(node),
        (None, _) => Err(ClientError::Protocol(format!(
            "no trusted Fabric peer named `{target}`"
        ))),
        _ => Err(ClientError::Protocol(format!(
            "ambiguous trusted Fabric peer name `{target}`; use a NodeID"
        ))),
    }
}

#[cfg(test)]
mod tests {
    const NODE_ID: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const NODE_BASE32: &str = "25NJQAMCWEFLPVKL73J4SZAHHIHOC4XT3KTCGJNPAINGR5YHKENA";
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn parses_only_native_client_endpoints() {
        for target in ["DEVBOX", "dev-box.local", &"a".repeat(64)] {
            let url = format!("fabric://{target}/st3-client-v0");
            assert_eq!(FabricEndpoint::parse(&url).unwrap().to_string(), url);
        }
        for value in ["fabric:box/st3-client-v0", "fabric:///st3-client-v0", "fabric://box/st3/pty/x", "fabric://box/st3-client-v0/", "fabric://box/st3-client-v0?x", "fabric://user@box/st3-client-v0", "fabric://box:1/st3-client-v0", "fabric://%62ox/st3-client-v0"] {
            assert!(FabricEndpoint::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn rejected_endpoints_never_echo_credentials() {
        for value in [
            "fabric://secret-user:secret-password@box/st3-client-v0",
            "fabric://box/st3-client-v0?token=secret-token",
            "fabric://box/st3-client-v0#secret-fragment",
            "fabric://box/secret-protocol",
        ] {
            let error = FabricEndpoint::parse(value).unwrap_err().to_string();
            assert!(!error.contains("secret"));
            assert!(!error.contains(value));
            assert!(error.contains("expected fabric://<NodeID-or-name>/st3-client-v0"));
        }
    }

    #[test]
    fn restricts_paths_before_dial() {
        for path in ["/v1/client/agents", "/v1/client/terminals/a/stream?after=a%2Fb", "/v1/health", "/v1/client/agents/agent%2Fdemo", "/v1/client/documents/file%2Emd"] {
            FabricEndpoint::validate_path(path).unwrap();
        }
        for path in ["/v1/clients/agents", "/v1/healthz", "/v1/health/live", "/v1/admin/stop", "//v1/client/agents", "/v1/client/../admin", "/v1/client/%2e%2e/admin", "/v1/client/%252e%252e/admin", "/v1/client/a%2f..", "/v1/client/a\\..", "/v1/client/a#fragment", "/v1/client/%00", "/v1/client/%zz"] {
            assert!(FabricEndpoint::validate_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn resolves_unique_trusted_names_and_exact_nodes() {
        let peers = vec![("node-a".into(), "Box".into()), ("node-b".into(), "Other".into())];
        assert_eq!(resolve_peer("bOX", &peers).unwrap(), "node-a");
        assert_eq!(resolve_peer("node-b", &peers).unwrap(), "node-b");
        assert!(resolve_peer("missing", &peers).is_err());
        let ambiguous = vec![("node-a".into(), "Box".into()), ("node-b".into(), "BOX".into())];
        assert!(resolve_peer("box", &ambiguous).is_err());
    }

    fn fake_fabric(root: &std::path::Path, body: &str) -> Fabric {
        let program = root.join("fabric");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        Fabric::new(program)
    }

    #[tokio::test]
    async fn fake_fabric_native_http_roundtrip_and_refusal() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("client.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let _fabric = fake_fabric(root.path(), &format!("case \"$*\" in\n'peers --json') echo '{{\"peers\":[{{\"node\":\"node-a\",\"name\":\"Box\"}}]}}';;\n'dial node-a st3-client-v0 --json') echo '{{\"socket\":\"{}\"}}';;\n*) exit 1;;\nesac", socket.display()));
        let mut endpoint = FabricEndpoint::parse("fabric://BOX/st3-client-v0").unwrap();
        endpoint.program = Some(root.path().join("fabric"));
        let client = crate::Client::fabric(endpoint).with_credential("paired-test");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /v1/health HTTP/1.1\r\n"));
            assert!(String::from_utf8_lossy(&request).to_ascii_lowercase().contains("authorization: bearer paired-test\r\n"));
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}").await.unwrap();
        });
        let result: serde_json::Value = client.request(hyper::Method::GET, "/v1/health", None).await.unwrap();
        assert_eq!(result["ok"], true);
        server.await.unwrap();
        let refusal_root = tempfile::tempdir().unwrap();
        let refusal = fake_fabric(refusal_root.path(), "echo '{\"error\":{\"code\":\"grant_denied\",\"class\":\"refusal\",\"message\":\"not granted\"}}'; exit 1");
        let endpoint = FabricEndpoint::parse(&format!("fabric://{NODE_ID}/st3-client-v0")).unwrap();
        assert!(endpoint.dial_with(&refusal).await.is_err());
        let mut endpoint = endpoint;
        endpoint.program = Some(refusal_root.path().join("fabric"));
        let client = crate::Client::fabric(endpoint);
        let error = client.request::<serde_json::Value>(hyper::Method::GET, "/v1/health", None).await.unwrap_err();
        assert!(matches!(error, ClientError::Transport(_)));
        assert!(client.collection_stream().await.is_err());
        assert!(client.conversation_stream("test", None).await.is_err());
        assert!(client.terminal_stream("test", None, "capability").await.is_err());
        assert!(client.open_raw_terminal("test", "incarnation", crate::RawTerminalMode::Peek, None).await.is_err());
        // Even when the Fabric executable refuses, disallowed routes fail locally first.
        let error = client.request::<serde_json::Value>(hyper::Method::GET, "/v1/admin/stop", None).await.unwrap_err();
        assert!(matches!(error, ClientError::Protocol(_)));
        assert!(matches!(client.conversation_stream("..", None).await, Err(ClientError::Protocol(_))));
    }

    #[tokio::test]
    async fn node_ids_use_fabrics_exact_parser_without_listing_peers() {
        let root = tempfile::tempdir().unwrap();
        let fabric = fake_fabric(root.path(), &format!(
            "case \"$*\" in\n'dial {NODE_ID} st3-client-v0 --json') echo '{{\"socket\":\"/native-tunnel.sock\"}}';;\n*) exit 1;;\nesac"
        ));
        for target in [NODE_ID.to_owned(), NODE_BASE32.to_owned(), NODE_BASE32.to_ascii_lowercase()] {
            let endpoint = FabricEndpoint::parse(&format!("fabric://{target}/st3-client-v0")).unwrap();
            assert_eq!(endpoint.dial_with(&fabric).await.unwrap(), PathBuf::from("/native-tunnel.sock"));
        }
        // Uppercase hex and noncanonical base32 trailing bits are not direct IDs:
        // Fabric itself would treat them as peer names.
        for target in [NODE_ID.to_ascii_uppercase(), format!("{}B", &NODE_BASE32[..51])] {
            assert!(target.parse::<iroh_base::EndpointId>().is_err());
        }
    }

    struct StreamHandshake {
        path: &'static str,
        protocol: &'static str,
    }

    // The handshake trait fixes the rejection type to an HTTP response. Implement
    // that contract directly rather than giving an infallible closure a large Err.
    impl tokio_tungstenite::tungstenite::handshake::server::Callback for StreamHandshake {
        fn on_request(
            self,
            request: &tokio_tungstenite::tungstenite::handshake::server::Request,
            mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
        ) -> Result<
            tokio_tungstenite::tungstenite::handshake::server::Response,
            tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
        > {
            assert_eq!(request.uri().path(), self.path);
            assert_eq!(request.headers()["authorization"], "Bearer paired-test");
            response.headers_mut().insert(
                hyper::header::SEC_WEBSOCKET_PROTOCOL,
                hyper::header::HeaderValue::from_static(self.protocol),
            );
            Ok(response)
        }
    }

    #[tokio::test]
    async fn fake_fabric_upgrades_every_stream_transport() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("streams.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let _fabric = fake_fabric(root.path(), &format!("case \"$1\" in\npeers) echo '{{\"peers\":[{{\"node\":\"node-a\",\"name\":\"Box\"}}]}}';;\ndial) echo '{{\"socket\":\"{}\"}}';;\n*) exit 1;;\nesac", socket.display()));
        let mut endpoint = FabricEndpoint::parse("fabric://box/st3-client-v0").unwrap();
        endpoint.program = Some(root.path().join("fabric"));
        let client = crate::Client::fabric(endpoint).with_credential("paired-test");
        let server = tokio::spawn(async move {
            let expected = [
                ("/v1/client/collections/stream", "st3.client.collections.v0"),
                ("/v1/client/conversations/test/stream", "st3.client.conversation.v0"),
                ("/v1/client/terminals/test/stream", "st3.client.terminal.v0"),
                ("/v1/client/terminals/test/raw-stream", "st3.client.pty.v0"),
            ];
            for (path, protocol) in expected {
                let (stream, _) = listener.accept().await.unwrap();
                let websocket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    StreamHandshake { path, protocol },
                ).await.unwrap();
                drop(websocket);
            }
        });
        drop(client.collection_stream().await.unwrap());
        drop(client.conversation_stream("test", None).await.unwrap());
        drop(client.terminal_stream("test", None, "capability").await.unwrap());
        drop(client.open_raw_terminal("test", "incarnation", crate::RawTerminalMode::Peek, None).await.unwrap());
        server.await.unwrap();
    }
}
