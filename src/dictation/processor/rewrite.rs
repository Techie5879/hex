//! Ordered local cleanup and optional OpenCode rewriting. Each failed step
//! keeps its input, so an OpenCode outage cannot discard successful cleanup.
use super::*;

impl Profile {
    fn meets_rewrite_threshold(&self, text: &str) -> bool {
        (self.min_words == 0 && self.min_chars == 0)
            || (self.min_words > 0 && text.split_whitespace().count() >= self.min_words)
            || (self.min_chars > 0 && text.chars().count() >= self.min_chars)
    }

    pub(super) fn rewrite(
        &self,
        mut text: String,
        local: bool,
        cancelled: &AtomicBool,
        polish: impl FnOnce(&str, &str) -> Result<String>,
        generate: impl FnOnce(&str) -> Result<String>,
    ) -> Processed {
        let started = Instant::now();
        let mut failures = Vec::new();
        if local && !cancelled.load(Ordering::Acquire) {
            // Empty instructions select SpeakoFlow's basic-cleanup default.
            // The mode's custom instruction belongs only to the OpenCode pass.
            let instructions = if self.ai_enabled { "" } else { &self.prompt };
            let result = polish(&text, instructions);
            apply(&mut text, result, "SpeakoFlow", &mut failures);
        }
        let run_ai = self.ai_enabled
            && self.meets_rewrite_threshold(&text)
            && !cancelled.load(Ordering::Acquire);
        if run_ai {
            let result = generate(&text);
            apply(&mut text, result, "OpenCode", &mut failures);
        }
        let observation = (local || run_ai).then(|| ProcessingObservation {
            profile: match (local, run_ai) {
                (true, true) => format!("{} / local SpeakoFlow + OpenCode", self.name),
                (true, false) => format!("{} / local SpeakoFlow", self.name),
                _ => self.name.clone(),
            },
            latency_ms: started.elapsed().as_millis() as u64,
            fallback: (!failures.is_empty()).then(|| failures.join("; ")),
        });
        if let Some(observation) = &observation {
            tracing::info!(
                profile = observation.profile,
                latency_ms = observation.latency_ms,
                fallback = observation.fallback,
                "dictation post-processing completed"
            );
        }
        Processed {
            text,
            observation,
            transformations: self.transformations.clone(),
        }
    }
}

fn apply(text: &mut String, result: Result<String>, stage: &str, failures: &mut Vec<String>) {
    match result {
        Ok(output) if !output.trim().is_empty() => *text = output.trim().into(),
        Ok(_) => failures.push(format!("{stage}: processor returned empty text")),
        Err(error) => failures.push(format!("{stage}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_inclusive_use_either_limit_and_count_unicode_characters() {
        let mut profile = Profile::new("Global", "").ai_enabled(true);
        assert!(profile.meets_rewrite_threshold("hi"));
        profile.min_words = 3;
        profile.min_chars = 10;
        assert!(!profile.meets_rewrite_threshold("one two"));
        assert!(profile.meets_rewrite_threshold("a b c"));
        assert!(profile.meets_rewrite_threshold("abcdefghij"));
        assert!(!profile.meets_rewrite_threshold("é".repeat(9).as_str()));
        assert!(profile.meets_rewrite_threshold("é".repeat(10).as_str()));
        profile.min_words = 0;
        assert!(!profile.meets_rewrite_threshold("a b c"));
        profile.min_chars = 0;
        profile.min_words = 3;
        assert!(!profile.meets_rewrite_threshold("abcdefghij"));
        assert!(profile.meets_rewrite_threshold("a\tb\nc"));
    }

    #[test]
    fn size_gate_uses_local_output_and_reports_only_executed_steps() {
        let mut profile = Profile::new("Global", "Only for OpenCode").ai_enabled(true);
        profile.min_words = 3;
        for local in [false, true] {
            let result = profile.rewrite(
                "uh one two".into(),
                local,
                &AtomicBool::new(false),
                |_, instructions| {
                    assert_eq!(instructions, "");
                    Ok("one two".into())
                },
                |_| {
                    assert!(!local, "cleaned input is below the cutoff");
                    Ok("one two three".into())
                },
            );
            assert_eq!(result.text, if local { "one two" } else { "one two three" });
            assert_eq!(
                result.observation.unwrap().profile,
                if local {
                    "Global / local SpeakoFlow"
                } else {
                    "Global"
                }
            );
        }
    }

    #[test]
    fn skipped_cloud_pass_without_local_cleanup_has_no_observation() {
        let mut profile = Profile::new("Global", "").ai_enabled(true);
        profile.min_words = 150;
        let result = profile.rewrite(
            "short".into(),
            false,
            &AtomicBool::new(false),
            |_, _| panic!("local disabled"),
            |_| panic!("below cutoff"),
        );
        assert_eq!(result.text, "short");
        assert!(result.observation.is_none());
    }

    #[test]
    fn failed_local_cleanup_checks_original_size_before_cloud_pass() {
        let settings = PostProcessingSettings {
            enabled: true,
            min_words: 3,
            min_chars: 100,
            ..Default::default()
        };
        let profile = Profile::configured("Global", ReplacementSet::default(), vec![], &settings);
        let result = profile.rewrite(
            "one two three".into(),
            true,
            &AtomicBool::new(false),
            |_, _| Err(eyre!("unavailable")),
            |text| Ok(text.into()),
        );
        assert_eq!(result.text, "one two three");
        assert_eq!(
            result.observation.unwrap().fallback.as_deref(),
            Some("SpeakoFlow: unavailable")
        );
    }

    #[test]
    fn local_cleanup_feeds_opencode_and_leaves_transformations_until_last() {
        let profile = Profile::new("Slack", "Turn this into bullet points.")
            .ai_enabled(true)
            .transformations(vec!["lowercase".into()]);
        let result = profile.rewrite(
            "Thursday, no, Friday.".into(),
            true,
            &AtomicBool::new(false),
            |text, instruction| {
                assert_eq!(text, "Thursday, no, Friday.");
                assert_eq!(
                    instruction, "",
                    "local cleanup uses its default instructions"
                );
                Ok("Friday.".into())
            },
            |text| {
                assert_eq!(text, "Friday.");
                Ok("- Friday".into())
            },
        );
        assert_eq!(result.text, "- Friday");
        assert_eq!(result.transformations, ["lowercase"]);
        let observation = result.observation.unwrap();
        assert_eq!(observation.profile, "Slack / local SpeakoFlow + OpenCode");
        assert!(observation.fallback.is_none());
    }

    #[test]
    fn failed_or_empty_opencode_keeps_successful_local_cleanup() {
        for output in [Ok(" \n".into()), Err(eyre!("unavailable"))] {
            let result = Profile::new("Global", "Format this.")
                .ai_enabled(true)
                .rewrite(
                    "Thursday, no, Friday.".into(),
                    true,
                    &AtomicBool::new(false),
                    |_, _| Ok("Friday.".into()),
                    |_| output,
                );
            assert_eq!(result.text, "Friday.");
            assert!(
                result
                    .observation
                    .unwrap()
                    .fallback
                    .unwrap()
                    .starts_with("OpenCode:")
            );
        }
    }

    #[test]
    fn failed_local_cleanup_still_allows_explicit_opencode_rewrite() {
        let result = Profile::new("Global", "Format this.")
            .ai_enabled(true)
            .rewrite(
                "Thursday, no, Friday.".into(),
                true,
                &AtomicBool::new(false),
                |_, _| Err(eyre!("load failed")),
                |text| {
                    assert_eq!(text, "Thursday, no, Friday.");
                    Ok("Friday.".into())
                },
            );
        assert_eq!(result.text, "Friday.");
        assert_eq!(
            result.observation.unwrap().fallback.as_deref(),
            Some("SpeakoFlow: load failed")
        );
    }

    #[test]
    fn local_only_keeps_mode_instructions_and_never_calls_opencode() {
        let result = Profile::new("Global", "Keep it short.").rewrite(
            "Some words.".into(),
            true,
            &AtomicBool::new(false),
            |_, instruction| {
                assert_eq!(instruction, "Keep it short.");
                Ok("Words.".into())
            },
            |_| panic!("OpenCode is disabled"),
        );
        assert_eq!(result.text, "Words.");
    }

    #[test]
    fn cancellation_between_steps_does_not_start_opencode() {
        let cancelled = AtomicBool::new(false);
        let result = Profile::new("Global", "Format this.")
            .ai_enabled(true)
            .rewrite(
                "Some words.".into(),
                true,
                &cancelled,
                |_, _| {
                    cancelled.store(true, Ordering::Release);
                    Ok("Words.".into())
                },
                |_| panic!("cancelled before OpenCode"),
            );
        assert_eq!(result.text, "Words.");
    }

    #[test]
    fn opencode_only_and_unprocessed_modes_keep_their_behavior() {
        for enabled in [false, true] {
            let result = Profile::new("Global", "Format this.")
                .ai_enabled(enabled)
                .rewrite(
                    "Some words.".into(),
                    false,
                    &AtomicBool::new(false),
                    |_, _| panic!("local cleanup is disabled"),
                    |_| {
                        assert!(enabled);
                        Ok("Words.".into())
                    },
                );
            assert_eq!(result.text, if enabled { "Words." } else { "Some words." });
            assert_eq!(result.observation.is_some(), enabled);
        }
    }
}
