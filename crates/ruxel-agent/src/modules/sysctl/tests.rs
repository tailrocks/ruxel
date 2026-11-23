//! The sysctl operation's tests: the name surface and the read of an
//! existing file, the entry rewrite's canonical key matching, the
//! publishability probe's timing with the boot shadows, and the boot
//! override's later-assignment refusal.

mod boot_byte_values;
mod boot_cache;
mod boot_names;
mod boot_override;
mod boot_override_canonical;
mod boot_shadows;
mod boot_surface;
mod boot_surface_safety;
mod boot_wildcards;
mod fallback;
mod live;
mod names;
mod no_log;
mod preflight;
mod publish;
mod publish_append_only;
mod rehearsal;
mod rewrite;
mod trim;
mod values;
