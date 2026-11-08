//! Snapshots and projects for the listing's tests, and the few words a test reads each row by.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::{Node, Row};
use crate::map::docker::{Container, DockerDisk, Store};
use crate::map::test_support::{agent_session, at, project_at, snapshot_of, subagent_at_work};
use crate::map::{
    AgentSession, AgentStatus, Docker, GroupKey, Project, Provenance, RepoInfo, ScratchSpace,
    Snapshot, Subagent, SubagentWithParent,
};

pub(in crate::dashboard) fn now() -> SystemTime {
    at(1_000_000_000)
}

/// A project last active `minutes_ago`, in focus for that alone.
pub(in crate::dashboard) fn project(name: &str, minutes_ago: u64) -> Project {
    Project {
        last_activity: Some(now() - Duration::from_secs(minutes_ago * 60)),
        ..quiet(name)
    }
}

/// A project out of focus: no activity known, and nothing in it.
pub(in crate::dashboard) fn quiet(name: &str) -> Project {
    project_at(PathBuf::from("/code").join(name))
}

/// `project` as the main worktree of the repository whose git dir is `common_dir`, on `branch`,
/// and with no remote, so grouped by its own folder, as the engine would group it.
pub(in crate::dashboard) fn main_worktree(
    common_dir: &str,
    branch: &str,
    project: Project,
) -> Project {
    let common_dir = PathBuf::from(common_dir);
    let repo = RepoInfo {
        git_dir: common_dir.clone(),
        common_dir,
        worktree_name: None,
        branch: Some(branch.to_string()),
        head: None,
    };
    let provenance = Provenance {
        group: GroupKey::LocalRepository(repo.folder().to_path_buf()),
        made_from: Vec::new(),
    };
    Project {
        repo: Some(repo),
        provenance,
        ..project
    }
}

/// `project` as a worktree linked to the repository whose git dir is `common_dir`, under its
/// folder's name, on `branch`, made from the repository's folder and grouped by it, as the engine
/// would for a repository with no remote.
pub(in crate::dashboard) fn linked_worktree(
    common_dir: &str,
    branch: &str,
    project: Project,
) -> Project {
    let common_dir = PathBuf::from(common_dir);
    let repo = RepoInfo {
        git_dir: common_dir.join("worktrees").join(&project.name),
        common_dir,
        worktree_name: Some(project.name.clone()),
        branch: Some(branch.to_string()),
        head: None,
    };
    let folder = repo.folder().to_path_buf();
    let provenance = Provenance {
        group: GroupKey::LocalRepository(folder.clone()),
        made_from: vec![folder],
    };
    Project {
        repo: Some(repo),
        provenance,
        ..project
    }
}

/// `project`, come from `group` and made from the projects at `made_from`, nearest first.
pub(in crate::dashboard) fn came_from(
    group: GroupKey,
    made_from: &[&str],
    project: Project,
) -> Project {
    Project {
        provenance: Provenance {
            group,
            made_from: made_from.iter().map(PathBuf::from).collect(),
        },
        ..project
    }
}

/// The group of the clones of the remote `remote`, as `github.com/owner/repo` reads.
pub(in crate::dashboard) fn upstream(remote: &str) -> GroupKey {
    GroupKey::Upstream(remote.to_string())
}

/// `project` as the scratch folder `folder` of the session `session_id`, which started in
/// `/code/app` and has ended, never named, so it goes by the name the engine gives it, as
/// `app e26ca359` reads.
pub(in crate::dashboard) fn scratch_folder(
    session_id: &str,
    folder: &str,
    project: Project,
) -> Project {
    let session_folder = PathBuf::from("/private/tmp/claude-501/-code-app").join(session_id);
    let space = ScratchSpace {
        session_folder,
        session_id: session_id.to_string(),
        started_in: "app".to_string(),
        started_in_folder: Some(PathBuf::from("/code/app")),
        session: None,
        session_name: None,
    };
    Project {
        path: space.session_folder.join("scratchpad").join(folder),
        name: folder.to_string(),
        scratch: Some(space),
        ..project
    }
}

/// `scratch`, a scratch folder, while its session runs under the name `label`.
pub(in crate::dashboard) fn running_as(label: &str, scratch: Project) -> Project {
    let mut space = scratch.scratch.expect("a scratch folder");
    space.session = Some(AgentSession {
        label: Some(label.to_string()),
        ..agent_session(&space.session_id, AgentStatus::Busy)
    });
    space.session_name = Some(label.to_string());
    Project {
        scratch: Some(space),
        ..scratch
    }
}

pub(in crate::dashboard) fn agent(cwd: &str, status: AgentStatus) -> AgentSession {
    AgentSession {
        cwd: PathBuf::from(cwd),
        ..agent_session(cwd, status)
    }
}

/// A subagent at work in `cwd`, run there and acting nowhere else, for an idle session in
/// `/code/app`.
pub(in crate::dashboard) fn subagent_in(cwd: &str) -> SubagentWithParent {
    let subagent = Subagent {
        cwd: PathBuf::from(cwd),
        ..subagent_at_work(cwd)
    };
    SubagentWithParent::new(subagent, &agent("/code/app", AgentStatus::Idle))
}

/// Docker read at `/run/docker.sock`, running `containers` for no project, made `minutes_ago`
/// each, and taking `bytes` on disk.
pub(in crate::dashboard) fn docker(containers: &[(&str, u64)], bytes: u64) -> Docker {
    let store = |bytes| Store {
        bytes,
        count: 1,
        in_use: 1,
        reclaimable: 0,
    };
    let container = |&(name, minutes_ago): &(&str, u64)| Container {
        id: name.to_string(),
        name: name.to_string(),
        image: "postgres:17".to_string(),
        compose_folder: None,
        bound: Vec::new(),
        ports: Vec::new(),
        created: now() - Duration::from_secs(minutes_ago * 60),
    };
    Docker {
        socket: PathBuf::from("/run/docker.sock"),
        containers: containers.iter().map(container).collect(),
        disk: Some(DockerDisk {
            images: store(bytes),
            containers: store(0),
            volumes: store(0),
            build_cache: store(0),
            read_at: now(),
        }),
    }
}

/// A map of `projects`, in the order given, which the engine delivers newest activity first.
pub(in crate::dashboard) fn snapshot(
    projects: Vec<Project>,
    unplaced_agents: Vec<AgentSession>,
) -> Snapshot {
    Snapshot {
        unplaced_agents,
        ..snapshot_of(projects)
    }
}

/// What `rows` show, a few words each: a group as `name: row, row`, each card by its folder's own
/// name, a checkout's line as `name(row, row)`, the agents elsewhere as `elsewhere`; Docker's group
/// as `docker`; and the quiet row as `quiet:<count>`.
pub(in crate::dashboard) fn shown(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Group(group) => format!("{}: {}", group.name, nodes(&group.children)),
            Row::Docker(_) => "docker".to_string(),
            Row::QuietProjects { count, .. } => format!("quiet:{count}"),
        })
        .collect()
}

/// The rows of one line, as `shown` writes them.
fn nodes(nodes: &[Node]) -> String {
    let words: Vec<String> = nodes
        .iter()
        .map(|node| match node {
            Node::Card { project, .. } => {
                let folder = project.path.file_name().unwrap_or_default();
                folder.to_string_lossy().into_owned()
            }
            Node::Checkout(checkout) => {
                format!("{}({})", checkout.name, self::nodes(&checkout.children))
            }
            Node::Elsewhere(_) => "elsewhere".to_string(),
        })
        .collect();
    words.join(", ")
}
