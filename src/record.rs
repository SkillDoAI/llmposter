//! VCR record/replay: proxy unmatched requests to a real provider API and
//! save the response as a replayable fixture in a cassette file (a YAML
//! fixture file the recorder appends to). Enabled by the `record` feature.

/// How the server treats incoming requests relative to the cassette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum VcrMode {
    /// Serve fixtures only; never contacts an upstream. This is the default.
    #[default]
    Replay,
    /// Forward every request upstream and record 2xx responses.
    /// Existing fixtures are ignored.
    Record,
    /// Serve matching fixtures locally; forward and record only misses.
    RecordOnMiss,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_default_to_replay() {
        assert_eq!(VcrMode::default(), VcrMode::Replay);
    }
}
