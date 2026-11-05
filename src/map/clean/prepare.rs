//! Everything a clean needs before it goes: what it deletes, whether a build is using any of it, how
//! large it is and who else builds there. All of that reads files and can walk a large folder, so
//! it runs apart from the window, which only words what comes of it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::run::{FolderIdentity, LockedPlan};
use super::{CargoEnv, CleanError, CleanPlan, Plan, PlanError, Reach, cargo_rules, lock, plan};
use crate::map::{ArtifactKind, DirUsage, FolderAccess, Project, Snapshot, disk};

/// A clean worked out and ready to go. It holds no lock: a build that starts while a question is
/// open or the clean waits its turn must not wait for it, so `Prepared::lock` takes them when the
/// clean starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub plan: Plan,
    /// The size of each folder `plan` deletes, in the same order: the map's, else measured now,
    /// which takes a moment for a large folder the map does not show (a build-dir outside every
    /// project); `None` when even that fails.
    pub sizes: Vec<Option<DirUsage>>,
    pub sharing: Sharing,
    /// Which folder each of `plan`'s paths led to as this was worked out.
    identities: Vec<FolderIdentity>,
}

impl Prepared {
    /// A clean as if worked out, for tests of what is said about it.
    #[cfg(test)]
    pub(crate) fn for_test(plan: Plan, sizes: Vec<Option<DirUsage>>, sharing: Sharing) -> Prepared {
        Prepared {
            plan,
            sizes,
            sharing,
            identities: Vec::new(),
        }
    }

    /// Take the locks for the delete, or refuse because a build holds one or a folder is no longer
    /// the one this was worked out for.
    pub fn lock(&self) -> Result<LockedPlan, CleanError> {
        let locked = lock(&self.plan)?;
        locked.check_still(&self.identities)?;
        Ok(locked)
    }
}

/// Why a click on Clean goes no further than a word to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    /// Sutra will not delete the folder, or `cargo clean` would refuse it.
    Plan(PlanError),
    /// A build is using one of the folders, or their locks cannot be tried.
    Locks(CleanError),
}

/// The other projects on the map a clean may reach into, by the name the map gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sharing {
    /// Those whose bare `cargo clean` deletes one of the clean's folders, a folder inside one or a
    /// folder holding one, so their builds go with it. Sorted, each once.
    pub projects: Vec<String>,
    /// Those whose workspace could not be worked out, because a file cargo reads for them cannot
    /// be read: any of them may build in these folders, or none. Sorted, each once.
    pub unknown: Vec<String>,
    /// Where on the map a project is that may be using these folders and could be at work while
    /// the clicked one is idle: each of the above, and any other project in the clean's own
    /// workspace, such as a member registered on its own. Sorted, each once.
    pub project_folders: Vec<PathBuf>,
}

impl Sharing {
    /// Whether the clean would take other projects' builds with it, or may, which the click on one
    /// card does not say. Another project in the clean's own workspace builds there by being part
    /// of it.
    pub fn reaches_other_projects(&self) -> bool {
        !self.projects.is_empty() || !self.unknown.is_empty()
    }
}

/// Work out the clean of `clicked`, a build folder in the project at `project`, as `map` shows them
/// and with `env` for the environment cargo would see.
pub fn prepare(
    clicked: &Path,
    project: &Path,
    map: &Snapshot,
    env: &CargoEnv,
) -> Result<Prepared, PrepareError> {
    let (plan, identities) = plan_unless_busy(clicked, project, map, env)?;
    Ok(prepared(
        plan,
        identities,
        project,
        map,
        &Builds::of(map, env),
    ))
}

/// `prepare`, among several that share what `builds` worked out.
pub(super) fn prepare_among(
    clicked: &Path,
    project: &Path,
    map: &Snapshot,
    env: &CargoEnv,
    builds: &Builds,
) -> Result<Prepared, PrepareError> {
    let (plan, identities) = plan_unless_busy(clicked, project, map, env)?;
    Ok(prepared(plan, identities, project, map, builds))
}

fn plan_unless_busy(
    clicked: &Path,
    project: &Path,
    map: &Snapshot,
    env: &CargoEnv,
) -> Result<(Plan, Vec<FolderIdentity>), PrepareError> {
    let reach = if map.sandboxed {
        Reach::Within(&map.roots)
    } else {
        Reach::Anywhere
    };
    let plan = plan(clicked, project, env, &reach).map_err(PrepareError::Plan)?;
    // Taken only to refuse now, not once the clean's turn comes, when a build is using a folder.
    let identities = lock(&plan).map_err(PrepareError::Locks)?.identities();
    Ok((plan, identities))
}

fn prepared(
    plan: Plan,
    identities: Vec<FolderIdentity>,
    project: &Path,
    map: &Snapshot,
    builds: &Builds,
) -> Prepared {
    Prepared {
        sizes: sizes(&plan, map),
        sharing: sharing(&plan, project, map, builds),
        plan,
        identities,
    }
}

fn sizes(plan: &Plan, map: &Snapshot) -> Vec<Option<DirUsage>> {
    let on_map = |folder| map.artifact(folder)?.measurement.usage().copied();
    plan.folders()
        .into_iter()
        .map(|folder| on_map(folder).or_else(|| disk::measure(folder).ok()))
        .collect()
}

/// What a bare `cargo clean` would delete for each project on the map, worked out once however many
/// cleans ask. It asks of each project's folder and of the folder beside each of its Cargo build
/// folders, as a workspace can be either. A project Sutra may not read is not asked: looking would
/// raise the privacy prompt, or it is gone.
pub(super) struct Builds<'a> {
    of_projects: Vec<OneBuild<'a>>,
}

struct OneBuild<'a> {
    project: &'a Project,
    asked_at: &'a Path,
    clean: Result<CleanPlan, PlanError>,
}

impl<'a> Builds<'a> {
    pub(super) fn of(map: &'a Snapshot, env: &CargoEnv) -> Builds<'a> {
        let mut seen: HashSet<&Path> = HashSet::new();
        let mut of_projects = Vec::new();
        let readable = map
            .projects
            .iter()
            .filter(|project| project.access == FolderAccess::Readable);
        for project in readable {
            for asked_at in workspace_candidates(project) {
                if seen.insert(asked_at) {
                    of_projects.push(OneBuild {
                        project,
                        asked_at,
                        clean: cargo_rules::clean_at_workspace_of(asked_at, env),
                    });
                }
            }
        }
        Builds { of_projects }
    }
}

/// Who besides the project at `project` `plan`'s folders may hold builds of. A workspace whose clean
/// cargo would refuse shares nothing, as it deletes nothing.
fn sharing(plan: &Plan, project: &Path, map: &Snapshot, builds: &Builds) -> Sharing {
    let own_workspace = match plan {
        Plan::CargoClean(clean) => Some(clean.workspace.as_path()),
        Plan::DeleteFolder { cargo_clean, .. } => {
            cargo_clean.as_ref().map(|clean| clean.workspace.as_path())
        }
    };
    let folders = plan.folders();
    let mut found = Sharing::default();
    for build in &builds.of_projects {
        match &build.clean {
            Ok(clean) => {
                let overlaps = clean.folders.iter().any(|folder| {
                    folders
                        .iter()
                        .any(|ours| folder.path.starts_with(ours) || ours.starts_with(&folder.path))
                });
                if overlaps {
                    if own_workspace != Some(clean.workspace.as_path()) {
                        found.projects.push(name_on(map, &clean.workspace));
                    }
                    if build.project.path != project {
                        found.project_folders.push(build.project.path.clone());
                    }
                }
            }
            Err(PlanError::Cargo(_)) => {}
            Err(_) => {
                found.unknown.push(name_on(map, build.asked_at));
                if build.project.path != project {
                    found.project_folders.push(build.project.path.clone());
                }
            }
        }
    }
    found.projects.sort();
    found.projects.dedup();
    found.project_folders.sort();
    found.project_folders.dedup();
    found.unknown.sort();
    found.unknown.dedup();
    found
}

/// The folders of `project` a workspace can be worked out from. One whose `Cargo.toml` cannot be
/// looked at may hold one, so it stays, and working it out says what cannot be read.
fn workspace_candidates(project: &Project) -> impl Iterator<Item = &Path> {
    let own = cargo_rules::exists(&project.path.join("Cargo.toml"))
        .unwrap_or(true)
        .then_some(project.path.as_path());
    let beside_build_folders = project
        .artifacts
        .iter()
        .filter(|dir| dir.kind == ArtifactKind::CargoTarget)
        .filter_map(|dir| dir.path.parent());
    own.into_iter().chain(beside_build_folders)
}

/// What the map calls the project at `folder`, else the folder's own name.
fn name_on(map: &Snapshot, folder: &Path) -> String {
    match (
        map.projects.iter().find(|project| project.path == folder),
        folder.file_name(),
    ) {
        (Some(project), _) => project.name.clone(),
        (None, Some(name)) => name.to_string_lossy().into_owned(),
        (None, None) => PathBuf::from(folder).display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::map::Measurement;
    use crate::map::clean::locks::tests::hold_like_cargo;
    use crate::map::clean::{CleanPlan, Origin, PlannedFolder, Refusal};
    use crate::map::test_support::{
        CACHEDIR_TAG, TempDir, cargo_target, project_at, snapshot_of, usage,
    };

    /// A package building in the fixture's `shared` folder.
    fn package_building_in_shared(fixture: &TempDir, dir: &str) {
        fixture.package(dir);
        fixture.write(
            &format!("{dir}/.cargo/config.toml"),
            "[build]\ntarget-dir = \"../shared\"\n",
        );
    }

    fn cleaning(folder: PathBuf, workspace: PathBuf) -> Plan {
        Plan::CargoClean(CleanPlan {
            workspace,
            folders: vec![PlannedFolder {
                path: folder,
                origin: Origin::Default,
            }],
        })
    }

    /// Who shares `plan`'s folders, when the project clicked is its workspace.
    fn sharing_with(plan: &Plan, map: &Snapshot, fixture: &TempDir) -> Sharing {
        let env = fixture.cargo_env();
        let project = match plan {
            Plan::CargoClean(clean) => clean.workspace.as_path(),
            Plan::DeleteFolder { folder, .. } => folder.as_path(),
        };
        sharing(plan, project, map, &Builds::of(map, &env))
    }

    fn shared_folder(fixture: &TempDir) -> PathBuf {
        let tag = fixture.write("shared/CACHEDIR.TAG", CACHEDIR_TAG);
        tag.parent().unwrap().to_path_buf()
    }

    #[test]
    fn a_clean_comes_prepared_with_its_folders_size_and_holding_no_lock() {
        let fixture = TempDir::new("prepare-ready");
        fixture.package("app");
        let target = fixture.dir("app/target");
        fixture.write("app/target/debug/.cargo-lock", "");
        fixture.write("app/target/debug/deps/lib.rlib", [1; 8192]);
        let map = snapshot_of(vec![project_at(fixture.path("app"))]);

        let prepared = prepare(&target, &fixture.path("app"), &map, &fixture.cargo_env()).unwrap();

        assert_eq!(
            prepared.plan,
            cleaning(target.clone(), fixture.path("app")),
            "the clean of the clicked folder"
        );
        assert!(prepared.sizes[0].is_some_and(|usage| usage.bytes >= 8192));
        // Panics unless the locks are free again, so a build can start while the question is open.
        let _building = hold_like_cargo(&target.join("debug/.cargo-lock"));
    }

    #[test]
    fn a_folder_replaced_while_the_question_is_open_stops_the_clean_with_both_untouched() {
        let fixture = TempDir::new("prepare-replaced");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let map = snapshot_of(Vec::new());
        let prepared = prepare(&target, &fixture.path("app"), &map, &fixture.cargo_env()).unwrap();

        fs::rename(&target, fixture.path("app/target.old")).unwrap();
        let other = fixture.write("app/target/other", "keep");

        assert_eq!(
            prepared.lock().map(drop),
            Err(CleanError::Changed { folder: target })
        );
        assert!(other.exists());
    }

    #[test]
    fn a_parent_swapped_for_a_link_while_the_question_is_open_stops_the_clean_with_both_untouched()
    {
        let fixture = TempDir::new("prepare-parent-link");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let elsewhere = fixture.write("elsewhere/target/precious", "keep");
        let map = snapshot_of(Vec::new());
        let prepared = prepare(&target, &fixture.path("app"), &map, &fixture.cargo_env()).unwrap();

        fs::rename(fixture.path("app"), fixture.path("app.old")).unwrap();
        std::os::unix::fs::symlink(fixture.path("elsewhere"), fixture.path("app")).unwrap();

        assert_eq!(
            prepared.lock().map(drop),
            Err(CleanError::Changed { folder: target })
        );
        assert!(elsewhere.exists());
    }

    #[test]
    fn a_folder_sutra_will_not_clean_goes_no_further() {
        let fixture = TempDir::new("prepare-refused");
        fixture.package("app");
        let gone = fixture.path("app/target");
        let map = snapshot_of(Vec::new());

        assert_eq!(
            prepare(&gone, &fixture.path("app"), &map, &fixture.cargo_env()),
            Err(PrepareError::Plan(PlanError::Gone { folder: gone }))
        );
    }

    #[test]
    fn a_sandboxed_map_reaches_only_inside_its_roots() {
        let fixture = TempDir::new("prepare-reach");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let mut map = snapshot_of(Vec::new());
        map.sandboxed = true;
        map.roots = vec![fixture.path("elsewhere")];

        assert_eq!(
            prepare(&target, &fixture.path("app"), &map, &fixture.cargo_env()),
            Err(PrepareError::Plan(PlanError::Refused {
                folder: target,
                reason: Refusal::OutsideGrant,
            }))
        );
    }

    #[test]
    fn a_folder_a_build_is_using_goes_no_further() {
        let fixture = TempDir::new("prepare-busy");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let busy = fixture.write("app/target/debug/.cargo-build-lock", "");
        let _building = hold_like_cargo(&busy);
        let map = snapshot_of(Vec::new());

        assert_eq!(
            prepare(&target, &fixture.path("app"), &map, &fixture.cargo_env()),
            Err(PrepareError::Locks(CleanError::BuildRunning { lock: busy }))
        );
    }

    #[test]
    fn a_size_comes_from_the_map_or_is_measured_now() {
        let fixture = TempDir::new("prepare-sizes");
        let unmapped = fixture.dir("cache");
        fixture.write("cache/lib.rlib", [1; 8192]);
        let on_map = fixture.path("app/target");
        let measured = usage(12_300_000_000);
        let map = snapshot_of(vec![Project {
            artifacts: vec![cargo_target(
                on_map.clone(),
                Measurement::Measured(measured),
            )],
            ..project_at(fixture.path("app"))
        }]);
        let plan = Plan::CargoClean(CleanPlan {
            workspace: fixture.path("app"),
            folders: [on_map, unmapped]
                .map(|path| PlannedFolder {
                    path,
                    origin: Origin::Default,
                })
                .to_vec(),
        });

        let sizes = sizes(&plan, &map);

        assert_eq!(sizes[0], Some(measured));
        assert!(sizes[1].is_some_and(|usage| usage.bytes >= 8192));
    }

    #[test]
    fn projects_whose_cargo_clean_deletes_the_same_folder_are_named_each_once() {
        let fixture = TempDir::new("prepare-sharing");
        let shared = shared_folder(&fixture);
        for name in ["one", "two", "three"] {
            package_building_in_shared(&fixture, name);
        }
        fixture.package("own");
        fixture.dir("own/target");
        let map = snapshot_of(
            ["one", "two", "three", "own"]
                .map(|name| project_at(fixture.path(name)))
                .to_vec(),
        );

        let found = sharing_with(&cleaning(shared, fixture.path("one")), &map, &fixture);

        assert_eq!(
            found,
            Sharing {
                projects: vec!["three".to_string(), "two".to_string()],
                project_folders: vec![fixture.path("three"), fixture.path("two")],
                unknown: Vec::new(),
            }
        );
    }

    #[test]
    fn a_build_folder_inside_or_around_the_one_cleaned_shares_it_too() {
        let fixture = TempDir::new("prepare-sharing-overlap");
        let shared = shared_folder(&fixture);
        for (name, target_dir) in [("inside", "../shared/inside"), ("around", "..")] {
            fixture.package(name);
            fixture.write(
                &format!("{name}/.cargo/config.toml"),
                format!("[build]\ntarget-dir = \"{target_dir}\"\n"),
            );
        }
        fixture.write("shared/inside/CACHEDIR.TAG", CACHEDIR_TAG);
        let map = snapshot_of(
            ["inside", "around"]
                .map(|name| project_at(fixture.path(name)))
                .to_vec(),
        );

        let found = sharing_with(&cleaning(shared, fixture.path("one")), &map, &fixture);

        assert_eq!(found.projects, ["around", "inside"]);
    }

    #[test]
    fn a_workspace_beside_a_build_folder_counts_though_no_project_is_its_root() {
        let fixture = TempDir::new("prepare-sharing-beside");
        let shared = shared_folder(&fixture);
        fixture.package("worktree/inner");
        fixture.write(
            "worktree/inner/.cargo/config.toml",
            "[build]\ntarget-dir = \"../../shared\"\n",
        );
        let map = snapshot_of(vec![Project {
            artifacts: vec![cargo_target(
                fixture.path("worktree/inner/target"),
                Measurement::Pending,
            )],
            ..project_at(fixture.path("worktree"))
        }]);

        let found = sharing_with(&cleaning(shared, fixture.path("one")), &map, &fixture);

        assert_eq!(found.projects, ["inner"]);
    }

    #[test]
    fn a_clean_reaches_other_projects_only_when_another_workspace_builds_there_or_may() {
        let named = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        assert!(!Sharing::default().reaches_other_projects());
        assert!(
            !Sharing {
                project_folders: vec![PathBuf::from("/code/ws/member")],
                ..Sharing::default()
            }
            .reaches_other_projects(),
            "a member of its own workspace"
        );
        assert!(
            Sharing {
                projects: named(&["other"]),
                ..Sharing::default()
            }
            .reaches_other_projects()
        );
        assert!(
            Sharing {
                unknown: named(&["sealed"]),
                ..Sharing::default()
            }
            .reaches_other_projects()
        );
    }

    #[test]
    fn a_project_whose_settings_cannot_be_read_may_share_and_is_named_as_unknown() {
        let fixture = TempDir::new("prepare-sharing-unknown");
        let shared = shared_folder(&fixture);
        fixture.package("sealed");
        let settings = fixture.dir("sealed/.cargo");
        fixture.restrict(&settings, 0o000);
        fixture.package("broken");
        fixture.write("broken/.cargo/config.toml", "[build\n");
        let map = snapshot_of(vec![
            project_at(fixture.path("sealed")),
            project_at(fixture.path("broken")),
        ]);

        let found = sharing_with(&cleaning(shared, fixture.path("one")), &map, &fixture);

        assert_eq!(
            found,
            Sharing {
                projects: Vec::new(),
                project_folders: vec![fixture.path("sealed")],
                unknown: vec!["sealed".to_string()],
            },
            "cargo refuses the broken one, so it deletes nothing"
        );
    }

    #[test]
    fn a_project_sutra_may_not_read_is_not_asked() {
        let fixture = TempDir::new("prepare-sharing-private");
        let shared = shared_folder(&fixture);
        package_building_in_shared(&fixture, "private");
        let private = Project {
            access: FolderAccess::PrivateFolder,
            ..project_at(fixture.path("private"))
        };

        let found = sharing_with(
            &cleaning(shared, fixture.path("one")),
            &snapshot_of(vec![private]),
            &fixture,
        );

        assert_eq!(found, Sharing::default());
    }
}
