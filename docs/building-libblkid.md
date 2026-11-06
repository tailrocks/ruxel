# Building the agent with libblkid

The Linux filesystem provider uses the safe `rsblkid` wrapper around libblkid.
The agent remains a static musl executable. The controller does not need this
native dependency on macOS.

## Static Linux agent

Install Docker and pkgconf through Homebrew or mise on macOS. Start Docker's
local Linux engine. Xcode Command Line Tools supply libclang; a different
installation can be selected with `LIBCLANG_PATH`.

```sh
tools/build-libblkid -- cargo zigbuild --target x86_64-unknown-linux-musl \
  -p ruxel-agent --release --locked
```

The script builds libblkid 2.41.6 from the official kernel.org release archive,
verified against SHA-256
`e596083744e746be7d2823b62b43f4418dd7bf56303b4dc09e6fe8112fe3d7ed`.
The archive checksum also matches the SHA-512 published in
[Alpine's source recipe](https://github.com/alpinelinux/aports/blob/3.23-stable/main/util-linux/APKBUILD).
Build tools come from Alpine's official repositories with signed package
metadata, inside a digest-pinned official Alpine Linux container. No tools or
downloaded executables are installed on the macOS host by this script.

Generated files live outside the checkout, below
`$XDG_CACHE_HOME/ruxel/native/libblkid/`, or the platform cache default:
`~/Library/Caches/ruxel/native/libblkid/` on macOS and
`~/.cache/ruxel/native/libblkid/` on Linux. It contains the original source archive, extracted source,
configuration/build logs, installed package versions, license copies, a
provenance record, artifact checksums, and the generated musl sysroot. A recipe
digest and verified artifact checksums allow reuse of an unchanged build. An
exclusive cache-directory lock serializes concurrent builders, which recheck the
cache after acquiring it. Cache paths include the complete build-recipe digest,
version and target, so a different checkout's recipe never replaces an active
sysroot. Completed caches are immutable: a failed artifact check refuses reuse
and repair in place. A competing build waits at most 60 seconds before
failing; no partial build is marked complete. An interrupted build can be retried
after confirming no builder is running and removing its stale `build.lock`.
Package versions are recorded rather than pinned: signed Alpine repository
updates may change the build tools. The source and container digest are pinned.

`--without-econf` is intentional. Alpine's packaged static libblkid contains
libeconf references missing from its pkg-config metadata, while its libeconf
development package supplies no static library. Building the upstream library
with econf disabled removes that dependency. The source build includes all
filesystem, RAID and partition signature probes; it does not filter signatures.

The wrapper's build script needs both pkg-config settings and explicit target
headers for bindgen and its C preprocessor. `tools/build-libblkid -- command`
sets these variables before executing the command:

- `PKG_CONFIG_ALLOW_CROSS=1`
- `PKG_CONFIG_SYSROOT_DIR=<output>/sysroot`
- `PKG_CONFIG_LIBDIR=<output>/sysroot/usr/lib/pkgconfig`
- `PKG_CONFIG_ALL_STATIC=1`
- `CC_SHELL_ESCAPED_FLAGS=1`
- `CFLAGS_x86_64_unknown_linux_musl=-I<output>/sysroot/usr/include`
- `BINDGEN_EXTRA_CLANG_ARGS=--sysroot=<output>/sysroot --target=x86_64-unknown-linux-musl -I<output>/sysroot/usr/include`
- `LIBCLANG_PATH` on macOS, defaulting to the selected Xcode tools' `usr/lib`.

Use `--print-env` to inspect those nonsecret settings. The script checks that
pkg-config reports exactly 2.41.6. Changing the archive requires changing its
pinned checksum and reviewing the native library's probe behavior.
Compiler and bindgen arguments use POSIX shell quoting, preserving spaces and
metacharacters in cache paths. Control characters are refused because the
native pkg-config metadata uses a line-oriented format.

Verify the resulting executable in Linux with `file`, `readelf -l`, and
`readelf -d`: it must have no ELF interpreter or dynamic dependencies.

## Native Linux tests and CI

Native Linux GNU builds use the runner's C compiler, make, pkg-config, tar,
HTTPS certificates/download client and libclang. They need no installed
libblkid development package and no sudo or Docker at build time. On a minimal
Debian or Ubuntu development image those prerequisite tools are available from
the official signed repositories as `build-essential`, `pkgconf`,
`libclang-dev`, `curl`, `ca-certificates` and `xz-utils`.

Cargo's build environment connects `rsblkid-sys` to the provisioning wrapper:

```toml
[env]
PKG_CONFIG = { value = "tools/pkg-config-libblkid", relative = true }
C_INCLUDE_PATH = { value = ".cache/native/include", relative = true }
```

The wrapper intercepts only queries containing the `blkid` package. It builds
the pinned upstream source with native `cc` and make into the platform cache,
checks the exact version, and delegates to the system pkg-config. Other package
queries are delegated unchanged. Native pkg-config metadata uses absolute
cache paths and is prepended to the ordinary search path, preserving other
packages' discovery. Supplied cross sysroots are used without a native build.

The wrapper atomically publishes `.cache/native/include` as a symlink to the
cached headers. `C_INCLUDE_PATH` makes these headers visible both to the C
version probe and to libclang/bindgen; the sys crate does not pass pkg-config's
include directories to bindgen. Source/header trees remain outside the
checkout and secret scanner. The symlink is ignored by Git.

Run `cargo test -p ruxel-agent --locked` normally. To inspect the native sysroot
independently, use `tools/build-libblkid --native --sysroot`. Do not carry the
musl build's environment variables into a native GNU build. The exact source pin
also exceeds the wrapper's documented minimum libblkid version 2.39.2.

## Redistribution and relinking

The Rust wrapper is licensed under MIT OR Apache-2.0. Native libblkid is
LGPL-2.1-or-later; its license remains applicable when linked statically.
See [the notice](../LICENSES/libblkid.txt) and the complete
[LGPL text](../LICENSES/LGPL-2.1-or-later.txt).

For each released static agent, publish the following together at the same
download location, following LGPL 2.1 section 6(a)/(d):

1. The libblkid notice and LGPL license text.
2. The exact libblkid source archive, any patches, the build script, package
   manifest and configuration logs used for that executable. The build output
   preserves these materials; distribute them with the release.
3. The corresponding Ruxel source at the release commit, `Cargo.lock`, Rust
   dependency sources, and build instructions sufficient to replace libblkid
   and rebuild/relink the executable. `cargo vendor --locked --versioned-dirs`
   can collect Rust dependency sources. Include its generated Cargo source
   configuration and the pinned toolchain configuration.
4. Instructions to rebuild the library with the desired modification, replace
   the sysroot archive, clear `rsblkid-sys`'s Cargo build artifacts, and rerun
   the same static build command with the sysroot environment. Clearing this
   dependency matters because Cargo can otherwise reuse an rlib containing
   the previous static library.

The source material must permit modification for the recipient's own use and
reverse engineering to debug those modifications, as required by section 6.
Do not describe a binary-only release as satisfying these source/relink steps.
Generated archives, sysroots and executables stay outside Git.
