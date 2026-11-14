use super::NativeDeliveryPeer;

/// Classify the socket peer's native command using its process arguments and environment.
pub(super) fn identity(pid: u32, args: &[String], env: &[String]) -> Option<NativeDeliveryPeer> {
    let (transport, archives_inbox) = if args.get(1).map(String::as_str) == Some("driver-hook") {
        match args.get(2).map(String::as_str)? {
            "claude-observe" | "claude-statusline" => ("claude-hook", false),
            _ => return None,
        }
    } else {
        args.split(|arg| arg == "--").next()?.windows(2).find_map(|pair| {
            if pair[0] != "driver" {
                return None;
            }
            match pair[1].as_str() {
                "omp-channel" | "omp" => Some(("omp-channel", false)),
                "pi-channel" | "pi" => Some(("pi-channel", false)),
                "claude-mcp" => Some(("claude-channel", false)),
                "claude" => Some(("claude-channel", true)),
                "codex" => Some(("app-server", true)),
                "opencode" => Some(("opencode-server", true)),
                "exec" => Some(("exec", false)),
                _ => None,
            }
        })?
    };
    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
    let agent = fractal_drivers::contracts::env_with("ST_AGENT", &|name| {
        env.iter().find_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
    }).filter(|agent| agent.starts_with("agent/"))?;
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
    Some(NativeDeliveryPeer { agent, transport, pid, archives_inbox })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(args: &[&str], env: &[&str]) -> Option<NativeDeliveryPeer> {
        identity(37, &args.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>(),
            &env.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn native_writers_retain_transport_and_archive_behavior() {
        for (command, transport, archives) in [
            ("omp", "omp-channel", false),
            ("omp-channel", "omp-channel", false),
            ("pi", "pi-channel", false),
            ("pi-channel", "pi-channel", false),
            ("claude", "claude-channel", true),
            ("claude-mcp", "claude-channel", false),
            ("codex", "app-server", true),
            ("opencode", "opencode-server", true),
            ("exec", "exec", false),
        ] {
            let peer = peer(&["fractal", "driver", command], &["ST_AGENT=agent/seat"]).unwrap();
            assert_eq!(peer.agent, "agent/seat");
            assert_eq!(peer.transport, transport);
            assert_eq!(peer.archives_inbox, archives);
            assert_eq!(peer.pid, 37);
        }
    }

    #[test]
    fn claude_hooks_are_non_archiving_native_writers() {
        for hook in ["claude-observe", "claude-statusline"] {
            let peer = peer(&["fractal", "driver-hook", hook], &["ST_AGENT=agent/seat"]).unwrap();
            assert_eq!(peer.agent, "agent/seat");
            assert_eq!(peer.transport, "claude-hook");
            assert!(!peer.archives_inbox);
        }
        assert!(peer(&["fractal", "driver-hook", "unknown"], &["ST_AGENT=agent/seat"]).is_none());
        assert!(peer(&["fractal", "driver-hook", "claude-session-start"], &["ST_AGENT=agent/seat"]).is_none());
    }

    #[test]
    fn native_writer_identity_comes_only_from_the_peer_environment() {
        for args in [
            vec!["fractal", "driver-hook", "claude-observe"],
            vec!["fractal", "driver", "exec"],
        ] {
            for env in [
                vec![],
                vec!["ST_AGENT=person/owner"],
                vec!["ST_AGENT=agent/seat", "FRACTAL_AGENT=agent/other"],
                vec!["ST_AGENT=agent/seat", "FRACTAL_AGENT="],
            ] {
                assert!(peer(&args, &env).is_none());
            }
            assert_eq!(peer(&args, &["FRACTAL_AGENT=agent/seat"]).unwrap().agent, "agent/seat");
        }
        assert!(peer(&["fractal", "claim", "--actor", "agent/seat"], &["ST_AGENT=agent/seat"]).is_none());
        assert!(peer(&["fractal", "driver", "unknown", "--", "driver", "codex"], &["ST_AGENT=agent/seat"]).is_none());
    }
}
