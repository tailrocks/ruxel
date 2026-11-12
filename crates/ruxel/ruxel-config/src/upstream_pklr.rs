//! Preparation for the switch from the `donbeave/pklr` fork to upstream
//! `jdx/pklr`.
//!
//! The default build of Ruxel evaluates Pkl with the fork (`pklr`); this
//! module is compiled only with the `pklr-upstream-path` feature, which is
//! off by default, so the repository's behaviour does not change. The module
//! does two things:
//!
//! 1. it records the capability families upstream still lacks, with the
//!    exact upstream location that carries the surrounding code (or the
//!    grep result that proves the family is absent), so the flip has a
//!    checklist instead of a compile error; and
//! 2. it calls the parts of the upstream API that exist today through the
//!    `pklr-upstream` alias, so the upstream crate is compiled and exercised
//!    before the fork is dropped.
//!
//! # The upstream revision this preparation targets
//!
//! [`UPSTREAM_REV`] is the revision the `pklr-upstream` alias pins. Every
//! location below was read from that revision, never from a working tree or
//! from a different tag.
//!
//! # How to reproduce every citation
//!
//! Each citation can be checked without a checkout:
//!
//! ```text
//! git clone https://github.com/jdx/pklr pklr
//! git -C pklr show 77828272d37b3e51f45582a75e345a0ae0b963a0:src/eval/setup.rs \
//!   | grep -n 'pub fn set_module_path'
//! ```
//!
//! The absence claims are greps over the whole upstream source tree at that
//! revision:
//!
//! ```text
//! git -C pklr grep -n 'ModuleSource' 77828272d37b3e51f45582a75e345a0ae0b963a0 -- src/
//! ```
//!
//! The counts this module records are the ones that command produces:
//! `ModuleSource` 0 hits, `add_module_path_source` 0 hits,
//! `add_module_path_directory` 0 hits, `GlobAccess` 0 hits,
//! `expand_glob_with` 0 hits, `Target::ModulePath` 0 hits,
//! `eval_source_json_blocking` 0 hits. Upstream has no `src/module_source.rs`
//! and no `src/eval/module_path.rs` file either.
//!
//! # The four capability families upstream lacks
//!
//! Those zero counts are why flipping the default dependency today fails to
//! compile (see `crates/ruxel/ruxel-config/src/eval.rs` and
//! `crates/ruxel/ruxel-config/src/eval/templates.rs`). [`CAPABILITY_GAPS`]
//! names each family, the upstream location that carries the surrounding
//! code today, the grep result that shows the family itself is absent, and
//! what Ruxel needs from it.
//!
//! # What the lane exercises today
//!
//! [`eval_confined`] drives upstream's evaluator through Ruxel's own
//! evaluation path, not through a hand-built upstream capability:
//!
//! - the roots are opened by `crate::eval::confined_roots`, the same
//!   descriptor-pinned, symlink-refusing opening the corpus uses;
//! - the host IO is `ConfinedCapabilities`, which forwards to the same
//!   confined implementation the fork's evaluator uses, so a read outside
//!   the roots is refused by Ruxel's own code;
//! - the policy is `crate::eval::apply_policy`, the same policy the corpus
//!   runs under; and
//! - upstream's evaluator reads the module and its imports through those
//!   capabilities and searches the module path directories
//!   (`src/eval/setup.rs:186`).
//!
//! [`expand_glob`] calls upstream's own glob expansion
//! (`src/eval/glob.rs:24`, re-exported at `src/eval.rs:57`).
//!
//! # What the lane cannot exercise today
//!
//! The four capability families upstream still lacks keep the rest of Ruxel's
//! evaluation path out of reach, and the lane says so instead of pretending
//! otherwise:
//!
//! - **Ruxel's module source.** Upstream has no `ModuleSource` (0 hits at
//!   [`UPSTREAM_REV`]), so upstream's evaluator reads the root module itself
//!   — through the confined capabilities, so the confinement still holds —
//!   instead of through Ruxel's `ModuleSource`, whose import validation
//!   (module globs and ancestor searches are refused) is fork-bound.
//! - **In-memory module path entries.** Upstream's `set_module_path`
//!   (`src/eval/setup.rs:186`) takes directories only, so the schema Ruxel
//!   compiles into the module path cannot be served from memory.
//! - **`modulepath:` resources.** Upstream resolves a `modulepath:` URI to a
//!   module file (`src/eval/loading.rs:22`), never to a resource.
//! - **The rendered output.** Upstream's JSON entry point is `pub(crate)`
//!   (`src/eval/output.rs:742`), so the lane sees the module value, not the
//!   rendered `output.value`.
//!
//! The flip of the default dependency therefore stays blocked on
//! `jdx/pklr#419`.

use crate::ConfigError;
use crate::eval::{Confined, EvalOptions, EvaluationPolicy, Resources};
use pklr::EvalCapabilities as ForkCapabilities;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The upstream revision the `pklr-upstream` alias pins.
pub const UPSTREAM_REV: &str = "77828272d37b3e51f45582a75e345a0ae0b963a0";

/// A capability family Ruxel needs and upstream `jdx/pklr` does not expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityGap {
    /// The family, named after the API Ruxel uses today.
    pub family: &'static str,
    /// The upstream location that carries the code around the family: the
    /// file and line the missing API would have to join, read at
    /// [`UPSTREAM_REV`].
    pub location: &'static str,
    /// The grep result that shows the family itself is absent upstream.
    pub absent: &'static str,
    /// What upstream has at that location today.
    pub upstream: &'static str,
    /// What Ruxel needs from the family.
    pub required: &'static str,
}

/// The capability families upstream still lacks, in the order the flip hits
/// them.
///
/// Every `location` is a file and line of upstream at [`UPSTREAM_REV`], and
/// every `absent` is the hit count of the corresponding grep over upstream's
/// whole `src/` tree at that revision.
pub const CAPABILITY_GAPS: &[CapabilityGap] = &[
    CapabilityGap {
        family: "public evaluation through custom capabilities",
        location: "src/eval/output.rs:742",
        absent: "0 hits for `eval_source_json_blocking`",
        upstream: "`pub(crate) fn eval_file_json_blocking` is not callable by \
                   an embedder, and upstream has no public JSON entry point \
                   for a source",
        required: "a public entry point that evaluates a source with the \
                   caller's `EvalCapabilities` and returns the rendered \
                   `output.value` as JSON",
    },
    CapabilityGap {
        family: "module path sources",
        location: "src/eval/setup.rs:186",
        absent: "0 hits for `add_module_path_source`, 0 hits for \
                 `add_module_path_directory`",
        upstream: "`pub fn set_module_path(dirs: Vec<PathBuf>)` takes \
                   directories only, and `module_path_file` \
                   (`src/eval/loading.rs:22`) resolves a `modulepath:` URI \
                   to a file in one of them; upstream has no \
                   `src/eval/module_path.rs` file",
        required: "`add_module_path_source` and `add_module_path_directory`, \
                   so the schema compiled into the module path can be served \
                   from memory",
    },
    CapabilityGap {
        family: "module sources",
        location: "src/capabilities.rs:35",
        absent: "0 hits for `ModuleSource`",
        upstream: "`pub trait EvalCapabilities` is the only IO seam and it \
                   is path-based; upstream has no `src/module_source.rs` file",
        required: "a `ModuleSource` seam, so modules can be read through \
                   Ruxel's confined roots instead of through host paths",
    },
    CapabilityGap {
        family: "glob and resource hooks",
        location: "src/capabilities.rs:138",
        absent: "0 hits for `GlobAccess`, 0 hits for `expand_glob_with`",
        upstream: "`fn glob(&mut self, base: &Path, pattern: &str)` is the \
                   only glob hook and it is part of `EvalCapabilities`; \
                   there is no separate glob gate and no per-resource hook",
        required: "the glob and resource hooks that let Ruxel refuse globs \
                   and confine `file:` resources to its roots",
    },
    CapabilityGap {
        family: "module path resources",
        location: "src/eval/resource.rs:148",
        absent: "0 hits for `Target::ModulePath`",
        upstream: "`fn check_resource_allowed` matches URI prefixes only, and \
                   `enum Target` (`src/eval/resource.rs:310`) has no \
                   `ModulePath` variant, so a `modulepath:` URI never names \
                   a resource",
        required: "a `Target::ModulePath` variant, so `modulepath:` \
                   resources can be read from the module path namespace",
    },
];

/// Expand a glob with upstream's own implementation.
///
/// This is the upstream counterpart of the fork's glob handling
/// (`src/eval/glob.rs:24` upstream, re-exported at `src/eval.rs:57`).
///
/// # Errors
/// Returns an error when the pattern is malformed or the base directory
/// cannot be read.
pub fn expand_glob(base: &Path, pattern: &str) -> pklr_upstream::Result<Vec<PathBuf>> {
    pklr_upstream::eval::expand_glob(base, pattern)
}

/// Ruxel's confined capabilities, adapted to upstream's `EvalCapabilities`.
///
/// The fork's `Resources` is the evaluator's real host IO: reads are confined
/// to the pinned roots, globs and writes are refused, and no environment
/// variable and no network are reachable. This adapter exposes that
/// implementation through upstream's trait, so the feature-gated lane
/// compiles the *project's* capabilities against upstream's interface
/// instead of upstream's own `NativeCapabilities`: an incompatible change to
/// upstream's trait breaks this file, and the lane fails before the default
/// dependency is flipped.
#[derive(Debug)]
pub(crate) struct ConfinedCapabilities {
    resources: Resources,
}

impl ConfinedCapabilities {
    /// Wrap Ruxel's confined capabilities for an upstream evaluator.
    #[must_use]
    pub(crate) fn new(files: Arc<Confined>) -> Self {
        Self {
            resources: Resources(files),
        }
    }
}

/// The fork's error as upstream's error.
///
/// The two crates carry their own error types. An IO error keeps its path and
/// its cause; every other error becomes an evaluation error that keeps the
/// message.
fn upstream_error(error: pklr::Error) -> pklr_upstream::Error {
    match error {
        pklr::Error::Io(path, cause) => pklr_upstream::Error::Io(path, cause),
        pklr::Error::Eval(message) => pklr_upstream::Error::Eval(message),
        other => pklr_upstream::Error::Eval(other.to_string()),
    }
}

impl pklr_upstream::EvalCapabilities for ConfinedCapabilities {
    fn read_to_string(&mut self, path: &Path) -> pklr_upstream::Result<String> {
        ForkCapabilities::read_to_string(&mut self.resources, path).map_err(upstream_error)
    }

    fn read_bytes(&mut self, path: &Path) -> pklr_upstream::Result<Vec<u8>> {
        ForkCapabilities::read_bytes(&mut self.resources, path).map_err(upstream_error)
    }

    fn path_exists(&mut self, path: &Path) -> pklr_upstream::Result<bool> {
        ForkCapabilities::path_exists(&mut self.resources, path).map_err(upstream_error)
    }

    fn canonicalize(&mut self, path: &Path) -> pklr_upstream::Result<PathBuf> {
        ForkCapabilities::canonicalize(&mut self.resources, path).map_err(upstream_error)
    }

    fn create_dir_all(&mut self, path: &Path) -> pklr_upstream::Result<()> {
        ForkCapabilities::create_dir_all(&mut self.resources, path).map_err(upstream_error)
    }

    fn write_atomic(&mut self, path: &Path, bytes: &[u8]) -> pklr_upstream::Result<()> {
        ForkCapabilities::write_atomic(&mut self.resources, path, bytes).map_err(upstream_error)
    }

    fn remove_file(&mut self, path: &Path) -> pklr_upstream::Result<()> {
        ForkCapabilities::remove_file(&mut self.resources, path).map_err(upstream_error)
    }

    fn read_env(&mut self, name: &str) -> pklr_upstream::Result<Option<String>> {
        ForkCapabilities::read_env(&mut self.resources, name).map_err(upstream_error)
    }

    fn fetch_text(&mut self, url: &str) -> pklr_upstream::Result<String> {
        ForkCapabilities::fetch_text(&mut self.resources, url).map_err(upstream_error)
    }

    fn fetch_bytes(&mut self, url: &str) -> pklr_upstream::Result<Vec<u8>> {
        ForkCapabilities::fetch_bytes(&mut self.resources, url).map_err(upstream_error)
    }

    fn temp_dir(&mut self, prefix: &str) -> pklr_upstream::Result<PathBuf> {
        ForkCapabilities::temp_dir(&mut self.resources, prefix).map_err(upstream_error)
    }

    fn glob(&mut self, base: &Path, pattern: &str) -> pklr_upstream::Result<Vec<PathBuf>> {
        ForkCapabilities::glob(&mut self.resources, base, pattern).map_err(upstream_error)
    }
}

/// Ruxel's evaluation policy, applied to upstream's evaluator.
///
/// The setters are the same ones the fork's evaluator gets, so the upstream
/// path runs under the policy the corpus runs under instead of a copy of it.
impl EvaluationPolicy for pklr_upstream::Evaluator {
    fn set_base_path(&mut self, path: &Path) {
        pklr_upstream::Evaluator::set_base_path(self, path);
    }

    fn set_allowed_resources(&mut self, resources: Vec<String>) {
        pklr_upstream::Evaluator::set_allowed_resources(self, resources);
    }

    fn set_environment_variables(&mut self, variables: std::collections::BTreeMap<String, String>) {
        pklr_upstream::Evaluator::set_environment_variables(self, variables);
    }

    fn set_external_properties(&mut self, properties: std::collections::BTreeMap<String, String>) {
        pklr_upstream::Evaluator::set_external_properties(self, properties);
    }

    fn set_offline(&mut self, offline: bool) {
        pklr_upstream::Evaluator::set_offline(self, offline);
    }

    fn set_max_depth(&mut self, depth: usize) {
        pklr_upstream::Evaluator::set_max_depth(self, depth);
    }
}

/// Evaluate a configuration with the upstream evaluator, through Ruxel's own
/// evaluation path.
///
/// This is the largest slice of Ruxel's evaluation path that can run against
/// upstream today, and every part of it is the production one:
///
/// - the roots are opened by `crate::eval::confined_roots`, the same
///   descriptor-pinned, symlink-refusing opening the corpus uses;
/// - the host IO is `ConfinedCapabilities`, the same confined
///   implementation the fork's evaluator uses;
/// - the policy is `crate::eval::apply_policy`, the same policy the corpus
///   runs under; and
/// - upstream's evaluator reads the module and its imports through those
///   capabilities, so a read outside the roots is refused by Ruxel's own
///   code.
///
/// The four capability families upstream still lacks keep the rest of the
/// path out of reach: there is no `ModuleSource`, so upstream's evaluator
/// reads the module itself (through the confined capabilities) instead of
/// through Ruxel's module source, whose import validation is fork-bound; the
/// module path serves directories only, so the compiled-in schema cannot be
/// served from memory; and upstream's JSON entry point is `pub(crate)`, so
/// the result is the module value rather than the rendered `output.value`.
///
/// # Errors
/// Returns an error when a root directory cannot be opened, when the module
/// lies outside the roots, when the evaluation reads something outside them,
/// or when the value cannot be rendered as JSON.
pub fn eval_confined(root: &Path, options: &EvalOptions) -> Result<serde_json::Value, ConfigError> {
    let roots = crate::eval::confined_roots(root, options)?;
    let mut evaluator =
        pklr_upstream::Evaluator::with_capabilities(ConfinedCapabilities::new(roots.files.clone()));
    crate::eval::apply_policy(&mut evaluator, &roots.module);
    evaluator.set_module_path(roots.module_path.clone());
    let value = evaluator
        .eval_file(&roots.module)
        .map_err(|error| ConfigError::Pkl {
            module: root.to_path_buf(),
            message: error.to_string(),
            located: None,
        })?;
    value.try_to_json().map_err(|error| ConfigError::Pkl {
        module: root.to_path_buf(),
        message: error.to_string(),
        located: None,
    })
}

#[cfg(test)]
mod tests;
