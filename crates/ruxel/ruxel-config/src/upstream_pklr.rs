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
//! # What the adapter covers today
//!
//! - [`expand_glob`] calls upstream's own glob expansion
//!   (`src/eval/glob.rs:24`, re-exported at `src/eval.rs:57`).
//! - [`eval_source_json`] builds an upstream evaluator over
//!   `EvalCapabilities` (`src/capabilities.rs:35`), searches `modulepath:`
//!   module URIs in the directories given to it
//!   (`src/eval/setup.rs:186`), and evaluates a source with
//!   `Evaluator::eval_source` (`src/eval/loading.rs:386`).
//!
//! # What the adapter cannot cover today
//!
//! - The confined evaluator: upstream reads modules through paths, not
//!   through a `ModuleSource`, so Ruxel's descriptor-pinned roots and its
//!   refusal to follow symlinks have no upstream counterpart.
//! - In-memory module path entries: upstream's `set_module_path`
//!   (`src/eval/setup.rs:186`) takes directories only, so the schema Ruxel
//!   compiles into the module path cannot be served from memory.
//! - `modulepath:` resources: upstream resolves a `modulepath:` URI to a
//!   module file (`src/eval/loading.rs:22`), never to a resource, so
//!   Ruxel's `modulepath:` resource reads have no upstream counterpart.
//! - The rendered output: upstream's JSON entry point is `pub(crate)`
//!   (`src/eval/output.rs:742`), so [`eval_source_json`] returns the module
//!   value, not the rendered `output.value`.
//!
//! The flip of the default dependency therefore stays blocked on
//! `jdx/pklr#419`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The upstream revision the `pklr-upstream` alias pins.
pub const UPSTREAM_REV: &str = "77828272d37b3e51f45582a75e345a0ae0b963a0";

/// The depth limit the upstream evaluator is given.
///
/// The fork's evaluator counts every nested expression against this limit;
/// 256 fits a chain of 16 commands that each react to the one before.
const MAX_DEPTH: usize = 256;

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

/// Evaluate a Pkl source with the upstream evaluator and return its JSON.
///
/// The evaluator searches `modulepath:` module URIs in `module_path`, in
/// order, refuses every resource scheme but `file:` and `modulepath:`, reads
/// no environment variable, takes no external property and stays offline.
///
/// The result is the JSON of the module value, not of the rendered
/// `output.value`: upstream's JSON entry point is `pub(crate)` (see
/// [`CAPABILITY_GAPS`]), so this is the closest upstream can get today.
///
/// This is a preparation adapter, not a replacement for the confined
/// evaluator: upstream reads through host paths, so nothing here confines
/// the evaluation to Ruxel's roots.
///
/// # Errors
/// Returns an error when the source does not parse, when it reads something
/// the evaluator refuses, or when its value cannot be rendered as JSON.
pub fn eval_source_json(
    source: &str,
    path: &Path,
    module_path: &[PathBuf],
) -> pklr_upstream::Result<serde_json::Value> {
    let mut evaluator =
        pklr_upstream::Evaluator::with_capabilities(pklr_upstream::NativeCapabilities::new());
    evaluator.set_base_path(path.parent().unwrap_or(Path::new("/")));
    evaluator.set_module_path(module_path.to_vec());
    evaluator.set_allowed_resources(vec!["file:".to_owned(), "modulepath:".to_owned()]);
    evaluator.set_environment_variables(BTreeMap::new());
    evaluator.set_external_properties(BTreeMap::new());
    evaluator.set_offline(true);
    evaluator.set_max_depth(MAX_DEPTH);
    let value = evaluator.eval_source(source, path)?;
    value.try_to_json()
}

#[cfg(test)]
mod tests;
