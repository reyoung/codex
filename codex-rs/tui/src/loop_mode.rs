use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_protocol::user_input::TextElement;
use uuid::Uuid;

const LOOP_MARKER_DIR: &str = ".codex/loop";
pub(crate) const DEFAULT_LOOP_INTERVAL: Duration = Duration::from_secs(10 * 60);
pub(crate) const DEFAULT_LOOP_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoopCommandArgs {
    pub(crate) interval: Duration,
    pub(crate) timeout: Duration,
    task_offset: usize,
}

impl LoopCommandArgs {
    pub(crate) fn task_offset(&self) -> usize {
        self.task_offset
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoopMessageState {
    marker_path: PathBuf,
    interval: Duration,
    timeout: Duration,
}

impl LoopMessageState {
    pub(crate) fn create(interval: Duration, timeout: Duration) -> Result<Self, String> {
        let marker_dir = std::env::temp_dir().join(LOOP_MARKER_DIR);
        std::fs::create_dir_all(&marker_dir).map_err(|err| {
            format!(
                "Failed to create /loop marker directory `{}`: {err}",
                marker_dir.display()
            )
        })?;

        Ok(Self {
            marker_path: marker_dir.join(format!("{}.done", Uuid::new_v4())),
            interval,
            timeout,
        })
    }

    pub(crate) fn completion_submitted_text(&self, user_text: &str) -> String {
        let marker_path = self.marker_path.to_string_lossy();
        let touch_command = shlex::try_join(["touch", "--", marker_path.as_ref()])
            .unwrap_or_else(|_| format!("touch -- {marker_path}"));

        let interval = format_duration(self.interval);
        let timeout = format_duration(self.timeout);
        format!(
            "{user_text}\n\nThis task is running in `/loop` mode. Codex CLI will automatically resend the same user task every {interval} until you create the internal completion marker or the loop times out after {timeout}. When and only when you have fully completed the task above, run `{touch_command}` to create the completion marker file at `{}`. This marker file is internal to Codex CLI, so do not ask the user to manage it. If the task is not fully solved yet, do not create that file.",
            self.marker_path.display()
        )
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.marker_path.exists()
    }

    pub(crate) fn marker_path(&self) -> &Path {
        &self.marker_path
    }

    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }
}

pub(crate) fn loop_usage() -> String {
    let interval = format_duration(DEFAULT_LOOP_INTERVAL);
    let timeout = format_duration(DEFAULT_LOOP_TIMEOUT);
    format!(
        "Usage: /loop [<interval>] [--timeout <timeout>] <task> (defaults: interval {interval}, timeout {timeout})"
    )
}

pub(crate) fn loop_started_message(interval: Duration, timeout: Duration) -> String {
    let interval = format_duration(interval);
    let timeout = format_duration(timeout);
    format!(
        "Loop armed: Codex will retry this task every {interval} until it creates the internal completion marker or the loop times out after {timeout}."
    )
}

pub(crate) fn loop_timeout_message(timeout: Duration) -> String {
    let timeout = format_duration(timeout);
    format!("Loop timed out after {timeout} without seeing the internal completion marker.")
}

pub(crate) fn parse_loop_command_args(args: &str) -> Result<LoopCommandArgs, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err(loop_usage());
    }

    let mut interval = DEFAULT_LOOP_INTERVAL;
    let mut remaining = trimmed;

    if let Some((token, rest)) = split_first_token(remaining)
        && let Some(parsed) = try_parse_duration_token(token)?
    {
        interval = parsed;
        remaining = rest;
    }

    let mut timeout = DEFAULT_LOOP_TIMEOUT;
    if let Some((token, rest)) = split_first_token(remaining) {
        if token == "--timeout" {
            let Some((value, rest_after_timeout)) = split_first_token(rest) else {
                return Err(loop_usage());
            };
            timeout = parse_duration_token(value)
                .ok_or_else(|| format!("Invalid /loop timeout `{value}`. {}", loop_usage()))?;
            remaining = rest_after_timeout;
        } else if let Some(value) = token.strip_prefix("--timeout=") {
            timeout = parse_duration_token(value)
                .ok_or_else(|| format!("Invalid /loop timeout `{value}`. {}", loop_usage()))?;
            remaining = rest;
        }
    }

    let task = remaining.trim_start();
    if task.is_empty() {
        return Err(loop_usage());
    }

    Ok(LoopCommandArgs {
        interval,
        timeout,
        task_offset: trimmed.len() - task.len(),
    })
}

pub(crate) fn split_task_text(
    args: String,
    text_elements: Vec<TextElement>,
    task_offset: usize,
) -> (String, Vec<TextElement>) {
    let task_text = args[task_offset..].to_string();
    let task_end = args.len();
    let text_elements = text_elements
        .into_iter()
        .filter_map(|element| {
            let byte_range = element.byte_range;
            if byte_range.start < task_offset || byte_range.end > task_end {
                return None;
            }

            Some(element.map_range(|byte_range| {
                (byte_range.start - task_offset..byte_range.end - task_offset).into()
            }))
        })
        .collect();
    (task_text, text_elements)
}

fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let split_at = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    Some((&trimmed[..split_at], &trimmed[split_at..]))
}

fn try_parse_duration_token(token: &str) -> Result<Option<Duration>, String> {
    if token.starts_with("--") {
        return Ok(None);
    }

    match parse_duration_token(token) {
        Some(duration) => Ok(Some(duration)),
        None if looks_like_invalid_duration_token(token) => Err(format!(
            "Invalid /loop interval `{token}`. {}",
            loop_usage()
        )),
        None => Ok(None),
    }
}

fn looks_like_invalid_duration_token(token: &str) -> bool {
    if token.len() < 2 {
        return false;
    }

    let (value, unit) = token.split_at(token.len() - 1);
    value.chars().all(|ch| ch.is_ascii_digit()) && unit.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn parse_duration_token(token: &str) -> Option<Duration> {
    let token = token.trim();
    if token.len() < 2 {
        return None;
    }

    let (value, unit) = token.split_at(token.len() - 1);
    let value: u64 = value.parse().ok()?;
    if value == 0 {
        return None;
    }

    let seconds = match unit {
        "s" => value,
        "m" => value.checked_mul(60)?,
        "h" => value.checked_mul(60 * 60)?,
        "d" => value.checked_mul(60 * 60 * 24)?,
        _ => return None,
    };
    Some(Duration::from_secs(seconds))
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds.is_multiple_of(60 * 60 * 24) {
        return format!("{}d", seconds / (60 * 60 * 24));
    }
    if seconds.is_multiple_of(60 * 60) {
        return format!("{}h", seconds / (60 * 60));
    }
    if seconds.is_multiple_of(60) {
        return format!("{}m", seconds / 60);
    }
    format!("{seconds}s")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::user_input::ByteRange;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_loop_command_args_uses_defaults_for_plain_task() {
        let args = "finish the migration";
        let parsed = parse_loop_command_args(args).expect("parsed");

        assert_eq!(parsed.interval, DEFAULT_LOOP_INTERVAL);
        assert_eq!(parsed.timeout, DEFAULT_LOOP_TIMEOUT);
        assert_eq!(&args[parsed.task_offset()..], "finish the migration");
    }

    #[test]
    fn parse_loop_command_args_supports_interval_and_timeout() {
        let args = "15m --timeout 2h finish the migration";
        let parsed = parse_loop_command_args(args).expect("parsed");

        assert_eq!(parsed.interval, Duration::from_secs(15 * 60));
        assert_eq!(parsed.timeout, Duration::from_secs(2 * 60 * 60));
        assert_eq!(&args[parsed.task_offset()..], "finish the migration");
    }

    #[test]
    fn split_task_text_rebases_text_elements() {
        let text = "15m --timeout 2h hello @world".to_string();
        let text_elements = vec![TextElement::new(ByteRange { start: 21, end: 27 }, None)];

        let (task_text, task_elements) = split_task_text(text, text_elements, 17);

        assert_eq!(task_text, "hello @world");
        assert_eq!(
            task_elements,
            vec![TextElement::new(ByteRange { start: 4, end: 10 }, None,)]
        );
    }
}
