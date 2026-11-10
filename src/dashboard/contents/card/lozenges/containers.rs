//! The Docker containers on a project's card or on Docker's folded line, a lozenge for each port
//! one publishes, as `(postgres :36684)` reads: up, as the container runs, and named by the image
//! it runs, short, as a process's lozenge is by the process. One that publishes no port shows too,
//! on a lozenge with none, as it runs all the same. The tooltip says who can reach the port, then
//! the container's name and its image, where the port goes inside it, and the folder it is for. On
//! the container's own card, which names it already, each lozenge says the port alone (see
//! `on_its_card`).

use std::collections::BTreeSet;
use std::path::Path;

use super::{Lozenge, LozengeLook, link, reach};
use crate::dashboard::format;
use crate::map::docker::{Container, PublishedPort};

/// Where the lozenges stand, which says which of a container's folders its tooltip names.
#[derive(Debug, Clone, Copy)]
pub(in crate::dashboard::contents) enum ShownOn<'a> {
    /// The card of the project at this folder: the first of the container's folders there.
    ProjectCard(&'a Path),
    /// Docker's folded line, for the containers no project accounts for: the first of their
    /// folders, if any.
    DockersLine,
}

/// The lozenges of `containers`, in their order, shown `on` a project's card or Docker's line,
/// leaving out each port in `shown`.
pub(in crate::dashboard::contents) fn lozenges<'c>(
    containers: impl IntoIterator<Item = &'c Container>,
    on: ShownOn,
    shown: &BTreeSet<u16>,
    home: &Path,
) -> Vec<Lozenge> {
    containers
        .into_iter()
        .flat_map(|container| of(container, on, shown, home))
        .collect()
}

/// The lozenges of `container`: one for each TCP port it publishes but those in `shown`, or one
/// with no port when it publishes none.
pub(super) fn of(
    container: &Container,
    on: ShownOn,
    shown: &BTreeSet<u16>,
    home: &Path,
) -> Vec<Lozenge> {
    let folder = match on {
        ShownOn::ProjectCard(project) => container.folder_in(project),
        ShownOn::DockersLine => container.folders().next(),
    };
    let folder = folder.map(|folder| format::tilde(folder, home));
    let named = format!("Container {}, of {}", container.name, container.image);
    let tcp: Vec<&PublishedPort> = container.ports.iter().filter(|port| !port.udp).collect();
    let name = image_name(container);
    if tcp.is_empty() {
        let mut tooltip = vec![format!("{named}, publishing no TCP port")];
        tooltip.extend(folder);
        return vec![lozenge(Some(name), None, tooltip)];
    }
    let unshown = tcp.into_iter().filter(|port| !shown.contains(&port.port));
    unshown
        .map(|port| {
            let mut tooltip = vec![reach(&port.addresses), named.clone(), goes_to(port)];
            tooltip.extend(folder.clone());
            lozenge(Some(name), Some(port.port), tooltip)
        })
        .collect()
}

/// The lozenges of `container` on its own card: one for each TCP port it publishes, with no name,
/// as the card names the container, and a tooltip that says who can reach the port and where it
/// goes inside.
pub(in crate::dashboard::contents) fn on_its_card(container: &Container) -> Vec<Lozenge> {
    let tcp = container.ports.iter().filter(|port| !port.udp);
    tcp.map(|port| {
        let tooltip = vec![reach(&port.addresses), goes_to(port)];
        lozenge(None, Some(port.port), tooltip)
    })
    .collect()
}

/// Where `port` goes inside its container, as `:46823 goes to 5432 inside it` reads.
fn goes_to(port: &PublishedPort) -> String {
    format!(":{} goes to {} inside it", port.port, port.to)
}

fn lozenge(name: Option<&str>, port: Option<u16>, tooltip: Vec<String>) -> Lozenge {
    Lozenge {
        name: name.map(str::to_string),
        port: port.map(|port| format!(":{port}")),
        look: LozengeLook::Up,
        beside: None,
        muted: false,
        notifications_off: false,
        tooltip: tooltip.join("\n"),
        link: port.map(link),
    }
}

/// What a container's image goes by on its lozenge: the last part of its name, without the registry
/// and the path before it, or the tag or the digest after it, as
/// `gcr.io/retailops-1/testdb:standard-local` goes by `testdb`. An image the engine names only by
/// its id says nothing of what runs, so the container goes by its own name.
fn image_name(container: &Container) -> &str {
    let image = container.image.split('@').next().unwrap_or_default();
    let last = image.rsplit('/').next().unwrap_or_default();
    let name = last.split(':').next().unwrap_or_default();
    let is_an_id =
        name == "sha256" || (name.len() >= 12 && name.bytes().all(|b| b.is_ascii_hexdigit()));
    if name.is_empty() || is_an_id {
        &container.name
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const HOME: &str = "/Users/dev";

    /// The running container `name`, of `image`, bind-mounting `bound`, publishing `ports`.
    fn container(name: &str, image: &str, bound: &[&str], ports: Vec<PublishedPort>) -> Container {
        Container {
            id: name.to_string(),
            name: name.to_string(),
            image: image.to_string(),
            compose_folder: None,
            bound: bound.iter().map(PathBuf::from).collect(),
            ports,
            started: None,
        }
    }

    /// `port` published at `addresses`, going to `to` inside the container.
    fn published(port: u16, to: u16, addresses: &[&str], udp: bool) -> PublishedPort {
        PublishedPort {
            port,
            to,
            addresses: addresses.iter().map(|ip| ip.parse().unwrap()).collect(),
            udp,
        }
    }

    #[test]
    fn a_published_port_shows_up_by_its_images_name_opens_in_the_browser_and_says_where_it_goes() {
        let database = container(
            "database",
            "gcr.io/retailops-1/testdb:standard-local",
            &["/Users/dev/GT/backplane", "/Users/dev/code/app/db"],
            vec![published(3306, 3306, &["0.0.0.0", "::"], false)],
        );

        let on_app = ShownOn::ProjectCard(Path::new("/Users/dev/code/app"));
        let [lozenge] = lozenges(&[database], on_app, &BTreeSet::new(), Path::new(HOME))
            .try_into()
            .unwrap();

        assert_eq!(
            (lozenge.name.as_deref(), lozenge.port.as_deref()),
            (Some("testdb"), Some(":3306"))
        );
        assert_eq!(lozenge.look, LozengeLook::Up);
        assert_eq!(lozenge.link.as_deref(), Some("http://localhost:3306"));
        assert_eq!(
            lozenge.tooltip,
            "Listening on the network, at 0.0.0.0 and ::\n\
             Container database, of gcr.io/retailops-1/testdb:standard-local\n\
             :3306 goes to 3306 inside it\n\
             ~/code/app/db",
            "the folder of the card's it is for"
        );
    }

    #[test]
    fn a_container_publishing_no_tcp_port_still_shows_and_one_whose_ports_show_already_does_not() {
        let payment = container(
            "payment",
            "gcr.io/retailops-1/gt-payment-service:2021-08-02-1",
            &["/Users/dev/GT/repo/gt-core"],
            vec![published(53, 53, &["0.0.0.0"], true)],
        );
        let web = container(
            "web",
            "nginx",
            &["/Users/dev/GT/repo/gt-core/public"],
            vec![published(8080, 80, &["127.0.0.1"], false)],
        );
        let shown = BTreeSet::from([8080]);
        let core = Path::new("/Users/dev/GT/repo/gt-core");

        let on_core = ShownOn::ProjectCard(core);
        let lozenges = lozenges(&[payment, web], on_core, &shown, Path::new(HOME));

        let [lozenge] = lozenges.as_slice() else {
            panic!("payment's alone, :8080 shown already: {lozenges:?}");
        };
        assert_eq!(
            (lozenge.name.as_deref(), &lozenge.port),
            (Some("gt-payment-service"), &None)
        );
        assert_eq!(lozenge.link, None);
        assert_eq!(
            lozenge.tooltip,
            "Container payment, of gcr.io/retailops-1/gt-payment-service:2021-08-02-1, \
             publishing no TCP port\n\
             ~/GT/repo/gt-core"
        );
    }

    #[test]
    fn on_dockers_line_a_container_names_the_first_folder_it_is_for_none_of_which_a_project_holds()
    {
        let api = container(
            "api",
            "gcr.io/retailops-1/gt-dispatcher:2020-06-15",
            &["/Users/dev/GT/backplane", "/Users/dev/GT/private/api"],
            vec![published(8440, 8443, &["0.0.0.0", "::"], false)],
        );

        let lozenges = lozenges(
            &[api],
            ShownOn::DockersLine,
            &BTreeSet::new(),
            Path::new(HOME),
        );

        let tooltip = &lozenges[0].tooltip;
        assert!(tooltip.ends_with("\n~/GT/backplane"), "{tooltip}");
    }

    #[test]
    fn on_its_own_card_a_container_shows_each_tcp_port_it_publishes_by_the_port_alone() {
        let api = container(
            "api",
            "gcr.io/retailops-1/gt-dispatcher:2020-06-15",
            &["/Users/dev/GT/backplane"],
            vec![
                published(8440, 8443, &["0.0.0.0", "::"], false),
                published(8443, 8443, &["0.0.0.0"], false),
                published(5353, 5353, &["0.0.0.0"], true),
            ],
        );
        let quiet = container(
            "rop-login",
            "gcr.io/retailops-1/rop-login-dev",
            &[],
            Vec::new(),
        );

        let shown = on_its_card(&api);

        let named: Vec<(Option<&str>, &str, Option<&str>)> = shown
            .iter()
            .map(|l| {
                (
                    l.name.as_deref(),
                    l.port.as_deref().unwrap(),
                    l.link.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            named,
            [
                (None, ":8440", Some("http://localhost:8440")),
                (None, ":8443", Some("http://localhost:8443")),
            ],
            "no UDP port"
        );
        assert_eq!(
            shown[0].tooltip,
            "Listening on the network, at 0.0.0.0 and ::\n:8440 goes to 8443 inside it"
        );
        assert_eq!(on_its_card(&quiet), [], "its card says it runs");
    }

    #[test]
    fn an_image_goes_by_the_last_part_of_its_name_and_one_known_by_its_id_by_its_containers_name() {
        let named = |image: &str| {
            let container = container("pg596", image, &[], Vec::new());
            image_name(&container).to_string()
        };
        assert_eq!(named("postgres:17"), "postgres");
        assert_eq!(named("ghcr.io/acme/api@sha256:0b65"), "api");
        assert_eq!(named("localhost:5000/team/app:dev"), "app");
        assert_eq!(named("sha256:0b657ff48d7f76a1e907"), "pg596");
        assert_eq!(named("0b657ff48d7f"), "pg596");
    }
}
