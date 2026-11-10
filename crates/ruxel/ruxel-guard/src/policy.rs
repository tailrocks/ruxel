use crate::COMPILED_DENY;
use ipnet::IpNet;
use std::net::IpAddr;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
#[error("invalid policy network: {0}")]
pub struct PolicyError(String);

#[derive(Debug)]
pub struct Policy {
    allow: Vec<IpNet>,
    pub(crate) deny: Vec<IpNet>,
}

pub(crate) fn normalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

pub(crate) fn mapped_network(net: IpNet) -> IpNet {
    match net {
        IpNet::V6(v6) if v6.prefix_len() >= 96 => {
            if let Some(v4) = v6.network().to_ipv4_mapped()
                && let Ok(v4net) = ipnet::Ipv4Net::new(v4, v6.prefix_len() - 96)
            {
                return IpNet::V4(v4net.trunc());
            }
            net
        }
        _ => net,
    }
}

pub(crate) fn intersects(a: IpNet, b: IpNet) -> bool {
    fn as_v6(net: IpNet) -> IpNet {
        match net {
            IpNet::V4(v4) => {
                ipnet::Ipv6Net::new(v4.network().to_ipv6_mapped(), 96 + v4.prefix_len())
                    .map_or(net, IpNet::V6)
            }
            IpNet::V6(_) => net,
        }
    }
    let (a, b) = (as_v6(a), as_v6(b));
    a.contains(&b.network()) || b.contains(&a.network())
}

impl Policy {
    /// Construct a policy with the union of the compiled and runtime denies.
    ///
    /// # Errors
    /// Returns an error if any address or network is invalid.
    pub fn new(runtime_allow: &[String], runtime_deny: &[String]) -> Result<Self, PolicyError> {
        let parse = |s: &str| {
            s.parse::<IpNet>()
                .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                .map(mapped_network)
                .map_err(|_| PolicyError(s.to_owned()))
        };
        Ok(Self {
            allow: runtime_allow
                .iter()
                .map(|s| parse(s))
                .collect::<Result<_, _>>()?,
            deny: COMPILED_DENY
                .iter()
                .copied()
                .chain(runtime_deny.iter().map(String::as_str))
                .map(parse)
                .collect::<Result<_, _>>()?,
        })
    }
    #[must_use]
    pub fn compiled_entries() -> usize {
        COMPILED_DENY.len()
    }
    /// Refuse an address covered by a deny entry, ignoring the allow list.
    /// IPv4-mapped IPv6 addresses match IPv4 entries.
    ///
    /// # Errors
    /// Returns the matching deny rule.
    pub fn check_deny(&self, ip: IpAddr) -> Result<(), String> {
        let ip = normalize(ip);
        match self
            .deny
            .iter()
            .find(|net| intersects(**net, IpNet::from(ip)))
        {
            Some(net) => Err(format!("deny {net}")),
            None => Ok(()),
        }
    }
    pub(crate) fn decide(&self, ip: IpAddr) -> Result<(), String> {
        self.check_deny(ip)?;
        let ip = normalize(ip);
        if !self.allow.iter().any(|net| net.contains(&ip)) {
            return Err("not in allow list".into());
        }
        Ok(())
    }
}
