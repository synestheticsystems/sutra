//! A line on the list: a group's, which names the upstream its cards came from, or within a group,
//! a checkout's, which holds the checkout's own card first, then the copies made from it. Each says
//! what all it holds takes on disk, and offers to clean the build folders in it; a group's offers
//! a choice of its own of cleaning them by itself once they sit idle a while, and says so while
//! that differs from the choice for every group. Folded shut (see `fold`), a line still says what
//! its cards would of their agents and their servers. Docker's group has a line too (see
//! `list::docker`).

mod fold;

pub(super) use fold::first_few;
pub use fold::{LineFold, MoreLozenges};

use std::path::Path;

use crate::dashboard::cleaning::{self, CleanWhenIdle, Cleaning};
use crate::dashboard::contents::card::{self, CardContext};
use crate::dashboard::listing::{Checkout, Group};
use crate::dashboard::{Action, MenuAction, MenuEntry, format};
use crate::map::{ArtifactDir, DirUsage, FolderAccess, FolderSize, GroupKey, Project};
pub(super) use fold::folded;

/// A line over rows of the list.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Line {
    /// What the line stands for, which the window knows it by, and a click on it names.
    pub key: LineKey,
    /// What it goes by: a group by its upstream's name, a checkout by its folder's, each made longer
    /// where another beside it would read alike.
    pub name: String,
    /// What it stands for, in full.
    pub tooltip: String,
    /// The disk all it holds on the map takes, listed or not, as their trays write it: the sizes
    /// measured so far, added up, and `None` before the first.
    pub total: Option<String>,
    /// The quiet mark beside its total while the user chose from a group's line how long its build
    /// folders may sit idle, or never, otherwise than for every group; `None` while it did not.
    pub when_idle_mark: Option<WhenIdleMark>,
    /// What a right click offers: the clean of every build folder in what it holds that a card
    /// would offer, while one is on offer; and on a group's line, how long its build folders may
    /// sit idle before Sutra cleans them by itself.
    pub menu: Vec<MenuEntry>,
    /// Whether its rows show under it, or it stands folded shut over them.
    pub fold: LineFold,
}

/// The mark on a group's line whose own choice of how long its build folders may sit idle differs
/// from the choice for every group, so that the exceptions stand out once every group cleans
/// itself. Its tip says what applies to the group.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WhenIdleMark {
    /// Its build folders are cleaned once idle for a time of its own.
    OwnTime { tip: String },
    /// Its build folders are never cleaned for sitting idle, where every other group's are.
    Never { tip: String },
}

/// What a line stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LineKey {
    /// A group's line, by what the group is keyed by.
    Group { kind: GroupKind },
    /// Within a group, the line of a checkout and the copies made from it, by its folder.
    Checkout { folder: String },
}

/// What a group is keyed by, which its line names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum GroupKind {
    /// The remote its checkouts' chains of origins end at, as `github.com/owner/repo` reads.
    Upstream { remote: String },
    /// A repository with no remote anywhere, by the folder at the root of its checkouts' chain.
    LocalRepository { folder: String },
    /// Folders with no git, by the package they build.
    Package { name: String },
    /// What nothing else groups, and the agents working outside every project.
    Other,
    /// Docker's containers, and the disk it takes.
    Docker,
}

impl GroupKind {
    /// What the group `key` names is keyed by.
    fn of(key: &GroupKey) -> GroupKind {
        match key {
            GroupKey::Upstream(remote) => GroupKind::Upstream {
                remote: remote.clone(),
            },
            GroupKey::LocalRepository(folder) => GroupKind::LocalRepository {
                folder: format::path_text(folder),
            },
            GroupKey::Package(name) => GroupKind::Package { name: name.clone() },
            GroupKey::Other => GroupKind::Other,
        }
    }
}

impl LineKey {
    /// The key of the line of the group `key` names.
    pub(in crate::dashboard) fn group(key: &GroupKey) -> LineKey {
        LineKey::Group {
            kind: GroupKind::of(key),
        }
    }

    /// The key of the line of the checkout at `folder`.
    pub(in crate::dashboard) fn checkout(folder: &Path) -> LineKey {
        LineKey::Checkout {
            folder: format::path_text(folder),
        }
    }
}

/// The line of `group`, folded shut or not as `fold` says, whose build folders Sutra cleans once
/// idle as `clean_when_idle` says of it.
pub(super) fn group_line(
    group: &Group,
    fold: LineFold,
    clean_when_idle: &CleanWhenIdle,
    cards: &CardContext,
) -> Line {
    let key = LineKey::group(group.key);
    let what = match group.key {
        GroupKey::Upstream(remote) => remote.clone(),
        GroupKey::LocalRepository(folder) => format!(
            "{}\nA repository with no remote",
            format::tilde(folder, cards.home)
        ),
        GroupKey::Package(name) => format!("Package {name}\nFolders with no git that build it"),
        GroupKey::Other => {
            "What no upstream, repository or package groups, and the agents working outside \
             every project"
                .to_string()
        }
    };
    // A line standing for its one checkout too says where that is, unless its key says so already.
    let said = |checkout: &&Project| match group.key {
        GroupKey::LocalRepository(folder) => *folder == checkout.path,
        _ => false,
    };
    let tooltip = match group.checkout.filter(|checkout| !said(checkout)) {
        Some(checkout) => format!("{what}\n{}", format::tilde(&checkout.path, cards.home)),
        None => what,
    };
    let mut menu = clean_build_folders(&key, &group.members, cards.cleaning);
    if !menu.is_empty() {
        menu.push(MenuEntry::Separator);
    }
    menu.push(when_idle_choices(group.key, clean_when_idle));
    Line {
        menu,
        key,
        name: group.name.clone(),
        tooltip,
        total: total_of(&group.members, cards),
        when_idle_mark: clean_when_idle.mark_of(group.key),
        fold,
    }
}

/// The line of `checkout`, folded shut or not as `fold` says.
pub(super) fn checkout_line(checkout: &Checkout, fold: LineFold, cards: &CardContext) -> Line {
    let key = LineKey::checkout(&checkout.project.path);
    Line {
        menu: clean_build_folders(&key, &checkout.members, cards.cleaning),
        key,
        name: checkout.name.clone(),
        tooltip: format::tilde(&checkout.project.path, cards.home),
        total: total_of(&checkout.members, cards),
        when_idle_mark: None,
        fold,
    }
}

/// What the line of the group `group` offers for cleaning its build folders by itself: first to
/// follow the choice for every group, which it names, then a choice of its own, never or how long
/// they may sit idle; the one that applies, as `clean_when_idle` says, checked.
fn when_idle_choices(group: &GroupKey, clean_when_idle: &CleanWhenIdle) -> MenuEntry {
    let own = clean_when_idle.own_choice_of(group);
    let choose = |own| MenuAction::Dashboard {
        action: Action::CleanWhenIdle {
            group: GroupKind::of(group),
            own,
        },
    };
    let follow = MenuEntry::Item {
        title: cleaning::as_in_settings(clean_when_idle.for_every_group()),
        checked: own.is_none(),
        action: choose(None),
    };
    let own_choices = cleaning::when_idle_choices()
        .into_iter()
        .map(|choice| MenuEntry::Item {
            title: choice.label,
            checked: own == Some(choice.choice),
            action: choose(Some(choice.choice)),
        });
    let entries = [follow, MenuEntry::Separator]
        .into_iter()
        .chain(own_choices);
    MenuEntry::Submenu {
        title: "Clean When Idle".to_string(),
        entries: entries.collect(),
    }
}

/// The build folders in `members`, what a line holds on the map, that its menu offers to clean,
/// each with the project it is in: those a card would offer to clean, in the folders Sutra reads.
/// A folder it cannot read offers none, as a scratch folder kept on the map while its scratchpad
/// cannot be listed is: the clean could only fail.
pub(in crate::dashboard) fn offered_cleans<'a>(
    members: &[&'a Project],
    cleaning: &Cleaning,
) -> Vec<(&'a Project, &'a ArtifactDir)> {
    members
        .iter()
        .copied()
        .filter(|member| member.access == FolderAccess::Readable)
        .flat_map(|member| member.artifacts.iter().map(move |dir| (member, dir)))
        .filter(|(_, dir)| card::offers_clean(dir, cleaning))
        .collect()
}

/// What a right click on the line of `line` offers: one clean of every build folder in `members`
/// on offer (see `offered_cleans`), with what they hold between them, which is what the clean can
/// give back, rather than the line's total; nothing while none is on offer.
fn clean_build_folders(
    line: &LineKey,
    members: &[&Project],
    cleaning: &Cleaning,
) -> Vec<MenuEntry> {
    let offered = offered_cleans(members, cleaning);
    if offered.is_empty() {
        return Vec::new();
    }
    let sized: Vec<&DirUsage> = offered
        .iter()
        .filter_map(|(_, dir)| dir.measurement.usage())
        .collect();
    let title = if sized.is_empty() {
        "Clean Build Folders".to_string()
    } else {
        let bytes = sized.iter().map(|usage| usage.bytes).sum();
        // What a folder not sized yet, or not read in full, holds adds to the sum unseen.
        let partial = sized.len() < offered.len() || sized.iter().any(|u| u.unreadable_dirs > 0);
        format!("Clean Build Folders ({})", format::size(bytes, partial))
    };
    vec![MenuEntry::Item {
        title,
        checked: false,
        action: MenuAction::Dashboard {
            action: Action::CleanBuildFolders { line: line.clone() },
        },
    }]
}

/// What `members` take on disk between them, as their trays write it: the totals the trays show,
/// added up, at least that when part of one could not be read. A member still being sized adds
/// nothing, and the sum says nothing of it; `None` while none is sized.
fn total_of(members: &[&Project], cards: &CardContext) -> Option<String> {
    let total = members
        .iter()
        .filter_map(|member| card::tray_total(member, cards))
        .reduce(|total, size| FolderSize {
            bytes: total.bytes + size.bytes,
            partial: total.partial || size.partial,
        })?;
    Some(format::size(total.bytes, total.partial))
}

#[cfg(test)]
pub(super) mod tests;
