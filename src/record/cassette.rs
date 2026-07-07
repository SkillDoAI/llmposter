//! Cassette writer: Serialize-only fixture mirrors and the append-only
//! YAML cassette file format.

use serde::Serialize;
use std::path::Path;

/// Priority stamped on every recorded fixture — below the default (0) so
/// hand-written fixtures always win over recordings.
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
    pub(crate) fn to_yaml_entry(&self) -> Result<String, String> {
        let entry = serde_yaml_ng::to_string(&[self])
            .map_err(|e| format!("cassette serialization failed: {}", e))?;
        Ok(entry.strip_prefix("---\n").unwrap_or(&entry).to_string())
    }
}

/// Parse a serialized entry back through the real fixture loader —
/// guarantees in-memory replay matches what a cassette reload produces.
/// The recorder calls this BEFORE any disk write: an entry that fails
/// here (e.g. an empty prompt) must never reach the cassette file, or
/// every later load of that cassette would error.
pub(crate) fn fixture_from_entry(entry: &str) -> Result<crate::fixture::Fixture, String> {
    let mut parsed: Vec<crate::fixture::Fixture> = serde_yaml_ng::from_str(entry)
        .map_err(|e| format!("recorded fixture failed to round-trip: {}", e))?;
    let mut fixture = parsed
        .pop()
        .ok_or("recorded fixture round-trip was empty")?;
    fixture.validate()?;
    Ok(fixture)
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

/// Append one pre-serialized entry (from [`RecordedFixture::to_yaml_entry`]).
/// Handles the pristine `fixtures: []` → `fixtures:` + block-list
/// transition on first append. Caller serializes access.
///
/// Pristine detection requires the last LINE at column 0 to be exactly
/// `fixtures: []` — a suffix check alone would also match that text at
/// the end of an indented block scalar inside a recorded entry and
/// silently corrupt it.
pub(crate) fn append_entry(path: &Path, entry: &str) -> Result<(), String> {
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
    write_cassette_atomic(path, &new_content)
}

/// Replace the cassette atomically: write the full content to
/// `<cassette>.tmp` in the same directory, then rename over the original
/// — a crash mid-write leaves the previous recordings intact. On Unix the
/// tmp file is CREATED with the existing cassette's mode (falling back to
/// `0o600`), so recorded content never exists on disk with looser
/// permissions, even transiently, and the swap never downgrades the
/// owner-only guarantee.
fn write_cassette_atomic(path: &Path, content: &str) -> Result<(), String> {
    use std::io::Write;

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o600);
        opts.mode(mode);
    }
    let write_result = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(content.as_bytes()));
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write cassette {}: {}", path.display(), e));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write cassette {}: {}", path.display(), e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cassette(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("llmposter_record_tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{}_{}.yaml", name, std::process::id()))
    }

    /// Serialize-then-append, as the recorder does across its
    /// validate-first flow — keeps the append tests one call.
    fn append_to_cassette(path: &Path, rec: &RecordedFixture) -> Result<(), String> {
        append_entry(path, &rec.to_yaml_entry()?)
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
        let entry = sample("hello").to_yaml_entry().unwrap();
        let fixture = fixture_from_entry(&entry).unwrap();
        assert_eq!(fixture.priority, Some(-1));
        assert!(fixture.response.is_some());
    }

    #[test]
    fn should_reject_entry_that_fails_fixture_validation() {
        // The empty-prompt shape a Responses API continuation produces:
        // serializes fine, but the reload-path validation must refuse it.
        let entry = sample("").to_yaml_entry().unwrap();
        let err = fixture_from_entry(&entry).unwrap_err();
        assert!(err.contains("user_message"), "got: {}", err);
    }

    #[test]
    fn should_clean_up_tmp_when_atomic_rename_fails() {
        // Target an existing DIRECTORY: the tmp write succeeds but the
        // rename refuses to replace a directory with a file.
        let dir = std::env::temp_dir()
            .join("llmposter_record_tests")
            .join(format!("rename_fail_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = write_cassette_atomic(&dir, "content").unwrap_err();
        assert!(err.contains("cannot write cassette"), "got: {}", err);
        let mut tmp = dir.as_os_str().to_os_string();
        tmp.push(".tmp");
        assert!(
            !std::path::Path::new(&tmp).exists(),
            "failed rename must remove the tmp file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn should_preserve_cassette_mode_across_atomic_append() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_cassette("atomic_mode");
        let _ = std::fs::remove_file(&path);
        ensure_cassette(&path).unwrap();
        // A pre-existing cassette keeps whatever mode the user gave it —
        // the tmp-then-rename swap must not reset it.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        append_to_cassette(&path, &sample("kept mode")).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "append must preserve the cassette's mode");
        let mut tmp = path.as_os_str().to_os_string();
        tmp.push(".tmp");
        assert!(
            !std::path::Path::new(&tmp).exists(),
            "tmp file renamed away on success"
        );
    }

    #[test]
    fn should_error_when_cassette_parent_cannot_be_created() {
        // A regular FILE in the parent chain makes create_dir_all fail.
        let dir = std::env::temp_dir().join("llmposter_record_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join(format!("parent_blocker_{}", std::process::id()));
        std::fs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("sub").join("cassette.yaml");
        let err = ensure_cassette(&path).unwrap_err();
        assert!(
            err.contains("cannot create cassette directory"),
            "got: {}",
            err
        );
        let _ = std::fs::remove_file(&blocker);
        // Degenerate path with no parent at all: the create itself fails
        // cleanly instead of panicking.
        let err = ensure_cassette(Path::new("")).unwrap_err();
        assert!(err.contains("cannot create cassette"), "got: {}", err);
    }

    #[cfg(unix)]
    #[test]
    fn should_treat_lost_create_race_as_ok() {
        // Deterministic stand-in for losing the create race: a dangling
        // symlink makes exists() false, but open(create_new) on the link
        // path fails with AlreadyExists — exactly the race arm's shape.
        let dir = std::env::temp_dir().join("llmposter_record_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join(format!("dangling_{}.yaml", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("no_such_target.yaml"), &link).unwrap();
        assert!(
            ensure_cassette(&link).is_ok(),
            "AlreadyExists during create is a lost race, not an error"
        );
        let _ = std::fs::remove_file(&link);
    }

    #[cfg(unix)]
    #[test]
    fn should_report_unwritable_cassette_location() {
        // Read-only parent directory: create_dir_all succeeds (it already
        // exists) but the create itself fails with PermissionDenied.
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir()
            .join("llmposter_record_tests")
            .join(format!("readonly_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let path = dir.join("cassette.yaml");
        let result = ensure_cassette(&path);
        // Restore before asserting so a failure still cleans up.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let err = result.unwrap_err();
        assert!(err.contains("cannot create cassette"), "got: {}", err);
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
