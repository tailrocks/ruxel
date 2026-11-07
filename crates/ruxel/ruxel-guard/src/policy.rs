use crate::COMPILED_DENY;
use ipnet::IpNet;
use std::net::IpAddr;

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
        _ => ip,
    }
}

impl Policy {
    pub fn new(runtime_allow: &[String], runtime_deny: &[String]) -> Result<Self, PolicyError> {
        let parse = |s: &str| {
            s.parse::<IpNet>()
                .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
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
    pub fn compiled_entries() -> usize {
        COMPILED_DENY.len()
    }
    pub(crate) fn decide(&self, ip: IpAddr) -> Result<(), String> {
        let ip = normalize(ip);
        if let Some(net) = self.deny.iter().find(|net| net.contains(&ip)) {
            return Err(format!("deny {net}"));
        }
        if !self.allow.iter().any(|net| net.contains(&ip)) {
            return Err("not in allow list".into());
        }
        Ok(())
    }
}
