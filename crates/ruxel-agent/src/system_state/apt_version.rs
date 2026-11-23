//! Debian version ordering, as implemented by dpkg's `verrevcmp`, so that
//! equivalent spellings such as `1.01` and `1.1` compare equal.

use std::cmp::Ordering;

/// Compare two Debian versions (`[epoch:]upstream[-revision]`).
pub(super) fn compare(a: &str, b: &str) -> Ordering {
    let (a_epoch, a_upstream, a_revision) = split(a);
    let (b_epoch, b_upstream, b_revision) = split(b);
    a_epoch
        .cmp(&b_epoch)
        .then_with(|| verrevcmp(a_upstream, b_upstream))
        .then_with(|| verrevcmp(a_revision, b_revision))
}

pub(super) fn equal(a: &str, b: &str) -> bool {
    compare(a, b) == Ordering::Equal
}

fn split(version: &str) -> (u64, &str, &str) {
    let (epoch, rest) = match version.split_once(':') {
        Some((epoch, rest)) => (epoch.parse().unwrap_or(0), rest),
        None => (0, version),
    };
    match rest.rsplit_once('-') {
        Some((upstream, revision)) => (epoch, upstream, revision),
        None => (epoch, rest, ""),
    }
}

/// Sort weight of a non-digit character: `~` sorts before everything, even
/// the end of the string; letters sort before other characters.
fn order(c: Option<u8>) -> i32 {
    match c {
        None => 0,
        Some(b'~') => -1,
        Some(c) if c.is_ascii_digit() => 0,
        Some(c) if c.is_ascii_alphabetic() => i32::from(c),
        Some(c) => i32::from(c) + 256,
    }
}

fn verrevcmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let ac = order(a.get(i).copied().filter(|c| !c.is_ascii_digit()));
            let bc = order(b.get(j).copied().filter(|c| !c.is_ascii_digit()));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while a.get(i) == Some(&b'0') {
            i += 1;
        }
        while b.get(j) == Some(&b'0') {
            j += 1;
        }
        let mut first_diff = Ordering::Equal;
        while a.get(i).is_some_and(u8::is_ascii_digit) && b.get(j).is_some_and(u8::is_ascii_digit) {
            if first_diff == Ordering::Equal {
                first_diff = a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
        if a.get(i).is_some_and(u8::is_ascii_digit) {
            return Ordering::Greater;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            return Ordering::Less;
        }
        if first_diff != Ordering::Equal {
            return first_diff;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::compare;
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn matches_dpkg_ordering() {
        for (a, b, expected) in [
            ("1.01", "1.1", Equal),
            ("1.0", "1.0-0", Equal),
            ("0:1.2-3", "1.2-3", Equal),
            ("1.2-3", "1.2-03", Equal),
            ("1.2-3", "1.2-4", Less),
            ("1:1.0", "2.0", Greater),
            ("1.0~rc1", "1.0", Less),
            ("1.0~~", "1.0~", Less),
            ("1.0", "1.0+b1", Less),
            ("1.0a", "1.0", Greater),
            ("1.0a", "1.0+", Less),
            ("2.10", "2.9", Greater),
            ("5.2.21-2ubuntu4", "5.2.21-2ubuntu4", Equal),
            ("5.2.21-2ubuntu4", "5.2.21-2ubuntu4.1", Less),
        ] {
            assert_eq!(compare(a, b), expected, "{a} vs {b}");
            assert_eq!(compare(b, a), expected.reverse(), "{b} vs {a}");
        }
    }
}
