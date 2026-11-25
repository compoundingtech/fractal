use std::path::Path;

pub fn agent_binary() -> &'static Path {
    test_bin!("fractal-light-node-agent")
}
