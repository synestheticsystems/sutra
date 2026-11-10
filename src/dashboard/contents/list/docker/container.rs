//! A Docker container's card, under Docker's line, drawn as a project's card is: the container's
//! own name, the image it runs where a checkout's card carries its branch, and how long it has run
//! where a card shows its age, so one left running for weeks stands out by itself; then a lozenge
//! for each port it publishes, saying the port alone, as the card names the container; and at the
//! end of that line, the projects that account for it, whose cards show it too. It opens no folder
//! and offers nothing on a right click, as Sutra only reads Docker.

use std::path::{Path, PathBuf};

use crate::dashboard::contents::card::CardContext;
use crate::dashboard::contents::card::lozenges::containers;
use crate::dashboard::{Lozenge, format};
use crate::map::docker::RunningContainer;

/// A Docker container's card.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ContainerCard {
    /// The container's id, which the window knows the card by.
    pub id: String,
    /// The container's own name, as `pg596` reads.
    pub name: String,
    /// Its name and the image it runs, then when it started.
    pub name_tooltip: String,
    /// The image it runs, beside the name where a checkout's card carries its branch: its reference
    /// without the registry and the path before its name, so the name leads, as `postgres:17` and
    /// `gt-dispatcher:2020-06-15` read (see `short_reference`).
    pub image: String,
    /// The image's reference in full, as the engine names it.
    pub image_tooltip: String,
    /// How long it has run since it last started, as `14d` reads; `None` when the engine would not
    /// say when that was.
    pub age: Option<String>,
    /// A lozenge for each TCP port it publishes, saying the port alone.
    pub lozenges: Vec<Lozenge>,
    /// The projects that account for it, whose cards show it too; `None` when none does.
    pub accounted_for: Option<AccountedFor>,
}

/// The projects that account for a container, as its card names them.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountedFor {
    /// Their folders' own names, as `foundry-studio-streams-provider` reads.
    pub names: String,
    /// Their folders in full, a line each, with `~` for the home folder.
    pub tooltip: String,
}

/// The card of `running`.
pub(super) fn card(running: &RunningContainer, ctx: &CardContext) -> ContainerCard {
    let container = &running.container;
    let named = format!("Container {}, of {}", container.name, container.image);
    let started = container.started.and_then(format::timestamp);
    ContainerCard {
        id: container.id.clone(),
        name: container.name.clone(),
        name_tooltip: match started {
            Some(at) => format!("{named}\nStarted {at}"),
            None => named,
        },
        image: short_reference(&container.image),
        image_tooltip: container.image.clone(),
        age: container.started.map(|then| format::age(then, ctx.now)),
        lozenges: containers::on_its_card(container),
        accounted_for: accounted_for(&running.projects, ctx.home),
    }
}

/// How many hexadecimal digits of a digest or an id tell one image from another, as Docker
/// shortens an id.
const SHORT_HEX: usize = 12;

/// `image`, a reference as the engine names an image, without the registry and the path before
/// the image's name: `gcr.io/retailops-1/gt-dispatcher:2020-06-15` reads
/// `gt-dispatcher:2020-06-15`. A reference pinned by a digest keeps its tag where it has one, as
/// the tag says more to a reader, and else the digest's first digits, as
/// `ghcr.io/acme/api@sha256:0b657ff48d7f76a1` reads `api@0b657ff48d7f`; an image the engine
/// knows only by its id reads as that id, shortened.
fn short_reference(image: &str) -> String {
    let short = |hex: &str| hex.chars().take(SHORT_HEX).collect::<String>();
    let (named, digest) = match image.split_once('@') {
        Some((named, digest)) => (named, Some(digest)),
        None => (image, None),
    };
    if let Some(id) = named.strip_prefix("sha256:") {
        return short(id);
    }
    let last = named.rsplit('/').next().unwrap_or(named);
    match digest {
        Some(digest) if !last.contains(':') => {
            let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
            format!("{last}@{}", short(hex))
        }
        _ => last.to_string(),
    }
}

/// What a card says of `projects`, those accounting for its container, by their folders.
fn accounted_for(projects: &[PathBuf], home: &Path) -> Option<AccountedFor> {
    let folders = projects.iter().map(PathBuf::as_path);
    let names: Vec<String> = folders
        .clone()
        .map(|folder| match folder.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => format::tilde(folder, home),
        })
        .collect();
    let in_full: Vec<String> = folders.map(|folder| format::tilde(folder, home)).collect();
    (!names.is_empty()).then(|| AccountedFor {
        names: format::all(&names.iter().map(String::as_str).collect::<Vec<_>>()),
        tooltip: in_full.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::now;
    use crate::map::docker::{Container, PublishedPort};
    use crate::notifications::Notifier;

    /// `studio-pg-74a9`, started `days` ago, publishing 127.0.0.1:46823, accounted for by
    /// `projects`.
    fn studio_pg(days: u64, projects: &[&str]) -> RunningContainer {
        RunningContainer {
            container: Container {
                id: "74a9f2fff535".to_string(),
                name: "studio-pg-74a9f2fff5352a75".to_string(),
                image: "postgres:17".to_string(),
                compose_folder: None,
                bound: Vec::new(),
                ports: vec![PublishedPort {
                    port: 46823,
                    to: 5432,
                    addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
                    udp: false,
                }],
                started: Some(now() - Duration::from_secs(days * 24 * 60 * 60)),
            },
            projects: projects.iter().map(PathBuf::from).collect(),
        }
    }

    fn card_of(running: &RunningContainer) -> ContainerCard {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        };
        card(running, &ctx)
    }

    #[test]
    fn a_containers_card_names_it_its_image_how_long_it_has_run_and_each_port_alone() {
        let card = card_of(&studio_pg(14, &[]));

        assert_eq!(card.id, "74a9f2fff535");
        assert_eq!(card.name, "studio-pg-74a9f2fff5352a75");
        assert_eq!(
            (card.image.as_str(), card.image_tooltip.as_str()),
            ("postgres:17", "postgres:17")
        );
        assert_eq!(card.age.as_deref(), Some("14d"));
        let started = format::timestamp(now() - Duration::from_secs(14 * 86_400)).unwrap();
        assert_eq!(
            card.name_tooltip,
            format!("Container studio-pg-74a9f2fff5352a75, of postgres:17\nStarted {started}")
        );
        let ports: Vec<(Option<&str>, Option<&str>)> = card
            .lozenges
            .iter()
            .map(|lozenge| (lozenge.name.as_deref(), lozenge.port.as_deref()))
            .collect();
        assert_eq!(ports, [(None, Some(":46823"))]);
        assert_eq!(card.accounted_for, None, "no project accounts for it");
    }

    #[test]
    fn a_container_a_project_accounts_for_names_its_folder_with_the_folder_in_full_in_the_tip() {
        let one = card_of(&studio_pg(
            0,
            &["/Users/dev/vt/foundry-studio-streams-provider"],
        ));
        assert_eq!(
            one.accounted_for,
            Some(AccountedFor {
                names: "foundry-studio-streams-provider".to_string(),
                tooltip: "~/vt/foundry-studio-streams-provider".to_string(),
            })
        );

        let two = card_of(&studio_pg(
            0,
            &["/Users/dev/code/app/api", "/Users/dev/code/core"],
        ));
        assert_eq!(
            two.accounted_for,
            Some(AccountedFor {
                names: "api and core".to_string(),
                tooltip: "~/code/app/api\n~/code/core".to_string(),
            })
        );
    }

    #[test]
    fn the_image_chip_leads_with_the_images_name_and_a_digest_with_its_first_digits() {
        let cases = [
            (
                "gcr.io/retailops-1/gt-dispatcher:2020-06-15-1153-ae68d96",
                "gt-dispatcher:2020-06-15-1153-ae68d96",
            ),
            ("postgres:17", "postgres:17"),
            ("localhost:5000/team/app:dev", "app:dev"),
            (
                "ghcr.io/acme/api@sha256:0b657ff48d7f76a1e907",
                "api@0b657ff48d7f",
            ),
            (
                "ghcr.io/acme/api:1.4@sha256:0b657ff48d7f76a1e907",
                "api:1.4",
            ),
            ("sha256:0b657ff48d7f76a1e907", "0b657ff48d7f"),
        ];
        for (image, shown) in cases {
            assert_eq!(short_reference(image), shown, "{image}");
        }
    }

    #[test]
    fn a_container_whose_start_the_engine_would_not_say_shows_no_age() {
        let mut running = studio_pg(3, &[]);
        running.container.started = None;

        let card = card_of(&running);

        assert_eq!(card.age, None);
        assert_eq!(
            card.name_tooltip,
            "Container studio-pg-74a9f2fff5352a75, of postgres:17"
        );
    }
}
