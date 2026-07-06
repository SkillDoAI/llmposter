//! Cassette writer: Serialize-only fixture mirrors and the append-only
//! YAML cassette file format.

use serde::Serialize;
use std::path::Path;

/// Priority stamped on every recorded fixture — below the default (0) so
/// hand-written fixtures always win over recordings.
// consumed by Task 4 (record path)
#[allow(dead_code)]
pub(crate) const RECORDED_PRIORITY: i32 = -1;
const CASSETTE_HEADER: &str = "# Recorded by llmposter (VCR mode). Safe to hand-edit; entries are\n# ordinary fixtures. priority: -1 keeps hand-written fixtures winning.\nfixtures: []\n";

/// A fixture as the recorder writes it — a Serialize-only mirror of the
/// minimal fixture schema. The Deserialize side stays on `crate::fixture`.
#[derive(Debug, Serialize)]
pub(crate) struct RecordedFixture {
    #[serde(rename = "match")]
    pub match_rule: RecordedMatch,
    pub provider: &'static str,
    pub priority: i32,
    pub response: RecordedResponse,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecordedMatch {
    pub user_message: String,
    pub model: String,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct RecordedResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<RecordedToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecordedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl RecordedFixture {
    /// One YAML list entry (starts with `- match:`). Round-trips through the
    /// normal loader so the in-memory fixture is byte-identical to a reload.
    fn to_yaml_entry(&self) -> Result<String, String> {
        let entry = serde_yaml_ng::to_string(&[self])
            .map_err(|e| format!("cassette serialization failed: {}", e))?;
        Ok(entry.strip_prefix("---\n").unwrap_or(&entry).to_string())
    }

    /// Parse the serialized entry back through the real fixture loader —
    /// guarantees in-memory replay matches what a cassette reload produces.
    pub(crate) fn into_fixture(self) -> Result<crate::fixture::Fixture, String> {
        let entry = self.to_yaml_entry()?;
        let mut parsed: Vec<crate::fixture::Fixture> = serde_yaml_ng::from_str(&entry)
            .map_err(|e| format!("recorded fixture failed to round-trip: {}", e))?;
        let mut fixture = parsed
            .pop()
            .ok_or("recorded fixture round-trip was empty")?;
        fixture.validate()?;
        Ok(fixture)
    }
}

/// Create the cassette file in the pristine state if it doesn't exist.
/// `fixtures: []` is valid for every loader path (dir scans, hot reload).
///
/// On Unix the file is created with mode `0o600` — the cassette can hold
/// sensitive response content, so it stays owner-only.
pub(crate) fn ensure_cassette(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "cannot create cassette directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(CASSETTE_HEADER.as_bytes())
                .map_err(|e| format!("cannot create cassette {}: {}", path.display(), e))
        }
        // Lost the create race to another process — the file now exists,
        // which is all this function guarantees.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("cannot create cassette {}: {}", path.display(), e)),
    }
}

/// Append one entry. Handles the pristine `fixtures: []` → `fixtures:` +
/// block-list transition on first append. Caller serializes access.
///
/// Pristine detection requires the last LINE at column 0 to be exactly
/// `fixtures: []` — a suffix check alone would also match that text at
/// the end of an indented block scalar inside a recorded entry and
/// silently corrupt it.
pub(crate) fn append_to_cassette(path: &Path, rec: &RecordedFixture) -> Result<(), String> {
    let entry = rec.to_yaml_entry()?;
    let current = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read cassette {}: {}", path.display(), e))?;
    let trimmed = current.trim_end();
    let new_content = if trimmed.is_empty() {
        // User-touched empty file: adopt it as a fresh cassette.
        format!("fixtures:\n{}", entry)
    } else if trimmed.lines().last() == Some("fixtures: []") {
        let prefix = &trimmed[..trimmed.len() - "fixtures: []".len()];
        format!("{}fixtures:\n{}", prefix, entry)
    } else {
        // Existing cassette with entries: append to the UNTRIMMED original —
        // trailing blank lines can belong to a `|+` (keep-chomping) block
        // scalar in the previous entry, and trimming them would silently
        // shorten that entry's content. Only guard the newline seam.
        let sep = if current.ends_with('\n') { "" } else { "\n" };
        format!("{}{}{}", current, sep, entry)
    };
    std::fs::write(path, new_content)
        .map_err(|e| format!("cannot write cassette {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cassette(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("llmposter_record_tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{}_{}.yaml", name, std::process::id()))
    }

    fn sample(msg: &str) -> RecordedFixture {
        RecordedFixture {
            match_rule: RecordedMatch {
                user_message: msg.to_string(),
                model: "gpt-test".to_string(),
            },
            provider: "openai",
            priority: RECORDED_PRIORITY,
            response: RecordedResponse {
                content: Some("hi there".to_string()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn should_create_cassette_with_pristine_empty_list() {
        let path = temp_cassette("pristine");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert!(fixtures.is_empty());
    }

    #[test]
    fn should_append_entries_that_reload_as_fixtures() {
        let path = temp_cassette("append");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        append_to_cassette(&path, &sample("q one")).unwrap();
        append_to_cassette(&path, &sample("q: two, with \"yaml\" #chars")).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert_eq!(fixtures.len(), 2);
        assert_eq!(fixtures[0].priority, Some(-1));
        assert_eq!(
            fixtures[0].match_rule.as_ref().unwrap().user_message,
            Some(crate::fixture::StringMatch::Substring("q one".to_string()))
        );
        assert_eq!(
            fixtures[1].match_rule.as_ref().unwrap().user_message,
            Some(crate::fixture::StringMatch::Substring(
                "q: two, with \"yaml\" #chars".to_string()
            ))
        );
        assert_eq!(
            fixtures[1].response.as_ref().unwrap().content.as_deref(),
            Some("hi there")
        );
    }

    #[test]
    fn should_not_corrupt_entry_whose_content_ends_with_pristine_marker() {
        let path = temp_cassette("marker_collision");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        let mut tricky = sample("marker one");
        tricky.response.content = Some("see below:\nfixtures: []".to_string());
        append_to_cassette(&path, &tricky).unwrap();
        append_to_cassette(&path, &sample("marker two")).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert_eq!(fixtures.len(), 2);
        assert_eq!(
            fixtures[0].response.as_ref().unwrap().content.as_deref(),
            Some("see below:\nfixtures: []")
        );
    }

    #[test]
    fn should_preserve_trailing_newlines_of_previous_entry() {
        let path = temp_cassette("keep_chomping");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        let mut rec = sample("keep one");
        rec.response.content = Some("ends with two newlines\n\n".to_string());
        append_to_cassette(&path, &rec).unwrap();
        append_to_cassette(&path, &sample("keep two")).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert_eq!(fixtures.len(), 2);
        assert_eq!(
            fixtures[0].response.as_ref().unwrap().content.as_deref(),
            Some("ends with two newlines\n\n")
        );
    }

    #[test]
    fn should_append_multiline_content_that_round_trips() {
        let path = temp_cassette("multiline");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        let content =
            "Here you go:\n```yaml\nkey: value\n- list item\n```\n# looks like a comment\n";
        let mut rec = sample("multi");
        rec.response.content = Some(content.to_string());
        append_to_cassette(&path, &rec).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(
            fixtures[0].response.as_ref().unwrap().content.as_deref(),
            Some(content)
        );
    }

    #[test]
    fn should_adopt_user_created_empty_cassette() {
        let path = temp_cassette("adopt_empty");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "").unwrap();
        append_to_cassette(&path, &sample("adopted")).unwrap();
        let fixtures = crate::fixture::load_yaml_file(&path).unwrap();
        assert_eq!(fixtures.len(), 1);
    }

    #[test]
    fn should_not_clobber_existing_cassette() {
        let path = temp_cassette("no_clobber");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        append_to_cassette(&path, &sample("keep me")).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        ensure_cassette(&path).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn should_convert_recorded_fixture_to_validated_fixture() {
        let fixture = sample("hello").into_fixture().unwrap();
        assert_eq!(fixture.priority, Some(-1));
        assert!(fixture.response.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn should_create_cassette_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_cassette("unit_perms");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "cassette should be owner-only, got {:o}",
            mode
        );
    }
}
