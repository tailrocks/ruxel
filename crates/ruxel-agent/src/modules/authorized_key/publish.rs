//! Writing the `authorized_keys` file, and the identity of the file it
//! was written from.

#[path = "publish/identity.rs"]
mod identity;
#[path = "publish/stage.rs"]
mod stage;

pub(super) use identity::read_authorized_keys_with_identity;
pub(super) use stage::publish_checked;
#[cfg(test)]
pub(super) use stage::{publish, publish_racing, publish_racing_at};
