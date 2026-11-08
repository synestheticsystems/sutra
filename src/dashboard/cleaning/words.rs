//! What Sutra says about a clean: the question before the clean of one folder and why it refused
//! one here, and in the child modules, what it says about cleaning every idle folder, about
//! cleaning by itself to keep free space, or a group's folders once idle, and when a clean stops
//! early.

mod idle;
mod keep_free;
mod outcome;
mod when_idle;

pub(super) use idle::{IDLE_SEARCH_STOPPED, idle_question, nothing_to_ask};
pub(super) use keep_free::{keep_free_choice, keep_free_question, made_room};
pub(super) use outcome::{all_stopped, stopped};
pub(super) use when_idle::{cleaned_when_idle, look_stopped};

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use crate::dashboard::format;
use crate::dashboard::state::Question;
use crate::map::clean::{
    CargoRefusal, CleanPlan, Held, IncludeProblem, Origin, Plan, PlanError, PrepareError, Refusal,
    Sharing,
};
use crate::map::{ArtifactDir, DirUsage, Project};

/// What a clean's words are about: the folder clicked in `project`, the home folder `~` stands
/// for, and whether Sutra runs in the sandbox, where an unreadable file is one outside the grant.
pub(super) struct Context<'a> {
    project: &'a Project,
    name: String,
    home: &'a Path,
    sandboxed: bool,
}

impl<'a> Context<'a> {
    pub(super) fn new(
        project: &'a Project,
        dir: &ArtifactDir,
        home: &'a Path,
        sandboxed: bool,
    ) -> Context<'a> {
        Context {
            project,
            name: format::artifact_name(dir, &project.path),
            home,
            sandboxed,
        }
    }

    /// The title of every refusal.
    pub(super) fn cannot_clean(&self) -> String {
        format!("Can't clean {} in {}", self.name, self.project.name)
    }

    /// The question for `plan`, given the size of each folder it deletes and who else builds there.
    pub(super) fn question(
        &self,
        plan: &Plan,
        sizes: &[Option<DirUsage>],
        sharing: &Sharing,
    ) -> Question {
        let mut question = match plan {
            Plan::CargoClean(clean) => self.cargo_clean_question(clean, sizes, &sharing.projects),
            Plan::DeleteFolder {
                folder,
                cargo_clean,
            } => {
                let size = sizes
                    .first()
                    .map_or("an unknown size".to_string(), size_text);
                self.delete_folder_question(folder, cargo_clean.as_ref(), &size, &sharing.projects)
            }
        };
        question.detail.push_str(&unknown_sharers(sharing));
        question
    }

    fn cargo_clean_question(
        &self,
        clean: &CleanPlan,
        sizes: &[Option<DirUsage>],
        sharers: &[String],
    ) -> Question {
        let folders: Vec<String> = clean
            .folders
            .iter()
            .zip(sizes)
            .map(|(folder, size)| {
                let (path, origin) = (self.folder(&folder.path), self.origin(&folder.origin));
                format!("{path} ({}, {origin})", size_text(size))
            })
            .collect();
        let (them, mut detail) = match folders.as_slice() {
            [one] => ("it", format!("Like cargo clean, this clears out {one}.")),
            many => (
                "them",
                format!("Like cargo clean, this clears out:\n{}", many.join("\n")),
            ),
        };
        detail.push_str(&format!(
            "\n\nCargo fills {them} again the next time it builds."
        ));
        if !sharers.is_empty() {
            detail.push_str(&format!(
                "\n\nBuilds of {} are in there too, and go with {them}.",
                list(sharers)
            ));
        }
        Question {
            title: format!("Clean {} in {}?", self.name, self.project.name),
            detail,
            action: "Clean".to_string(),
        }
    }

    /// The question for `folder`, which no bare `cargo clean` of its own workspace deletes. When
    /// another workspace builds there, the folder is no leftover, and the question says whose
    /// builds go. A `target` its workspace's clean passes over is left over from an earlier build.
    /// Any other folder is where cargo builds because `CARGO_TARGET_DIR` or `--target-dir` says so,
    /// in no workspace or beside one, and only its tag let it be cleaned, so the question says it
    /// is a cache.
    fn delete_folder_question(
        &self,
        folder: &Path,
        cargo_clean: Option<&CleanPlan>,
        size: &str,
        sharers: &[String],
    ) -> Question {
        if !sharers.is_empty() {
            return Question {
                title: format!("Clean {} in {}?", self.name, self.project.name),
                detail: format!(
                    "Builds of {} are in this folder, so it isn't left over.\n\n\
                     Cleaning it frees {size}, and those builds go with it.",
                    list(sharers)
                ),
                action: "Clean".to_string(),
            };
        }
        let leftover = |why: String| Question {
            title: format!("Clean the leftover {} in {}?", self.name, self.project.name),
            detail: format!("{why}\n\nCleaning it frees {size}."),
            action: "Clean".to_string(),
        };
        let cache = |why: String| Question {
            title: format!("Clean {} in {}?", self.name, self.project.name),
            detail: format!(
                "{why} Cargo tagged it as its cache, and fills it again the next time it builds \
                 there.\n\nCleaning it frees {size}."
            ),
            action: "Clean".to_string(),
        };
        let cargos_default = folder.file_name() == Some(OsStr::new("target"));
        match cargo_clean {
            Some(clean) if clean.folders.is_empty() => {
                let why = format!(
                    "cargo clean in {} doesn't delete this folder",
                    self.workspace(&clean.workspace)
                );
                if cargos_default {
                    leftover(format!("{why}, so it's left over from an earlier build."))
                } else {
                    cache(format!("{why}."))
                }
            }
            Some(clean) => {
                let elsewhere: Vec<String> =
                    clean.folders.iter().map(|f| self.folder(&f.path)).collect();
                let why = format!(
                    "cargo clean in {} deletes {}, not this folder",
                    self.workspace(&clean.workspace),
                    list(&elsewhere)
                );
                if cargos_default {
                    leftover(format!(
                        "{why}, so this one is left over from an earlier build."
                    ))
                } else {
                    cache(format!("{why}."))
                }
            }
            None => cache(
                "No Cargo.toml is beside this folder or above it, so no cargo clean reaches it."
                    .to_string(),
            ),
        }
    }

    /// Why a click on Clean whose clean would take `folder`, which another question asks about,
    /// goes no further.
    pub(super) fn asked_already(&self, folder: &Path) -> String {
        format!(
            "A question about cleaning {} is waiting for your answer. Try again once you've \
             answered it.",
            self.folder(folder)
        )
    }

    /// Why a click on Clean goes no further.
    pub(super) fn refused(&self, err: &PrepareError) -> String {
        match err {
            PrepareError::Plan(err) => self.plan_refused(err),
            PrepareError::Locks(err) => outcome::failure(err, &|path| self.folder(path)),
        }
    }

    fn plan_refused(&self, err: &PlanError) -> String {
        match err {
            PlanError::Gone { .. } => "It's gone already.".to_string(),
            PlanError::Cargo(refusal) => {
                format!("cargo clean would fail here too: {}.", self.cargo(refusal))
            }
            PlanError::Unreadable { path, kind }
                if self.sandboxed && *kind == io::ErrorKind::PermissionDenied =>
            {
                format!(
                    "cargo clean reads {}, which lies outside the folder Sutra may use.",
                    self.folder(path)
                )
            }
            PlanError::Unreadable { path, kind } => {
                format!(
                    "cargo clean reads {}, which Sutra can't read: {kind}.",
                    self.folder(path)
                )
            }
            PlanError::Refused { folder, reason } => self.refusal(folder, reason),
        }
    }

    fn cargo(&self, refusal: &CargoRefusal) -> String {
        match refusal {
            CargoRefusal::NoManifest { searched_from } => {
                format!(
                    "there's no Cargo.toml in {} or above it",
                    self.folder(searched_from)
                )
            }
            CargoRefusal::Invalid { file, problem } => {
                format!("{} isn't valid: {problem}", self.folder(file))
            }
            CargoRefusal::BadInclude {
                file,
                include,
                problem,
            } => {
                let file = self.folder(file);
                match problem {
                    IncludeProblem::Missing => {
                        format!("{file} includes {include}, which doesn't exist")
                    }
                    IncludeProblem::NotToml => {
                        format!("{file} includes {include}, and cargo includes only .toml files")
                    }
                    IncludeProblem::Pattern => {
                        format!("{file} includes {include}, and cargo takes no patterns there")
                    }
                    IncludeProblem::Cycle => {
                        format!("{file} includes {include}, which was included already")
                    }
                }
            }
            CargoRefusal::Empty { origin } => {
                format!("{} is set to an empty string", self.origin(origin))
            }
            CargoRefusal::BadTemplate { origin, template } => format!(
                "{} is \"{template}\", with a placeholder cargo doesn't know",
                self.origin(origin)
            ),
            CargoRefusal::NotUnicode { origin } => {
                format!("{} isn't valid UTF-8", self.origin(origin))
            }
        }
    }

    fn refusal(&self, folder: &Path, reason: &Refusal) -> String {
        let folder = self.folder(folder);
        match reason {
            Refusal::Untagged { origin } => format!(
                "{folder} comes from {}, but cargo never marked it as a build folder of its own: \
                 it holds no CACHEDIR.TAG. Sutra cleans a folder that a setting names only when \
                 cargo made it, in case the setting points somewhere it shouldn't.",
                self.origin(origin)
            ),
            Refusal::Holds { held, .. } => {
                let held = match held {
                    Held::Workspace => "the workspace",
                    Held::Project => "this project",
                    Held::Home => "your home folder",
                    Held::CargoHome => "cargo's home folder",
                };
                format!("Cleaning {folder} would delete {held} with it.")
            }
            Refusal::Link => format!(
                "{folder} is a link. cargo clean would remove only the link, which frees \
                 nothing, so Sutra leaves it alone."
            ),
            Refusal::NotAFolder => format!("{folder} is a file, not a folder."),
            Refusal::OutsideGrant => format!(
                "{folder} lies outside the folder Sutra may use. Choose a folder that holds it, \
                 or run cargo clean yourself."
            ),
            Refusal::NotCargoMade => {
                format!("Sutra cleans only folders Cargo made, and {folder} doesn't look like one.")
            }
        }
    }

    /// A path inside the project as its path there, any other with `~` for the home folder.
    fn folder(&self, path: &Path) -> String {
        match path.strip_prefix(&self.project.path) {
            Ok(inside) if !inside.as_os_str().is_empty() => inside.display().to_string(),
            _ => format::tilde(path, self.home),
        }
    }

    /// A workspace by its project's name when it is the project, else by its folder.
    fn workspace(&self, root: &Path) -> String {
        if root == self.project.path {
            self.project.name.clone()
        } else {
            self.folder(root)
        }
    }

    fn origin(&self, origin: &Origin) -> String {
        match origin {
            Origin::Default => "cargo's default".to_string(),
            Origin::Env(var) => format!("{var} in Sutra's environment"),
            Origin::Config { key, file } => format!("{key} in {}", self.folder(file)),
        }
    }
}

/// What the question adds about the projects that may share the folders and may not: nothing when
/// there are none.
fn unknown_sharers(sharing: &Sharing) -> String {
    if sharing.unknown.is_empty() {
        return String::new();
    }
    format!(
        "\n\nSutra can't read the Cargo settings of {}, so it can't tell whether their builds go \
         too.",
        list(&sharing.unknown)
    )
}

/// A folder's size for the question: `≥` when part of it could not be read.
fn size_text(size: &Option<DirUsage>) -> String {
    size.map_or("size unknown".to_string(), |usage| {
        format::usage_size(&usage)
    })
}

/// `a`, `a and b`, `a, b and c`, and past three, `a, b, c and 2 more`.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, c, rest @ ..] => format!("{a}, {b}, {c} and {} more", rest.len()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::Measurement;
    use crate::map::clean::{CleanError, PlannedFolder};
    use crate::map::test_support::{cargo_target, project_at, usage};

    fn project() -> Project {
        project_at("/Users/dev/code/app")
    }

    fn clicked(rel: &str) -> ArtifactDir {
        cargo_target(
            PathBuf::from("/Users/dev/code/app").join(rel),
            Measurement::Pending,
        )
    }

    fn size(bytes: u64) -> Option<DirUsage> {
        Some(usage(bytes))
    }

    fn planned(path: &str, origin: Origin) -> PlannedFolder {
        PlannedFolder {
            path: PathBuf::from(path),
            origin,
        }
    }

    const HOME: &str = "/Users/dev";

    fn sharing(projects: &[&str]) -> Sharing {
        Sharing {
            projects: projects.iter().map(|name| name.to_string()).collect(),
            project_folders: Vec::new(),
            unknown: Vec::new(),
        }
    }

    #[test]
    fn the_question_names_each_folder_its_size_and_where_it_comes_from() {
        let project = project();
        let words = Context::new(&project, &clicked("target"), Path::new(HOME), false);
        let one = Plan::CargoClean(CleanPlan {
            workspace: project.path.clone(),
            folders: vec![planned("/Users/dev/code/app/target", Origin::Default)],
        });
        assert_eq!(
            words.question(&one, &[size(4_200_000_000)], &sharing(&[])),
            Question {
                title: "Clean target in app?".to_string(),
                detail: "Like cargo clean, this clears out target (4.2 GB, cargo's default).\n\n\
                         Cargo fills it again the next time it builds."
                    .to_string(),
                action: "Clean".to_string(),
            }
        );

        let config = Origin::Config {
            key: "build.build-dir",
            file: PathBuf::from("/Users/dev/code/app/.cargo/config.toml"),
        };
        let two = Plan::CargoClean(CleanPlan {
            workspace: project.path.clone(),
            folders: vec![
                planned("/Users/dev/code/app/target", Origin::Default),
                planned("/Users/dev/.cargo/build/36/b2ed60f52b47bc", config),
            ],
        });
        assert_eq!(
            words
                .question(
                    &two,
                    &[size(12_000_000), None],
                    &sharing(&["fwd", "naming-spaces"])
                )
                .detail,
            "Like cargo clean, this clears out:\n\
             target (12 MB, cargo's default)\n\
             ~/.cargo/build/36/b2ed60f52b47bc (size unknown, build.build-dir in .cargo/config.toml)\n\n\
             Cargo fills them again the next time it builds.\n\n\
             Builds of fwd and naming-spaces are in there too, and go with them."
        );
    }

    #[test]
    fn a_leftover_is_said_to_be_one_and_what_cargo_clean_deletes_instead_is_named() {
        let project = project();
        let words = Context::new(
            &project,
            &clicked("crates/a/target"),
            Path::new(HOME),
            false,
        );
        let leftover = Plan::DeleteFolder {
            folder: PathBuf::from("/Users/dev/code/app/crates/a/target"),
            cargo_clean: Some(CleanPlan {
                workspace: project.path.clone(),
                folders: vec![planned("/Users/dev/code/app/target", Origin::Default)],
            }),
        };
        assert_eq!(
            words.question(&leftover, &[size(1_200_000_000)], &sharing(&[])),
            Question {
                title: "Clean the leftover crates/a/target in app?".to_string(),
                detail: "cargo clean in app deletes target, not this folder, so this one is left \
                         over from an earlier build.\n\nCleaning it frees 1.2 GB."
                    .to_string(),
                action: "Clean".to_string(),
            }
        );
        let orphan = Plan::DeleteFolder {
            folder: PathBuf::from("/Users/dev/code/app/crates/a/target"),
            cargo_clean: None,
        };
        let shared = words.question(&orphan, &[size(1_000)], &sharing(&["fwd"]));
        assert_eq!(shared.title, "Clean crates/a/target in app?");
        assert_eq!(
            shared.detail,
            "Builds of fwd are in this folder, so it isn't left over.\n\n\
             Cleaning it frees 1 KB, and those builds go with it."
        );
    }

    #[test]
    fn a_folder_no_workspace_holds_is_said_to_be_cargos_cache_and_no_leftover() {
        let project = Project {
            name: "foundry e26ca359".to_string(),
            ..project_at("/private/tmp/claude-501/-Users-dev-vt-foundry/e26ca359")
        };
        let clicked = cargo_target(
            project.path.join("scratchpad/target-567-fix"),
            Measurement::Pending,
        );
        let words = Context::new(&project, &clicked, Path::new(HOME), false);
        let orphan = Plan::DeleteFolder {
            folder: clicked.path.clone(),
            cargo_clean: None,
        };

        assert_eq!(
            words.question(&orphan, &[size(84_000_000_000)], &sharing(&[])),
            Question {
                title: "Clean scratchpad/target-567-fix in foundry e26ca359?".to_string(),
                detail: "No Cargo.toml is beside this folder or above it, so no cargo clean \
                         reaches it. Cargo tagged it as its cache, and fills it again the next \
                         time it builds there.\n\nCleaning it frees 84 GB."
                    .to_string(),
                action: "Clean".to_string(),
            }
        );
    }

    #[test]
    fn a_folder_cargo_tagged_by_another_name_beside_a_workspace_is_its_cache_and_no_leftover() {
        let project = project();
        let words = Context::new(&project, &clicked("target-mas"), Path::new(HOME), false);
        let named_otherwise = Plan::DeleteFolder {
            folder: PathBuf::from("/Users/dev/code/app/target-mas"),
            cargo_clean: Some(CleanPlan {
                workspace: project.path.clone(),
                folders: vec![planned("/Users/dev/code/app/target", Origin::Default)],
            }),
        };

        assert_eq!(
            words.question(&named_otherwise, &[size(3_000_000_000)], &sharing(&[])),
            Question {
                title: "Clean target-mas in app?".to_string(),
                detail: "cargo clean in app deletes target, not this folder. Cargo tagged it as \
                         its cache, and fills it again the next time it builds there.\n\n\
                         Cleaning it frees 3 GB."
                    .to_string(),
                action: "Clean".to_string(),
            }
        );
    }

    #[test]
    fn projects_that_may_share_the_folder_are_named_after_either_question() {
        let project = project();
        let words = Context::new(&project, &clicked("target"), Path::new(HOME), false);
        let plan = Plan::DeleteFolder {
            folder: PathBuf::from("/Users/dev/code/app/target"),
            cargo_clean: None,
        };
        let sharing = Sharing {
            projects: Vec::new(),
            project_folders: Vec::new(),
            unknown: vec!["sealed".to_string(), "shut".to_string()],
        };
        assert!(words.question(&plan, &[size(1_000)], &sharing).detail.ends_with(
            "Cleaning it frees 1 KB.\n\nSutra can't read the Cargo settings of sealed and shut, \
             so it can't tell whether their builds go too."
        ));
        let plan = Plan::CargoClean(CleanPlan {
            workspace: project.path.clone(),
            folders: vec![planned("/Users/dev/code/app/target", Origin::Default)],
        });
        assert!(words.question(&plan, &[size(1_000)], &sharing).detail.ends_with(
            "the next time it builds.\n\nSutra can't read the Cargo settings of sealed and shut, \
             so it can't tell whether their builds go too."
        ));
    }

    #[test]
    fn a_build_using_a_folder_is_a_refusal_naming_the_folder() {
        let project = project();
        let words = Context::new(&project, &clicked("target"), Path::new(HOME), false);
        let busy = PrepareError::Locks(CleanError::BuildRunning {
            lock: PathBuf::from("/Users/dev/code/app/target/debug/.cargo-build-lock"),
        });
        assert_eq!(
            words.refused(&busy),
            "A build is using target/debug. Try again once it finishes."
        );
    }

    #[test]
    fn a_folder_another_question_asks_about_is_a_refusal_naming_it() {
        let project = project();
        let words = Context::new(&project, &clicked("target"), Path::new(HOME), false);
        assert_eq!(
            words.asked_already(Path::new("/Users/dev/.cargo/build/36")),
            "A question about cleaning ~/.cargo/build/36 is waiting for your answer. Try again \
             once you've answered it."
        );
    }

    #[test]
    fn refusals_name_the_folder_and_the_reason() {
        let project = project();
        let words = Context::new(&project, &clicked("target"), Path::new(HOME), true);
        let untagged = PlanError::Refused {
            folder: PathBuf::from("/Users/dev/out"),
            reason: Refusal::Untagged {
                origin: Origin::Env("CARGO_TARGET_DIR"),
            },
        };
        assert!(words.plan_refused(&untagged).starts_with(
            "~/out comes from CARGO_TARGET_DIR in Sutra's environment, but cargo never marked it"
        ));
        let holds = PlanError::Refused {
            folder: PathBuf::from("/Users/dev"),
            reason: Refusal::Holds {
                held: Held::Home,
                path: PathBuf::from("/Users/dev"),
            },
        };
        assert_eq!(
            words.plan_refused(&holds),
            "Cleaning ~ would delete your home folder with it."
        );
        let outside = PlanError::Unreadable {
            path: PathBuf::from("/.cargo/config.toml"),
            kind: io::ErrorKind::PermissionDenied,
        };
        assert_eq!(
            words.plan_refused(&outside),
            "cargo clean reads /.cargo/config.toml, which lies outside the folder Sutra may use."
        );
        let invalid = PlanError::Cargo(CargoRefusal::Invalid {
            file: PathBuf::from("/Users/dev/code/app/.cargo/config.toml"),
            problem: "unclosed table, expected `]` on line 1".to_string(),
        });
        assert_eq!(
            words.plan_refused(&invalid),
            "cargo clean would fail here too: .cargo/config.toml isn't valid: unclosed table, \
             expected `]` on line 1."
        );
        assert_eq!(words.cannot_clean(), "Can't clean target in app");
    }

    #[test]
    fn a_folder_that_changed_before_the_clean_started_is_named() {
        let changed = CleanError::Changed {
            folder: PathBuf::from("/Users/dev/code/app/target"),
        };
        assert_eq!(
            stopped(
                Path::new("/Users/dev/code/app/target"),
                Some(&changed),
                0,
                Path::new(HOME)
            )
            .1,
            "~/code/app/target changed before the clean could start. Nothing was deleted."
        );
    }

    #[test]
    fn lists_name_three_and_count_the_rest() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        assert_eq!(list(&names[..1]), "a");
        assert_eq!(list(&names[..2]), "a and b");
        assert_eq!(list(&names[..3]), "a, b and c");
        assert_eq!(list(&names), "a, b, c and 2 more");
    }
}
