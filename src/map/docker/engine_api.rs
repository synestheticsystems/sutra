//! What the engine's API answers, as the map keeps it: the containers `/containers/json` lists,
//! when one of them started, as `/containers/{id}/json` tells of it alone, and the disk use
//! `/system/df` gives. Only the fields read are named; the engine sends many more.
//!
//! The engine has answered `/system/df` in two shapes. Since API 1.52 (Docker 29) it sums up each
//! kind of store itself (`ImageUsage` and the rest), as `docker system df` shows them, and from
//! 1.53 on it may send nothing else; before, it listed every image, container, volume and build
//! record, which the CLI added up. Each store is read from the sums when the engine sends them,
//! and else added up from the lists as the CLI did.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;

use super::{Container, DockerDisk, PublishedPort, Store};
use crate::map::utc;

/// The label compose gives each container it makes, naming the folder `docker compose` ran in.
const COMPOSE_WORKING_DIR: &str = "com.docker.compose.project.working_dir";

/// Where Docker Desktop's virtual machine sees the folders of this Mac it shares, which it names a
/// bind mount's source by: `/host_mnt/Users/dev/code/app` is `/Users/dev/code/app`.
const DOCKER_DESKTOP_HOST_FOLDERS: &str = "/host_mnt";

/// The containers in `body`, an answer to `/containers/json`, which lists the running ones.
pub(super) fn containers(body: &[u8]) -> serde_json::Result<Vec<Container>> {
    let listed: Vec<ListedContainer> = serde_json::from_slice(body)?;
    let mut containers: Vec<Container> = listed.into_iter().map(ListedContainer::into).collect();
    containers.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    Ok(containers)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListedContainer {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    image: String,
    /// `null` for a container that publishes none, as for one that exposes none.
    #[serde(default)]
    ports: Option<Vec<ListedPort>>,
    #[serde(default)]
    labels: Option<HashMap<String, String>>,
    #[serde(default)]
    mounts: Option<Vec<ListedMount>>,
}

#[derive(Deserialize)]
struct ListedPort {
    /// Where on this Mac a published port listens; missing or empty for a port only exposed.
    #[serde(rename = "IP", default)]
    ip: Option<String>,
    #[serde(rename = "PrivatePort")]
    private_port: u16,
    /// Missing for a port the container exposes and does not publish.
    #[serde(rename = "PublicPort", default)]
    public_port: Option<u16>,
    #[serde(rename = "Type", default)]
    protocol: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListedMount {
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(default)]
    source: String,
}

impl From<ListedContainer> for Container {
    fn from(listed: ListedContainer) -> Container {
        let compose_folder = listed
            .labels
            .as_ref()
            .and_then(|labels| labels.get(COMPOSE_WORKING_DIR))
            .map(PathBuf::from)
            .filter(|folder| folder.is_absolute());
        let mounts = listed.mounts.unwrap_or_default().into_iter();
        let binds = mounts.filter(|mount| mount.kind == "bind");
        let bound = binds.map(|mount| host_folder(&mount.source));
        let bound: BTreeSet<PathBuf> = bound.filter(|folder| folder.is_absolute()).collect();
        let name = listed
            .names
            .first()
            .map(|name| name.trim_start_matches('/'));
        Container {
            name: name.unwrap_or(&listed.id).to_string(),
            id: listed.id,
            image: listed.image,
            compose_folder,
            bound: bound.into_iter().collect(),
            ports: published(listed.ports.unwrap_or_default()),
            // The list says when a container was made, not when it last started.
            started: None,
        }
    }
}

/// When the container `body` tells of, an answer to `/containers/{id}/json`, last started; `None`
/// when the engine names no moment of its own form, as for one that never started.
pub(super) fn started(body: &[u8]) -> serde_json::Result<Option<SystemTime>> {
    let inspected: InspectedContainer = serde_json::from_slice(body)?;
    Ok(utc::parse(&inspected.state.started_at))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedContainer {
    state: InspectedState,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedState {
    /// As Go writes a moment in UTC: `2026-10-29T23:34:41.97809393Z`.
    started_at: String,
}

/// The folder of this Mac a bind mount's `source` names.
fn host_folder(source: &str) -> PathBuf {
    let on_this_mac = source.strip_prefix(DOCKER_DESKTOP_HOST_FOLDERS);
    let on_this_mac = on_this_mac.filter(|rest| rest.starts_with('/'));
    PathBuf::from(on_this_mac.unwrap_or(source))
}

/// The ports among `listed` a container publishes, each once with every address it listens at,
/// by port: the engine lists a port published on every address once for IPv4 and once for IPv6.
fn published(listed: Vec<ListedPort>) -> Vec<PublishedPort> {
    let mut ports: BTreeMap<(u16, bool), PublishedPort> = BTreeMap::new();
    for listed in listed {
        let Some(port) = listed.public_port else {
            continue;
        };
        let udp = listed.protocol == "udp";
        let ip = listed.ip.as_deref().filter(|ip| !ip.is_empty());
        // An engine that names no address publishes the port on every one.
        let ip = ip.map_or(Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)), |ip| {
            ip.parse().ok()
        });
        let entry = ports.entry((port, udp)).or_insert_with(|| PublishedPort {
            port,
            to: listed.private_port,
            addresses: BTreeSet::new(),
            udp,
        });
        entry.addresses.extend(ip);
    }
    ports.into_values().collect()
}

/// The disk use in `body`, an answer to `/system/df`, as the engine says at `read_at`. An answer
/// that gives neither the sums nor the lists of one of the stores is an error.
pub(super) fn disk_use(body: &[u8], read_at: SystemTime) -> serde_json::Result<DockerDisk> {
    let answer: DiskUsage = serde_json::from_slice(body)?;
    let images = answer.image_usage.as_ref().map(Usage::store).or_else(|| {
        let images = answer.images.as_ref()?;
        let layers = answer.layers_size?;
        Some(images_added_up(layers, images))
    });
    let containers = (answer.container_usage.as_ref().map(Usage::store))
        .or_else(|| answer.containers.as_deref().map(containers_added_up));
    let volumes = (answer.volume_usage.as_ref().map(Usage::store))
        .or_else(|| answer.volumes.as_deref().map(volumes_added_up));
    let build_cache = (answer.build_cache_usage.as_ref().map(CacheUsage::store))
        .or_else(|| answer.build_cache.as_deref().map(cache_added_up));
    let missing = |store: &str| serde::de::Error::custom(format!("no {store} in the answer"));
    Ok(DockerDisk {
        images: images.ok_or_else(|| missing("images"))?,
        containers: containers.ok_or_else(|| missing("containers"))?,
        volumes: volumes.ok_or_else(|| missing("volumes"))?,
        build_cache: build_cache.ok_or_else(|| missing("build cache"))?,
        read_at,
    })
}

/// An answer to `/system/df`: the sums of API 1.52 and later, and the lists of earlier ones.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DiskUsage {
    // Docker's API reference spells the sums `ImagesUsage` and so on; Docker 29.4 sends these.
    #[serde(alias = "ImagesUsage")]
    image_usage: Option<Usage>,
    #[serde(alias = "ContainersUsage")]
    container_usage: Option<Usage>,
    #[serde(alias = "VolumesUsage")]
    volume_usage: Option<Usage>,
    build_cache_usage: Option<CacheUsage>,
    layers_size: Option<i64>,
    images: Option<Vec<ListedImage>>,
    containers: Option<Vec<SizedContainer>>,
    volumes: Option<Vec<ListedVolume>>,
    build_cache: Option<Vec<CacheRecord>>,
}

/// The engine's sum over one store.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Usage {
    #[serde(default)]
    total_count: u64,
    #[serde(default)]
    active_count: u64,
    #[serde(default)]
    total_size: i64,
    #[serde(default)]
    reclaimable: i64,
}

impl Usage {
    fn store(&self) -> Store {
        Store {
            bytes: bytes(self.total_size),
            count: self.total_count,
            in_use: self.active_count,
            reclaimable: bytes(self.reclaimable),
        }
    }
}

/// The engine's sum over its build cache, which counts none in use: its records say which are.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CacheUsage {
    #[serde(default)]
    total_count: u64,
    #[serde(default)]
    total_size: i64,
    #[serde(default)]
    reclaimable: i64,
    #[serde(default)]
    items: Vec<CacheRecord>,
}

impl CacheUsage {
    fn store(&self) -> Store {
        Store {
            bytes: bytes(self.total_size),
            count: self.total_count,
            in_use: self.items.iter().filter(|record| record.in_use).count() as u64,
            reclaimable: bytes(self.reclaimable),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListedImage {
    /// How many containers use it; -1 when the engine did not count.
    #[serde(default)]
    containers: i64,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    shared_size: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SizedContainer {
    #[serde(default)]
    size_rw: i64,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListedVolume {
    usage_data: Option<VolumeUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeUsage {
    /// -1 when the engine could not size the volume.
    size: i64,
    ref_count: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CacheRecord {
    #[serde(default)]
    size: i64,
    #[serde(default)]
    in_use: bool,
    #[serde(default)]
    shared: bool,
}

/// The images, added up as `docker system df` did before the engine summed them: all the layers
/// they hold, of which what the images in use hold alone is not to be had back.
fn images_added_up(layers_size: i64, images: &[ListedImage]) -> Store {
    let in_use: Vec<&ListedImage> = images.iter().filter(|image| image.containers > 0).collect();
    let held_alone = in_use
        .iter()
        .filter(|image| image.size >= 0 && image.shared_size >= 0)
        .map(|image| image.size - image.shared_size)
        .sum::<i64>();
    Store {
        bytes: bytes(layers_size),
        count: images.len() as u64,
        in_use: in_use.len() as u64,
        reclaimable: bytes(layers_size - held_alone),
    }
}

/// The containers' writable layers, of which those of the containers not running are to be had
/// back.
fn containers_added_up(containers: &[SizedContainer]) -> Store {
    let running = |container: &&SizedContainer| container.state == "running";
    let size = |container: &SizedContainer| bytes(container.size_rw);
    Store {
        bytes: containers.iter().map(size).sum(),
        count: containers.len() as u64,
        in_use: containers.iter().filter(running).count() as u64,
        reclaimable: containers.iter().filter(|c| !running(c)).map(size).sum(),
    }
}

/// The volumes the engine could size, of which those no container uses are to be had back.
fn volumes_added_up(volumes: &[ListedVolume]) -> Store {
    let sized: Vec<&VolumeUsage> = volumes
        .iter()
        .filter_map(|volume| volume.usage_data.as_ref())
        .filter(|usage| usage.size >= 0)
        .collect();
    let unused = sized.iter().filter(|usage| usage.ref_count == 0);
    Store {
        bytes: sized.iter().map(|usage| bytes(usage.size)).sum(),
        count: volumes.len() as u64,
        in_use: volumes
            .iter()
            .filter_map(|volume| volume.usage_data.as_ref())
            .filter(|usage| usage.ref_count > 0)
            .count() as u64,
        reclaimable: unused.map(|usage| bytes(usage.size)).sum(),
    }
}

/// The build cache records no other cache shares, of which those no build holds are to be had back.
fn cache_added_up(records: &[CacheRecord]) -> Store {
    let own: Vec<&CacheRecord> = records.iter().filter(|record| !record.shared).collect();
    Store {
        bytes: own.iter().map(|record| bytes(record.size)).sum(),
        count: records.len() as u64,
        in_use: records.iter().filter(|record| record.in_use).count() as u64,
        reclaimable: own
            .iter()
            .filter(|record| !record.in_use)
            .map(|record| bytes(record.size))
            .sum(),
    }
}

/// A size the engine sends, which is -1 when it does not know, as no bytes.
fn bytes(size: i64) -> u64 {
    u64::try_from(size).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::UNIX_EPOCH;

    use super::*;

    /// Two running containers as Docker Desktop 4.73 lists them: a Postgres publishing one port
    /// on 127.0.0.1, with no folder of this Mac, and a database whose port is published on every
    /// address, for IPv4 and IPv6 apiece, bind-mounting two folders, one of them twice and the
    /// latter first, and exposing one port it does not publish; and a container compose made,
    /// which says where compose ran.
    const LISTED: &str = r#"[
        {"Id": "e99d", "Names": ["/studio-pg-e99dc5594cc57332"], "Image": "postgres:17",
         "Created": 1793390138, "State": "running",
         "Ports": [{"IP": "127.0.0.1", "PrivatePort": 5432, "PublicPort": 36684, "Type": "tcp"}],
         "Labels": {}, "Mounts": [{"Type": "volume", "Name": "b4bb", "Source": ""}]},
        {"Id": "db01", "Names": ["/database"], "Image": "gcr.io/retailops-1/testdb:standard-local",
         "Created": 1791843273, "State": "running",
         "Ports": [{"IP": "0.0.0.0", "PrivatePort": 3306, "PublicPort": 3306, "Type": "tcp"},
                   {"IP": "::", "PrivatePort": 3306, "PublicPort": 3306, "Type": "tcp"},
                   {"PrivatePort": 33060, "Type": "tcp"}],
         "Labels": null,
         "Mounts": [{"Type": "bind", "Source": "/Users/dev/GT/private/database"},
                    {"Type": "bind", "Source": "/host_mnt/Users/dev/GT/backplane"},
                    {"Type": "bind", "Source": "/Users/dev/GT/backplane"}]},
        {"Id": "c0", "Names": ["/app-web-1"], "Image": "nginx", "Created": 0, "State": "running",
         "Ports": null,
         "Labels": {"com.docker.compose.project.working_dir": "/Users/dev/code/app"},
         "Mounts": [{"Type": "bind", "Source": "/Users/dev/code/app"}]}
    ]"#;

    #[test]
    fn a_listed_container_keeps_its_name_image_published_ports_and_the_folders_it_is_for() {
        let containers = containers(LISTED.as_bytes()).unwrap();

        let names: Vec<&str> = containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["app-web-1", "database", "studio-pg-e99dc5594cc57332"]
        );
        let [web, database, postgres] = containers.as_slice() else {
            unreachable!()
        };
        assert_eq!(
            postgres.ports,
            [PublishedPort {
                port: 36684,
                to: 5432,
                addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
                udp: false,
            }]
        );
        assert_eq!(
            postgres.folders().count(),
            0,
            "a volume is no folder of this Mac"
        );
        let [mysql] = database.ports.as_slice() else {
            panic!(
                "one port, published twice, and one only exposed: {:?}",
                database.ports
            );
        };
        assert_eq!(
            mysql.addresses,
            BTreeSet::from(["0.0.0.0".parse().unwrap(), "::".parse().unwrap()])
        );
        assert_eq!(
            database.bound,
            [
                Path::new("/Users/dev/GT/backplane"),
                Path::new("/Users/dev/GT/private/database")
            ],
            "by path, Docker Desktop's /host_mnt being this Mac's /"
        );
        assert_eq!(database.compose_folder, None);
        assert_eq!(
            web.compose_folder.as_deref(),
            Some(Path::new("/Users/dev/code/app"))
        );
        assert_eq!(web.bound, [Path::new("/Users/dev/code/app")]);
        assert!(web.ports.is_empty());
        assert_eq!(database.image, "gcr.io/retailops-1/testdb:standard-local");
        assert_eq!(
            database.started, None,
            "the list says when it was made alone"
        );
    }

    #[test]
    fn a_container_inspected_alone_says_when_it_last_started_and_one_that_never_did_says_nothing() {
        let inspected = |started_at: &str| {
            let body = format!(
                r#"{{"Id": "db01", "Created": "2026-10-12T22:14:33.597837507Z",
                    "State": {{"Status": "running", "Running": true,
                               "StartedAt": "{started_at}",
                               "FinishedAt": "0001-01-01T00:00:00Z"}}}}"#
            );
            started(body.as_bytes()).unwrap()
        };
        assert_eq!(
            inspected("2026-11-09T22:53:36.426100545Z"),
            utc::parse("2026-11-09T22:53:36.426100545Z"),
            "made a month before, started again today"
        );
        assert_eq!(inspected("0001-01-01T00:00:00Z"), None);
        assert!(started(br#"{"message": "No such container: db01"}"#).is_err());
    }

    #[test]
    fn a_list_that_is_no_list_of_containers_is_an_error() {
        assert!(containers(br#"{"message": "page not found"}"#).is_err());
    }

    /// Disk use as Docker 29.4 answers it, with the sums, and as earlier engines did, with the
    /// lists alone, for the same stores.
    const SUMMED: &str = r#"{
        "ImageUsage": {"ActiveCount": 14, "Reclaimable": 11446905, "TotalCount": 20,
                       "TotalSize": 19143582596, "Items": []},
        "ContainerUsage": {"ActiveCount": 14, "Reclaimable": 340410368, "TotalCount": 33,
                           "TotalSize": 659615744},
        "VolumeUsage": {"ActiveCount": 21, "Reclaimable": 5039641769, "TotalCount": 68,
                        "TotalSize": 22705049435},
        "BuildCacheUsage": {"Reclaimable": 5415259336, "TotalCount": 2, "TotalSize": 5415259336,
                            "Items": [{"Size": 5415259336, "InUse": false, "Shared": false},
                                      {"Size": 0, "InUse": true, "Shared": false}]}
    }"#;
    const LISTED_ONLY: &str = r#"{
        "LayersSize": 1000,
        "Images": [{"Containers": 2, "Size": 600, "SharedSize": 100},
                   {"Containers": 0, "Size": 300, "SharedSize": 100},
                   {"Containers": 1, "Size": -1, "SharedSize": -1}],
        "Containers": [{"SizeRw": 40, "State": "running"}, {"SizeRw": 60, "State": "exited"}],
        "Volumes": [{"UsageData": {"Size": 500, "RefCount": 1}},
                    {"UsageData": {"Size": 200, "RefCount": 0}},
                    {"UsageData": {"Size": -1, "RefCount": 0}}],
        "BuildCache": [{"Size": 70, "InUse": false, "Shared": false},
                       {"Size": 30, "InUse": true, "Shared": false},
                       {"Size": 99, "InUse": false, "Shared": true}]
    }"#;

    fn store(bytes: u64, count: u64, in_use: u64, reclaimable: u64) -> Store {
        Store {
            bytes,
            count,
            in_use,
            reclaimable,
        }
    }

    #[test]
    fn disk_use_is_read_from_the_engines_sums_when_it_sends_them() {
        let disk = disk_use(SUMMED.as_bytes(), UNIX_EPOCH).unwrap();
        assert_eq!(disk.images, store(19_143_582_596, 20, 14, 11_446_905));
        assert_eq!(disk.containers, store(659_615_744, 33, 14, 340_410_368));
        assert_eq!(disk.volumes, store(22_705_049_435, 68, 21, 5_039_641_769));
        assert_eq!(disk.build_cache, store(5_415_259_336, 2, 1, 5_415_259_336));
        assert_eq!(disk.bytes(), 47_923_507_111);
    }

    #[test]
    fn disk_use_is_added_up_from_the_lists_as_the_cli_did_when_the_engine_sends_no_sums() {
        let disk = disk_use(LISTED_ONLY.as_bytes(), UNIX_EPOCH).unwrap();
        assert_eq!(
            disk.images,
            store(1000, 3, 2, 500),
            "what the images in use hold alone, 600 less 100 shared, stays"
        );
        assert_eq!(disk.containers, store(100, 2, 1, 60));
        assert_eq!(disk.volumes, store(700, 3, 1, 200));
        assert_eq!(
            disk.build_cache,
            store(100, 3, 1, 70),
            "a shared record is another's"
        );
    }

    #[test]
    fn disk_use_missing_a_store_in_both_shapes_is_an_error() {
        let no_volumes = r#"{"ImageUsage": {}, "ContainerUsage": {}, "BuildCacheUsage": {}}"#;
        assert!(disk_use(no_volumes.as_bytes(), UNIX_EPOCH).is_err());
    }
}
