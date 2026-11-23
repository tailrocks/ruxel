use super::*;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    #[test]
    fn filesystem_policy_rejects_opaque_or_remote_backing() {
        for kind in [0xef53, 0x5846_5342, 0x9123_683e, 0x0102_1994, 0x8584_58f6] {
            assert!(supported_linux_filesystem(kind));
        }
        for kind in [0, 0x6969, 0xff53_4d42, 0x6573_5546, 0x794c_7630] {
            assert!(!supported_linux_filesystem(kind));
        }
    }

    // Linux UAPI posix_acl_xattr.h: version 2, little-endian tag/perm/id.
    fn acl_bytes() -> Vec<u8> {
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        for (tag, permissions, uid) in [
            (1_u16, 7_u16, u32::MAX),
            (2, 7, 65534),
            (4, 7, u32::MAX),
            (16, 7, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(permissions.to_le_bytes());
            bytes.extend(uid.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn precreation_caps_inherited_named_user_write_permissions() {
        let fixture = Fixture::new();
        rustix::fs::setxattr(
            &fixture.0,
            "system.posix_acl_default",
            &acl_bytes(),
            rustix::fs::XattrFlags::empty(),
        )
        .unwrap();
        assert_eq!(fs::metadata(&fixture.0).unwrap().mode() & 0o777, 0o700);
        known_hosts(&fixture.file()).unwrap();
        let mut bytes = [0; 256];
        let length =
            rustix::fs::getxattr(fixture.file(), "system.posix_acl_access", &mut bytes).unwrap();
        let entries = bytes[4..length].as_chunks::<8>().0;
        let named_user = entries.iter().find(|entry| entry[0] == 2).unwrap();
        assert_eq!(named_user[2], 7);
        let mask = entries.iter().find(|entry| entry[0] == 16).unwrap();
        assert_eq!(mask[2], 0);
        assert_eq!(fs::metadata(fixture.file()).unwrap().mode() & 0o777, 0o600);
        // Existing named WRITE is safe because its effective mask is empty.
        known_hosts(&fixture.file()).unwrap();
        // Exposing the mask makes that same entry unsafe and must be refused.
        fs::set_permissions(fixture.file(), fs::Permissions::from_mode(0o620)).unwrap();
        assert!(known_hosts(&fixture.file()).is_err());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use exacl::{AclEntry, Flag, Perm};

    fn allow_write(flags: Flag) -> AclEntry {
        AclEntry::allow_user(
            &rustix::process::geteuid().as_raw().to_string(),
            Perm::WRITE,
            flags,
        )
    }

    #[test]
    fn rejects_acl_grants_with_private_unix_modes() {
        let fixture = Fixture::new();
        let path = fixture.file();
        known_hosts(&path).unwrap();
        exacl::setfacl(&[&path], &[allow_write(Flag::empty())], None).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert!(known_hosts(&path).is_err());
        exacl::setfacl(&[&path], &[], None).unwrap();

        exacl::setfacl(&[&fixture.0], &[allow_write(Flag::FILE_INHERIT)], None).unwrap();
        assert_eq!(fs::metadata(&fixture.0).unwrap().mode() & 0o777, 0o700);
        assert!(known_hosts(&path).is_err());
        assert!(known_hosts(&fixture.0.join("missing")).is_err());
        assert!(!fixture.0.join("missing").exists());
        assert!(crate::mux::prepare(&fixture.0.join("mux")).is_err());
        exacl::setfacl(&[&fixture.0], &[], None).unwrap();
    }

    #[test]
    fn permits_deny_only_acl_and_its_inheritance() {
        let fixture = Fixture::new();
        let deny = AclEntry::deny_user(
            &rustix::process::geteuid().as_raw().to_string(),
            Perm::WRITE,
            Flag::FILE_INHERIT | Flag::ONLY_INHERIT,
        );
        exacl::setfacl(&[&fixture.0], &[deny], None).unwrap();
        known_hosts(&fixture.file()).unwrap();
        assert_eq!(exacl::getfacl(fixture.file(), None).unwrap().len(), 1);
        exacl::setfacl(&[&fixture.0], &[], None).unwrap();
    }
}
