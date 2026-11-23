#[derive(Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub changed: bool,
    pub extents: u64,
}

pub(super) fn parse(size: &str) -> Result<Option<u32>, String> {
    if !size.contains('%')
        || size.eq_ignore_ascii_case("100%FREE")
        || size.eq_ignore_ascii_case("+100%FREE")
    {
        return Ok(None);
    }
    let percentage = size.split_once('%').and_then(|(number, basis)| {
        (basis.eq_ignore_ascii_case("VG") && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| number.parse::<u32>().ok())
            .flatten()
            .filter(|percent| (1..=100).contains(percent))
    });
    percentage.map(Some).ok_or_else(|| {
        format!("lvol: unsupported percentage size {size:?}; use 1..100%VG, 100%FREE or +100%FREE")
    })
}

pub(super) fn target(
    percent: u32,
    current: u64,
    extent: u64,
    total: u64,
) -> Result<Target, String> {
    if !(1..=100).contains(&percent)
        || extent == 0
        || total == 0
        || !current.is_multiple_of(extent)
        || current / extent > total
    {
        return Err("lvol: invalid percentage geometry".into());
    }
    let extents = u64::try_from(u128::from(total) * u128::from(percent) / 100)
        .map_err(|_| "lvol: percentage extent overflow")?;
    let bytes = extents
        .checked_mul(extent)
        .filter(|bytes| *bytes > 0)
        .ok_or("lvol: invalid or overflowing percentage target")?;
    if bytes < current {
        return Err("lvol: refusing to shrink an existing logical volume".into());
    }
    Ok(Target {
        changed: bytes > current,
        extents,
    })
}
