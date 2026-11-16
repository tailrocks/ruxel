//! The rule an operation names: the arguments of the rule, and the match
//! they make.

use super::super::str_param;
use serde_json::Value;

/// The arguments of the rule: the match and the jump, and the comment when
/// the rule carries one.
///
/// The destination is checked here: it must be an address the firewall
/// can read, with a prefix the family holds. A prefix that is not a
/// number read as a host would name a rule the author never wrote, and a
/// prefix past the end of the family would underflow the mask.
pub(super) fn rule_spec(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    let mut spec = Vec::new();
    for (flag, value) in [
        ("-p", str_param(obj, "protocol")),
        ("-d", str_param(obj, "destination")),
        ("-o", str_param(obj, "out_interface")),
        ("-j", str_param(obj, "jump")),
    ] {
        if let Some(value) = value {
            if flag == "-d" {
                full_mask(value)?;
                validate_family(value, str_param(obj, "ip_version"))?;
            }
            spec.extend([flag.into(), value.into()]);
        }
    }
    if let Some(comment) = str_param(obj, "comment") {
        spec.extend([
            "-m".into(),
            "comment".into(),
            "--comment".into(),
            comment.into(),
        ]);
    }
    Ok(spec)
}

/// The match of a rule: every option but the jump and the comment, which
/// are not part of the match. An option the rule carries and the declared
/// match does not is a different match, so a rule with `--dport 22` is not
/// the rule that declares only a protocol.
pub(super) fn match_pairs(args: &[String]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1);
        if matches!(flag.as_str(), "-j" | "--jump") {
            // The target begins here: the options the target carries
            // (`--reject-with` and the like) belong to the target, and
            // nothing after it is part of the match.
            break;
        }
        if flag == "--comment" {
            // The comment is not part of the match.
            index += 2;
            continue;
        }
        if flag == "-m" && value.map(String::as_str) == Some("comment") {
            // The comment carries its own options.
            index += 4;
            continue;
        }
        if let Some(value) = value {
            // `-p all` is the default the listing leaves out, a host
            // without a prefix carries its full mask in the listing, and
            // the universal network is the destination the listing leaves
            // out altogether.
            if flag != "-p" || value != "all" {
                let value = if flag == "-d" {
                    printed_destination(value)
                } else {
                    value.clone()
                };
                if !(flag == "-d" && universal(&value)) {
                    pairs.push((flag.clone(), value));
                }
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    pairs.sort();
    pairs
}

/// The jump of a rule: the target it hands the packet to. It is not part
/// of the match, but it is what a stale rule shadows the declared one
/// with.
pub(super) fn rule_jump(args: &[String]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if matches!(args[index].as_str(), "-j" | "--jump") {
            return args.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}

/// The comment of a rule: the text the firewall prints after `--comment`.
/// It is not part of the match, but it is part of what the rule says.
pub(super) fn rule_comment(args: &[String]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--comment" {
            return args.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}

/// The arguments of a listed rule, without the chain it belongs to. The
/// listing is in the form of `iptables-save`, so a value with whitespace
/// in it is quoted and stays one argument.
pub(super) fn rule_arguments(rule: &str, chain: &str) -> Option<Vec<String>> {
    let rest = rule.strip_prefix(&format!("-A {chain} "))?;
    Some(split_arguments(rest))
}

/// The arguments of a command line, with the quoted values kept whole.
fn split_arguments(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            argument.push(character);
            escaped = false;
            started = true;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => {
                quoted = !quoted;
                started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut argument));
                    started = false;
                }
            }
            character => {
                argument.push(character);
                started = true;
            }
        }
    }
    if started {
        arguments.push(argument);
    }
    arguments
}

/// The destination in the form the firewall prints it: a host without a
/// prefix carries its full mask, and the host bits of a prefix are masked
/// off, the way the firewall masks them.
///
/// The destination must be an address of the family with a prefix the
/// family holds: a prefix that is not a number would be read as a host
/// address, and a prefix past the end of the family would underflow the
/// mask.
pub(super) fn full_mask(destination: &str) -> Result<String, String> {
    let (address, prefix) = match destination.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (destination, None),
    };
    // An IPv6 address may carry the zone it belongs to.
    let (bare, zone) = match address.split_once('%') {
        Some((bare, zone)) => (bare, Some(zone)),
        None => (address, None),
    };
    if let Ok(bits) = bare.parse::<std::net::Ipv4Addr>() {
        let prefix = prefix_bits(prefix, 32, destination)?;
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(prefix))
        };
        return Ok(format!(
            "{}/{}",
            std::net::Ipv4Addr::from(u32::from(bits) & mask),
            prefix
        ));
    }
    if let Ok(bits) = bare.parse::<std::net::Ipv6Addr>() {
        let prefix = prefix_bits(prefix, 128, destination)?;
        if let Some(zone) = zone {
            // The zone belongs to the address, not to the network, so a
            // zoned address keeps its host bits.
            return Ok(format!("{bare}%{zone}/{prefix}"));
        }
        let mask = if prefix == 0 {
            0
        } else {
            u128::MAX << (128 - u32::from(prefix))
        };
        return Ok(format!(
            "{}/{}",
            std::net::Ipv6Addr::from(u128::from(bits) & mask),
            prefix
        ));
    }
    Err(malformed(destination))
}

/// The destination must belong to the family of the rule's `ip_version`:
/// `iptables` holds the IPv4 rules and `ip6tables` the IPv6 ones. Without
/// this, an absent IPv4 rule declaring the IPv6 wildcard `::/0` would
/// reach `match_pairs`, erase the wildcard as the omitted destination and
/// delete the valid no-destination rule of the selected firewall.
fn validate_family(destination: &str, ip_version: Option<&str>) -> Result<(), String> {
    let selected = match ip_version {
        None | Some("ipv4") => "ipv4",
        Some("ipv6") => "ipv6",
        Some(other) => return Err(format!("iptables: ip_version {other:?} invalid")),
    };
    let address = destination.split('/').next().unwrap_or(destination);
    let bare = address.split('%').next().unwrap_or(address);
    // `full_mask` has already proved the address parses as one of the two
    // families.
    let found = if bare.parse::<std::net::Ipv4Addr>().is_ok() {
        "ipv4"
    } else {
        "ipv6"
    };
    if found == selected {
        return Ok(());
    }
    Err(format!(
        "iptables: destination {destination:?} is an {} address, but the rule is an {selected} rule; iptables administers the IPv4 rules and ip6tables the IPv6 ones",
        if found == "ipv4" { "IPv4" } else { "IPv6" }
    ))
}

/// The prefix of a destination: the bits the family holds, or the whole
/// address when the destination names none.
fn prefix_bits(prefix: Option<&str>, family: u8, destination: &str) -> Result<u8, String> {
    match prefix {
        None => Ok(family),
        Some(prefix) => match prefix.parse::<u8>() {
            Ok(bits) if bits <= family => Ok(bits),
            _ => Err(malformed(destination)),
        },
    }
}

/// The destination as the firewall printed it. A destination the firewall
/// printed is one it understands; one it did not is kept as it is.
fn printed_destination(destination: &str) -> String {
    full_mask(destination).unwrap_or_else(|_| destination.to_owned())
}

/// Whether the destination is the network that matches every address: the
/// firewall leaves it out of the listing, so it is the omitted
/// destination.
fn universal(destination: &str) -> bool {
    destination == "0.0.0.0/0" || destination == "::/0"
}

/// The destination the firewall will not read.
fn malformed(destination: &str) -> String {
    format!(
        "iptables: destination {destination:?} is not an address with a prefix the family holds"
    )
}
