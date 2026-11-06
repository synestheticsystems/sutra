//! Claude Code records on each entry of a subagent's transcript the folder it runs the subagent
//! in, which a `cd` in the subagent's commands does not change (see `Tail::cwd`). A subagent that a
//! session sets to work in another folder starts in the session's and gets there by `cd` and by
//! absolute paths, so where it works is read from its tool calls: the folders its shell commands
//! enter, and those holding the files it edits or writes. Reading or searching a folder is looking
//! there, not working there, and tells nothing.

use std::path::Path;

use serde::Deserialize;

/// How many folders of each kind of act are kept: the newest, each once. A subagent at work in a
/// worktree also steps into folders no project holds, as its scratch folder, a folder of notes or
/// a memory folder, and the map passes over those for the newest folder that a project holds (see
/// `Place::of_subagent`). Keeping only the newest of each kind would have sent the subagents of
/// the last two weeks on Daniel's Mac back to the card of the folder they run in for 427 spells,
/// of 13 requests on average, after a `cd` had put them on a project's card; keeping three, for 46
/// spells of 40, and keeping four, for 27 (2026-11-06). Each folder kept is one more to resolve
/// on every load while the subagent works.
const KEPT: usize = 3;

/// Where a subagent acted, as the newest of its tool calls that tell it say.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::map::agents::claude) struct ActedIn {
    /// The folders the `cd`s in its shell commands entered (see `entered`), newest first.
    entered: Vec<String>,
    /// The folders holding the files it changed, newest first, with Claude Code's Edit,
    /// MultiEdit, Write or NotebookEdit tool, each of which takes a file's absolute path.
    edited: Vec<String>,
}

/// The fields of a tool call's input that tell where it acts. Each tool names its own fields, so
/// these are read loosely, as any JSON value: one of another type costs only what it would tell,
/// not what its entry tells of the turn.
#[derive(Deserialize)]
pub(super) struct ToolInput {
    /// The shell command Bash runs.
    #[serde(default)]
    command: serde_json::Value,
    /// The file Edit, MultiEdit and Write change.
    #[serde(default)]
    file_path: serde_json::Value,
    /// The notebook NotebookEdit changes.
    #[serde(default)]
    notebook_path: serde_json::Value,
}

impl ActedIn {
    /// Where a call of the tool `tool` with `input` acts.
    pub(super) fn by_call(tool: &str, input: &ToolInput) -> ActedIn {
        let file = match tool {
            "Edit" | "MultiEdit" | "Write" => input.file_path.as_str(),
            "NotebookEdit" => input.notebook_path.as_str(),
            _ => None,
        };
        let command = input.command.as_str().filter(|_| tool == "Bash");
        ActedIn {
            entered: command.and_then(entered).into_iter().collect(),
            edited: file.and_then(folder_holding).into_iter().collect(),
        }
    }

    /// These acts, followed by `older`, the acts before them: of each kind, the newest `KEPT`
    /// folders, each once.
    pub(super) fn after(mut self, older: &ActedIn) -> ActedIn {
        keep_newest(&mut self.entered, &older.entered);
        keep_newest(&mut self.edited, &older.edited);
        self
    }

    /// Whether these hold as many folders of each kind as are kept, so that no older act can
    /// tell more.
    pub(super) fn is_whole(&self) -> bool {
        self.entered.len() == KEPT && self.edited.len() == KEPT
    }

    /// The folders the subagent acted in, the most telling first, each once: those its `cd`s
    /// entered, newest first, then those holding the files it edited, however much newer the
    /// edits. A subagent at work in a worktree `cd`s into it for nearly every command, and also
    /// writes its notes and scratch files outside it: 1,016 of the 5,273 edits that subagents made
    /// after a `cd` into a worktree, in the last two weeks on Daniel's Mac, lay outside it
    /// (2026-11-05).
    pub(in crate::map::agents::claude) fn folders(&self) -> impl Iterator<Item = &str> {
        let edited = self.edited.iter();
        let edited = edited.filter(|folder| !self.entered.contains(folder));
        self.entered.iter().chain(edited).map(String::as_str)
    }
}

/// Extends `newer`, the newest folders of one kind, with those of `older` it does not hold, up to
/// `KEPT` in all.
fn keep_newest(newer: &mut Vec<String>, older: &[String]) {
    for folder in older {
        if newer.len() >= KEPT {
            break;
        }
        if !newer.contains(folder) {
            newer.push(folder.clone());
        }
    }
}

/// The folder holding the file at `path`, when that is an absolute path.
fn folder_holding(path: &str) -> Option<String> {
    let path = Path::new(path);
    let folder = path.parent().filter(|_| path.is_absolute())?;
    folder.to_str().map(str::to_string)
}

/// The words after which the shell reads a command's first word again, as `then` in
/// `if …; then cd /code/app; fi`.
const LEAD_INTO_A_COMMAND: [&str; 9] = [
    "!", "{", "do", "elif", "else", "if", "then", "until", "while",
];

/// The folder the last `cd` in the shell command `command` enters that Sutra can read without a
/// shell: an absolute path, quoted or not, holding no `$`, backquote or `~`, which the shell would
/// expand. A `cd` to anything else is passed over, as is a `cd` that is no command's first word,
/// as in `echo cd /tmp`. Of the 81,864 `cd`s in the commands of the subagents of the last two weeks
/// on Daniel's Mac, 67,758 named an absolute path, 10,446 a variable, 2,189 a folder under `~` and
/// 1,389 a relative path (2026-11-05).
fn entered(command: &str) -> Option<String> {
    let mut cds = Cds::new();
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => cds.end_word(),
            '\n' | ';' | '&' | '|' | '(' | ')' => cds.end_command(),
            // A redirection, whose target is no folder a `cd` enters.
            '<' | '>' => cds.end_word(),
            '\'' => {
                cds.begin_word();
                chars
                    .by_ref()
                    .take_while(|&c| c != '\'')
                    .for_each(|c| cds.push(c));
            }
            '"' => {
                cds.begin_word();
                while let Some(c) = chars.next().filter(|&c| c != '"') {
                    // The shell keeps a backslash before most characters here, but no folder a
                    // `cd` names is told apart by one.
                    let c = if c == '\\' { chars.next() } else { Some(c) };
                    c.into_iter().for_each(|c| cds.push(c));
                }
            }
            // Outside quotes a backslash makes the character after it plain, and a newline after
            // it continues the line.
            '\\' => chars
                .next()
                .filter(|&c| c != '\n')
                .into_iter()
                .for_each(|c| cds.push(c)),
            '#' if !cds.in_word => {
                chars.by_ref().take_while(|&c| c != '\n').for_each(drop);
                cds.end_command();
            }
            c => cds.push(c),
        }
    }
    cds.end_word();
    cds.entered
}

/// A reading of a shell command, word by word, for the folder its last `cd` enters.
struct Cds {
    /// The folder the last `cd` read so far enters, when Sutra can read it.
    entered: Option<String>,
    /// The word being read, with its quotes and escapes taken out.
    word: String,
    /// Whether a word is being read: quotes begin one, even one they leave empty.
    in_word: bool,
    /// Whether the word being read holds a character the shell would expand.
    expands: bool,
    /// Whether the word being read is a command's first, where `cd` names the command.
    first: bool,
    /// Whether the word being read names the folder a `cd` enters.
    target: bool,
}

impl Cds {
    fn new() -> Cds {
        Cds {
            entered: None,
            word: String::new(),
            in_word: false,
            expands: false,
            first: true,
            target: false,
        }
    }

    fn begin_word(&mut self) {
        self.in_word = true;
    }

    fn push(&mut self, c: char) {
        self.in_word = true;
        self.expands |= matches!(c, '$' | '`' | '~');
        self.word.push(c);
    }

    /// The word being read ends: a `cd` begins, or its target is read, or the command goes on.
    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        if self.target {
            if self.word.starts_with('/') && !self.expands {
                // `/code/app/` and `/code/app` are one folder, kept once, as an edit's is named.
                let folder = self.word.trim_end_matches('/');
                let folder = if folder.is_empty() { "/" } else { folder };
                self.entered = Some(folder.to_string());
            }
            self.target = false;
        } else if self.first {
            self.target = self.word == "cd";
            self.first = LEAD_INTO_A_COMMAND.contains(&self.word.as_str());
        }
        self.word.clear();
        self.in_word = false;
        self.expands = false;
    }

    /// An operator ends the command being read, and the next word begins another. A `cd` with no
    /// folder named, which goes home, enters none Sutra reads.
    fn end_command(&mut self) {
        self.end_word();
        self.first = true;
        self.target = false;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_cd_to_an_absolute_path_enters_it_and_the_last_in_a_command_counts() {
        let cases = [
            ("cd /code/app-wt/fix && cargo test", "/code/app-wt/fix"),
            (
                r#"cd "/code/notes/handoff 2" && ls"#,
                "/code/notes/handoff 2",
            ),
            ("cd '/code/app' ; git status", "/code/app"),
            ("cd /code/app/ && make", "/code/app"),
            ("cd / && ls", "/"),
            ("cd /code/notes\\ 2 && ls", "/code/notes 2"),
            ("(cd /code/app && make) 2>&1", "/code/app"),
            ("cd /code/app>/dev/null && make", "/code/app"),
            ("git status; cd /code/site\nnpm test", "/code/site"),
            ("cd /code/app && cd /code/site && make", "/code/site"),
            ("if [ -d /code/app ]; then cd /code/app; fi", "/code/app"),
            ("{ cd /code/app; make; }", "/code/app"),
        ];
        for (command, folder) in cases {
            assert_eq!(entered(command).as_deref(), Some(folder), "{command}");
        }
    }

    #[test]
    fn a_cd_the_shell_would_have_to_expand_is_passed_over_for_the_one_before() {
        let cases = [
            "cd $WT/fix-2 && make",
            r#"cd "$HOME/code/app""#,
            "cd ~/code/app",
            "cd `git rev-parse --show-toplevel`",
            "cd /code/app/$(cat branch)",
        ];
        for command in cases {
            assert_eq!(entered(command), None, "{command}");
            let after_one = format!("cd /code/app-wt/fix && make; {command}");
            assert_eq!(
                entered(&after_one).as_deref(),
                Some("/code/app-wt/fix"),
                "{after_one}"
            );
        }
    }

    #[test]
    fn a_relative_or_missing_folder_or_a_cd_that_is_no_command_enters_nothing() {
        let cases = [
            "cd crates/core && cargo test",
            "cd ../.. && ls",
            "cd",
            "cd && ls",
            "cd -",
            "cd -P /code/app",
            "cd '' /code/app",
            "echo cd /code/app",
            r#"echo "done; cd /code/app""#,
            "git -C /code/app status",
            "ls # cd /code/app",
            "# cd /code/app\nls",
        ];
        for command in cases {
            assert_eq!(entered(command), None, "{command}");
        }
        assert_eq!(
            entered("cd /code/app && cd crates/core").as_deref(),
            Some("/code/app"),
            "a relative cd is passed over for the one before"
        );
    }

    /// Where the call of `tool` with `input` acts, as its entry is read.
    fn acted_in(tool: &str, input: serde_json::Value) -> ActedIn {
        let input = serde_json::from_value(input).expect("an object");
        ActedIn::by_call(tool, &input)
    }

    /// Acts that entered the folders `entered` and edited in the folders `edited`, newest first.
    fn acts(entered: &[&str], edited: &[&str]) -> ActedIn {
        let folders = |folders: &[&str]| folders.iter().map(|folder| folder.to_string()).collect();
        ActedIn {
            entered: folders(entered),
            edited: folders(edited),
        }
    }

    #[test]
    fn a_shell_command_enters_a_folder_and_an_edit_or_write_works_in_the_files_folder() {
        let command = json!({"command": "cd /code/app && cargo test", "description": "Test"});
        assert_eq!(acted_in("Bash", command), acts(&["/code/app"], &[]));
        let file =
            json!({"file_path": "/code/notes/plan.md", "old_string": "a", "new_string": "b"});
        for tool in ["Edit", "MultiEdit", "Write"] {
            let edited = acted_in(tool, file.clone());
            assert_eq!(edited, acts(&[], &["/code/notes"]), "{tool}");
        }
        let notebook = json!({"notebook_path": "/code/lab/run.ipynb", "new_source": "x = 1"});
        assert_eq!(
            acted_in("NotebookEdit", notebook),
            acts(&[], &["/code/lab"])
        );
    }

    #[test]
    fn looking_at_a_folder_or_naming_a_file_by_a_relative_path_or_oddly_tells_nothing() {
        for (tool, input) in [
            ("Read", json!({"file_path": "/code/site/index.html"})),
            ("Grep", json!({"pattern": "fn main", "path": "/code/site"})),
            ("Glob", json!({"pattern": "**/*.rs", "path": "/code/site"})),
            ("Edit", json!({"file_path": "src/main.rs"})),
            ("Write", json!({"file_path": 7})),
            ("Bash", json!({"command": ["cd", "/code/site"]})),
            ("Bash", json!({})),
        ] {
            assert_eq!(acted_in(tool, input.clone()), ActedIn::default(), "{input}");
        }
    }

    #[test]
    fn of_each_kind_the_newest_folders_are_kept_each_once_up_to_a_few() {
        let older = acts(
            &["/code/app-wt/fix", "/code/app"],
            &["/code/app-wt/fix/src"],
        );
        let newer = acts(&["/tmp/scratch", "/code/app-wt/fix"], &["/code/notes"]);
        let acted = newer.after(&older);
        assert_eq!(
            acted,
            acts(
                &["/tmp/scratch", "/code/app-wt/fix", "/code/app"],
                &["/code/notes", "/code/app-wt/fix/src"]
            ),
            "the newer first, a folder entered again once"
        );
        assert!(!acted.is_whole(), "an older edit may still tell");

        let full = acts(&["/a", "/b", "/c"], &["/d", "/e", "/f"]);
        assert!(full.is_whole());
        let newest = acts(&["/z"], &[]).after(&full);
        let kept = acts(&["/z", "/a", "/b"], &["/d", "/e", "/f"]);
        assert_eq!(newest, kept, "no more than {KEPT} of a kind");
    }

    #[test]
    fn the_folders_entered_come_before_those_edited_in_and_each_comes_once() {
        let acted = acts(
            &["/tmp/scratch", "/code/app-wt/fix"],
            &["/code/notes", "/code/app-wt/fix"],
        );
        let folders: Vec<&str> = acted.folders().collect();
        assert_eq!(
            folders,
            ["/tmp/scratch", "/code/app-wt/fix", "/code/notes"],
            "however much newer the edits"
        );
        assert_eq!(ActedIn::default().folders().count(), 0);
    }
}
