use crate::{
    Policy,
    policy::{intersects, mapped_network},
};
use ipnet::IpNet;
use std::net::IpAddr;

#[must_use]
pub fn deny_hits(policy: &Policy, literals: &[&str]) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for literal in literals {
        for token in literal
            .split(|c: char| !c.is_ascii_hexdigit() && !matches!(c, ':' | '.' | '/'))
            .filter(|s| !s.is_empty())
        {
            let candidate = token
                .parse::<IpNet>()
                .ok()
                .or_else(|| token.parse::<IpAddr>().ok().map(IpNet::from))
                .or_else(|| {
                    if token.matches(':').count() == 1 && token.contains('.') {
                        token
                            .rsplit_once(':')
                            .and_then(|(ip, _)| ip.parse::<IpAddr>().ok())
                            .map(IpNet::from)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    token
                        .trim_end_matches(['.', ':'])
                        .parse::<IpAddr>()
                        .ok()
                        .map(IpNet::from)
                });
            if let Some(net) = candidate {
                let net = mapped_network(net);
                if net.prefix_len() < if net.addr().is_ipv4() { 8 } else { 16 } {
                    continue;
                }
                for deny in &policy.deny {
                    if intersects(*deny, net) {
                        hits.push(((*literal).to_owned(), format!("deny {deny}")));
                    }
                }
            }
        }
    }
    hits
}
