//! The closed apt package selector surface.
//!
//! Only `name`, `name=version`, `name:arch` and `name:arch=version` are
//! accepted. Globs, regular expressions, APT escapes, version patterns, and
//! the `:any`, `:native` and `:all` pseudo-architectures are refused instead
//! of approximating APT's matcher, so the installed state of every request is
//! one exact dpkg status entry.

use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackageRequest {
    name: String,
    arch: Option<String>,
    version: Option<String>,
}

impl PackageRequest {
    /// # Errors
    /// Returns an error naming the rejected selector.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let refuse = |why: &str| {
            Err(format!(
                "apt: unsupported package selector {text:?}: {why}; use name, name=version, name:arch or name:arch=version"
            ))
        };
        let (selector, version) = match text.split_once('=') {
            Some((selector, version)) => (selector, Some(version)),
            None => (text, None),
        };
        let (name, arch) = match selector.split_once(':') {
            Some((name, arch)) => (name, Some(arch)),
            None => (selector, None),
        };
        if !valid_name(name) {
            return refuse(
                "package names are two or more of a-z 0-9 + - . starting with a letter or digit",
            );
        }
        if let Some(arch) = arch {
            if matches!(arch, "any" | "native" | "all") {
                return refuse("the any, native and all pseudo-architectures are not supported");
            }
            if !valid_arch(arch) {
                return refuse("architectures are a-z 0-9 - with no wildcards");
            }
        }
        if let Some(version) = version
            && !valid_version(version)
        {
            return refuse("versions must be one exact Debian version, without patterns");
        }
        Ok(Self {
            name: name.to_owned(),
            arch: arch.map(str::to_owned),
            version: version.map(str::to_owned),
        })
    }

    /// Parse every selector and refuse requests for the same package more
    /// than once. A name may repeat only when every occurrence names a
    /// distinct explicit architecture.
    ///
    /// # Errors
    /// Returns the first rejected selector or duplicate.
    pub(crate) fn parse_all(names: &[String]) -> Result<Vec<Self>, String> {
        let requests = names
            .iter()
            .map(|name| Self::parse(name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen: HashMap<&str, Vec<Option<&str>>> = HashMap::new();
        for request in &requests {
            seen.entry(&request.name)
                .or_default()
                .push(request.arch.as_deref());
        }
        for (name, arches) in seen {
            let mut explicit: Vec<_> = arches.iter().flatten().collect();
            explicit.sort_unstable();
            explicit.dedup();
            if arches.len() > 1 && explicit.len() != arches.len() {
                return Err(format!(
                    "apt: package {name:?} is requested more than once; list each package once, or once per explicit architecture"
                ));
            }
        }
        Ok(requests)
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn arch(&self) -> Option<&str> {
        self.arch.as_deref()
    }

    pub(crate) fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The package without a version: `name` or `name:arch`.
    pub(crate) fn selector(&self) -> String {
        match &self.arch {
            Some(arch) => format!("{}:{arch}", self.name),
            None => self.name.clone(),
        }
    }

    /// The package pinned to an exact version.
    pub(crate) fn pinned(&self, version: &str) -> String {
        format!("{}={version}", self.selector())
    }
}

impl fmt::Display for PackageRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(version) => f.write_str(&self.pinned(version)),
            None => f.write_str(&self.selector()),
        }
    }
}

fn valid_name(name: &str) -> bool {
    name.len() >= 2
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
}

fn valid_arch(arch: &str) -> bool {
    !arch.is_empty()
        && arch.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && arch
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `[epoch:]upstream[-revision]`: an optional nonzero epoch without leading
/// zeros (dpkg records epoch 0 as no epoch, so `0:` could never match), then a value
/// that starts with a digit and uses only `A-Z a-z 0-9 . + ~ -`.
fn valid_version(version: &str) -> bool {
    let upstream = match version.split_once(':') {
        Some((epoch, rest))
            if epoch.starts_with(|c: char| matches!(c, '1'..='9'))
                && epoch.chars().all(|c| c.is_ascii_digit()) =>
        {
            rest
        }
        Some(_) => return false,
        None => version,
    };
    upstream.starts_with(|c: char| c.is_ascii_digit())
        && upstream
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '~' | '-'))
}

#[cfg(test)]
mod tests {
    use super::PackageRequest;

    #[test]
    fn accepts_exact_selectors() {
        for (text, name, arch, version) in [
            ("bash", "bash", None, None),
            ("libc6:i386", "libc6", Some("i386"), None),
            ("pkg=1.2-3", "pkg", None, Some("1.2-3")),
            (
                "g++-14:arm64=1:14.2.0-4ubuntu2~24.04+b1",
                "g++-14",
                Some("arm64"),
                Some("1:14.2.0-4ubuntu2~24.04+b1"),
            ),
        ] {
            let request = PackageRequest::parse(text).expect("supported selector");
            assert_eq!(
                (request.name(), request.arch(), request.version()),
                (name, arch, version)
            );
            assert_eq!(request.to_string(), text);
        }
    }

    #[test]
    fn refuses_patterns_escapes_and_pseudo_architectures() {
        for text in [
            "",
            "b",
            "Bash",
            "-bash",
            "lib*",
            "lib?",
            "lib[ab]",
            "/bash/",
            "~nbash",
            "bash=",
            "bash=5.2*",
            "bash=5.2.?",
            "bash=5.2.21-\\2ubuntu4",
            "bash=[5]",
            "bash=:1",
            "bash=0:1",
            "bash=01:1",
            "bash=a1",
            "bash=1:2:3",
            "bash:any",
            "bash:native",
            "bash:all",
            "bash:*",
            "bash:amd*",
            "bash:",
            "bash:amd64:x",
            "bash/noble",
            "bash=1/noble",
            "bash 1",
        ] {
            let error = PackageRequest::parse(text).expect_err(text);
            assert!(error.contains("unsupported package selector"), "{error}");
        }
    }

    #[test]
    fn refuses_duplicate_packages() {
        let parse = |names: &[&str]| {
            PackageRequest::parse_all(&names.iter().map(|n| (*n).into()).collect::<Vec<_>>())
        };
        for names in [
            &["bash", "bash"][..],
            &["bash=1", "bash=1"],
            &["bash=1", "bash=2"],
            &["bash", "bash:amd64"],
            &["bash:amd64=1", "bash:amd64"],
        ] {
            let error = parse(names).expect_err("duplicate");
            assert!(error.contains("requested more than once"), "{error}");
        }
        assert_eq!(
            parse(&["libc6:amd64", "libc6:i386=2", "bash"])
                .expect("distinct architectures")
                .len(),
            3
        );
    }
}
