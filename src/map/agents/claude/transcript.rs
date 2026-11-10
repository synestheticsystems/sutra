//! Claude Code keeps a transcript of each session and of each subagent, which holds one JSON entry
//! per line, appended as the agent works: the prompt and tool results handed to the model, each
//! request sent to it, its replies, and the context recorded around them. Only its end is read: of
//! a subagent's, what its turn is doing and where it works (see `Tail`), and of a session's, the
//! model of its newest reply (see `newest_model`) and the name it last went by (see
//! `newest_name`). The format is undocumented; what is read here was checked against 2,200
//! subagent transcripts written by Claude Code 2.1.18 to 2.1.317, the model on each reply against
//! the newest 400 subagent transcripts and all 325 session transcripts, and the tool calls that
//! tell where a subagent acts against the 923 subagent transcripts of the last two weeks
//! (2026-11-05); the names against the 71 transcripts of the sessions in the scratch root
//! (2026-11-07).

mod acted_in;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;
#[cfg(test)]
use std::time::{Duration, UNIX_EPOCH};

use serde::Deserialize;

use self::acted_in::{ActedIn, ToolInput};
use crate::map::utc;

/// How much of a transcript's end is read. The newest entry that tells of a subagent's turn began
/// within the last 80 KB of each of 2,200 transcripts, and the newest reply of 305 of 313 session
/// transcripts lay within it (2026-11-05); only a tool's result is likely to be larger.
pub(super) const TAIL_BYTES: u64 = 128 * 1024;

/// The model Claude Code records on a reply it made up itself, such as the note of an API error,
/// which no model wrote: on 427 replies in 118 of the 325 session transcripts, and on 17 in the
/// newest 400 subagent transcripts (2026-11-05).
const MADE_UP: &str = "<synthetic>";

/// The entry kinds that tell whether the turn goes on, as they appear in a line.
const TURN_KINDS: [&str; 3] = [
    r#""type":"assistant""#,
    r#""type":"user""#,
    r#""type":"api-request""#,
];

/// The entry kinds that name the session, as they appear in a line: the title the user or the
/// Claude app gave it, and the name it goes by among agents, which the registry's `name` repeats
/// while it runs. Claude Code records both after each prompt, so the newest lies near the end.
const NAME_KINDS: [&str; 2] = [r#""type":"custom-title""#, r#""type":"agent-name""#];

/// What the end of a subagent's transcript says about it: the newest entry that tells of its turn,
/// and while the turn goes on, the folder Claude Code runs the subagent in, where it acted and the
/// model it works with.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Tail {
    pub(super) mid_turn: bool,
    /// When that entry was written, by its own date. A transcript's modification time also moves
    /// when nothing is added to it: on Daniel's Mac, dozens of old transcripts at a time, several
    /// times a week (2026-11-05).
    pub(super) written: Option<SystemTime>,
    /// The working folder the newest entry naming one records, as Claude Code wrote it. Each of
    /// the subagent's prompts, replies and results names the folder Claude Code runs it in: its
    /// worktree when it has one, and else its session's folder as its turn began. A `cd` in its
    /// commands does not move it: 860 of the 923 subagent transcripts of the last two weeks on
    /// Daniel's Mac name one folder throughout, and 633 of those `cd` out of it (2026-11-05), so
    /// where the subagent acted tells more of where it works (see `acted_in`). A request to the
    /// model names no folder. Only a subagent mid-turn is placed by its folder, so only then is it
    /// looked for. `None` when no entry in the end read names one: in 1,594 of 73,826 moments
    /// mid-turn in the transcripts on Daniel's Mac (2026-11-05), the newest that did began further
    /// back, behind a large result.
    pub(super) cwd: Option<String>,
    /// Where the subagent acted, as the newest tool calls in the end read that tell it say. The
    /// end alone often holds none, as a subagent reads far more than it acts: of the 66,255
    /// requests that subagents sent after a `cd` in the last two weeks on Daniel's Mac, 11,014
    /// came more than the end read after the newest one, and of the 35,943 sent after an edit,
    /// 21,514 did (2026-11-05). So what an earlier load read holds, behind any newer act (see
    /// `after`). Like the folder, it is looked for only mid-turn.
    pub(super) acted_in: ActedIn,
    /// The model the newest reply in the end read names, by the id Claude Code records, as
    /// `claude-opus-5-5` reads: the model that ran, where the subagent's meta file holds only the
    /// alias it was asked for. Like the folder, it is looked for only mid-turn. `None` before the
    /// subagent first replies, and when no reply in the end read names one.
    pub(super) model: Option<String>,
}

impl Tail {
    /// This end of a transcript, read after `earlier`, its end before it grew. Claude Code only
    /// appends, so mid-turn the folder and the model `earlier` named still hold while this end
    /// names none, and the acts it named hold behind those this end names.
    pub(super) fn after(mut self, earlier: &Tail) -> Tail {
        if self.mid_turn {
            if self.cwd.is_none() {
                self.cwd.clone_from(&earlier.cwd);
            }
            if self.model.is_none() {
                self.model.clone_from(&earlier.model);
            }
            self.acted_in = self.acted_in.after(&earlier.acted_in);
        }
        self
    }
}

/// The bytes of the transcript at `path` from `from` to `len`, its length when listed.
pub(super) fn read_span(path: &Path, from: u64, len: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    // Claude Code may append while this reads; the limit keeps the read to the span measured.
    let span = len.saturating_sub(from);
    file.take(span).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// What the end of the transcript at `path`, `len` bytes long when listed, says, given that it was
/// last read when `read_before` bytes long, or never for `0`.
pub(super) fn read_tail(path: &Path, len: u64, read_before: u64) -> io::Result<Tail> {
    let from = len.saturating_sub(TAIL_BYTES);
    let bytes = read_span(path, from, len)?;
    // The earlier reading was no longer than this one, so it ended at most `TAIL_BYTES` into it.
    Ok(tail_of(&bytes, read_before.saturating_sub(from) as usize))
}

/// What the newest entry in `bytes` that tells of the turn says, with, mid-turn, the folder the
/// newest entry naming one records, the model the newest reply names, and where the newest tool
/// calls that tell it acted. The window's first line may begin partway through an entry, and its
/// last may be one Claude Code is still appending; neither parses, so neither counts.
///
/// The calls are looked for only in the lines that a reading of the window's first `read_before`
/// bytes did not see whole. Claude Code only appends, so that reading found the acts in the lines
/// before already (see `Tail::after`), or, at rest, none, which a subagent resumed after its turn
/// ended starts afresh without. A subagent's calls are so few among its entries that looking for
/// them through the whole window made each read four times as long, on Daniel's Mac (2026-11-06).
fn tail_of(bytes: &[u8], read_before: usize) -> Tail {
    let (seen, appended) = bytes.split_at(appended_from(bytes, read_before));
    let appended = newest_first(appended).map(|line| (line, true));
    let mut lines = appended.chain(newest_first(seen).map(|line| (line, false)));
    let newest = lines.find_map(|(line, appended)| Some((turn_entry(line)?, appended)));
    let Some((mut tail, appended)) = newest else {
        return Tail::default();
    };
    if !tail.mid_turn {
        return Tail {
            mid_turn: false,
            written: tail.written,
            ..Tail::default()
        };
    }
    if !appended {
        tail.acted_in = ActedIn::default();
    }
    for (line, appended) in lines {
        let acts_to_find = appended && !tail.acted_in.is_whole();
        if tail.cwd.is_some() && tail.model.is_some() && !acts_to_find {
            break;
        }
        let Some(older) = turn_entry(line) else {
            continue;
        };
        tail.cwd = tail.cwd.or(older.cwd);
        tail.model = tail.model.or(older.model);
        if appended {
            tail.acted_in = tail.acted_in.after(&older.acted_in);
        }
    }
    tail
}

/// Where the lines of `bytes` begin that a reading of its first `read_before` bytes did not see
/// whole: after the last newline among those bytes.
fn appended_from(bytes: &[u8], read_before: usize) -> usize {
    let seen = &bytes[..read_before.min(bytes.len())];
    let newline = seen.iter().rposition(|&byte| byte == b'\n');
    newline.map_or(0, |newline| newline + 1)
}

/// The lines of `bytes`, newest first.
fn newest_first(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|&byte| byte == b'\n').rev()
}

/// The model the newest reply among the lines of `bytes` names (see `Tail::model`), or `None` when
/// no reply there names one.
pub(super) fn newest_model(bytes: &[u8]) -> Option<String> {
    let lines = bytes.split(|&byte| byte == b'\n');
    let mut entries = lines.rev().filter_map(turn_entry);
    entries.find_map(|entry| entry.model)
}

/// The name the newest entry among the lines of `bytes` that names the session gives it, or `None`
/// when no entry there names it. The window's first line may begin partway through an entry, and
/// its last may be one Claude Code is still appending; neither parses, so neither counts.
pub(super) fn newest_name(bytes: &[u8]) -> Option<String> {
    newest_first(bytes).find_map(name_entry)
}

/// The name `line` gives the session, when it is an entry that names it, and the name is not blank.
fn name_entry(line: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(line).ok()?;
    if !NAME_KINDS.iter().any(|kind| line.contains(kind)) {
        return None;
    }
    let name = match serde_json::from_str(line).ok()? {
        NameEntry::CustomTitle { custom_title } => custom_title,
        NameEntry::AgentName { agent_name } => agent_name,
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// An entry that names the session.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum NameEntry {
    #[serde(rename = "custom-title")]
    CustomTitle {
        #[serde(rename = "customTitle")]
        custom_title: String,
    },
    #[serde(rename = "agent-name")]
    AgentName {
        #[serde(rename = "agentName")]
        agent_name: String,
    },
}

/// What `line` says, when it is an entry that tells of the turn.
fn turn_entry(line: &[u8]) -> Option<Tail> {
    // Most lines are large records of the context sent with each request; only those naming a
    // kind that tells are parsed.
    let line = std::str::from_utf8(line).ok()?;
    if !TURN_KINDS.iter().any(|kind| line.contains(kind)) {
        return None;
    }
    let entry: Entry = serde_json::from_str(line).ok()?;
    let (model, acted_in) = (entry.model(), entry.acted_in());
    Some(Tail {
        mid_turn: entry.mid_turn()?,
        written: entry.timestamp.as_deref().and_then(utc::parse),
        cwd: entry.cwd,
        model,
        acted_in,
    })
}

/// One transcript entry, reduced to what tells whether the agent's turn goes on, where it works,
/// and the model that wrote a reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    cwd: Option<String>,
    message: Option<Message>,
    /// Marks the result of a tool that ends the turn, such as the one with which a subagent of the
    /// Claude app hands its report back.
    #[serde(default)]
    tool_ends_turn: bool,
}

#[derive(Deserialize)]
struct Message {
    stop_reason: Option<String>,
    content: Option<Content>,
    /// Read loosely, as any JSON value, so that one that is not a string costs only the model, not
    /// what the entry tells of the turn.
    #[serde(default)]
    model: serde_json::Value,
}

/// A message's content: plain text, or a list of blocks.
#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    /// The tool a `tool_use` block calls, as `Bash` reads, and what the call hands it.
    name: Option<String>,
    input: Option<ToolInput>,
}

impl Entry {
    /// Whether the turn goes on after this entry, or `None` for one that does not tell.
    fn mid_turn(&self) -> Option<bool> {
        match self.kind.as_str() {
            // A request to the model is out.
            "api-request" => Some(true),
            "assistant" => Some(self.message.as_ref()?.reply_goes_on()),
            "user" if self.tool_ends_turn => Some(false),
            // A prompt or a tool's result, handed to the model next, unless the user stopped the
            // subagent.
            "user" => Some(!self.message.as_ref()?.is_interruption()),
            _ => None,
        }
    }

    /// The model that wrote this entry, when it is a reply a model wrote.
    fn model(&self) -> Option<String> {
        if self.kind != "assistant" {
            return None;
        }
        let model = self.message.as_ref()?.model.as_str()?;
        (model != MADE_UP).then(|| model.to_string())
    }

    /// Where the tool calls this entry makes act, when it is a reply that makes any, newest
    /// first: a reply's blocks come in the order the model wrote them.
    fn acted_in(&self) -> ActedIn {
        let message = self.message.as_ref().filter(|_| self.kind == "assistant");
        let blocks = message.into_iter().flat_map(Message::blocks).rev();
        blocks.fold(ActedIn::default(), |newer, block| {
            newer.after(&block.acted_in())
        })
    }
}

impl Message {
    /// Whether the turn goes on after this reply.
    fn reply_goes_on(&self) -> bool {
        if self.blocks().any(|block| block.kind == "tool_use") {
            return true;
        }
        match self.stop_reason.as_deref() {
            Some("end_turn" | "stop_sequence" | "refusal") => false,
            // Before 2.1.308, Claude Code wrote each block of a reply as it streamed, with no stop
            // reason. A reply that ends the turn closes with text and calls no tool; thinking
            // comes before more blocks.
            None => !self.blocks().any(|block| block.kind == "text"),
            // `tool_use` on a block written before the call it names, `max_tokens` and
            // `pause_turn`, after which Claude Code carries on, and any reason a later version adds.
            Some(_) => true,
        }
    }

    /// Whether this is the note Claude Code writes when the user stops the subagent.
    fn is_interruption(&self) -> bool {
        let interrupted = |text: &str| text.starts_with("[Request interrupted by user");
        match &self.content {
            Some(Content::Text(text)) => interrupted(text),
            Some(Content::Blocks(_)) | None => self
                .blocks()
                .filter_map(|block| block.text.as_deref())
                .any(interrupted),
        }
    }

    fn blocks(&self) -> std::slice::Iter<'_, Block> {
        match &self.content {
            Some(Content::Blocks(blocks)) => blocks.as_slice(),
            Some(Content::Text(_)) | None => &[],
        }
        .iter()
    }
}

impl Block {
    /// Where the tool this block calls acts, when it calls one.
    fn acted_in(&self) -> ActedIn {
        match (self.kind.as_str(), &self.name, &self.input) {
            ("tool_use", Some(tool), Some(input)) => ActedIn::by_call(tool, input),
            _ => ActedIn::default(),
        }
    }
}

/// `at` as Claude Code dates an entry.
#[cfg(test)]
pub(super) fn timestamp(at: SystemTime) -> String {
    let ms = at.duration_since(UNIX_EPOCH).unwrap().as_millis();
    let (days, ms_of_day) = (ms / 86_400_000, ms % 86_400_000);
    // Howard Hinnant's `civil_from_days`, the inverse of the `days_from_civil` `utc` counts by.
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted / 146_097, shifted % 146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = (month_from_march + 2) % 12 + 1;
    let year = era * 400 + year_of_era + u128::from(month <= 2);
    let (hour, minute) = (ms_of_day / 3_600_000, ms_of_day / 60_000 % 60);
    let (second, milli) = (ms_of_day / 1_000 % 60, ms_of_day % 1_000);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
}

/// The folder the agents of the tests' transcripts work in, unless an entry names another.
#[cfg(test)]
pub(super) const TEST_FOLDER: &str = "/code/app";

/// The model that writes the replies of the tests' transcripts, unless a test names another.
#[cfg(test)]
pub(super) const TEST_MODEL: &str = "claude-opus-5-5";

/// An entry of the kind `kind`, dated `at`, with `fields` beside, shaped as Claude Code writes one:
/// a prompt, a result or a reply names the folder its agent works in, `TEST_FOLDER` unless
/// `fields` name another, and a request to the model names none.
#[cfg(test)]
fn test_entry(kind: &str, at: SystemTime, fields: serde_json::Value) -> String {
    let mut entry = serde_json::json!({
        "parentUuid": "9a0c",
        "isSidechain": true,
        "agentId": "a1b2c3",
        "type": kind,
        "timestamp": timestamp(at),
    });
    if kind != "api-request" {
        entry["cwd"] = serde_json::json!(TEST_FOLDER);
    }
    let fields = fields.as_object().expect("fields are an object").clone();
    entry.as_object_mut().unwrap().extend(fields);
    entry.to_string()
}

/// The entry Claude Code writes as it sends a request to the model, at `at`.
#[cfg(test)]
pub(super) fn test_request(at: SystemTime) -> String {
    test_entry("api-request", at, serde_json::json!({}))
}

/// A record of the context sent with a request, larger than the end read, which names no folder.
#[cfg(test)]
pub(super) fn test_context_past_the_end_read() -> String {
    let message = serde_json::json!({"role": "user", "content": "x".repeat(TAIL_BYTES as usize)});
    serde_json::json!({
        "type": "api-request-blob", "agentId": "a1b2c3", "hash": "45d3", "message": message,
    })
    .to_string()
}

/// The prompt Claude Code hands a subagent it has set to work in `cwd`, at `at`.
#[cfg(test)]
pub(super) fn test_prompt(at: SystemTime, cwd: &str) -> String {
    let message =
        serde_json::json!({"role": "user", "content": "Find where the map reads agents."});
    test_entry(
        "user",
        at,
        serde_json::json!({"message": message, "cwd": cwd}),
    )
}

/// A reply by `TEST_MODEL` at `at` that stopped for `stop_reason` with one block of the kind
/// `block`.
#[cfg(test)]
pub(super) fn test_reply(at: SystemTime, stop_reason: Option<&str>, block: &str) -> String {
    let block = match block {
        "tool_use" => {
            serde_json::json!({"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}})
        }
        "thinking" => serde_json::json!({"type": "thinking", "thinking": "Next, the tests."}),
        _ => serde_json::json!({"type": "text", "text": "Done: the report follows."}),
    };
    let message = serde_json::json!({
        "role": "assistant", "model": TEST_MODEL, "stop_reason": stop_reason, "content": [block],
    });
    test_entry("assistant", at, serde_json::json!({"message": message}))
}

/// A reply by `TEST_MODEL` at `at` that calls the tool `tool` with `input`.
#[cfg(test)]
fn test_call(at: SystemTime, tool: &str, input: serde_json::Value) -> String {
    let block =
        serde_json::json!({"type": "tool_use", "id": "toolu_2", "name": tool, "input": input});
    let message = serde_json::json!({
        "role": "assistant", "model": TEST_MODEL, "stop_reason": "tool_use", "content": [block],
    });
    test_entry("assistant", at, serde_json::json!({"message": message}))
}

/// A tool call that tells where the subagent making it acts.
#[cfg(test)]
pub(crate) enum TestCall<'a> {
    /// A shell command it runs, with Claude Code's Bash tool.
    Command(&'a str),
    /// A file it changes, with Claude Code's Edit tool.
    Edit(&'a str),
}

#[cfg(test)]
impl TestCall<'_> {
    /// The reply by `TEST_MODEL` at `at` that makes this call.
    pub(super) fn reply(&self, at: SystemTime) -> String {
        let (tool, input) = match self {
            TestCall::Command(command) => (
                "Bash",
                serde_json::json!({"command": command, "description": "Run the tests"}),
            ),
            TestCall::Edit(file) => (
                "Edit",
                serde_json::json!({"file_path": file, "old_string": "a", "new_string": "b"}),
            ),
        };
        test_call(at, tool, input)
    }
}

/// `reply` as Claude Code records it when `model` wrote it, which may be any JSON value.
#[cfg(test)]
pub(super) fn test_written_by(reply: String, model: serde_json::Value) -> String {
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["message"]["model"] = model;
    reply.to_string()
}

/// The entries Claude Code writes after a prompt to the session `session_id` named `name`: its
/// title, then its name among agents.
#[cfg(test)]
pub(super) fn test_named(session_id: &str, name: &str) -> [String; 2] {
    [
        serde_json::json!({"type": "custom-title", "customTitle": name, "sessionId": session_id}),
        serde_json::json!({"type": "agent-name", "agentName": name, "sessionId": session_id}),
    ]
    .map(|entry| entry.to_string())
}

/// Writes a transcript holding `lines`.
#[cfg(test)]
pub(super) fn write_test_transcript(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{test_reply as reply, test_request as request, *};
    use crate::map::test_support::{TempDir, at};

    /// A tool's result handed back to the model, marked as ending the turn when `ends_turn`.
    fn result(at: SystemTime, ends_turn: bool) -> String {
        let block = json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"});
        let mut fields = json!({"message": {"role": "user", "content": [block]}});
        if ends_turn {
            fields["toolEndsTurn"] = json!(true);
        }
        test_entry("user", at, fields)
    }

    fn prompt(at: SystemTime) -> String {
        test_prompt(at, TEST_FOLDER)
    }

    /// `entry` as written in `cwd`.
    fn written_in(entry: String, cwd: &str) -> String {
        let mut entry: serde_json::Value = serde_json::from_str(&entry).unwrap();
        entry["cwd"] = json!(cwd);
        entry.to_string()
    }

    fn stopped(at: SystemTime) -> String {
        let text = json!({"type": "text", "text": "[Request interrupted by user]"});
        let message = json!({"role": "user", "content": [text]});
        test_entry("user", at, json!({"message": message}))
    }

    /// Context recorded for a request, which names a turn kind inside without being one.
    fn context(at: SystemTime) -> String {
        let attachment = json!({"type": "edited_text_file", "message": {"type": "user"}});
        test_entry("attachment", at, json!({"attachment": attachment}))
    }

    /// What a first read of a transcript holding `lines` finds at its end.
    fn tail(lines: &[String]) -> Tail {
        tail_of((lines.join("\n") + "\n").as_bytes(), 0)
    }

    #[test]
    fn the_newest_entry_that_tells_of_the_turn_says_whether_it_goes_on() {
        let t = at(1_000);
        let going_on = [
            vec![prompt(t)],
            vec![prompt(t), request(t)],
            vec![request(t), reply(t, Some("tool_use"), "thinking")],
            vec![request(t), reply(t, Some("tool_use"), "tool_use")],
            vec![reply(t, Some("tool_use"), "tool_use"), result(t, false)],
            vec![reply(t, Some("max_tokens"), "text")],
            vec![request(t), context(t)],
            // Before 2.1.308 each block streamed in on a line of its own, with no stop reason.
            vec![request(t), reply(t, None, "thinking")],
            vec![reply(t, None, "thinking"), reply(t, None, "tool_use")],
        ];
        let ended = [
            vec![request(t), reply(t, Some("end_turn"), "text")],
            vec![request(t), reply(t, Some("stop_sequence"), "text")],
            vec![request(t), reply(t, Some("refusal"), "text")],
            vec![reply(t, Some("tool_use"), "tool_use"), result(t, true)],
            vec![reply(t, Some("tool_use"), "tool_use"), stopped(t)],
            vec![reply(t, Some("end_turn"), "text"), context(t)],
            vec![reply(t, None, "thinking"), reply(t, None, "text")],
        ];
        for lines in going_on {
            assert!(tail(&lines).mid_turn, "{lines:#?}");
        }
        for lines in ended {
            assert!(!tail(&lines).mid_turn, "{lines:#?}");
        }
    }

    #[test]
    fn a_line_still_being_appended_is_no_entry_yet() {
        let t = at(1_000);
        // All but its closing brace written, so the line names its kind and does not parse.
        let unfinished = |line: String| line[..line.len() - 1].to_string();

        let ended = reply(t, Some("end_turn"), "text");
        let asking = unfinished(request(t));
        assert!(!tail_of(format!("{ended}\n{asking}").as_bytes(), 0).mid_turn);

        let working = request(t);
        let handing_back = unfinished(result(t, true));
        assert!(tail_of(format!("{working}\n{handing_back}").as_bytes(), 0).mid_turn);
        assert_eq!(tail_of(handing_back.as_bytes(), 0), Tail::default());
    }

    #[test]
    fn the_end_of_a_long_transcript_is_read_and_only_its_end() {
        let home = TempDir::new("transcript-tail");
        let path = home.path("agent-long.jsonl");
        let t = at(1_000);
        let padding = context(t);
        let lines_to_fill_window = TAIL_BYTES as usize / padding.len() + 1;
        let mut lines = vec![request(t)];
        lines.extend(std::iter::repeat_n(padding, lines_to_fill_window));
        write_test_transcript(&path, &lines);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(
            read_tail(&path, len, 0).unwrap(),
            Tail::default(),
            "the request lies before the window"
        );

        lines.push(reply(t, Some("end_turn"), "text"));
        write_test_transcript(&path, &lines);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(
            read_tail(&path, len, 0).unwrap(),
            Tail {
                mid_turn: false,
                written: Some(t),
                cwd: None,
                model: None,
                acted_in: ActedIn::default(),
            }
        );
    }

    #[test]
    fn mid_turn_a_subagent_runs_in_the_folder_its_newest_entry_naming_one_records() {
        let t = at(1_000);
        let worktree = "/code/app/.claude/worktrees/agent-a1";
        let asking = [
            prompt(t),
            written_in(result(t, false), worktree),
            request(t),
        ];
        assert_eq!(
            tail(&asking).cwd.as_deref(),
            Some(worktree),
            "a request names no folder, so the result before it tells"
        );
        let moved = [
            written_in(prompt(t), worktree),
            written_in(reply(t, Some("tool_use"), "tool_use"), "/code/app/web"),
        ];
        assert_eq!(tail(&moved).cwd.as_deref(), Some("/code/app/web"));
        assert_eq!(tail(&[request(t)]).cwd, None, "no entry read names one");
    }

    #[test]
    fn mid_turn_the_folder_an_earlier_end_named_holds_until_a_later_end_names_another() {
        let t = at(1_000);
        let worktree = "/code/app/.claude/worktrees/agent-a1";
        let earlier = tail(&[written_in(prompt(t), worktree), request(t)]);
        assert_eq!(earlier.cwd.as_deref(), Some(worktree));

        let behind_a_large_result = tail(&[request(t)]);
        let held = behind_a_large_result.after(&earlier);
        assert_eq!(held.cwd.as_deref(), Some(worktree));

        let moved = tail(&[written_in(result(t, false), "/code/app/web"), request(t)]);
        assert_eq!(moved.after(&earlier).cwd.as_deref(), Some("/code/app/web"));

        let ended = tail(&[reply(t, Some("end_turn"), "text")]);
        assert_eq!(ended.after(&earlier).cwd, None, "only mid-turn");
    }

    #[test]
    fn the_folder_model_and_acts_of_a_subagent_whose_turn_has_ended_are_not_looked_for() {
        let t = at(1_000);
        let entered = TestCall::Command("cd /code/site && npm test").reply(t);
        let mut lines = vec![prompt(t), entered, result(t, false)];
        assert_ne!(
            tail(&lines).acted_in,
            ActedIn::default(),
            "mid-turn they are"
        );

        lines.push(reply(t, Some("end_turn"), "text"));
        let ended = tail(&lines);
        assert!(!ended.mid_turn);
        assert_eq!((&ended.cwd, &ended.model), (&None, &None));
        assert_eq!(ended.acted_in, ActedIn::default());
    }

    /// The model `TEST_MODEL` names, as a tail holds it.
    fn test_model() -> Option<String> {
        Some(TEST_MODEL.to_string())
    }

    /// `reply` with no model recorded on it.
    fn unattributed(reply: String) -> String {
        let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        reply["message"].as_object_mut().unwrap().remove("model");
        reply.to_string()
    }

    #[test]
    fn mid_turn_a_subagent_works_with_the_model_its_newest_reply_a_model_wrote_names() {
        let t = at(1_000);
        let calling = || reply(t, Some("tool_use"), "tool_use");
        let asking = [calling(), result(t, false), request(t)];
        assert_eq!(
            tail(&asking).model,
            test_model(),
            "a request and a result name no model, so the reply before them tells"
        );
        let switched = [
            test_written_by(calling(), json!("claude-sonnet-5-5")),
            result(t, false),
            calling(),
        ];
        assert_eq!(tail(&switched).model, test_model(), "the newest reply");
        // An API error ends the turn with a reply Claude Code made up, and a new prompt resumes it.
        let failed = test_written_by(reply(t, Some("stop_sequence"), "text"), json!(MADE_UP));
        let resumed = [calling(), result(t, false), failed, prompt(t), request(t)];
        assert_eq!(
            tail(&resumed).model,
            test_model(),
            "no model wrote the error"
        );
        assert_eq!(tail(&[prompt(t), request(t)]).model, None, "no reply yet");
    }

    #[test]
    fn a_reply_whose_model_is_missing_or_not_a_string_names_none_and_still_tells_of_the_turn() {
        let t = at(1_000);
        let calling = || reply(t, Some("tool_use"), "tool_use");
        for line in [
            unattributed(calling()),
            test_written_by(calling(), json!(null)),
            test_written_by(calling(), json!(7)),
            test_written_by(calling(), json!({"id": TEST_MODEL})),
        ] {
            let tail = tail(std::slice::from_ref(&line));
            assert!(tail.mid_turn, "{line}");
            assert_eq!(tail.model, None, "{line}");
            assert_eq!(tail.cwd.as_deref(), Some(TEST_FOLDER), "{line}");
        }
    }

    #[test]
    fn mid_turn_the_model_an_earlier_end_named_holds_until_a_later_reply_names_another() {
        let t = at(1_000);
        let earlier = tail(&[reply(t, Some("tool_use"), "tool_use"), request(t)]);
        assert_eq!(earlier.model, test_model());

        let behind_a_large_result = tail(&[request(t)]);
        assert_eq!(behind_a_large_result.after(&earlier).model, test_model());

        let calling = reply(t, Some("tool_use"), "tool_use");
        let by_fable = test_written_by(calling, json!("claude-fable-5-1"));
        let switched = tail(&[by_fable, request(t)]).after(&earlier).model;
        assert_eq!(switched.as_deref(), Some("claude-fable-5-1"));

        let ended = tail(&[reply(t, Some("end_turn"), "text")]);
        assert_eq!(ended.after(&earlier).model, None, "only mid-turn");
    }

    /// The folders `tail` says its subagent acted in, the most telling first.
    fn acted_in(tail: &Tail) -> Vec<&str> {
        tail.acted_in.folders().collect()
    }

    #[test]
    fn mid_turn_a_subagent_acted_where_its_newest_cds_and_edits_in_the_end_read_say() {
        let t = at(1_000);
        let lines = [
            prompt(t),
            TestCall::Command("cd /code/app-wt/fix && cargo build").reply(t),
            result(t, false),
            TestCall::Edit("/code/app-wt/fix/src/main.rs").reply(t),
            result(t, false),
            TestCall::Command("cd /tmp/scratch && ls").reply(t),
            result(t, false),
            TestCall::Edit("/code/notes/fix.md").reply(t),
            result(t, false),
            TestCall::Command("cd $WT/fix-2 && git log").reply(t),
            result(t, false),
            test_call(t, "Read", json!({"file_path": "/code/site/index.html"})),
            result(t, false),
            request(t),
        ];
        let working = tail(&lines);
        assert_eq!(
            acted_in(&working),
            [
                "/tmp/scratch",
                "/code/app-wt/fix",
                "/code/notes",
                "/code/app-wt/fix/src"
            ],
            "back past a cd it cannot read and a read, newest first"
        );
        let unacted = tail(&[prompt(t), request(t)]);
        assert_eq!(acted_in(&unacted), [""; 0], "no call yet");
    }

    #[test]
    fn mid_turn_the_acts_an_earlier_end_named_hold_behind_those_a_later_end_names() {
        let t = at(1_000);
        let earlier = tail(&[
            TestCall::Command("cd /code/app-wt/fix && cargo build").reply(t),
            result(t, false),
            TestCall::Edit("/code/notes/fix.md").reply(t),
            request(t),
        ]);

        let behind_a_large_result = tail(&[request(t)]).after(&earlier);
        let held = acted_in(&behind_a_large_result);
        assert_eq!(held, ["/code/app-wt/fix", "/code/notes"]);

        let edited = TestCall::Edit("/code/app-wt/fix/src/main.rs").reply(t);
        let edited_since = tail(&[edited, request(t)]).after(&earlier);
        let newer = acted_in(&edited_since);
        assert_eq!(
            newer,
            ["/code/app-wt/fix", "/code/app-wt/fix/src", "/code/notes"]
        );

        let ended = tail(&[reply(t, Some("end_turn"), "text")]).after(&earlier);
        assert_eq!(acted_in(&ended), [""; 0], "only mid-turn");
    }

    #[test]
    fn of_a_transcript_read_before_only_the_calls_appended_since_are_looked_for() {
        let t = at(1_000);
        let entered = TestCall::Command("cd /code/app-wt/fix && cargo build").reply(t);
        let seen = format!("{entered}\n{}\n", result(t, false));
        let appended = [TestCall::Edit("/code/notes/fix.md").reply(t), request(t)];
        let bytes = seen.clone() + &appended.join("\n") + "\n";
        let read_on = |read_before: usize| tail_of(bytes.as_bytes(), read_before);

        let both = ["/code/app-wt/fix", "/code/notes"];
        assert_eq!(acted_in(&read_on(0)), both, "a first read");
        assert_eq!(
            acted_in(&read_on(seen.len())),
            ["/code/notes"],
            "the cd lay in what the earlier reading saw, which tells of it"
        );
        assert_eq!(
            acted_in(&read_on(entered.len() - 10)),
            both,
            "the earlier reading saw the cd's line only in part"
        );
        let nothing_appended = read_on(bytes.len());
        assert_eq!(acted_in(&nothing_appended), [""; 0]);
        assert!(nothing_appended.mid_turn);
        assert_eq!(
            (nothing_appended.cwd.as_deref(), nothing_appended.model),
            (Some(TEST_FOLDER), test_model()),
            "the folder and the model are looked for through the whole end"
        );
    }

    #[test]
    fn a_tool_call_whose_input_is_shaped_oddly_tells_nothing_of_where_but_still_of_the_turn() {
        let t = at(1_000);
        let fields = json!({"command": ["cd", "/code/site"], "file_path": {"path": "/code/site"}});
        for call in [
            test_call(t, "mcp__docker__run", fields),
            test_call(t, "Bash", json!({"command": 7})),
        ] {
            let tail = tail(std::slice::from_ref(&call));
            assert!(tail.mid_turn, "{call}");
            assert_eq!(tail.model, test_model(), "{call}");
            assert_eq!(tail.cwd.as_deref(), Some(TEST_FOLDER), "{call}");
            assert_eq!(tail.acted_in, ActedIn::default(), "{call}");
        }
    }

    #[test]
    fn a_sessions_model_is_the_one_its_newest_reply_a_model_wrote_names_whatever_its_turn() {
        let t = at(1_000);
        let model = |lines: &[String]| newest_model((lines.join("\n") + "\n").as_bytes());
        let at_rest = [prompt(t), request(t), reply(t, Some("end_turn"), "text")];
        assert_eq!(model(&at_rest), test_model());
        let asking = [reply(t, Some("end_turn"), "text"), prompt(t), request(t)];
        assert_eq!(model(&asking), test_model(), "the reply before the prompt");
        let earlier = test_written_by(reply(t, Some("end_turn"), "text"), json!("claude-fable-5"));
        assert_eq!(model(&[earlier, reply(t, None, "text")]), test_model());
        let failed = test_written_by(reply(t, Some("stop_sequence"), "text"), json!(MADE_UP));
        assert_eq!(
            model(&[reply(t, None, "text"), failed.clone()]),
            test_model(),
            "the reply before the error"
        );
        assert_eq!(model(&[failed]), None, "no model wrote the error");
        let unnamed = test_written_by(reply(t, None, "text"), json!(7));
        assert_eq!(model(&[unnamed, prompt(t)]), None);
        assert_eq!(model(&[prompt(t), request(t)]), None, "no reply yet");
    }

    #[test]
    fn a_sessions_name_is_the_one_its_newest_naming_entry_gives_of_either_kind() {
        let t = at(1_000);
        let name = |lines: &[String]| newest_name((lines.join("\n") + "\n").as_bytes());
        let [first_title, _] = test_named("e26ca359", "Foundry");
        let [title, agent_name] = test_named("e26ca359", "Foundry Agent Studio architecture");
        let after_a_turn = [
            first_title.clone(),
            title,
            prompt(t),
            reply(t, None, "text"),
        ];
        assert_eq!(
            name(&after_a_turn).as_deref(),
            Some("Foundry Agent Studio architecture"),
            "renamed since its first prompt"
        );
        assert_eq!(
            name(&[first_title, agent_name]).as_deref(),
            Some("Foundry Agent Studio architecture"),
            "the name among agents, written last"
        );
        let [blank, _] = test_named("e26ca359", "  ");
        let [kept, _] = test_named("e26ca359", "Foundry");
        assert_eq!(name(&[kept, blank]).as_deref(), Some("Foundry"));
        assert_eq!(name(&[prompt(t), request(t)]), None, "never named");
    }

    #[test]
    fn a_naming_entry_cut_by_the_ends_of_the_read_names_nothing() {
        let [title, _] = test_named("e26ca359", "Foundry");
        let (start, end) = title.split_at(title.len() / 2);
        assert_eq!(newest_name(end.as_bytes()), None, "begun before the read");
        assert_eq!(newest_name(start.as_bytes()), None, "still being written");
    }

    #[test]
    fn a_tail_dates_its_entry_by_the_entrys_own_timestamp() {
        let written = UNIX_EPOCH + Duration::from_millis(1_793_905_215_325);
        assert_eq!(timestamp(written), "2026-11-05T19:00:15.325Z");
        assert_eq!(tail(&[request(written)]).written, Some(written));
    }
}
