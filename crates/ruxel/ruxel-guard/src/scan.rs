use crate::{
    Policy,
    policy::{intersects, mapped_network},
};
use ipnet::IpNet;
use std::net::{IpAddr, Ipv4Addr};

fn numeric_ipv4(value: &str) -> Option<IpAddr> {
    let parts: Vec<u32> = value
        .split('.')
        .map(|part| {
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16).ok()
            } else if part.len() > 1 && part.starts_with('0') {
                u32::from_str_radix(&part[1..], 8).ok()
            } else {
                part.parse().ok()
            }
        })
        .collect::<Option<_>>()?;
    let value = match parts.as_slice() {
        [a] => *a,
        [a, b] if *a <= 255 && *b <= 0x00ff_ffff => (a << 24) | b,
        [a, b, c] if *a <= 255 && *b <= 255 && *c <= 0xffff => (a << 24) | (b << 16) | c,
        [a, b, c, d] if *a <= 255 && *b <= 255 && *c <= 255 && *d <= 255 => {
            (a << 24) | (b << 16) | (c << 8) | d
        }
        _ => return None,
    };
    Some(IpAddr::V4(Ipv4Addr::from(value)))
}

#[must_use]
pub fn deny_hits(policy: &Policy, literals: &[&str]) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for literal in literals {
        for token in literal
            .split(|c: char| !c.is_ascii_hexdigit() && !matches!(c, ':' | '.' | '/' | 'x' | 'X'))
            .filter(|s| !s.is_empty())
        {
            let token = token
                .strip_prefix("://")
                .map_or(token, |url| url.split('/').next().unwrap_or(url));
            let candidate = token
                .parse::<IpNet>()
                .ok()
                .or_else(|| token.parse::<IpAddr>().ok().map(IpNet::from))
                .or_else(|| numeric_ipv4(token).map(IpNet::from))
                .or_else(|| {
                    if token.matches(':').count() == 1 {
                        token
                            .rsplit_once(':')
                            .and_then(|(ip, _)| {
                                ip.parse::<IpAddr>().ok().or_else(|| numeric_ipv4(ip))
                            })
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
