use ipnet::IpNet;
use std::net::IpAddr;

pub(crate) fn parse(text: &str) -> Result<Vec<String>, String> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                return None;
            }
            Some(
                line.parse::<IpNet>()
                    .or_else(|_| line.parse::<IpAddr>().map(IpNet::from))
                    .map(|net| net.trunc().to_string())
                    .map_err(|_| format!("invalid CIDR on line {}", i + 1)),
            )
        })
        .collect()
}
