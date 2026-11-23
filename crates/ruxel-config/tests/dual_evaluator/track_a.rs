//! Track A's examples: a branch checkout over SSH, a fetched installer,
//! line entries composing in one file, and controller notices — each
//! valid case and its refusals lower the same way under both evaluators.

/// Track A's examples, in the valid/invalid order the harness reports.
pub(super) const EXAMPLES: &[&str] = &[
    "valid/git-branch.pkl",
    "valid/fetched-script.pkl",
    "valid/ensure-line.pkl",
    "valid/notice.pkl",
    "invalid/git-branch-sha.pkl",
    "invalid/ensure-line-empty-regexp.pkl",
    "invalid/ensure-line-multiline.pkl",
    "invalid/replace-line-empty-regexp.pkl",
    "invalid/notice-empty.pkl",
    "invalid/notice-secret.pkl",
    "invalid/notice-two-captures.pkl",
    "invalid/fetched-script-plain-http.pkl",
];
