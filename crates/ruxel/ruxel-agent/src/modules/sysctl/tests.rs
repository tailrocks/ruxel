//! The sysctl operation's tests: the name surface and the read of an
//! existing file, the entry rewrite's canonical key matching, the
//! publishability probe's timing with the boot shadows, and the boot
//! override's later-assignment refusal.

mod boot_override;
mod boot_surface;
mod boot_wildcards;
mod names;
mod publish;
mod rewrite;
