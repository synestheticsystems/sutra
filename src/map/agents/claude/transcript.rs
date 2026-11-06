//! Claude Code keeps a transcript of each session and of each subagent, which holds one JSON entry
//! per line, appended as the agent works: the prompt and tool results handed to the model, each
//! request sent to it, its replies, and the context recorded around them. Only its end is read: of
//! a subagent's, what its turn is doing (see `Tail`), and of a session's, the model of its newest
//! reply (see `newest_model`). The format is undocumented; what is read here was checked against
//! 2,200 subagent transcripts written by Claude Code 2.1.18 to 2.1.317, and the model on each reply
//! against the newest 400 subagent transcripts and all 325 session transcripts (2026-11-05).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

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

/// What the end of a subagent's transcript says about it: the newest entry that tells of its turn,
/// and while the turn goes on, the folder the subagent works in and the model it works with.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Tail {
    pub(super) mid_turn: bool,
    /// When that entry was written, by its own date. A transcript's modification time also moves
    /// when nothing is added to it: on Daniel's Mac, dozens of old transcripts at a time, several
    /// times a week (2026-11-05).
    pub(super) written: Option<SystemTime>,
    /// The working folder the newest entry naming one records, as Claude Code wrote it. Each of
    /// the subagent's replies and results names its own folder, which is its worktree when it has
    /// one and follows it wherever it changes directory; a request to the model names none. Only
    /// a subagent mid-turn is placed by its folder, so only then is it looked for. `None` when no
    /// entry in the end read names one: in 1,594 of 73,826 moments mid-turn in the transcripts on
    /// Daniel's Mac (2026-11-05), the newest that did began further back, behind a large result.
    pub(super) cwd: Option<String>,
    /// The model the newest reply in the end read names, by the id Claude Code records, as
    /// `claude-opus-5-5` reads: the model that ran, where the subagent's meta file holds only the
    /// alias it was asked for. Like the folder, it is looked for only mid-turn. `None` before the
    /// subagent first replies, and when no reply in the end read names one.
    pub(super) model: Option<String>,
}

impl Tail {
    /// This end of a transcript, read after `earlier`, its end before it grew. Claude Code only
    /// appends, so mid-turn the folder and the model `earlier` named still hold while this end
    /// names none.
    pub(super) fn after(mut self, earlier: &Tail) -> Tail {
        if self.mid_turn {
            if self.cwd.is_none() {
                self.cwd.clone_from(&earlier.cwd);
            }
            if self.model.is_none() {
                self.model.clone_from(&earlier.model);
            }
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

/// What the end of the transcript at `path`, `len` bytes long when listed, says.
pub(super) fn read_tail(path: &Path, len: u64) -> io::Result<Tail> {
    let bytes = read_span(path, len.saturating_sub(TAIL_BYTES), len)?;
    Ok(tail_of(&bytes))
}

/// What the newest entry in `bytes` that tells of the turn says, with, mid-turn, the folder the
/// newest entry naming one records and the model the newest reply names. The window's first line
/// may begin partway through an entry, and its last may be one Claude Code is still appending;
/// neither parses, so neither counts.
fn tail_of(bytes: &[u8]) -> Tail {
    let lines = bytes.split(|&byte| byte == b'\n');
    let mut entries = lines.rev().filter_map(turn_entry);
    let Some(mut tail) = entries.next() else {
        return Tail::default();
    };
    if !tail.mid_turn {
        return Tail {
            cwd: None,
            model: None,
            ..tail
        };
    }
    while tail.cwd.is_none() || tail.model.is_none() {
        let Some(older) = entries.next() else {
            break;
        };
        tail.cwd = tail.cwd.or(older.cwd);
        tail.model = tail.model.or(older.model);
    }
    tail
}

/// The model the newest reply among the lines of `bytes` names (see `Tail::model`), or `None` when
/// no reply there names one.
pub(super) fn newest_model(bytes: &[u8]) -> Option<String> {
    let lines = bytes.split(|&byte| byte == b'\n');
    let mut entries = lines.rev().filter_map(turn_entry);
    entries.find_map(|entry| entry.model)
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
    let model = entry.model();
    Some(Tail {
        mid_turn: entry.mid_turn()?,
        written: entry.timestamp.as_deref().and_then(parse_timestamp),
        cwd: entry.cwd,
        model,
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

    fn blocks(&self) -> impl Iterator<Item = &Block> {
        match &self.content {
            Some(Content::Blocks(blocks)) => blocks.as_slice(),
            Some(Content::Text(_)) | None => &[],
        }
        .iter()
    }
}

/// The moment `text` names in the form JavaScript's `toISOString` writes, which is how Claude Code
/// dates each entry: `2026-11-05T19:00:15.325Z`, in UTC. `None` for any other form.
fn parse_timestamp(text: &str) -> Option<SystemTime> {
    let bytes = text.as_bytes();
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];
    if bytes.len() != 24 || separators.iter().any(|&(at, byte)| bytes[at] != byte) {
        return None;
    }
    let number = |from: usize, to: usize| {
        let digits = &bytes[from..to];
        digits.iter().all(u8::is_ascii_digit).then(|| {
            digits
                .iter()
                .fold(0, |number, digit| number * 10 + u64::from(digit - b'0'))
        })
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    if !valid {
        return None;
    }
    let days = days_since_epoch(year, month, day)?;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds) + Duration::from_millis(number(20, 23)?))
}

/// The days from 1970-01-01 to a date of the Gregorian calendar, or `None` for a date before it.
/// This is Howard Hinnant's `days_from_civil`, which counts each year from March so that a leap
/// day ends it.
fn days_since_epoch(year: u64, month: u64, day: u64) -> Option<u64> {
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let (era, year_of_era) = (year / 400, year % 400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era).checked_sub(719_468)
}

/// `at` as Claude Code dates an entry.
#[cfg(test)]
pub(super) fn timestamp(at: SystemTime) -> String {
    let ms = at.duration_since(UNIX_EPOCH).unwrap().as_millis();
    let (days, ms_of_day) = (ms / 86_400_000, ms % 86_400_000);
    // Howard Hinnant's `civil_from_days`, the inverse of `days_since_epoch`.
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

/// `reply` as Claude Code records it when `model` wrote it, which may be any JSON value.
#[cfg(test)]
pub(super) fn test_written_by(reply: String, model: serde_json::Value) -> String {
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["message"]["model"] = model;
    reply.to_string()
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

    fn tail(lines: &[String]) -> Tail {
        tail_of((lines.join("\n") + "\n").as_bytes())
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
        assert!(!tail_of(format!("{ended}\n{asking}").as_bytes()).mid_turn);

        let working = request(t);
        let handing_back = unfinished(result(t, true));
        assert!(tail_of(format!("{working}\n{handing_back}").as_bytes()).mid_turn);
        assert_eq!(tail_of(handing_back.as_bytes()), Tail::default());
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
            read_tail(&path, len).unwrap(),
            Tail::default(),
            "the request lies before the window"
        );

        lines.push(reply(t, Some("end_turn"), "text"));
        write_test_transcript(&path, &lines);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(
            read_tail(&path, len).unwrap(),
            Tail {
                mid_turn: false,
                written: Some(t),
                cwd: None,
                model: None,
            }
        );
    }

    #[test]
    fn mid_turn_a_subagent_works_in_the_folder_its_newest_entry_naming_one_records() {
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
    fn the_folder_and_model_of_a_subagent_whose_turn_has_ended_are_not_looked_for() {
        let t = at(1_000);
        let ended = tail(&[prompt(t), reply(t, Some("end_turn"), "text")]);
        assert!(!ended.mid_turn);
        assert_eq!((ended.cwd, ended.model), (None, None));
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
    fn a_tail_dates_its_entry_by_the_entrys_own_timestamp() {
        let written = UNIX_EPOCH + Duration::from_millis(1_793_905_215_325);
        assert_eq!(timestamp(written), "2026-11-05T19:00:15.325Z");
        assert_eq!(tail(&[request(written)]).written, Some(written));
    }

    #[test]
    fn timestamps_in_the_form_javascript_writes_parse_to_their_moment() {
        let ms = |ms: u64| Some(UNIX_EPOCH + Duration::from_millis(ms));
        let cases = [
            ("2026-11-05T19:00:15.325Z", ms(1_793_905_215_325)),
            ("2024-02-29T23:59:59.999Z", ms(1_709_251_199_999)),
            ("2000-03-01T00:00:00.000Z", ms(951_868_800_000)),
            ("1970-01-01T00:00:00.000Z", ms(0)),
            ("2026-11-05T19:00:15Z", None),
            ("2026-11-05T19:00:15.325+01:00", None),
            ("2026-13-05T19:00:15.325Z", None),
            ("2026-11-05T24:00:15.325Z", None),
            ("2026-11-05T19:00:+5.325Z", None),
            ("1969-12-31T23:59:59.999Z", None),
            ("yesterday", None),
        ];
        for (text, moment) in cases {
            assert_eq!(parse_timestamp(text), moment, "{text}");
        }
    }
}
