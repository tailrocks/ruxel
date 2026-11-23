use super::unescape_mount;

pub(super) fn rewrite_fstab(
    current: &str,
    src: &str,
    path: &str,
    fstype: &str,
    opts: &str,
) -> (String, bool) {
    let want = [src, path, fstype, opts, "0", "0"].map(escape_fstab);
    let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
    let mut found = false;
    let mut changed = false;
    lines.retain_mut(|line| {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            return true;
        }
        let comment = line.char_indices().find_map(|(i, c)| {
            (c == '#' && i > 0 && line[..i].ends_with(char::is_whitespace)).then_some(i)
        });
        let data = comment.map_or(line.as_str(), |i| &line[..i]);
        let f: Vec<&str> = data.split_whitespace().collect();

        if f.len() >= 2 && unescape_mount(f[1]) == path {
            if found {
                changed = true;
                return false;
            }
            found = true;
            let matches = (4..=6).contains(&f.len())
                && unescape_mount(f[0]) == src
                && unescape_mount(f[2]) == fstype
                && unescape_mount(f[3]) == opts
                && f.get(4).is_none_or(|value| *value == "0")
                && f.get(5).is_none_or(|value| *value == "0");
            if !matches {
                let suffix = comment.map(|i| {
                    let start = line[..i].trim_end().len();
                    line[start..].to_string()
                });
                *line = want.join("\t");
                if let Some(suffix) = suffix {
                    line.push_str(&suffix);
                }
                changed = true;
            }
        }
        true
    });
    if !found {
        lines.push(want.join("\t"));
        changed = true;
    }
    if changed {
        let mut content = lines.join("\n");
        content.push('\n');
        (content, true)
    } else {
        (current.to_string(), false)
    }
}

fn escape_fstab(value: &str) -> String {
    value
        .replace('\\', "\\134")
        .replace(' ', "\\040")
        .replace('\t', "\\011")
        .replace('\n', "\\012")
}

#[cfg(test)]
mod tests {
    use super::rewrite_fstab;

    #[test]
    fn removes_duplicate_entries_and_preserves_other_lines() {
        for first in ["/dev/a /srv xfs rw 0 0", "/dev/old /srv ext4 ro 0 0"] {
            let current = format!(
                "# heading\n{first}\n/dev/other /elsewhere xfs rw 0 0\n/dev/b /srv ext4 ro 0 0\n# trailing\n"
            );
            let (next, changed) = rewrite_fstab(&current, "/dev/a", "/srv", "xfs", "rw");
            assert!(changed);
            assert_eq!(next.lines().filter(|line| line.contains("/srv")).count(), 1);
            assert!(next.contains("/dev/other /elsewhere xfs rw 0 0\n# trailing\n"));
            assert_eq!(
                rewrite_fstab(&next, "/dev/a", "/srv", "xfs", "rw"),
                (next, false)
            );
        }
    }
}
