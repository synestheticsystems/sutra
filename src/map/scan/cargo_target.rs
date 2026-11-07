//! How a Cargo build folder is told from any other folder: by its place beside a `Cargo.toml`, or by
//! the tag cargo writes into it. The walk, the searches for build folders and the clean's checks all
//! read it this one way.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

/// The file cargo writes into every build folder it makes, which marks the folder as a cache
/// (<https://bford.info/cachedir/>).
pub const CACHEDIR_TAG_FILE: &str = "CACHEDIR.TAG";

/// The line every cache directory tag starts with, whichever tool wrote it.
const TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// What cargo writes after the signature, which tells its tag from another tool's: pytest and ruff
/// tag their caches too.
const WRITTEN_BY_CARGO: &[u8] = b"created by cargo";

/// The most of a tag read: cargo's runs to 177 bytes, and its words come in the first 100.
const TAG_BYTES: u64 = 256;

/// The file cargo locks in its home folder while it downloads packages, which marks that folder
/// as a cargo home.
const PACKAGE_CACHE_LOCK: &str = ".package-cache";

/// Whether `folder` is a Cargo build folder: a `target` beside a `Cargo.toml`, cargo's default, or
/// any folder holding cargo's `CACHEDIR.TAG` whatever its name, as `CARGO_TARGET_DIR` and
/// `--target-dir` name it.
pub fn is_cargo_target(folder: &Path) -> bool {
    let beside_manifest = folder.file_name() == Some(OsStr::new("target"))
        && folder
            .parent()
            .is_some_and(|parent| fs::symlink_metadata(parent.join("Cargo.toml")).is_ok());
    beside_manifest || is_tagged_build_folder(folder)
}

/// Whether cargo tagged `folder` as a build folder of its own. Cargo tags the download caches in
/// its home folder (`registry`, `git`) the same way, and those are no build folder: no `cargo
/// clean` deletes them, and cargo downloads into them under a lock of its own, not a build's.
pub(super) fn is_tagged_build_folder(folder: &Path) -> bool {
    let in_cargo_home = folder
        .parent()
        .is_some_and(|home| fs::symlink_metadata(home.join(PACKAGE_CACHE_LOCK)).is_ok());
    holds_cargo_tag(folder) && !in_cargo_home
}

/// Whether `folder` holds the `CACHEDIR.TAG` cargo writes into a folder it makes: a regular file,
/// not a link, that starts with the cache tag's signature and says cargo created it.
pub fn holds_cargo_tag(folder: &Path) -> bool {
    let tag = folder.join(CACHEDIR_TAG_FILE);
    if !fs::symlink_metadata(&tag).is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    let mut start = Vec::new();
    File::open(&tag)
        .and_then(|file| file.take(TAG_BYTES).read_to_end(&mut start))
        .is_ok()
        && start.starts_with(TAG_SIGNATURE)
        && start
            .windows(WRITTEN_BY_CARGO.len())
            .any(|words| words == WRITTEN_BY_CARGO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{CACHEDIR_TAG, PYTEST_TAG, TempDir};

    #[test]
    fn a_cargo_target_is_a_target_beside_a_manifest_or_a_folder_cargo_tagged_by_any_name() {
        let fixture = TempDir::new("cargo-target-told");
        fixture.write("app/Cargo.toml", "");
        let default = fixture.dir("app/target");
        let beside_but_named_otherwise = fixture.dir("app/src");
        let tagged = fixture.write("scratch/r567-target/CACHEDIR.TAG", CACHEDIR_TAG);
        let pytest = fixture.write("app/.pytest_cache/CACHEDIR.TAG", PYTEST_TAG);
        let signature_alone = fixture.write(
            "scratch/old/CACHEDIR.TAG",
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        );
        let linked = fixture.dir("scratch/linked");
        std::os::unix::fs::symlink(&tagged, linked.join("CACHEDIR.TAG")).unwrap();

        assert!(is_cargo_target(&default));
        assert!(is_cargo_target(tagged.parent().unwrap()));
        assert!(!is_cargo_target(&beside_but_named_otherwise));
        for not_cargos in [pytest.parent().unwrap(), signature_alone.parent().unwrap()] {
            assert!(!is_cargo_target(not_cargos), "{}", not_cargos.display());
        }
        assert!(!is_cargo_target(&linked), "a tag that is a link");
    }

    #[test]
    fn the_download_caches_cargo_tags_in_its_home_are_no_build_folder() {
        let fixture = TempDir::new("cargo-target-cargo-home");
        fixture.write("cold-cargo-home/.package-cache", "");
        let registry = fixture.write("cold-cargo-home/registry/CACHEDIR.TAG", CACHEDIR_TAG);
        let build_dir = fixture.write("cold-cargo-home/build/36/b2ed/CACHEDIR.TAG", CACHEDIR_TAG);

        assert!(holds_cargo_tag(registry.parent().unwrap()));
        assert!(!is_cargo_target(registry.parent().unwrap()));
        assert!(
            is_cargo_target(build_dir.parent().unwrap()),
            "a build-dir cargo keeps under its home"
        );
    }
}
