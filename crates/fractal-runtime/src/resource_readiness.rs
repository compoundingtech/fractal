//! Host operator attestations required before observations may lower declared seeds.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct HostReadiness {
    pub nix_per_build_cgroups: bool,
    pub io_accounting: bool,
    pub disk_measurement: bool,
}

impl HostReadiness {
    pub fn permits_below_seed(self) -> bool {
        self.nix_per_build_cgroups && self.io_accounting && self.disk_measurement
    }
}
