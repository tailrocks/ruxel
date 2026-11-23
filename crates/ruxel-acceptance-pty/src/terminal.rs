use anyhow::Result;
use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags, open as open_slave},
    io::{FdFlags, fcntl_setfd},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr},
};

pub(crate) fn open() -> Result<(OwnedFd, OwnedFd)> {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    fcntl_setfd(&master, FdFlags::CLOEXEC)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let slave = open_slave(
        ptsname(&master, Vec::new())?,
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut attrs = tcgetattr(&slave)?;
    attrs.local_modes.remove(LocalModes::ECHO);
    attrs
        .local_modes
        .insert(LocalModes::ISIG | LocalModes::ICANON);
    attrs.special_codes[SpecialCodeIndex::VINTR] = 3;
    attrs.special_codes[SpecialCodeIndex::VEOF] = 4;
    tcsetattr(&slave, OptionalActions::Now, &attrs)?;
    Ok((master, slave))
}
