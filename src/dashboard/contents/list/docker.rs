//! Docker's own group, under a line of its own: it came from no upstream, and what it holds is
//! Docker's, not a project's. Open, it holds a card for each container running, whether a project
//! accounts for it or not (see `container`), and the disk Docker takes, in a row of its own with no
//! title, as its place under Docker's line says whose it is (see `disk`): first in the Disk sort,
//! which weighs it, and after the containers in the others. Folded shut, the line shows the
//! containers no project accounts for, a lozenge for each port they publish, as those a project
//! accounts for show on that project's card and line. It offers no clean, as Sutra cleans nothing
//! of Docker's, and nothing under it opens a folder, as Docker keeps what it holds in its own
//! virtual machine.

mod container;
mod disk;

pub use container::{AccountedFor, ContainerCard};

use std::collections::BTreeSet;

use super::line::first_few;
use super::rows::{ListRow, Place};
use crate::dashboard::contents::card::CardContext;
use crate::dashboard::contents::card::lozenges::containers::{self, ShownOn};
use crate::dashboard::listing::{DOCKER, DockerGroup};
use crate::dashboard::{GroupKind, Line, LineFold, LineKey, SortOrder, format};
use crate::map::docker::{DockerDisk, RunningContainer};

/// What Docker's line stands for, in its tooltip.
const SHOWN: &str = "The Docker containers running, and the disk Docker takes";

impl LineKey {
    /// The key of Docker's line, which stands for Docker's group.
    pub(in crate::dashboard) const DOCKER: LineKey = LineKey::Group {
        kind: GroupKind::Docker,
    };
}

/// The rows of Docker's group as the list shows it, `shown`, its line `folded` shut or open, in
/// `sort`: the line, then while it is open what it holds.
pub(super) fn rows(
    shown: &DockerGroup,
    folded: bool,
    sort: SortOrder,
    ctx: &CardContext,
) -> Vec<ListRow> {
    let line = ListRow::Line {
        line: Box::new(line(shown, folded, ctx)),
        place: Place {
            depth: 0,
            first: false,
        },
    };
    if folded {
        return vec![line];
    }
    let containers = shown.containers.iter().copied().map(Under::Container);
    let disk = shown.docker.disk.filter(|_| shown.disk).map(Under::Disk);
    let under: Vec<Under> = match sort {
        SortOrder::Disk => disk.into_iter().chain(containers).collect(),
        SortOrder::Activity | SortOrder::Name => containers.chain(disk).collect(),
    };
    let under = under.into_iter().enumerate().map(|(at, row)| {
        let place = Place {
            depth: 1,
            first: at == 0,
        };
        match row {
            Under::Container(running) => ListRow::Container {
                card: Box::new(container::card(running, ctx)),
                place,
            },
            Under::Disk(taken) => ListRow::DockerDisk {
                tray: disk::tray(&taken),
                location: format::tilde(&shown.docker.socket, ctx.home),
                place,
            },
        }
    });
    std::iter::once(line).chain(under).collect()
}

/// A row under Docker's line.
enum Under<'a> {
    Container(&'a RunningContainer),
    Disk(DockerDisk),
}

/// Docker's line, `folded` shut or open: Docker's name, and the disk Docker takes, as its row
/// totals it; folded shut, the first lozenges of the containers listed under it that no project
/// accounts for, named by their images as on a project's card, with no agent, as no agent works in
/// Docker. It offers no clean, nor to clean by itself once idle: Sutra cleans nothing of Docker's.
fn line(shown: &DockerGroup, folded: bool, ctx: &CardContext) -> Line {
    let fold = if folded {
        let listed = shown.containers.iter();
        let unaccounted = listed.filter(|running| running.projects.is_empty());
        let unaccounted = unaccounted.map(|running| &running.container);
        let none_shown = BTreeSet::new();
        let lozenges =
            containers::lozenges(unaccounted, ShownOn::DockersLine, &none_shown, ctx.home);
        let (lozenges, more_lozenges) = first_few(lozenges);
        LineFold::Folded {
            sessions: Vec::new(),
            subagents: Vec::new(),
            waiting: false,
            lozenges,
            more_lozenges,
        }
    } else {
        LineFold::Open
    };
    let docker = shown.docker;
    Line {
        key: LineKey::DOCKER,
        name: DOCKER.to_string(),
        tooltip: format!("{SHOWN}\n{}", format::tilde(&docker.socket, ctx.home)),
        total: docker.disk.map(|disk| format::size(disk.bytes(), false)),
        when_idle_mark: None,
        menu: Vec::new(),
        fold,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::disk::tests::disk;
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::now;
    use crate::map::Docker;
    use crate::map::docker::{Container, PublishedPort};
    use crate::notifications::Notifier;

    /// The running container `name`, of `name:1`, publishing `port` to a web server's inside it,
    /// accounted for by `projects`.
    fn running(name: &str, port: u16, projects: &[&str]) -> RunningContainer {
        RunningContainer {
            container: Container {
                id: name.to_string(),
                name: name.to_string(),
                image: format!("{name}:1"),
                compose_folder: None,
                bound: Vec::new(),
                ports: vec![PublishedPort {
                    port,
                    to: 8080,
                    addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
                    udp: false,
                }],
                started: Some(now()),
            },
            projects: projects.iter().map(PathBuf::from).collect(),
        }
    }

    /// Docker at `~/.docker/run/docker.sock`, running `running`, and taking what it took on
    /// Daniel's Mac on 2026-11-07.
    fn docker(running: Vec<RunningContainer>) -> Docker {
        Docker {
            socket: PathBuf::from("/Users/dev/.docker/run/docker.sock"),
            running,
            disk: Some(disk()),
        }
    }

    /// The rows of all of `docker`'s group, its line `folded` or open, in `sort`.
    fn rows_of(docker: &Docker, folded: bool, sort: SortOrder) -> Vec<ListRow> {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        };
        rows(&DockerGroup::whole(docker), folded, sort, &ctx)
    }

    /// `rows` as `line`, `container name` and `disk`, each with its place.
    fn shapes(rows: &[ListRow]) -> Vec<(String, Place)> {
        let shape = |row: &ListRow| match row {
            ListRow::Line { line, place } => (format!("line {}", line.name), *place),
            ListRow::Container { card, place } => (format!("container {}", card.name), *place),
            ListRow::DockerDisk { place, .. } => ("disk".to_string(), *place),
            other => panic!("not of Docker's group: {other:?}"),
        };
        rows.iter().map(shape).collect()
    }

    fn at(depth: u32, first: bool) -> Place {
        Place { depth, first }
    }

    #[test]
    fn open_dockers_line_holds_a_card_for_every_container_then_its_disk_first_in_the_disk_sort() {
        let docker = docker(vec![
            running("api", 8440, &[]),
            running("payment", 8090, &["/Users/dev/GT/repo/gt-core"]),
        ]);

        let by_activity = rows_of(&docker, false, SortOrder::Activity);
        assert_eq!(
            shapes(&by_activity),
            [
                ("line Docker".to_string(), at(0, false)),
                ("container api".to_string(), at(1, true)),
                ("container payment".to_string(), at(1, false)),
                ("disk".to_string(), at(1, false)),
            ],
            "payment too, which gt-core accounts for"
        );
        assert_eq!(
            shapes(&rows_of(&docker, false, SortOrder::Name)),
            shapes(&by_activity)
        );
        assert_eq!(
            shapes(&rows_of(&docker, false, SortOrder::Disk)),
            [
                ("line Docker".to_string(), at(0, false)),
                ("disk".to_string(), at(1, true)),
                ("container api".to_string(), at(1, false)),
                ("container payment".to_string(), at(1, false)),
            ],
            "the size it is"
        );
        let Some(ListRow::DockerDisk { tray, location, .. }) = by_activity.last() else {
            unreachable!()
        };
        assert_eq!(tray.disk.as_ref().unwrap().total.as_deref(), Some("48 GB"));
        assert_eq!(location, "~/.docker/run/docker.sock");
    }

    #[test]
    fn dockers_line_folded_shows_only_the_containers_no_project_accounts_for_and_cleans_nothing() {
        let docker = docker(vec![
            running("api", 8440, &[]),
            running("payment", 8090, &["/Users/dev/GT/repo/gt-core"]),
            running("redis", 8001, &[]),
            running("minio", 9001, &[]),
        ]);

        let folded = rows_of(&docker, true, SortOrder::Activity);

        let [ListRow::Line { line, place }] = folded.as_slice() else {
            panic!("the line alone: {folded:?}");
        };
        assert_eq!((line.key.clone(), *place), (LineKey::DOCKER, at(0, false)));
        assert_eq!(line.name, "Docker");
        assert_eq!(line.total.as_deref(), Some("48 GB"));
        assert_eq!(
            line.tooltip,
            "The Docker containers running, and the disk Docker takes\n~/.docker/run/docker.sock"
        );
        assert_eq!(line.menu, [], "Sutra cleans nothing of Docker's");
        let LineFold::Folded {
            lozenges,
            more_lozenges,
            sessions,
            ..
        } = &line.fold
        else {
            panic!("folded: {:?}", line.fold);
        };
        let named: Vec<String> = lozenges
            .iter()
            .map(|l| {
                let name = l.lozenge.name.as_deref().unwrap();
                format!("{name} {}", l.lozenge.port.as_deref().unwrap())
            })
            .collect();
        assert_eq!(
            named,
            ["api :8440", "redis :8001"],
            "payment shows on gt-core's"
        );
        let more = more_lozenges.as_ref().unwrap();
        assert_eq!(
            (more.label.as_str(), more.tooltip.as_str()),
            ("+1", "minio :9001")
        );
        assert_eq!(sessions, &[], "no agent works in Docker");
    }

    #[test]
    fn a_docker_that_has_not_said_what_it_takes_lists_no_disk_row_and_no_total() {
        let docker = Docker {
            disk: None,
            ..docker(vec![running("pg596", 55596, &[])])
        };

        let open = rows_of(&docker, false, SortOrder::Disk);

        assert_eq!(
            shapes(&open),
            [
                ("line Docker".to_string(), at(0, false)),
                ("container pg596".to_string(), at(1, true)),
            ]
        );
        let ListRow::Line { line, .. } = &open[0] else {
            unreachable!()
        };
        assert_eq!(line.total, None);
    }
}
