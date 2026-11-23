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
    ] {
        if let Some(value) = value {
            if flag == "-d" {
                full_mask(value)?;
                validate_family(value, str_param(obj, "ip_version"))?;
            }
            spec.extend([flag.into(), value.into()]);
        }
    }
    // The firewall prints the match info before the target, so the
    // declared comment stands before the jump too: a comment past the
    // jump would read as target options, and every listed rule would
    // look stale next to it.
    if let Some(comment) = str_param(obj, "comment") {
        spec.extend([
            "-m".into(),
            "comment".into(),
            "--comment".into(),
            comment.into(),
        ]);
    }
    if let Some(jump) = str_param(obj, "jump") {
        spec.extend(["-j".into(), jump.into()]);
    }
    Ok(spec)
}

/// The protocol in the form the firewall prints it: `ip6tables` reads
/// `icmpv6` as protocol 58 but prints the first name of that number,
/// `ipv6-icmp`, so the declared spelling and the listed one meet here
/// before they are compared.
fn canonical_protocol(protocol: &str) -> &str {
    if protocol == "icmpv6" {
        "ipv6-icmp"
    } else if protocol == "0" {
        // Protocol 0 is the wildcard the firewall omits from the
        // listing, the way it omits `all`.
        "all"
    } else {
        // The firewall prints the first name of a protocol number, so
        // a declared number meets the listed name here.
        match protocol {
            "1" => "icmp",
            "2" => "igmp",
            "6" => "tcp",
            "17" => "udp",
            "47" => "gre",
            "50" => "esp",
            "51" => "ah",
            "58" => "ipv6-icmp",
            "132" => "sctp",
            _ => protocol,
        }
    }
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
        if matches!(flag.as_str(), "-j" | "--jump" | "-g" | "--goto") {
            // The target and its options are not part of the match:
            // they say what happens to the packet, not which packets
            // match. Everything past the jump is the target's own. A
            // goto hands the packet to its chain the same way a jump
            // does, so it ends the match too.
            break;
        }
        if flag.as_str() == "--comment" {
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
            // `-p all` is the default the listing leaves out, and so is
            // `-p 0`: protocol 0 is the wildcard spelled as a number. A
            // host without a prefix carries its full mask in the
            // listing, and the universal network is the destination the
            // listing leaves out altogether.
            if flag != "-p" || canonical_protocol(value) != "all" {
                let value = if flag == "-p" {
                    canonical_protocol(value).to_owned()
                } else if flag == "-d" {
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
/// with. A `--goto` hands the packet to its chain the same way, so it is
/// the jump of the rule too.
///
/// The scan walks option boundaries: a comment value is the value of
/// `--comment`, whatever it spells (`--comment -j` names the comment
/// `-j`, not a jump), and a module name is the value of `-m`.
pub(super) fn rule_jump(args: &[String]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if matches!(args[index].as_str(), "-j" | "--jump" | "-g" | "--goto") {
            return args.get(index + 1).cloned();
        }
        index = step_option(args, index);
    }
    None
}

/// The comment of a rule: the text the firewall prints after `--comment`.
/// It is not part of the match, but it is part of what the rule says.
///
/// The scan walks option boundaries like the jump does: a value that
/// spells `--comment` (a jump target never does) is not the comment.
pub(super) fn rule_comment(args: &[String]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--comment" {
            return args.get(index + 1).cloned();
        }
        index = step_option(args, index);
    }
    None
}

/// The target options of a rule: the options of the target past its name
/// (`--reject-with` and friends). They say what the target does, not
/// which packets match.
pub(super) fn rule_target_options(args: &[String]) -> Vec<String> {
    let mut index = 0;
    while index < args.len() {
        if matches!(args[index].as_str(), "-j" | "--jump" | "-g" | "--goto") {
            return args.iter().skip(index + 2).cloned().collect();
        }
        index = step_option(args, index);
    }
    Vec::new()
}

/// The next token past an option and the value it takes: `--comment`
/// takes its text, `-m` takes its module name, and a jump or a goto
/// takes its target. Anything else is stepped over one token at a time,
/// so an unknown flag never swallows the token behind it.
fn step_option(args: &[String], index: usize) -> usize {
    if args[index] == "--comment" {
        return index + 2;
    }
    if args[index] == "-m" {
        return index + 2;
    }
    if matches!(args[index].as_str(), "-j" | "--jump" | "-g" | "--goto") {
        return index + 2;
    }
    index + 1
}

/// The arguments of a listed rule, without the chain it belongs to. The
/// listing is in the form of `iptables-save`, so a value with whitespace
/// in it is quoted and stays one argument.
pub(super) fn rule_arguments(rule: &str, chain: &str) -> Option<Vec<String>> {
    // A rule with no arguments is listed as exactly `-A CHAIN`, with no
    // trailing space: it is the empty rule, not a line to skip.
    if rule == format!("-A {chain}") {
        return Some(Vec::new());
    }
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
    // An IPv6 address may carry the zone it belongs to. The zone is
    // dropped below: the firewall never prints it.
    let (bare, _zone) = match address.split_once('%') {
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
        // A zoned address keeps no zone: the zone names the interface
        // the address was seen on, and the firewall never prints it, so
        // a declared zone meets the listed network without one.
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
