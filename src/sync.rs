// Rust guideline compliant 2026-02-21

use crate::models::{ActionType, DiffField, SyncAction, SyncResult, SyncStatus, TrackerEntry};
use std::collections::{HashMap, HashSet};
use tracing::{Level, event};

/// Configuration for the synchronization process.
#[derive(Debug, Default, Clone, Copy)]
pub struct SyncConfig {
    /// If true, existing entries on the target will not be updated.
    pub preserve_existing: bool,
    /// If true, downgrades in status or progress will be prevented.
    pub no_downgrade: bool,
}

/// A manager for comparing and executing synchronization between trackers.
pub struct SyncManager;

impl SyncManager {
    /// This function performs a fuzzy match between source and target entries
    /// using available external IDs (`MAL`, `AniList`, `Kitsu`).
    #[must_use]
    pub fn compare_lists(
        source_entries: &[TrackerEntry],
        target_entries: &[TrackerEntry],
        target_client: &dyn crate::models::TrackerClient,
        config: SyncConfig,
    ) -> Vec<SyncResult> {
        let mut results = Vec::new();

        let mut target_by_mal_id: HashMap<i64, Vec<&TrackerEntry>> = HashMap::new();
        let mut target_by_ani_id: HashMap<i64, Vec<&TrackerEntry>> = HashMap::new();
        let mut target_by_kitsu_id: HashMap<i64, Vec<&TrackerEntry>> = HashMap::new();

        for entry in target_entries {
            if let Some(mal_id) = entry.mal_id {
                target_by_mal_id.entry(mal_id).or_default().push(entry);
            }
            if let Some(ani_id) = entry.ani_id {
                target_by_ani_id.entry(ani_id).or_default().push(entry);
            }
            if let Some(kitsu_id) = entry.kitsu_id {
                target_by_kitsu_id.entry(kitsu_id).or_default().push(entry);
            }
        }

        let mut matched_target_ids = HashSet::new();

        for source_entry in source_entries {
            let mut matched_targets: Vec<&TrackerEntry> = Vec::new();

            if let Some(mal_id) = source_entry.mal_id
                && let Some(targets) = target_by_mal_id.get(&mal_id)
            {
                matched_targets.extend(targets);
            }

            if matched_targets.is_empty()
                && let Some(ani_id) = source_entry.ani_id
                && let Some(targets) = target_by_ani_id.get(&ani_id)
            {
                matched_targets.extend(targets);
            }

            if matched_targets.is_empty()
                && let Some(kitsu_id) = source_entry.kitsu_id
                && let Some(targets) = target_by_kitsu_id.get(&kitsu_id)
            {
                matched_targets.extend(targets);
            }

            if matched_targets.is_empty() {
                // NEW ENTRY logic
                results.push(SyncResult {
                    source_entry: Some(source_entry.clone()),
                    target_entry: None,
                    is_in_sync: false,
                    diff: vec![DiffField {
                        field_name: "presence".to_string(),
                        old_value: serde_json::json!("-"),
                        new_value: serde_json::json!("Target"),
                    }],
                });
            } else {
                for target_entry in matched_targets {
                    matched_target_ids.insert(target_entry.id);
                    results.push(Self::compare(
                        source_entry,
                        target_entry,
                        target_client,
                        config,
                    ));
                }
            }
        }

        // Entries on target but not on source
        for target_entry in target_entries {
            if !matched_target_ids.contains(&target_entry.id) {
                results.push(SyncResult {
                    source_entry: None,
                    target_entry: Some(target_entry.clone()),
                    is_in_sync: false,
                    diff: vec![DiffField {
                        field_name: "presence".to_string(),
                        old_value: serde_json::json!("-"),
                        new_value: serde_json::json!("Source"),
                    }],
                });
            }
        }

        results
    }

    fn normalize_date(
        date: Option<&HashMap<String, Option<i64>>>,
    ) -> Option<HashMap<String, Option<i64>>> {
        if let Some(d) = date
            && d.values().all(std::option::Option::is_none)
        {
            return None;
        }
        date.cloned()
    }

    /// Compares two tracker entries and returns a `SyncResult`.
    ///
    /// This function checks for differences in status, score, progress, volumes,
    /// repeats, notes, and dates.
    #[must_use]
    #[expect(clippy::too_many_lines)]
    pub fn compare(
        source: &TrackerEntry,
        target: &TrackerEntry,
        target_client: &dyn crate::models::TrackerClient,
        config: SyncConfig,
    ) -> SyncResult {
        let mut diffs = Vec::new();
        let mut is_protected = false;

        // 1. Status comparison
        let skip_status = config.no_downgrade && source.status < target.status;
        if source.status != target.status && !skip_status {
            diffs.push(DiffField {
                field_name: "status".to_string(),
                old_value: serde_json::json!(target.status),
                new_value: serde_json::json!(source.status),
            });
        } else if skip_status {
            is_protected = true;
            event!(
                name: "sync.compare.status_downgrade_prevented",
                Level::DEBUG,
                title = source.title,
                target_status = ?target.status,
                source_status = ?source.status,
                "Prevented status downgrade for {}: from {:?} to {:?}",
                source.title,
                target.status,
                source.status
            );
        }

        // 2. Score comparison
        let source_score_for_target = target_client.get_round_trip_score(source.score);

        if source_score_for_target != target.score {
            diffs.push(DiffField {
                field_name: "score".to_string(),
                old_value: serde_json::json!(target.score),
                new_value: serde_json::json!(source_score_for_target),
            });
        }

        // 3. Progress comparison
        let target_maxed = target.max_progress > 0 && target.progress >= target.max_progress;
        let source_maxed = source.max_progress > 0 && source.progress >= source.max_progress;

        let skip_progress = config.no_downgrade && source.progress < target.progress;

        if skip_progress {
            is_protected = true;
            event!(
                name: "sync.compare.progress_downgrade_prevented",
                Level::DEBUG,
                title = source.title,
                target_progress = target.progress,
                source_progress = source.progress,
                "Prevented progress downgrade for {}: from {} to {}",
                source.title,
                target.progress,
                source.progress
            );
        } else if source.status == SyncStatus::Completed && target_maxed {
            // Already maxed out on target, skip
        } else if source.status == SyncStatus::Completed
            && target.status == SyncStatus::Completed
            && target.max_progress == 0
        {
            // Both completed, target max unknown, assume maxed
        } else if source.status == SyncStatus::Completed && target.max_progress > 0 {
            if target.progress != target.max_progress {
                diffs.push(DiffField {
                    field_name: "progress".to_string(),
                    old_value: serde_json::json!(target.progress),
                    new_value: serde_json::json!(target.max_progress),
                });
            }
        } else if source.progress != target.progress && !(source_maxed && target_maxed) {
            diffs.push(DiffField {
                field_name: "progress".to_string(),
                old_value: serde_json::json!(target.progress),
                new_value: serde_json::json!(source.progress),
            });
        }

        // 4. Volumes comparison
        if source.media_type == crate::models::MediaType::Manga {
            let target_maxed_vol = target.max_volumes > 0 && target.volumes >= target.max_volumes;
            let source_maxed_vol = source.max_volumes > 0 && source.volumes >= source.max_volumes;

            let skip_volumes = config.no_downgrade && source.volumes < target.volumes;

            if skip_volumes {
                is_protected = true;
                event!(
                    name: "sync.compare.volumes_downgrade_prevented",
                    Level::DEBUG,
                    title = source.title,
                    target_volumes = target.volumes,
                    source_volumes = source.volumes,
                    "Prevented volumes downgrade for {}: from {} to {}",
                    source.title,
                    target.volumes,
                    source.volumes
                );
            } else if source.status == SyncStatus::Completed && target_maxed_vol {
                // Skip
            } else if source.status == SyncStatus::Completed
                && target.status == SyncStatus::Completed
                && target.max_volumes == 0
            {
                // Both completed, target max unknown, assume maxed
            } else if source.status == SyncStatus::Completed && target.max_volumes > 0 {
                if target.volumes != target.max_volumes {
                    diffs.push(DiffField {
                        field_name: "volumes".to_string(),
                        old_value: serde_json::json!(target.volumes),
                        new_value: serde_json::json!(target.max_volumes),
                    });
                }
            } else if source.volumes != target.volumes && !(source_maxed_vol && target_maxed_vol) {
                diffs.push(DiffField {
                    field_name: "volumes".to_string(),
                    old_value: serde_json::json!(target.volumes),
                    new_value: serde_json::json!(source.volumes),
                });
            }
        }

        // 5. Repeat comparison
        let skip_repeat = config.no_downgrade && source.repeat < target.repeat;
        if source.repeat != target.repeat && !skip_repeat {
            diffs.push(DiffField {
                field_name: "repeat".to_string(),
                old_value: serde_json::json!(target.repeat),
                new_value: serde_json::json!(source.repeat),
            });
        } else if skip_repeat {
            is_protected = true;
            event!(
                name: "sync.compare.repeat_downgrade_prevented",
                Level::DEBUG,
                title = source.title,
                target_repeat = target.repeat,
                source_repeat = source.repeat,
                "Prevented repeat downgrade for {}: from {} to {}",
                source.title,
                target.repeat,
                source.repeat
            );
        }

        // 6. Notes comparison
        if source.notes != target.notes {
            diffs.push(DiffField {
                field_name: "notes".to_string(),
                old_value: serde_json::json!(target.notes),
                new_value: serde_json::json!(source.notes),
            });
        }

        // 7. Dates comparison
        let source_started = Self::normalize_date(source.started_at.as_ref());
        let target_started = Self::normalize_date(target.started_at.as_ref());

        if source_started != target_started {
            diffs.push(DiffField {
                field_name: "started_at".to_string(),
                old_value: serde_json::json!(target_started),
                new_value: serde_json::json!(source_started),
            });
        }

        let source_completed = Self::normalize_date(source.completed_at.as_ref());
        let target_completed = Self::normalize_date(target.completed_at.as_ref());
        if source_completed != target_completed {
            diffs.push(DiffField {
                field_name: "completed_at".to_string(),
                old_value: serde_json::json!(target_completed),
                new_value: serde_json::json!(source_completed),
            });
        }

        if is_protected && diffs.is_empty() {
            diffs.push(DiffField {
                field_name: "presence".to_string(),
                old_value: serde_json::json!("-"),
                new_value: serde_json::json!("Downgrade Prevented"),
            });
        }

        SyncResult {
            source_entry: Some(source.clone()),
            target_entry: Some(target.clone()),
            is_in_sync: diffs.is_empty(),
            diff: diffs,
        }
    }

    /// Generates a list of `SyncAction`s from a `SyncResult`.
    ///
    /// This function determines whether to Add, Update, or Skip based on the
    /// comparison results and the provided configuration.
    #[must_use]
    pub fn generate_actions(
        source_name: &str,
        target_name: &str,
        sync_result: &SyncResult,
        target_supported_ids: Option<Vec<&str>>,
        config: SyncConfig,
    ) -> Option<SyncAction> {
        let default_ids = vec!["mal_id", "ani_id", "kitsu_id"];
        let supported_ids = target_supported_ids.unwrap_or(default_ids);

        let mut diffs = sync_result.diff.clone();
        let mut action = ActionType::Skip;

        if let Some(source_entry) = &sync_result.source_entry {
            let mut has_overlap = false;

            let source_has_mal = source_entry.mal_id.is_some();
            let source_has_ani = source_entry.ani_id.is_some();
            let source_has_kitsu = source_entry.kitsu_id.is_some();

            let target_wants_mal = supported_ids.contains(&"mal_id");
            let target_wants_ani = supported_ids.contains(&"ani_id");
            let target_wants_kitsu = supported_ids.contains(&"kitsu_id");

            if (target_wants_mal && source_has_mal)
                || (target_wants_ani && source_has_ani)
                || (target_wants_kitsu && source_has_kitsu)
            {
                has_overlap = true;
            }

            if !has_overlap {
                action = ActionType::Skip;
                diffs = vec![DiffField {
                    field_name: "ID".to_string(),
                    old_value: serde_json::json!("-"),
                    new_value: serde_json::json!(format!("No supported IDs for {}", target_name)),
                }];
            } else if sync_result.target_entry.is_none() {
                action = ActionType::Add;
                diffs = vec![DiffField {
                    field_name: "presence".to_string(),
                    old_value: serde_json::json!("-"),
                    new_value: serde_json::json!(format!("Will add to {}", target_name)),
                }];
            } else if config.preserve_existing {
                action = ActionType::Skip;
                diffs = vec![DiffField {
                    field_name: "presence".to_string(),
                    old_value: serde_json::json!("-"),
                    new_value: serde_json::json!("Preserved existing target"),
                }];
            } else if config.no_downgrade
                && diffs.len() == 1
                && diffs[0].field_name == "presence"
                && diffs[0].new_value == "Downgrade Prevented"
            {
                action = ActionType::Skip;
            } else if !sync_result.is_in_sync {
                action = ActionType::Update;
            }
        } else {
            // Target but not source -> non-destructive
            return None;
        }

        if action == ActionType::Skip && diffs.is_empty() {
            return None;
        }

        let media_type = if let Some(ref s) = sync_result.source_entry {
            s.media_type
        } else if let Some(ref t) = sync_result.target_entry {
            t.media_type
        } else {
            return None;
        };

        Some(SyncAction {
            action,
            source: source_name.to_string(),
            target: target_name.to_string(),
            media_type,
            source_entry: sync_result.source_entry.clone(),
            target_entry: sync_result.target_entry.clone(),
            reasons: diffs,
        })
    }

    /// Executes the sync plan.
    ///
    /// # Panics
    ///
    /// Panics if a `SyncAction` missing expected entries is encountered.
    pub async fn execute_sync(
        plan: Vec<SyncAction>,
        target_client: &dyn crate::models::TrackerClient,
    ) {
        use crate::models::UpdateOptions;
        use crate::ui::{print_error, print_info, print_success, print_warning};

        for action in plan {
            if action.action == ActionType::Skip {
                continue;
            }

            let Some(source_entry) = &action.source_entry else {
                continue;
            };

            print_info(&format!(
                "[{:?}] Syncing {} to {}...",
                action.action, source_entry.title, action.target
            ));

            let mut entry_id = None;

            // Try resolving media ID via overlapping IDs for both ADD and UPDATE
            // to ensure we have the correct target-specific media identifier.
            if let Some(mal_id) = source_entry.mal_id
                && let Ok(Some(id)) = target_client
                    .get_media_id_by_mal_id(mal_id, action.media_type)
                    .await
            {
                entry_id = Some(id);
            }
            if entry_id.is_none()
                && let Some(ani_id) = source_entry.ani_id
                && let Ok(Some(id)) = target_client
                    .get_media_id_by_ani_id(ani_id, action.media_type)
                    .await
            {
                entry_id = Some(id);
            }
            if entry_id.is_none()
                && let Some(kitsu_id) = source_entry.kitsu_id
                && let Ok(Some(id)) = target_client
                    .get_media_id_by_kitsu_id(kitsu_id, action.media_type)
                    .await
            {
                entry_id = Some(id);
            }

            // Kitsu specific fix: Updates require the library entry ID, not the media ID.
            if action.target == "kitsu" && action.action == ActionType::Update {
                entry_id = Some(
                    action
                        .target_entry
                        .as_ref()
                        .expect("target_entry should exist for Update")
                        .id,
                );
            }

            let Some(entry_id) = entry_id else {
                print_warning(&format!(
                    "[SKIPPED] {} - could not find mapped ID on {}.",
                    source_entry.title, action.target
                ));
                continue;
            };

            let update_options = UpdateOptions::from_sync_action(&action);

            if action.action == ActionType::Add {
                event!(
                    name: "sync.execute.add_entry",
                    Level::DEBUG,
                    target = action.target,
                    resolved_id = entry_id,
                    "Preparing to ADD entry to {}: resolved media ID {}",
                    action.target,
                    entry_id
                );
                event!(
                    name: "sync.execute.payload",
                    Level::DEBUG,
                    payload = ?update_options,
                    "UpdateOptions payload: {:?}",
                    update_options
                );
            }

            // Perform the update
            match target_client
                .update_entry(entry_id, action.media_type, update_options)
                .await
            {
                Ok(true) => print_success(&format!("Successfully synced {}.", source_entry.title)),
                Ok(false) => print_error(&format!("Failed to sync {}.", source_entry.title)),
                Err(e) => print_error(&format!("Error syncing {}: {}", source_entry.title, e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MediaType, TrackerClient, UpdateOptions};
    use async_trait::async_trait;

    struct MockClient;

    #[async_trait]
    impl TrackerClient for MockClient {
        fn supported_ids(&self) -> Vec<&'static str> {
            vec!["mal_id"]
        }
        fn supports_anime(&self) -> bool {
            true
        }
        fn supports_manga(&self) -> bool {
            true
        }
        fn get_round_trip_score(&self, internal_score: i32) -> i32 {
            internal_score
        }
        async fn get_viewer_name(&self) -> color_eyre::Result<String> {
            Ok("test".to_string())
        }
        async fn fetch_anime_list(
            &self,
            _user_name: &str,
        ) -> color_eyre::Result<Vec<TrackerEntry>> {
            Ok(vec![])
        }
        async fn fetch_manga_list(
            &self,
            _user_name: &str,
        ) -> color_eyre::Result<Vec<TrackerEntry>> {
            Ok(vec![])
        }
        async fn update_entry(
            &self,
            _id: i64,
            _type: MediaType,
            _opts: UpdateOptions,
        ) -> color_eyre::Result<bool> {
            Ok(true)
        }
        async fn get_media_id_by_mal_id(
            &self,
            _id: i64,
            _type: MediaType,
        ) -> color_eyre::Result<Option<i64>> {
            Ok(None)
        }
        async fn get_media_id_by_ani_id(
            &self,
            _id: i64,
            _type: MediaType,
        ) -> color_eyre::Result<Option<i64>> {
            Ok(None)
        }
        async fn get_media_id_by_kitsu_id(
            &self,
            _id: i64,
            _type: MediaType,
        ) -> color_eyre::Result<Option<i64>> {
            Ok(None)
        }
    }

    fn dummy_entry() -> TrackerEntry {
        TrackerEntry {
            id: 1,
            mal_id: None,
            ani_id: None,
            kitsu_id: None,
            title: "Test".to_string(),
            media_type: MediaType::Anime,
            status: SyncStatus::Planning,
            score: 0,
            progress: 0,
            volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
            max_progress: 12,
            max_volumes: 0,
        }
    }

    #[test]
    fn test_sync_manager_compare_progress() {
        let mut source = dummy_entry();
        source.progress = 5;

        let target = dummy_entry();
        let client = MockClient;

        let result = SyncManager::compare(&source, &target, &client, SyncConfig::default());
        assert!(!result.is_in_sync);
        assert_eq!(result.diff.len(), 1);
        assert_eq!(result.diff[0].field_name, "progress");
        assert_eq!(result.diff[0].new_value, serde_json::json!(5));
    }

    #[test]
    fn test_sync_manager_compare_status_equality() {
        let mut source = dummy_entry();
        source.status = SyncStatus::Current;

        let mut target = dummy_entry();
        target.status = SyncStatus::Planning;
        let client = MockClient;

        let result = SyncManager::compare(&source, &target, &client, SyncConfig::default());
        assert!(!result.is_in_sync);

        // Reverse should also NOT be in sync since equality is symmetric
        let result_reverse = SyncManager::compare(&target, &source, &client, SyncConfig::default());
        assert!(!result_reverse.is_in_sync);
    }

    #[test]
    fn test_sync_manager_match_by_mal_id() {
        let mut source = dummy_entry();
        source.id = 1;
        source.mal_id = Some(12345);
        source.title = "Source Title".to_string();

        let mut target = dummy_entry();
        target.id = 2;
        target.mal_id = Some(12345);
        target.title = "Target Title".to_string(); // Different title

        let client = MockClient;
        let results =
            SyncManager::compare_lists(&[source], &[target], &client, SyncConfig::default());
        assert_eq!(results.len(), 1);
        assert!(results[0].target_entry.is_some());
        assert_eq!(
            results[0].target_entry.as_ref().unwrap().title,
            "Target Title"
        );
    }

    #[test]
    fn test_sync_manager_detect_add() {
        let mut source = dummy_entry();
        source.mal_id = Some(12345);

        let client = MockClient;
        let results = SyncManager::compare_lists(&[source], &[], &client, SyncConfig::default());
        assert_eq!(results.len(), 1);
        assert!(results[0].target_entry.is_none());
        assert_eq!(results[0].diff[0].field_name, "presence");
    }

    #[test]
    fn test_sync_manager_generate_skip_action() {
        let entry = TrackerEntry {
            id: 1,
            mal_id: None,
            ani_id: Some(86707),
            kitsu_id: None,
            title: "Unmapped Anime".to_string(),
            media_type: crate::models::MediaType::Anime,
            status: SyncStatus::Current,
            score: 80,
            progress: 5,
            max_progress: 12,
            volumes: 0,
            max_volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
        };

        let result = SyncResult {
            source_entry: Some(entry),
            target_entry: None,
            is_in_sync: false,
            diff: vec![DiffField {
                field_name: "presence".to_string(),
                old_value: serde_json::json!("-"),
                new_value: serde_json::json!("Target"),
            }],
        };

        let action = SyncManager::generate_actions(
            "source",
            "target",
            &result,
            Some(vec!["mal_id"]),
            SyncConfig::default(),
        )
        .unwrap();

        assert_eq!(action.action, ActionType::Skip);
    }

    #[test]
    fn test_sync_manager_no_downgrade() {
        let mut source = dummy_entry();
        source.status = SyncStatus::Planning;
        source.progress = 5;
        source.volumes = 1;
        source.repeat = 0;
        source.media_type = MediaType::Manga;

        let mut target = dummy_entry();
        target.status = SyncStatus::Completed;
        target.progress = 10;
        target.volumes = 5;
        target.repeat = 2;
        target.media_type = MediaType::Manga;

        let client = MockClient;
        let config = SyncConfig {
            preserve_existing: false,
            no_downgrade: true,
        };

        // All of these should be skipped due to no_downgrade
        let result = SyncManager::compare(&source, &target, &client, config);

        assert!(!result.diff.iter().any(|r| r.field_name == "status"
            || r.field_name == "progress"
            || r.field_name == "volumes"
            || r.field_name == "repeat"));
    }
    #[test]
    fn test_sync_manager_generate_preserve_existing_skip_action() {
        let entry = TrackerEntry {
            id: 1,
            mal_id: Some(12345),
            ani_id: None,
            kitsu_id: None,
            title: "Existing Anime".to_string(),
            media_type: crate::models::MediaType::Anime,
            status: SyncStatus::Current,
            score: 80,
            progress: 5,
            max_progress: 12,
            volumes: 0,
            max_volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
        };

        let result = SyncResult {
            source_entry: Some(entry.clone()),
            target_entry: Some(entry),
            is_in_sync: false,
            diff: vec![DiffField {
                field_name: "progress".to_string(),
                old_value: serde_json::json!(5),
                new_value: serde_json::json!(6),
            }],
        };

        let config = SyncConfig {
            preserve_existing: true,
            no_downgrade: false,
        };

        let action = SyncManager::generate_actions(
            "source",
            "target",
            &result,
            Some(vec!["mal_id"]),
            config,
        )
        .unwrap();

        assert_eq!(action.action, ActionType::Skip);
        assert_eq!(
            action.reasons[0].new_value,
            serde_json::json!("Preserved existing target")
        );
    }

    #[test]
    fn test_sync_manager_score_rounding() {
        struct RoundingMock;
        #[async_trait]
        impl TrackerClient for RoundingMock {
            fn supported_ids(&self) -> Vec<&'static str> {
                vec![]
            }
            fn supports_anime(&self) -> bool {
                true
            }
            fn supports_manga(&self) -> bool {
                true
            }
            fn get_round_trip_score(&self, internal_score: i32) -> i32 {
                // Simulate Kitsu rounding: (score / 5).round() * 5
                #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let s = (internal_score as f32 / 5.0).round() as i32 * 5;
                s
            }
            async fn get_viewer_name(&self) -> color_eyre::Result<String> {
                Ok("test".to_string())
            }
            async fn fetch_anime_list(
                &self,
                _user_name: &str,
            ) -> color_eyre::Result<Vec<TrackerEntry>> {
                Ok(vec![])
            }
            async fn fetch_manga_list(
                &self,
                _user_name: &str,
            ) -> color_eyre::Result<Vec<TrackerEntry>> {
                Ok(vec![])
            }
            async fn update_entry(
                &self,
                _id: i64,
                _type: MediaType,
                _opts: UpdateOptions,
            ) -> color_eyre::Result<bool> {
                Ok(true)
            }
            async fn get_media_id_by_mal_id(
                &self,
                _id: i64,
                _type: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
            async fn get_media_id_by_ani_id(
                &self,
                _id: i64,
                _type: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
            async fn get_media_id_by_kitsu_id(
                &self,
                _id: i64,
                _type: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
        }

        let mut source = dummy_entry();
        source.score = 9; // AniList score

        let mut target = dummy_entry();
        target.score = 10; // Kitsu score (which is 2/20 * 5)

        let client = RoundingMock;
        let result = SyncManager::compare(&source, &target, &client, SyncConfig::default());

        // Even though 9 != 10, when 9 is "round-tripped" through Kitsu it becomes 10.
        // So they should be considered in sync.
        assert!(result.is_in_sync);
        assert_eq!(result.diff.len(), 0);
    }

    #[test]
    fn test_sync_manager_score_zero() {
        struct RoundingMock;
        #[async_trait]
        impl TrackerClient for RoundingMock {
            fn supported_ids(&self) -> Vec<&'static str> {
                vec![]
            }
            fn supports_anime(&self) -> bool {
                true
            }
            fn supports_manga(&self) -> bool {
                true
            }
            fn get_round_trip_score(&self, internal_score: i32) -> i32 {
                #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let mut score_val = if internal_score == 0 {
                    0
                } else {
                    (internal_score as f32 / 5.0).round() as i32
                };
                if score_val == 0 && internal_score > 0 {
                    score_val = 1;
                }
                score_val * 5
            }
            async fn get_viewer_name(&self) -> color_eyre::Result<String> {
                Ok("test".to_string())
            }
            async fn fetch_anime_list(&self, _u: &str) -> color_eyre::Result<Vec<TrackerEntry>> {
                Ok(vec![])
            }
            async fn fetch_manga_list(&self, _u: &str) -> color_eyre::Result<Vec<TrackerEntry>> {
                Ok(vec![])
            }
            async fn update_entry(
                &self,
                _i: i64,
                _m: MediaType,
                _o: crate::models::UpdateOptions,
            ) -> color_eyre::Result<bool> {
                Ok(true)
            }
            async fn get_media_id_by_mal_id(
                &self,
                _i: i64,
                _m: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
            async fn get_media_id_by_ani_id(
                &self,
                _i: i64,
                _m: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
            async fn get_media_id_by_kitsu_id(
                &self,
                _i: i64,
                _m: MediaType,
            ) -> color_eyre::Result<Option<i64>> {
                Ok(None)
            }
        }

        let mut source = dummy_entry();
        source.score = 0;

        let mut target = dummy_entry();
        target.score = 80;

        let client = RoundingMock;
        let result = SyncManager::compare(&source, &target, &client, SyncConfig::default());

        assert!(!result.is_in_sync);
        assert!(
            result
                .diff
                .iter()
                .any(|r| r.field_name == "score" && r.new_value == serde_json::json!(0))
        );

        let mut target_zero = dummy_entry();
        target_zero.score = 0;
        let result_zero =
            SyncManager::compare(&source, &target_zero, &client, SyncConfig::default());
        assert!(result_zero.is_in_sync);
        assert!(result_zero.diff.is_empty());
    }
}
