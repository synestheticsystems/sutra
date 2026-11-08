//! Docker's own card: the containers running for no project on the map, a lozenge for each port
//! they publish, as a project's card shows what serves in it, and along its bottom the disk Docker
//! takes, as a project's tray shows its folder's: the total, then the stores large enough to name,
//! largest first, as `48 GB  volumes  images  build cache` reads. Each store's tooltip gives its
//! size, how many it holds and how many are in use, and what a prune would give back: Sutra itself
//! cleans nothing of Docker's, as its volumes hold data. The card opens no folder, as Docker keeps
//! what it holds in its own virtual machine.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use crate::dashboard::contents::card::lozenges::containers::{self, CardOf};
use crate::dashboard::contents::card::{CardContext, worth_showing};
use crate::dashboard::listing::DOCKER;
use crate::dashboard::{DiskUse, Tray, TrayFolder, UnitLozenge, format};
use crate::map::Docker;
use crate::map::docker::{DockerDisk, Store};

/// Docker's card.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DockerCard {
    /// `Docker`.
    pub name: String,
    /// What the card shows.
    pub name_tooltip: String,
    /// How long ago the newest container on the card was made, as `8d` reads.
    pub age: Option<String>,
    /// The containers running for no project, a lozenge for each port one publishes, by name. The
    /// name keeps the word from when every lozenge was a unit's.
    pub units: Vec<UnitLozenge>,
    /// The disk Docker takes, as a project's tray holds its folder's: `None` until Docker has said.
    pub tray: Option<Tray>,
    /// The socket Docker is read through, with `~` for the home folder: shown while the pointer is
    /// over the card, where a project's card shows its folder.
    pub location: String,
}

/// The card of `docker`.
pub(super) fn card(docker: &Docker, ctx: &CardContext) -> DockerCard {
    let newest = docker
        .containers
        .iter()
        .map(|container| container.created)
        .max();
    let ports = BTreeSet::new();
    DockerCard {
        name: DOCKER.to_string(),
        name_tooltip: "The Docker containers running for no project on the list, and the disk \
                       Docker takes"
            .to_string(),
        age: newest.map(|then| format::age(then, ctx.now)),
        units: containers::lozenges(&docker.containers, CardOf::Docker, &ports, ctx.home),
        tray: docker.disk.map(|disk| Tray {
            disk: Some(disk_use(&disk)),
            sessions: Vec::new(),
            subagents: Vec::new(),
        }),
        location: format::tilde(&docker.socket, ctx.home),
    }
}

/// One of Docker's stores, as its card names it.
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
        folders: shown
            .iter()
            .map(|named| TrayFolder {
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
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::now;
    use crate::map::docker::{Container, PublishedPort};
    use crate::notifications::Notifier;

    fn store(bytes: u64, count: u64, in_use: u64, reclaimable: u64) -> Store {
        Store {
            bytes,
            count,
            in_use,
            reclaimable,
        }
    }

    /// Docker's disk use on Daniel's Mac on 2026-11-07, as its engine said at `now()`.
    fn disk() -> DockerDisk {
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
        let shown = disk_use(&disk());

        assert_eq!(shown.total.as_deref(), Some("48 GB"));
        let labels: Vec<&str> = shown.folders.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["volumes", "images", "build cache"]);
        assert_eq!(
            shown.folders[0].tooltip,
            "23 GB in 68 volumes, 21 in use\n5 GB reclaimable"
        );
        assert_eq!(
            shown.folders[2].tooltip,
            "5.4 GB in 28 records, none in use\n5.4 GB reclaimable"
        );
        let said = format::timestamp(now()).unwrap();
        assert_eq!(
            shown.total_tooltip,
            Some(format!("containers 660 MB\nAs Docker said at {said}"))
        );
        assert!(
            shown.folders.iter().all(|folder| !folder.cleaning),
            "Sutra cleans none"
        );
    }

    #[test]
    fn the_card_shows_the_containers_for_no_project_and_opens_no_folder() {
        let pg = Container {
            id: "e99d".to_string(),
            name: "pg596".to_string(),
            image: "postgres:17".to_string(),
            compose_folder: None,
            bound: Vec::new(),
            ports: vec![PublishedPort {
                port: 55596,
                to: 5432,
                addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
                udp: false,
            }],
            created: now() - Duration::from_secs(9 * 24 * 60 * 60),
        };
        let docker = Docker {
            socket: PathBuf::from("/Users/dev/.docker/run/docker.sock"),
            containers: vec![pg],
            disk: None,
        };
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        };

        let card = card(&docker, &ctx);

        assert_eq!(card.name, "Docker");
        assert_eq!(card.age.as_deref(), Some("9d"));
        let units: Vec<(&str, Option<&str>)> = card
            .units
            .iter()
            .map(|unit| (unit.name.as_str(), unit.port.as_deref()))
            .collect();
        assert_eq!(units, [("postgres", Some(":55596"))]);
        assert_eq!(card.tray, None, "Docker has not said what it takes");
        assert_eq!(card.location, "~/.docker/run/docker.sock");
    }
}
