use ruxel_proto::v1::Facts;

pub(crate) fn gather() -> Facts {
    Facts {
        default_ipv4_interface: default_ipv4_interface().unwrap_or_default(),
        distribution_release: distribution_release().unwrap_or_default(),
        architecture: std::env::consts::ARCH.to_string(),
        hostname: hostname().unwrap_or_default(),
    }
}

fn default_ipv4_interface() -> Option<String> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in table.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let iface = fields.next()?;
        let dest = fields.next()?;
        if dest == "00000000" {
            return Some(iface.to_string());
        }
    }
    None
}

fn distribution_release() -> Option<String> {
    let os_release = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in os_release.lines() {
        if let Some(value) = line.strip_prefix("VERSION_CODENAME=") {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn hostname() -> Option<String> {
    for path in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(name) = std::fs::read_to_string(path) {
            let name = name.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}
