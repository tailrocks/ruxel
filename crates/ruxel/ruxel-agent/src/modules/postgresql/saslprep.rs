// PostgreSQL unicode_norm.c, unicode_norm_table.h and saslprep.c, REL_18_STABLE commit
// 1da6ddbdc32c5bf640429828bc0353ffcb40628e.
// Copyright (c) 1996-2025, PostgreSQL Global Development Group.
// Copyright (c) 1994, The Regents of the University of California.
//
// Permission to use, copy, modify, and distribute this software and its
// documentation for any purpose, without fee, and without a written agreement
// is hereby granted, provided that the above copyright notice and this
// paragraph and the following two paragraphs appear in all copies.
//
// IN NO EVENT SHALL THE UNIVERSITY OF CALIFORNIA BE LIABLE TO ANY PARTY FOR
// DIRECT, INDIRECT, SPECIAL, INCIDENTAL, OR CONSEQUENTIAL DAMAGES, INCLUDING
// LOST PROFITS, ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS DOCUMENTATION,
// EVEN IF THE UNIVERSITY OF CALIFORNIA HAS BEEN ADVISED OF THE POSSIBILITY OF
// SUCH DAMAGE.
//
// THE UNIVERSITY OF CALIFORNIA SPECIFICALLY DISCLAIMS ANY WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
// AND FITNESS FOR A PARTICULAR PURPOSE. THE SOFTWARE PROVIDED HEREUNDER IS
// ON AN "AS IS" BASIS, AND THE UNIVERSITY OF CALIFORNIA HAS NO OBLIGATIONS TO
// PROVIDE MAINTENANCE, SUPPORT, UPDATES, ENHANCEMENTS, OR MODIFICATIONS.

#[path = "saslprep_data.rs"]
mod data;

#[path = "saslprep_codes.rs"]
mod codes;
#[path = "saslprep_compose.rs"]
mod compose_table;
#[path = "saslprep_decomp0.rs"]
mod decomp0;
#[path = "saslprep_decomp1.rs"]
mod decomp1;
#[path = "saslprep_decomp2.rs"]
mod decomp2;

use codes::CODES;
use compose_table::COMPOSE;

fn contains(ranges: &[u32], cp: u32) -> bool {
    ranges
        .as_chunks::<2>()
        .0
        .iter()
        .any(|r| (r[0]..=r[1]).contains(&cp))
}

fn entry(cp: u32) -> Option<(u32, u8, u8, u16)> {
    [decomp0::ENTRIES, decomp1::ENTRIES, decomp2::ENTRIES]
        .into_iter()
        .find_map(|entries| {
            entries
                .binary_search_by_key(&cp, |e| e.0)
                .ok()
                .map(|i| entries[i])
        })
}

fn decompose(cp: u32, out: &mut Vec<u32>) {
    if (0xac00..0xac00 + 11172).contains(&cp) {
        let index = cp - 0xac00;
        out.push(0x1100 + index / 588);
        out.push(0x1161 + (index % 588) / 28);
        if !index.is_multiple_of(28) {
            out.push(0x11a7 + index % 28);
        }
    } else if let Some((_, _, flags, index)) = entry(cp).filter(|e| e.2 & 31 != 0) {
        if flags & 64 != 0 {
            decompose(u32::from(index), out);
        } else {
            for &part in &CODES[usize::from(index)..usize::from(index) + usize::from(flags & 31)] {
                decompose(part, out);
            }
        }
    } else {
        out.push(cp);
    }
}

fn decomposition_size(cp: u32) -> Option<usize> {
    if (0xac00..0xac00 + 11172).contains(&cp) {
        return Some(if (cp - 0xac00).is_multiple_of(28) {
            2
        } else {
            3
        });
    }
    if let Some((_, _, flags, index)) = entry(cp).filter(|e| e.2 & 31 != 0) {
        if flags & 64 != 0 {
            return decomposition_size(u32::from(index));
        }
        return CODES[usize::from(index)..usize::from(index) + usize::from(flags & 31)]
            .iter()
            .try_fold(0usize, |sum, cp| sum.checked_add(decomposition_size(*cp)?));
    }
    Some(1)
}

fn compose(a: u32, b: u32) -> Option<u32> {
    if (0x1100..0x1113).contains(&a) && (0x1161..0x1176).contains(&b) {
        return Some(0xac00 + ((a - 0x1100) * 21 + b - 0x1161) * 28);
    }
    if (0xac00..0xac00 + 11172).contains(&a)
        && (a - 0xac00).is_multiple_of(28)
        && (0x11a8..0x11c3).contains(&b)
    {
        return Some(a + b - 0x11a7);
    }
    COMPOSE
        .binary_search_by_key(&(a, b), |e| (e.0, e.1))
        .ok()
        .map(|i| COMPOSE[i].2)
}

fn class(cp: u32) -> u8 {
    entry(cp).map_or(0, |e| e.1)
}

fn canonical_order(decomposed: &mut [u32]) -> Result<(), String> {
    let size = decomposed.len();
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(size)
        .map_err(|_| "SASLprep allocation failed")?;
    scratch.resize(size, 0);
    let mut start = 0;
    for end in 0..=size {
        if end == size || class(decomposed[end]) == 0 {
            if end > start + 1 {
                let mut counts = [0usize; 256];
                for &cp in &decomposed[start..end] {
                    counts[usize::from(class(cp))] += 1;
                }
                let mut offset = start;
                for count in &mut counts {
                    let len = *count;
                    *count = offset;
                    offset += len;
                }
                for &cp in &decomposed[start..end] {
                    let pos = &mut counts[usize::from(class(cp))];
                    scratch[*pos] = cp;
                    *pos += 1;
                }
                decomposed[start..end].copy_from_slice(&scratch[start..end]);
            }
            start = end + 1;
        }
    }
    Ok(())
}

fn normalize(input: &str) -> Result<Option<String>, String> {
    let size = input
        .chars()
        .try_fold(0usize, |sum, cp| {
            sum.checked_add(decomposition_size(u32::from(cp))?)
        })
        .ok_or("SASLprep input is too large")?;
    if size == 0 {
        return Ok(None);
    }
    let mut decomposed = Vec::new();
    decomposed
        .try_reserve_exact(size)
        .map_err(|_| "SASLprep allocation failed")?;
    for cp in input.chars() {
        decompose(u32::from(cp), &mut decomposed);
    }
    canonical_order(&mut decomposed)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(size)
        .map_err(|_| "SASLprep allocation failed")?;
    let mut starter = decomposed[0];
    output.push(starter);
    let mut starter_pos = 0;
    let mut last_class = -1;
    for &cp in &decomposed[1..] {
        let current_class = i16::from(class(cp));
        if let Some(composite) = (last_class < current_class)
            .then(|| compose(starter, cp))
            .flatten()
        {
            output[starter_pos] = composite;
            starter = composite;
        } else if current_class == 0 {
            starter_pos = output.len();
            starter = cp;
            last_class = -1;
            output.push(cp);
        } else {
            last_class = current_class;
            output.push(cp);
        }
    }
    let bytes = output
        .iter()
        .try_fold(0usize, |sum, cp| {
            sum.checked_add(char::from_u32(*cp)?.len_utf8())
        })
        .ok_or("invalid SASLprep output")?;
    let mut normalized = String::new();
    normalized
        .try_reserve_exact(bytes)
        .map_err(|_| "SASLprep allocation failed")?;
    for cp in output {
        normalized.push(char::from_u32(cp).ok_or("invalid SASLprep output")?);
    }
    Ok(Some(normalized))
}

pub(super) fn prepare(password: &str) -> Result<Option<String>, String> {
    if password.is_ascii() {
        return Ok(None);
    }
    let mut mapped = String::new();
    mapped
        .try_reserve_exact(password.len())
        .map_err(|_| "SASLprep allocation failed")?;
    for c in password.chars() {
        let cp = u32::from(c);
        if contains(data::NON_ASCII_SPACE_RANGES, cp) {
            mapped.push(' ');
        } else if !contains(data::COMMONLY_MAPPED_TO_NOTHING_RANGES, cp) {
            mapped.push(c);
        }
    }
    if mapped.is_empty()
        || mapped.chars().any(|c| {
            contains(data::PROHIBITED_OUTPUT_RANGES, u32::from(c))
                || contains(data::UNASSIGNED_CODEPOINT_RANGES, u32::from(c))
        })
    {
        return Ok(None);
    }
    let randal = |c: char| contains(data::RANDALCAT_CODEPOINT_RANGES, u32::from(c));
    if mapped.chars().any(randal)
        && (mapped
            .chars()
            .any(|c| contains(data::LCAT_CODEPOINT_RANGES, u32::from(c)))
            || !mapped.chars().next().is_some_and(randal)
            || !mapped.chars().next_back().is_some_and(randal))
    {
        return Ok(None);
    }
    normalize(&mapped)
}

#[cfg(test)]
mod tests {
    use super::{normalize, prepare};
    fn normalized(s: &str) -> Option<String> {
        normalize(s).unwrap()
    }
    fn prepared(s: &str) -> Option<String> {
        prepare(s).unwrap()
    }

    #[test]
    fn prepares_mapping_fallback_and_bidirectional_text() {
        assert_eq!(prepared("IX"), None);
        assert_eq!(prepared("I\u{ad}X"), Some("IX".into()));
        assert_eq!(prepared("I\u{200b}X"), Some("I X".into()));
        assert_eq!(prepared("\u{2168}"), Some("IX".into()));
        assert_eq!(prepared("\u{ad}"), None);
        assert_eq!(prepared("\u{1d2c}"), None);
        assert_eq!(prepared("ا\u{17b4}\u{ad}ا"), None);
        assert_eq!(prepared("ا\u{cbf}\u{ad}ا"), Some("ا\u{cbf}ا".into()));
        assert_eq!(prepared("I\u{e000}X"), None);
    }

    #[test]
    fn normalizes_decomposition_order_and_hangul() {
        assert_eq!(normalized("A\u{30a}"), Some("Å".into()));
        assert_eq!(normalized("\u{fb03}"), Some("ffi".into()));
        assert_eq!(normalized("\u{1100}\u{1161}\u{11a8}"), Some("각".into()));
        assert_eq!(normalized("\u{ac01}"), Some("각".into()));
        assert_eq!(normalized("a\u{315}\u{300}"), Some("à\u{315}".into()));
    }
}
