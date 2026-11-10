//! The disk Docker takes, on a tray as a project's card shows its folder's, in a row of its own
//! under Docker's line: the total, then the stores large enough to name, largest first, as
//! `48 GB  volumes  images  build cache` reads. Each store's tooltip gives its size, how many it
//! holds and how many are in use, and what a prune would give back: Sutra itself cleans nothing of
//! Docker's, as its volumes hold data.

use std::cmp::Reverse;

use crate::dashboard::contents::card::worth_showing;
use crate::dashboard::{DiskUse, Tray, TrayEntry, format};
use crate::map::docker::{DockerDisk, Store};

/// The tray of `disk`, with no agent on it, as no agent works in Docker.
pub(super) fn tray(disk: &DockerDisk) -> Tray {
    Tray {
        disk: Some(disk_use(disk)),
        sessions: Vec::new(),
        subagents: Vec::new(),
    }
}

/// One of Docker's stores, as its tray names it.
struct Named {
    /// As the tray writes it: `volumes`.
    label: &'static str,
    /// What one of its things is: `volume`.
    noun: &'static str,
    /// What one in use is: `in use`, or for a container, `running`.
    in_use: &'static str,
    store: Store,
}

/// Docker's disk use as a tray shows it: the total, then the stores large enough to name, largest
/// first; the others, and when Docker said so, in the total's tooltip.
fn disk_use(disk: &DockerDisk) -> DiskUse {
    let named = |label, noun, in_use, store| Named {
        label,
        noun,
        in_use,
        store,
    };
    let stores = [
        named("images", "image", "in use", disk.images),
        named("containers", "container", "running", disk.containers),
        named("volumes", "volume", "in use", disk.volumes),
        named("build cache", "record", "in use", disk.build_cache),
    ];
    let (mut shown, small): (Vec<Named>, Vec<Named>) = stores
        .into_iter()
        .partition(|named| worth_showing(named.store.bytes));
    shown.sort_by_key(|named| Reverse(named.store.bytes));
    let small = small.iter().map(|named| {
        let size = format::size(named.store.bytes, false);
        format!("{} {size}", named.label)
    });
    let said = format::timestamp(disk.read_at).map(|at| format!("As Docker said at {at}"));
    let total_tooltip: Vec<String> = small.chain(said).collect();
    DiskUse {
        total: Some(format::size(disk.bytes(), false)),
        total_tooltip: (!total_tooltip.is_empty()).then(|| total_tooltip.join("\n")),
        warning: None,
        entries: shown
            .iter()
            .map(|named| TrayEntry {
                label: named.label.to_string(),
                tooltip: store_tooltip(named),
                cleaning: false,
            })
            .collect(),
        progress: None,
    }
}

/// What a store's tooltip says: its size, how many it holds and how many are in use, then what a
/// prune would give back, as `23 GB in 68 volumes, 21 in use` and `5 GB reclaimable` read.
fn store_tooltip(named: &Named) -> String {
    let store = &named.store;
    let in_use = match store.in_use {
        0 => format!("none {}", named.in_use),
        n => format!("{n} {}", named.in_use),
    };
    format!(
        "{} in {}, {in_use}\n{} reclaimable",
        format::size(store.bytes, false),
        format::plural(store.count, named.noun),
        format::size(store.reclaimable, false)
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::dashboard::listing::fixtures::now;

    fn store(bytes: u64, count: u64, in_use: u64, reclaimable: u64) -> Store {
        Store {
            bytes,
            count,
            in_use,
            reclaimable,
        }
    }

    /// Docker's disk use on Daniel's Mac on 2026-11-07, as its engine said at `now()`.
    pub(in crate::dashboard::contents::list::docker) fn disk() -> DockerDisk {
        DockerDisk {
            images: store(19_143_582_596, 20, 14, 11_446_905),
            containers: store(659_615_744, 33, 14, 340_410_368),
            volumes: store(22_705_049_435, 68, 21, 5_039_641_769),
            build_cache: store(5_415_259_336, 28, 0, 5_415_259_336),
            read_at: now(),
        }
    }

    #[test]
    fn the_tray_gives_the_total_then_the_stores_large_enough_to_name_largest_first() {
        let Tray {
            disk: Some(shown),
            sessions,
            subagents,
        } = tray(&disk())
        else {
            panic!("a tray of Docker's disk use");
        };

        assert_eq!(shown.total.as_deref(), Some("48 GB"));
        let labels: Vec<&str> = shown
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(labels, ["volumes", "images", "build cache"]);
        assert_eq!(
            shown.entries[0].tooltip,
            "23 GB in 68 volumes, 21 in use\n5 GB reclaimable"
        );
        assert_eq!(
            shown.entries[2].tooltip,
            "5.4 GB in 28 records, none in use\n5.4 GB reclaimable"
        );
        let said = format::timestamp(now()).unwrap();
        assert_eq!(
            shown.total_tooltip,
            Some(format!("containers 660 MB\nAs Docker said at {said}"))
        );
        assert!(
            shown.entries.iter().all(|entry| !entry.cleaning),
            "Sutra cleans none"
        );
        assert!(
            sessions.is_empty() && subagents.is_empty(),
            "no agent works in Docker"
        );
    }
}
