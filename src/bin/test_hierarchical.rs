//! Standalone Hierarchical Scheduler Test Binary
//!
//! Self-contained implementation of 7-parameter hierarchical scheduling system.
//! This binary can be tested independently before integration with the main system.
//!
//! Usage: test_hierarchical <input.xlsx> <output.xlsx>

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use std::path::Path;
use conference_scheduler::conflict_detection;

// ============================================================================
// CORE TYPE DEFINITIONS
// ============================================================================

/// Represents a minisymposium (either regular or CL-grouped)
#[derive(Clone, Debug)]
pub struct Minisymposium {
    pub id: i32,
    pub title: String,
    pub notes: String,
    pub organizers: Vec<String>,
    pub speakers: Vec<String>,
    pub classification: Vec<i32>,
    pub series_key: Option<String>,
    pub part_number: Option<i32>,
    pub is_cl_minisymposium: bool,
    pub cl_lecture_ids: Vec<i32>,
}

/// Represents an individual contributed lecture (before grouping)
#[derive(Clone, Debug)]
pub struct ContributedLecture {
    pub id: i32,
    pub title: String,
    pub speaker: String,
    pub classification: Vec<i32>,
    pub notes: String,
}

/// Schedule represented as time slots × rooms
/// Each cell contains the index of a Minisymposium
pub type Schedule = Vec<Vec<usize>>;

/// Configuration for the hierarchical scheduler with 5 ranked parameters
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Parameter rankings (0=ignore, 1=highest priority, 5=lowest priority)
    pub classification_rank: u8,
    pub speaker_org_rank: u8,
    pub personal_class_rank: u8,
    pub series_room_rank: u8,
    pub series_back_rank: u8,

    /// Strict hierarchy mode: never allow higher-priority conflicts to increase
    /// true = strict (recommended), false = allow small violations (1-2 conflicts)
    pub strict_hierarchy: bool,

    /// Maximum number of solutions to retain at each optimization level
    pub max_solutions_per_level: usize,

    /// Time limit per parameter optimization (seconds)
    pub parameter_timeout: u64,

    /// Random seed for reproducible results
    pub random_seed: u64,

    /// Number of parallel time slots
    pub num_sessions: usize,

    /// Number of rooms per session
    pub num_rooms: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            classification_rank: 5,         // Rank 5: Lowest priority
            speaker_org_rank: 1,            // Rank 1: HIGHEST priority (personal conflicts)
            personal_class_rank: 2,         // Rank 2: Personal-Classification conflicts
            series_room_rank: 4,            // Rank 4: Series Same-Room conflicts
            series_back_rank: 3,            // Rank 3: Series Back-to-Back conflicts
            strict_hierarchy: true,         // Strict mode by default
            max_solutions_per_level: 10,
            parameter_timeout: 30,
            random_seed: 42,
            num_sessions: 8,
            num_rooms: 12,
        }
    }
}

/// Individual conflict types that can be optimized
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictType {
    Classification,
    SpeakerOrganizer,
    PersonalClassification,
    SeriesSameRoom,
    SeriesBackToBack,
}

impl ConflictType {
    pub fn get_rank(&self, config: &SchedulerConfig) -> u8 {
        match self {
            Self::Classification => config.classification_rank,
            Self::SpeakerOrganizer => config.speaker_org_rank,
            Self::PersonalClassification => config.personal_class_rank,
            Self::SeriesSameRoom => config.series_room_rank,
            Self::SeriesBackToBack => config.series_back_rank,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Classification => "Classification Conflicts",
            Self::SpeakerOrganizer => "Speaker/Organizer Conflicts",
            Self::PersonalClassification => "Personal-Classification Conflicts",
            Self::SeriesSameRoom => "Series Same-Room Conflicts",
            Self::SeriesBackToBack => "Series Back-to-Back Conflicts",
        }
    }
}

/// Complete conflict count for a schedule
#[derive(Debug, Clone, Default)]
pub struct ConflictCounts {
    pub classification: i32,
    pub speaker_organizer: i32,
    pub personal_classification: i32,
    pub series_same_room: i32,
    pub series_back_to_back: i32,
}

impl ConflictCounts {
    pub fn get(&self, conflict_type: ConflictType) -> i32 {
        match conflict_type {
            ConflictType::Classification => self.classification,
            ConflictType::SpeakerOrganizer => self.speaker_organizer,
            ConflictType::PersonalClassification => self.personal_classification,
            ConflictType::SeriesSameRoom => self.series_same_room,
            ConflictType::SeriesBackToBack => self.series_back_to_back,
        }
    }

    pub fn set(&mut self, conflict_type: ConflictType, count: i32) {
        match conflict_type {
            ConflictType::Classification => self.classification = count,
            ConflictType::SpeakerOrganizer => self.speaker_organizer = count,
            ConflictType::PersonalClassification => self.personal_classification = count,
            ConflictType::SeriesSameRoom => self.series_same_room = count,
            ConflictType::SeriesBackToBack => self.series_back_to_back = count,
        }
    }

    pub fn total(&self) -> i32 {
        self.classification
            + self.speaker_organizer
            + self.personal_classification
            + self.series_same_room
            + self.series_back_to_back
    }
}

/// A candidate solution with its conflict analysis
#[derive(Debug, Clone)]
pub struct Solution {
    pub schedule: Schedule,
    pub conflicts: ConflictCounts,
    pub fitness_score: f64,
}

impl Solution {
    pub fn new(schedule: Schedule, conflicts: ConflictCounts) -> Self {
        Self {
            fitness_score: Self::calculate_fitness(&conflicts),
            schedule,
            conflicts,
        }
    }

    pub fn calculate_fitness(conflicts: &ConflictCounts) -> f64 {
        conflicts.classification as f64 * 10.0
            + conflicts.speaker_organizer as f64 * 8.0
            + conflicts.personal_classification as f64 * 6.0
            + conflicts.series_same_room as f64 * 4.0
            + conflicts.series_back_to_back as f64 * 3.0
    }

    pub fn update_fitness(&mut self) {
        self.fitness_score = Self::calculate_fitness(&self.conflicts);
    }
}

// ============================================================================
// FEASIBILITY ANALYSIS
// ============================================================================

#[derive(Debug)]
struct FeasibilityReport {
    speaker_organizer_feasible: bool,
    speaker_organizer_reason: String,
    personal_classification_feasible: bool,
    personal_classification_reason: String,
    series_room_feasible: bool,
    series_room_reason: String,
    series_back_to_back_feasible: bool,
    series_back_to_back_reason: String,
}

/// Analyze if zero conflicts are theoretically possible for each parameter
fn analyze_feasibility(all_sessions: &[Minisymposium], num_time_slots: usize, num_rooms: usize) -> FeasibilityReport {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║       PRE-SCHEDULING FEASIBILITY ANALYSIS            ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("\nTotal sessions: {}", all_sessions.len());
    println!("Available time slots: {}", num_time_slots);
    println!("Available rooms per slot: {}", num_rooms);
    println!("Total capacity: {} sessions\n", num_time_slots * num_rooms);

    // 1. Check Speaker/Organizer conflicts
    let (speaker_feasible, speaker_reason) = check_speaker_organizer_feasibility(all_sessions, num_time_slots, num_rooms);

    // 2. Check Personal-Classification conflicts
    let (personal_feasible, personal_reason) = check_personal_classification_feasibility(all_sessions, num_time_slots, num_rooms);

    // 3. Check Series Room conflicts
    let (series_room_feasible, series_room_reason) = check_series_room_feasibility(all_sessions, num_rooms);

    // 4. Check Series Back-to-Back conflicts
    let (series_back_feasible, series_back_reason) = check_series_back_to_back_feasibility(all_sessions, num_time_slots);

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║              FEASIBILITY SUMMARY                      ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    FeasibilityReport {
        speaker_organizer_feasible: speaker_feasible,
        speaker_organizer_reason: speaker_reason,
        personal_classification_feasible: personal_feasible,
        personal_classification_reason: personal_reason,
        series_room_feasible: series_room_feasible,
        series_room_reason: series_room_reason,
        series_back_to_back_feasible: series_back_feasible,
        series_back_to_back_reason: series_back_reason,
    }
}

fn check_speaker_organizer_feasibility(all_sessions: &[Minisymposium], num_time_slots: usize, num_rooms: usize) -> (bool, String) {
    // Build a graph of "must be in different time slots"
    // If any person appears in more sessions than we have time slots, it's impossible

    let mut person_sessions: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, ms) in all_sessions.iter().enumerate() {
        for person in ms.speakers.iter().chain(ms.organizers.iter()) {
            let normalized = conflict_detection::normalize_exact(person);
            if !normalized.is_empty() {
                person_sessions.entry(normalized)
                    .or_insert_with(Vec::new)
                    .push(idx);
            }
        }
    }

    // Find the most constrained person
    let mut max_sessions_per_person = 0;
    let mut most_constrained_person = String::new();

    for (person, sessions) in &person_sessions {
        if sessions.len() > max_sessions_per_person {
            max_sessions_per_person = sessions.len();
            most_constrained_person = person.clone();
        }
    }

    let feasible = max_sessions_per_person <= num_time_slots;

    let reason = if feasible {
        format!("✓ FEASIBLE: Max sessions per person = {} (fits in {} time slots)",
                max_sessions_per_person, num_time_slots)
    } else {
        format!("✗ IMPOSSIBLE: Person '{}' appears in {} sessions but only {} time slots available",
                most_constrained_person, max_sessions_per_person, num_time_slots)
    };

    println!("1. Speaker/Organizer Conflicts:");
    println!("   {}", reason);

    (feasible, reason)
}

fn check_personal_classification_feasibility(all_sessions: &[Minisymposium], num_time_slots: usize, num_rooms: usize) -> (bool, String) {
    // Count sessions with each notes keyword
    let mut notes_groups: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, ms) in all_sessions.iter().enumerate() {
        let normalized = conflict_detection::normalize_exact(&ms.notes);
        if !normalized.is_empty() && !normalized.contains("notes") {
            notes_groups.entry(normalized)
                .or_insert_with(Vec::new)
                .push(idx);
        }
    }

    // Find largest group
    let mut max_same_notes = 0;
    let mut largest_group_key = String::new();

    for (notes, sessions) in &notes_groups {
        if sessions.len() > max_same_notes {
            max_same_notes = sessions.len();
            largest_group_key = notes.clone();
        }
    }

    let capacity_per_slot = num_rooms;
    let feasible = max_same_notes <= num_time_slots * capacity_per_slot;

    let reason = if max_same_notes == 0 {
        "✓ FEASIBLE: No sessions with matching notes keywords".to_string()
    } else if feasible {
        format!("✓ FEASIBLE: Max {} sessions with same notes '{}' (capacity: {} slots × {} rooms = {})",
                max_same_notes, largest_group_key, num_time_slots, capacity_per_slot,
                num_time_slots * capacity_per_slot)
    } else {
        format!("✗ IMPOSSIBLE: {} sessions with notes '{}' but capacity is only {} ({}×{})",
                max_same_notes, largest_group_key, num_time_slots * capacity_per_slot,
                num_time_slots, capacity_per_slot)
    };

    println!("\n2. Personal-Classification Conflicts:");
    println!("   {}", reason);
    if max_same_notes > 0 {
        println!("   Note: {} sessions share notes, need to spread across different time slots", max_same_notes);
    }

    (feasible, reason)
}

fn check_series_room_feasibility(all_sessions: &[Minisymposium], num_rooms: usize) -> (bool, String) {
    // Count series and check if we have enough rooms
    let mut series_count: HashMap<String, usize> = HashMap::new();

    for ms in all_sessions {
        if let Some(series_key) = &ms.series_key {
            *series_count.entry(series_key.clone()).or_insert(0) += 1;
        }
    }

    // All parts of a series should be in the same room
    // So we need at least as many rooms as the maximum concurrent series
    let total_series = series_count.len();

    let feasible = total_series <= num_rooms;

    let reason = if total_series == 0 {
        "✓ FEASIBLE: No multi-part series detected".to_string()
    } else if feasible {
        format!("✓ FEASIBLE: {} series detected, {} rooms available", total_series, num_rooms)
    } else {
        format!("✗ MIGHT BE DIFFICULT: {} series but only {} rooms (series may need to share rooms)",
                total_series, num_rooms)
    };

    println!("\n3. Series Room Conflicts:");
    println!("   {}", reason);

    (feasible, reason)
}

fn check_series_back_to_back_feasibility(all_sessions: &[Minisymposium], num_time_slots: usize) -> (bool, String) {
    // Check if series parts can fit consecutively
    let mut series_parts: HashMap<String, usize> = HashMap::new();

    for ms in all_sessions {
        if let Some(series_key) = &ms.series_key {
            *series_parts.entry(series_key.clone()).or_insert(0) += 1;
        }
    }

    let mut max_series_length = 0;
    let mut longest_series = String::new();

    for (series, parts) in &series_parts {
        if *parts > max_series_length {
            max_series_length = *parts;
            longest_series = series.clone();
        }
    }

    let feasible = max_series_length <= num_time_slots;

    let reason = if max_series_length == 0 {
        "✓ FEASIBLE: No multi-part series detected".to_string()
    } else if feasible {
        format!("✓ FEASIBLE: Longest series has {} parts, {} time slots available",
                max_series_length, num_time_slots)
    } else {
        format!("✗ IMPOSSIBLE: Series '{}' has {} parts but only {} time slots available",
                longest_series, max_series_length, num_time_slots)
    };

    println!("\n4. Series Back-to-Back Conflicts:");
    println!("   {}", reason);

    (feasible, reason)
}

// ============================================================================
// CONSTRAINT PRESERVATION HELPERS
// ============================================================================

/// Check if new conflicts violate protected (higher-priority) constraints
/// Returns true if the swap should be rejected
fn violates_protected_constraints(
    new_conflicts: &ConflictCounts,
    old_conflicts: &ConflictCounts,
    protected_types: &[ConflictType],
    strict: bool,
) -> bool {
    for &conflict_type in protected_types {
        let old_count = old_conflicts.get(conflict_type);
        let new_count = new_conflicts.get(conflict_type);

        if strict {
            // Strict mode: reject ANY increase
            if new_count > old_count {
                return true;
            }
        } else {
            // Flexible mode: allow small increases (up to 2)
            if new_count > old_count + 2 {
                return true;
            }
        }
    }

    false
}

// ============================================================================
// CONFLICT DETECTION FUNCTIONS
// ============================================================================

/// Check if two minisymposia have classification conflicts
/// Uses shared conflict detection logic
fn has_classification_conflict(ms1: &Minisymposium, ms2: &Minisymposium) -> bool {
    // Convert classification codes to strings for shared module
    let class1: Vec<String> = ms1.classification.iter().map(|c| c.to_string()).collect();
    let class2: Vec<String> = ms2.classification.iter().map(|c| c.to_string()).collect();

    conflict_detection::has_classification_conflict(&class1, &class2)
}

/// Check if two minisymposia have speaker/organizer conflicts
/// ALL speakers AND organizers must be unique across sessions in same time slot
/// Uses shared conflict detection logic (exact match with normalization)
fn has_speaker_organizer_conflict(ms1: &Minisymposium, ms2: &Minisymposium) -> bool {
    conflict_detection::has_speaker_organizer_conflict(
        &ms1.speakers,
        &ms1.organizers,
        &ms2.speakers,
        &ms2.organizers,
    )
}

/// Check if two minisymposia have personal-classification conflicts
/// This checks if both sessions have the same notes keyword (exact match only)
/// Uses shared conflict detection logic (exact match with normalization, no substring)
fn has_personal_classification_conflict(ms1: &Minisymposium, ms2: &Minisymposium) -> bool {
    conflict_detection::has_personal_classification_conflict(&ms1.notes, &ms2.notes)
}

/// Check if two minisymposia are part of the same series
/// Uses shared conflict detection logic
fn is_series_sibling(ms1: &Minisymposium, ms2: &Minisymposium) -> bool {
    conflict_detection::is_series_sibling(&ms1.title, &ms2.title)
}

// TBD conflict detection removed - not part of the 5 core conflict types

// ============================================================================
// CL PREPROCESSING MODULE
// ============================================================================

/// CL Preprocessor: Groups contributed lectures into sets of exactly 4
pub struct CLPreprocessor {
    group_size: usize,
}

impl CLPreprocessor {
    pub fn new() -> Self {
        Self { group_size: 4 }
    }

    /// Group contributed lectures into minisymposia of 4 lectures each
    /// Two-phase grouping:
    /// Phase 1: Group by same notes keyword (different speakers)
    /// Phase 2: Group remaining lectures by classification similarity
    pub fn group_lectures(&self, lectures: Vec<ContributedLecture>) -> Vec<Minisymposium> {
        println!("\n=== CL PREPROCESSING ===");
        println!("Total lectures to group: {}", lectures.len());

        if lectures.is_empty() {
            return Vec::new();
        }

        let mut grouped_ms = Vec::new();
        let mut used_indices = HashSet::new();
        let mut group_id = 10000; // Start CL groups at 10000

        // PHASE 1: Group by same notes keyword
        println!("\nPhase 1: Grouping by notes keyword...");
        let notes_groups = self.group_by_notes(&lectures, &mut used_indices, &mut group_id);
        println!("  Created {} groups from notes-based grouping", notes_groups.len());
        grouped_ms.extend(notes_groups);

        // PHASE 2: Group remaining lectures by classification
        println!("\nPhase 2: Grouping remaining lectures by classification...");
        let classification_groups = self.group_by_classification(&lectures, &mut used_indices, &mut group_id);
        println!("  Created {} groups from classification-based grouping", classification_groups.len());
        grouped_ms.extend(classification_groups);

        println!("\nTotal CL groups created: {}", grouped_ms.len());
        println!("Lectures grouped: {} / {}", used_indices.len(), lectures.len());
        grouped_ms
    }

    /// Phase 1: Group lectures with the same notes keyword
    fn group_by_notes(&self, lectures: &[ContributedLecture], used_indices: &mut HashSet<usize>,
                      group_id: &mut i32) -> Vec<Minisymposium> {
        let mut groups = Vec::new();

        // Build notes keyword index: keyword -> list of lecture indices
        let mut notes_index: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, lecture) in lectures.iter().enumerate() {
            if used_indices.contains(&idx) {
                continue;
            }
            let keyword = lecture.notes.trim().to_lowercase();
            if !keyword.is_empty() {
                notes_index.entry(keyword).or_insert_with(Vec::new).push(idx);
            }
        }

        // For each notes keyword, create groups of 4
        for (keyword, mut indices) in notes_index {
            if indices.len() < 2 {
                continue; // Need at least 2 lectures to form a meaningful group
            }

            // Filter out lectures from same speaker
            indices = self.filter_duplicate_speakers(lectures, &indices);

            // Create groups of 4
            while indices.len() >= 4 {
                let group: Vec<usize> = indices.drain(..4).collect();

                // Mark as used
                for &idx in &group {
                    used_indices.insert(idx);
                }

                let ms = self.create_cl_minisymposium(lectures, &group, *group_id);
                self.print_group_debug(&ms, *group_id, "notes");
                groups.push(ms);
                *group_id += 1;
            }
        }

        groups
    }

    /// Phase 2: Group remaining lectures by classification similarity
    fn group_by_classification(&self, lectures: &[ContributedLecture], used_indices: &mut HashSet<usize>,
                                group_id: &mut i32) -> Vec<Minisymposium> {
        let mut groups = Vec::new();

        // Build compatibility matrix for remaining lectures
        let remaining: Vec<usize> = (0..lectures.len())
            .filter(|idx| !used_indices.contains(idx))
            .collect();

        if remaining.is_empty() {
            return groups;
        }

        let compat_matrix = self.build_compatibility_matrix(lectures);

        // Greedy grouping by classification
        let mut local_used = HashSet::new();
        for &i in &remaining {
            if local_used.contains(&i) {
                continue;
            }

            let mut group = vec![i];
            local_used.insert(i);

            // Find 3 more compatible lectures by classification
            let mut candidates: Vec<(usize, f64)> = remaining.iter()
                .filter(|&&idx| !local_used.contains(&idx))
                .filter(|&&idx| {
                    // Ensure different speakers
                    let speaker_i = lectures[i].speaker.trim().to_lowercase();
                    let speaker_j = lectures[idx].speaker.trim().to_lowercase();
                    speaker_i.is_empty() || speaker_j.is_empty() || speaker_i != speaker_j
                })
                .map(|&idx| {
                    let compatibility = self.calculate_group_compatibility(&group, idx, &compat_matrix);
                    (idx, compatibility)
                })
                .collect();

            candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

            // Take top 3
            for (idx, _) in candidates.iter().take(self.group_size - 1) {
                group.push(*idx);
                local_used.insert(*idx);
            }

            // Mark as used globally
            for &idx in &group {
                used_indices.insert(idx);
            }

            let ms = self.create_cl_minisymposium(lectures, &group, *group_id);
            self.print_group_debug(&ms, *group_id, "classification");
            groups.push(ms);
            *group_id += 1;
        }

        groups
    }

    /// Filter out lectures from the same speaker
    fn filter_duplicate_speakers(&self, lectures: &[ContributedLecture], indices: &[usize]) -> Vec<usize> {
        let mut filtered = Vec::new();
        let mut seen_speakers = HashSet::new();

        for &idx in indices {
            let speaker = lectures[idx].speaker.trim().to_lowercase();
            if speaker.is_empty() || !seen_speakers.contains(&speaker) {
                filtered.push(idx);
                if !speaker.is_empty() {
                    seen_speakers.insert(speaker);
                }
            }
        }

        filtered
    }

    /// Debug print for group creation
    fn print_group_debug(&self, ms: &Minisymposium, group_id: i32, method: &str) {
        let classifications: Vec<String> = ms.classification.iter().map(|c| c.to_string()).collect();
        let speakers_list = ms.speakers.join(", ");
        println!("  [{}] CL group {} with {} lectures",
                 method, group_id, ms.speakers.len());
        println!("    Classifications: [{}] | Speakers: {}",
                 classifications.join(", "),
                 &speakers_list[..speakers_list.len().min(80)]);
        println!("    Notes: {}", &ms.notes[..ms.notes.len().min(60)]);
    }

    /// Build compatibility matrix between lectures
    fn build_compatibility_matrix(&self, lectures: &[ContributedLecture]) -> Vec<Vec<f64>> {
        let n = lectures.len();
        let mut matrix = vec![vec![0.0; n]; n];

        for i in 0..n {
            for j in (i + 1)..n {
                let compatibility = self.calculate_lecture_compatibility(&lectures[i], &lectures[j]);
                matrix[i][j] = compatibility;
                matrix[j][i] = compatibility;
            }
        }

        matrix
    }

    /// Calculate compatibility between two lectures (higher = more compatible)
    /// Prioritizes grouping lectures with:
    /// 1. Same classifications (topic coherence)
    /// 2. Different speakers (avoid speaker conflicts)
    /// 3. Same notes keywords (personal-classification similarity)
    fn calculate_lecture_compatibility(&self, l1: &ContributedLecture, l2: &ContributedLecture) -> f64 {
        let mut score = 0.0;

        // BONUS for classification overlap (group similar topics together)
        let mut classification_matches = 0;
        for code1 in &l1.classification {
            for code2 in &l2.classification {
                if code1 == code2 {
                    classification_matches += 1;
                }
            }
        }
        score += classification_matches as f64 * 2.0; // Strong bonus for matching classifications

        // PENALTY for same speaker (avoid speaker conflicts within CL group)
        let speaker1 = l1.speaker.trim().to_lowercase();
        let speaker2 = l2.speaker.trim().to_lowercase();
        if !speaker1.is_empty() && !speaker2.is_empty() && speaker1 == speaker2 {
            score -= 5.0; // Strong penalty - same person can't present multiple talks in one group
        }

        // BONUS for same notes keyword (personal-classification similarity)
        let notes1 = l1.notes.trim().to_lowercase();
        let notes2 = l2.notes.trim().to_lowercase();
        if !notes1.is_empty() && !notes2.is_empty() {
            if notes1 == notes2 {
                score += 1.5; // Same keyword = good compatibility
            } else if notes1.contains(&notes2) || notes2.contains(&notes1) {
                score += 1.0; // Partial keyword match = moderate compatibility
            }
        }

        // Additional bonus for topic similarity in notes text
        let topic_sim = conflict_detection::calculate_topic_similarity(&l1.notes, &l2.notes);
        score += topic_sim * 0.5;

        score
    }

    /// Calculate how compatible a new lecture is with an existing group
    fn calculate_group_compatibility(&self, group: &[usize], new_idx: usize,
                                     compat_matrix: &[Vec<f64>]) -> f64 {
        if group.is_empty() {
            return 1.0;
        }

        let mut total = 0.0;
        for &existing_idx in group {
            total += compat_matrix[existing_idx][new_idx];
        }

        total / group.len() as f64
    }

    /// Create a CL minisymposium from a group of lectures
    fn create_cl_minisymposium(&self, lectures: &[ContributedLecture],
                               group: &[usize], id: i32) -> Minisymposium {
        let mut all_classifications = Vec::new();
        let mut speakers = Vec::new();
        let mut cl_ids = Vec::new();
        let mut notes_parts = Vec::new();
        let mut titles_parts = Vec::new();

        for &idx in group {
            if idx < lectures.len() {
                let lecture = &lectures[idx];
                speakers.push(lecture.speaker.clone());
                cl_ids.push(lecture.id);
                all_classifications.extend(lecture.classification.clone());
                if !lecture.notes.is_empty() {
                    notes_parts.push(lecture.notes.clone());
                }
                if !lecture.title.is_empty() {
                    titles_parts.push(lecture.title.clone());
                }
            }
        }

        // Deduplicate classifications
        all_classifications.sort();
        all_classifications.dedup();

        // Create title with group ID and all lecture titles
        let all_titles = titles_parts.join(" | ");
        let title = format!("CL Group {} - {}", id, all_titles);
        let notes = notes_parts.join(" | ");

        Minisymposium {
            id,
            title,
            notes,
            organizers: Vec::new(), // CL groups have no organizers
            speakers,
            classification: all_classifications,
            series_key: None,
            part_number: None,
            is_cl_minisymposium: true,
            cl_lecture_ids: cl_ids,
        }
    }
}

// ============================================================================
// PARAMETER OPTIMIZERS
// ============================================================================

/// Common trait for all parameter optimizers
trait ParameterOptimizer {
    fn optimize_parameter(
        &self,
        solutions: &[Solution],
        all_sessions: &[Minisymposium],
        config: &SchedulerConfig,
        protected_types: &[ConflictType]  // Higher-priority parameters to preserve
    ) -> Vec<Solution>;

    fn conflict_type(&self) -> ConflictType;
    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32;
    fn algorithm_name(&self) -> &'static str;
}

// Classification Optimizer: Graph Coloring + Simulated Annealing
struct ClassificationOptimizer;

impl ParameterOptimizer for ClassificationOptimizer {
    fn optimize_parameter(&self, solutions: &[Solution], all_sessions: &[Minisymposium],
                         config: &SchedulerConfig, protected_types: &[ConflictType]) -> Vec<Solution> {
        let mut optimized_solutions = Vec::new();

        for solution in solutions {
            let current_conflicts = self.calculate_conflicts(&solution.schedule, all_sessions);
            let mut best_schedule = solution.schedule.clone();
            let mut best_conflicts = current_conflicts;

            // Random swap optimization
            // OPTIMIZED: Increased from 100 to 300 iterations for better quality
            let max_iterations = 300;
            for _ in 0..max_iterations {
                let mut test_schedule = best_schedule.clone();

                if let Some((s1, s2)) = pick_random_swap(&test_schedule) {
                    apply_swap(&mut test_schedule, s1, s2);

                    // Calculate conflicts (OPTIMIZED: only needed types)
                    let test_all_conflicts = calculate_selective_conflicts(
                        &test_schedule, all_sessions,
                        ConflictType::Classification, protected_types
                    );

                    // Check if this swap violates protected constraints
                    if violates_protected_constraints(&test_all_conflicts, &solution.conflicts,
                                                      protected_types, config.strict_hierarchy) {
                        continue; // Reject swap silently
                    }

                    let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);

                    if test_conflicts < best_conflicts {
                        best_schedule = test_schedule;
                        best_conflicts = test_conflicts;
                    }
                }
            }

            let mut new_conflicts = solution.conflicts.clone();
            new_conflicts.classification = best_conflicts;
            optimized_solutions.push(Solution::new(best_schedule, new_conflicts));
        }

        optimized_solutions
    }

    fn conflict_type(&self) -> ConflictType {
        ConflictType::Classification
    }

    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32 {
        let mut conflicts = 0;

        for session_slot in schedule {
            for i in 0..session_slot.len() {
                for j in (i + 1)..session_slot.len() {
                    let ms1_idx = session_slot[i];
                    let ms2_idx = session_slot[j];

                    if ms1_idx < all_sessions.len() && ms2_idx < all_sessions.len() {
                        if has_classification_conflict(&all_sessions[ms1_idx], &all_sessions[ms2_idx]) {
                            conflicts += 1;
                        }
                    }
                }
            }
        }

        conflicts
    }

    fn algorithm_name(&self) -> &'static str {
        "Graph Coloring + Simulated Annealing"
    }
}

// Speaker/Organizer Optimizer: Targeted Conflict Resolution
struct SpeakerOrganizerOptimizer;

impl SpeakerOrganizerOptimizer {
    /// Find all sessions in a time slot that have speaker/organizer conflicts
    fn find_conflicting_sessions(&self, slot: &[usize], all_sessions: &[Minisymposium]) -> Vec<usize> {
        let mut conflicting = HashSet::new();

        for i in 0..slot.len() {
            for j in (i + 1)..slot.len() {
                let ms1_idx = slot[i];
                let ms2_idx = slot[j];

                if ms1_idx < all_sessions.len() && ms2_idx < all_sessions.len() {
                    if has_speaker_organizer_conflict(&all_sessions[ms1_idx], &all_sessions[ms2_idx]) {
                        conflicting.insert(i);
                        conflicting.insert(j);
                    }
                }
            }
        }

        conflicting.into_iter().collect()
    }

    /// Find all sessions involved in speaker/organizer conflicts
    /// Returns list of (slot_idx, room_idx) for sessions that have conflicts
    fn find_all_conflicting_sessions(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> Vec<(usize, usize)> {
        let mut conflicting = Vec::new();

        for (slot_idx, slot) in schedule.iter().enumerate() {
            for (room_idx, &ms_idx) in slot.iter().enumerate() {
                // Check if this session conflicts with any other in same slot
                for (other_room_idx, &other_ms_idx) in slot.iter().enumerate() {
                    if room_idx != other_room_idx &&
                       ms_idx < all_sessions.len() &&
                       other_ms_idx < all_sessions.len() {
                        if has_speaker_organizer_conflict(&all_sessions[ms_idx], &all_sessions[other_ms_idx]) {
                            conflicting.push((slot_idx, room_idx));
                            break;
                        }
                    }
                }
            }
        }

        conflicting
    }

    /// Find the best time slot for a session (where it has no speaker/org conflicts)
    fn find_best_slot(&self, ms_idx: usize, schedule: &Schedule, all_sessions: &[Minisymposium]) -> Option<usize> {
        let mut best_slot = None;
        let mut min_conflicts = i32::MAX;

        for (slot_idx, slot) in schedule.iter().enumerate() {
            // Check if this session would conflict with any in this slot
            let mut has_conflict = false;
            for &other_ms_idx in slot {
                if other_ms_idx < all_sessions.len() && ms_idx < all_sessions.len() {
                    if has_speaker_organizer_conflict(&all_sessions[ms_idx], &all_sessions[other_ms_idx]) {
                        has_conflict = true;
                        break;
                    }
                }
            }

            if !has_conflict {
                // Count other types of conflicts as tiebreaker
                let mut other_conflicts = 0;
                for &other_ms_idx in slot {
                    if other_ms_idx < all_sessions.len() && ms_idx < all_sessions.len() {
                        if has_classification_conflict(&all_sessions[ms_idx], &all_sessions[other_ms_idx]) {
                            other_conflicts += 1;
                        }
                        if has_personal_classification_conflict(&all_sessions[ms_idx], &all_sessions[other_ms_idx]) {
                            other_conflicts += 1;
                        }
                    }
                }

                if other_conflicts < min_conflicts {
                    min_conflicts = other_conflicts;
                    best_slot = Some(slot_idx);
                }
            }
        }

        best_slot
    }

    /// Move a session from one slot to another
    fn move_session(&self, schedule: &mut Schedule, from_slot: usize, from_room: usize, to_slot: usize, config: &SchedulerConfig) -> bool {
        if from_slot < schedule.len() && to_slot < schedule.len() && from_room < schedule[from_slot].len() {
            // Check if target slot has room capacity
            if schedule[to_slot].len() >= config.num_rooms {
                return false; // Cannot move - target slot is full
            }

            let ms_idx = schedule[from_slot].remove(from_room);
            schedule[to_slot].push(ms_idx);
            return true;
        }
        false
    }
}

impl ParameterOptimizer for SpeakerOrganizerOptimizer {
    fn optimize_parameter(&self, solutions: &[Solution], all_sessions: &[Minisymposium],
                         config: &SchedulerConfig, protected_types: &[ConflictType]) -> Vec<Solution> {
        let mut optimized_solutions = Vec::new();

        for solution in solutions {
            let mut best_schedule = solution.schedule.clone();
            let mut best_conflicts = self.calculate_conflicts(&best_schedule, all_sessions);

            println!("  Phase 1: Targeted conflict resolution (initial conflicts: {})", best_conflicts);
            let phase1_start = Instant::now();

            // Phase 1: Targeted conflict resolution - runs ALL iterations
            let max_targeted_iterations = 100; // Reduced from 300 for faster performance
            let mut zero_achieved_at = None;

            for iteration in 0..max_targeted_iterations {
                if best_conflicts == 0 && zero_achieved_at.is_none() {
                    println!("  ✓ Zero conflicts achieved at iteration {}!", iteration);
                    zero_achieved_at = Some(iteration);
                    // Continue iterating to see if we maintain zero
                }

                // GREEDY SWAP ALGORITHM: Find all conflicting sessions and try pairwise swaps
                let mut improved_this_iteration = false;
                let conflicting_sessions = self.find_all_conflicting_sessions(&best_schedule, all_sessions);

                if conflicting_sessions.is_empty() {
                    break; // No conflicts left, we're done!
                }

                // Try greedy swaps: for each conflicting session, try swapping with ALL other sessions
                for &(slot_a, room_a) in &conflicting_sessions {
                    if improved_this_iteration {
                        break;
                    }

                    let ms_a = best_schedule[slot_a][room_a];

                    // Try swapping with every session in different time slots
                    for slot_b in 0..best_schedule.len() {
                        if slot_b == slot_a || improved_this_iteration {
                            continue; // Skip same slot
                        }

                        for room_b in 0..best_schedule[slot_b].len() {
                            let ms_b = best_schedule[slot_b][room_b];

                            // Perform swap
                            let mut test_schedule = best_schedule.clone();
                            test_schedule[slot_a][room_a] = ms_b;
                            test_schedule[slot_b][room_b] = ms_a;

                            // Check protected constraints (OPTIMIZED: only calculate needed conflict types)
                            let test_all_conflicts = calculate_selective_conflicts(
                                &test_schedule, all_sessions,
                                ConflictType::SpeakerOrganizer, protected_types
                            );
                            if violates_protected_constraints(&test_all_conflicts, &solution.conflicts,
                                                              protected_types, config.strict_hierarchy) {
                                continue;
                            }

                            // Check if this swap reduces speaker/organizer conflicts
                            let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);
                            if test_conflicts < best_conflicts {
                                best_schedule = test_schedule;
                                best_conflicts = test_conflicts;
                                improved_this_iteration = true;

                                if iteration % 10 == 0 || best_conflicts <= 10 {
                                    println!("    Iteration {}: {} → {} conflicts (greedy swap)", iteration, best_conflicts + 1, best_conflicts);
                                }
                                break; // Greedy: accept first improvement
                            }
                        }

                        if improved_this_iteration {
                            break;
                        }
                    }
                }

                // Continue through all iterations even without improvement
            }

            let phase1_time = phase1_start.elapsed();
            println!("  Phase 1 complete: {} iterations executed, final conflicts: {}, time: {:.2}s",
                     max_targeted_iterations, best_conflicts, phase1_time.as_secs_f64());

            // Phase 2: Random exploration - always runs to find better solutions
            println!("  Phase 2: Random exploration (starting conflicts: {})", best_conflicts);
            let phase2_start = Instant::now();

            let max_random_iterations = 50; // Reduced from 200 for faster performance
            let mut improvements_found = 0;

            for iteration in 0..max_random_iterations {
                let mut test_schedule = best_schedule.clone();

                if let Some((s1, s2)) = pick_random_swap(&test_schedule) {
                    apply_swap(&mut test_schedule, s1, s2);

                    // Calculate conflicts (OPTIMIZED: only needed types)
                    let test_all_conflicts = calculate_selective_conflicts(
                        &test_schedule, all_sessions,
                        ConflictType::SpeakerOrganizer, protected_types
                    );

                    // Check if this swap violates protected constraints
                    if violates_protected_constraints(&test_all_conflicts, &solution.conflicts,
                                                      protected_types, config.strict_hierarchy) {
                        continue; // Reject swap silently
                    }

                    let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);

                    if test_conflicts < best_conflicts {
                        best_schedule = test_schedule;
                        best_conflicts = test_conflicts;
                        improvements_found += 1;

                        if best_conflicts <= 5 || improvements_found % 5 == 0 {
                            println!("    Random iter {}: improved to {} conflicts", iteration, best_conflicts);
                        }

                        if best_conflicts == 0 && zero_achieved_at.is_none() {
                            println!("  ✓ Zero conflicts achieved at iteration {}!", iteration);
                            zero_achieved_at = Some(iteration);
                            break; // No need to continue if perfect
                        }
                    }
                }
            }

            let phase2_time = phase2_start.elapsed();
            println!("  Phase 2 complete: {} improvements found, final conflicts: {}, time: {:.2}s",
                     improvements_found, best_conflicts, phase2_time.as_secs_f64());

            let mut new_conflicts = solution.conflicts.clone();
            new_conflicts.speaker_organizer = best_conflicts;
            optimized_solutions.push(Solution::new(best_schedule, new_conflicts));
        }

        optimized_solutions
    }

    fn conflict_type(&self) -> ConflictType {
        ConflictType::SpeakerOrganizer
    }

    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32 {
        let mut conflicts = 0;

        for session_slot in schedule {
            for i in 0..session_slot.len() {
                for j in (i + 1)..session_slot.len() {
                    let ms1_idx = session_slot[i];
                    let ms2_idx = session_slot[j];

                    if ms1_idx < all_sessions.len() && ms2_idx < all_sessions.len() {
                        if has_speaker_organizer_conflict(&all_sessions[ms1_idx], &all_sessions[ms2_idx]) {
                            conflicts += 1;
                        }
                    }
                }
            }
        }

        conflicts
    }

    fn algorithm_name(&self) -> &'static str {
        "Bipartite Matching + Swap Optimization"
    }
}

// Helper function to find all personal-classification conflicts in a schedule
fn find_personal_classification_conflicts(schedule: &Schedule, all_sessions: &[Minisymposium]) -> Vec<(usize, usize)> {
    let mut conflicts = Vec::new();

    for session_slot in schedule {
        for i in 0..session_slot.len() {
            for j in (i + 1)..session_slot.len() {
                let ms1_idx = session_slot[i];
                let ms2_idx = session_slot[j];

                if ms1_idx < all_sessions.len() && ms2_idx < all_sessions.len() {
                    if has_personal_classification_conflict(&all_sessions[ms1_idx], &all_sessions[ms2_idx]) {
                        conflicts.push((ms1_idx, ms2_idx));
                    }
                }
            }
        }
    }

    conflicts
}

// Helper function to find the position (session_idx, room_idx) of a minisymposium in the schedule
fn find_session_position(schedule: &Schedule, ms_idx: usize) -> Option<(usize, usize)> {
    for (session_idx, session_slot) in schedule.iter().enumerate() {
        for (room_idx, &slot_ms_idx) in session_slot.iter().enumerate() {
            if slot_ms_idx == ms_idx {
                return Some((session_idx, room_idx));
            }
        }
    }
    None
}

/// Find all sessions involved in personal-classification conflicts
/// Returns list of (slot_idx, room_idx) for sessions that have conflicts
fn find_all_personal_classification_conflicting_sessions(schedule: &Schedule, all_sessions: &[Minisymposium]) -> Vec<(usize, usize)> {
    let mut conflicting = Vec::new();

    for (slot_idx, slot) in schedule.iter().enumerate() {
        for (room_idx, &ms_idx) in slot.iter().enumerate() {
            // Check if this session conflicts with any other in same slot
            for (other_room_idx, &other_ms_idx) in slot.iter().enumerate() {
                if room_idx != other_room_idx &&
                   ms_idx < all_sessions.len() &&
                   other_ms_idx < all_sessions.len() {
                    if has_personal_classification_conflict(&all_sessions[ms_idx], &all_sessions[other_ms_idx]) {
                        conflicting.push((slot_idx, room_idx));
                        break;
                    }
                }
            }
        }
    }

    conflicting
}

/// Find time slots where a session would have NO personal-classification conflicts
/// Returns vector of slot indices where the session can be placed safely
fn find_safe_slots_for_session(ms_idx: usize, schedule: &Schedule, all_sessions: &[Minisymposium]) -> Vec<usize> {
    let mut safe_slots = Vec::new();

    if ms_idx >= all_sessions.len() {
        return safe_slots;
    }

    let session = &all_sessions[ms_idx];

    for (slot_idx, slot) in schedule.iter().enumerate() {
        let mut has_conflict = false;

        // Check if this session would conflict with any session in this slot
        for &other_ms_idx in slot {
            if other_ms_idx < all_sessions.len() {
                if has_personal_classification_conflict(session, &all_sessions[other_ms_idx]) {
                    has_conflict = true;
                    break;
                }
            }
        }

        if !has_conflict {
            safe_slots.push(slot_idx);
        }
    }

    safe_slots
}

/// Pick a random 3-way swap: A→B, B→C, C→A (cyclic permutation)
/// Returns Some((pos_a, pos_b, pos_c)) where each pos is (slot_idx, room_idx)
fn pick_random_3way_swap(schedule: &Schedule) -> Option<((usize, usize), (usize, usize), (usize, usize))> {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    // Collect all session positions
    let mut positions = Vec::new();
    for (slot_idx, slot) in schedule.iter().enumerate() {
        for (room_idx, _) in slot.iter().enumerate() {
            positions.push((slot_idx, room_idx));
        }
    }

    if positions.len() < 3 {
        return None;
    }

    // Pick 3 random distinct positions
    let idx_a = rng.gen_range(0..positions.len());
    let mut idx_b = rng.gen_range(0..positions.len());
    while idx_b == idx_a {
        idx_b = rng.gen_range(0..positions.len());
    }
    let mut idx_c = rng.gen_range(0..positions.len());
    while idx_c == idx_a || idx_c == idx_b {
        idx_c = rng.gen_range(0..positions.len());
    }

    Some((positions[idx_a], positions[idx_b], positions[idx_c]))
}

/// Apply a 3-way swap: A→B, B→C, C→A (cyclic permutation)
fn apply_3way_swap(schedule: &mut Schedule, pos_a: (usize, usize), pos_b: (usize, usize), pos_c: (usize, usize)) {
    let temp_a = schedule[pos_a.0][pos_a.1];
    let temp_b = schedule[pos_b.0][pos_b.1];
    let temp_c = schedule[pos_c.0][pos_c.1];

    schedule[pos_a.0][pos_a.1] = temp_c; // A ← C
    schedule[pos_b.0][pos_b.1] = temp_a; // B ← A
    schedule[pos_c.0][pos_c.1] = temp_b; // C ← B
}

// Personal-Classification Optimizer
struct PersonalClassificationOptimizer;

impl ParameterOptimizer for PersonalClassificationOptimizer {
    fn optimize_parameter(&self, solutions: &[Solution], all_sessions: &[Minisymposium],
                         config: &SchedulerConfig, protected_types: &[ConflictType]) -> Vec<Solution> {
        let mut optimized_solutions = Vec::new();

        for solution in solutions {
            let current_conflicts = self.calculate_conflicts(&solution.schedule, all_sessions);
            let mut best_schedule = solution.schedule.clone();
            let mut best_conflicts = current_conflicts;

            println!("\n  Optimizing Personal-Classification conflicts (initial: {})", current_conflicts);
            println!("  Goal: ZERO conflicts (rank 2 priority)");

            let mut zero_achieved_at: Option<usize> = None;

            // Phase 1: Smart swap selection with targeted conflict resolution
            println!("  Phase 1: Smart swap selection (initial conflicts: {})", current_conflicts);
            let phase1_start = Instant::now();
            let max_targeted_iterations = 30; // Reduced iterations with smarter swap selection

            for iteration in 0..max_targeted_iterations {
                if best_conflicts == 0 {
                    if zero_achieved_at.is_none() {
                        println!("  ✓ Zero conflicts achieved in Phase 1 at iteration {}!", iteration);
                        zero_achieved_at = Some(iteration);
                    }
                    break;
                }

                // SMART SWAP ALGORITHM: Find conflicting sessions and only try swaps in safe slots
                let mut improved_this_iteration = false;
                let conflicting_sessions = find_all_personal_classification_conflicting_sessions(&best_schedule, all_sessions);

                if conflicting_sessions.is_empty() {
                    break; // No conflicts left, we're done!
                }

                // Try smart swaps: for each conflicting session, find safe slots and try swapping
                for &(slot_a, room_a) in &conflicting_sessions {
                    if improved_this_iteration {
                        break;
                    }

                    let ms_a = best_schedule[slot_a][room_a];

                    // Find safe slots where this session would have no personal-classification conflicts
                    let safe_slots = find_safe_slots_for_session(ms_a, &best_schedule, all_sessions);

                    // Only try swapping with sessions in safe slots (not all 8 slots)
                    for &slot_b in &safe_slots {
                        if slot_b == slot_a || improved_this_iteration {
                            continue; // Skip same slot
                        }

                        for room_b in 0..best_schedule[slot_b].len() {
                            let ms_b = best_schedule[slot_b][room_b];

                            // Perform swap
                            let mut test_schedule = best_schedule.clone();
                            test_schedule[slot_a][room_a] = ms_b;
                            test_schedule[slot_b][room_b] = ms_a;

                            // Check protected constraints (OPTIMIZED: only calculate needed conflict types)
                            let test_all_conflicts = calculate_selective_conflicts(
                                &test_schedule, all_sessions,
                                ConflictType::PersonalClassification, protected_types
                            );
                            if violates_protected_constraints(&test_all_conflicts, &solution.conflicts,
                                                              protected_types, config.strict_hierarchy) {
                                continue;
                            }

                            // Check if this swap reduces personal-classification conflicts
                            let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);
                            if test_conflicts < best_conflicts {
                                best_schedule = test_schedule;
                                best_conflicts = test_conflicts;
                                improved_this_iteration = true;

                                if iteration % 10 == 0 || best_conflicts <= 10 {
                                    println!("    Iteration {}: {} → {} conflicts (greedy swap)", iteration, best_conflicts + 1, best_conflicts);
                                }
                                break; // Greedy: accept first improvement
                            }
                        }

                        if improved_this_iteration {
                            break;
                        }
                    }
                }

                // Continue through all iterations even without improvement
            }

            let phase1_time = phase1_start.elapsed();
            println!("  Smart swap complete: {} iterations executed, final conflicts: {}, time: {:.2}s",
                     max_targeted_iterations, best_conflicts, phase1_time.as_secs_f64());

            let mut new_conflicts = solution.conflicts.clone();
            new_conflicts.personal_classification = best_conflicts;
            optimized_solutions.push(Solution::new(best_schedule, new_conflicts));
        }

        optimized_solutions
    }

    fn conflict_type(&self) -> ConflictType {
        ConflictType::PersonalClassification
    }

    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32 {
        let mut conflicts = 0;

        for session_slot in schedule {
            for i in 0..session_slot.len() {
                for j in (i + 1)..session_slot.len() {
                    let ms1_idx = session_slot[i];
                    let ms2_idx = session_slot[j];

                    if ms1_idx < all_sessions.len() && ms2_idx < all_sessions.len() {
                        if has_personal_classification_conflict(&all_sessions[ms1_idx], &all_sessions[ms2_idx]) {
                            conflicts += 1;
                        }
                    }
                }
            }
        }

        conflicts
    }

    fn algorithm_name(&self) -> &'static str {
        "Constraint Propagation"
    }
}

// Series Same-Room Optimizer
struct SeriesRoomOptimizer;

impl ParameterOptimizer for SeriesRoomOptimizer {
    fn optimize_parameter(&self, solutions: &[Solution], all_sessions: &[Minisymposium],
                         config: &SchedulerConfig, protected_types: &[ConflictType]) -> Vec<Solution> {
        let mut optimized_solutions = Vec::new();

        for solution in solutions {
            let current_conflicts = self.calculate_conflicts(&solution.schedule, all_sessions);
            let mut best_schedule = solution.schedule.clone();
            let mut best_conflicts = current_conflicts;

            // Active optimization: consolidate series parts into same rooms
            // OPTIMIZED: Increased from 10 to 30 iterations for better quality
            let max_iterations = 30;
            for _ in 0..max_iterations {
                // Early termination if no conflicts remain
                if best_conflicts == 0 {
                    break;
                }

                let mut test_schedule = best_schedule.clone();

                // Find a series that spans multiple rooms
                if let Some(swap) = self.find_series_room_swap(&test_schedule, all_sessions) {
                    apply_swap(&mut test_schedule, swap.0, swap.1);

                    // HARD CONSTRAINT: Check for chronological order violations
                    // Series parts must ALWAYS appear in correct chronological order (Part I before Part II, etc.)
                    use conflict_detection::has_series_chronological_violation;

                    let period_names = [
                        "TUE First Period", "TUE Second Period",
                        "WED First Period", "WED Second Period",
                        "THU First Period", "THU Second Period",
                        "FRI First Period", "FRI Second Period",
                    ];

                    let mut has_chronological_violation = false;
                    for (time_idx1, time_slot1) in test_schedule.iter().enumerate() {
                        for (room_idx1, &ms_idx1) in time_slot1.iter().enumerate() {
                            if ms_idx1 >= all_sessions.len() {
                                continue;
                            }
                            let title1 = &all_sessions[ms_idx1].title;
                            let period1 = period_names.get(time_idx1).unwrap_or(&"Unknown");

                            for (time_idx2, time_slot2) in test_schedule.iter().enumerate() {
                                for (room_idx2, &ms_idx2) in time_slot2.iter().enumerate() {
                                    if ms_idx2 >= all_sessions.len() || ms_idx1 == ms_idx2 {
                                        continue;
                                    }
                                    let title2 = &all_sessions[ms_idx2].title;
                                    let period2 = period_names.get(time_idx2).unwrap_or(&"Unknown");

                                    if has_series_chronological_violation(title1, period1, title2, period2) {
                                        has_chronological_violation = true;
                                        break;
                                    }
                                }
                                if has_chronological_violation {
                                    break;
                                }
                            }
                            if has_chronological_violation {
                                break;
                            }
                        }
                        if has_chronological_violation {
                            break;
                        }
                    }

                    if has_chronological_violation {
                        continue; // REJECT: This swap violates chronological order!
                    }

                    // Calculate conflicts (OPTIMIZED: only needed types)
                    let test_all_conflicts = calculate_selective_conflicts(
                        &test_schedule, all_sessions,
                        ConflictType::SeriesSameRoom, protected_types
                    );

                    // Check if this swap violates protected constraints
                    if violates_protected_constraints(&test_all_conflicts, &solution.conflicts,
                                                      protected_types, config.strict_hierarchy) {
                        continue; // Reject swap silently
                    }

                    let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);

                    if test_conflicts < best_conflicts {
                        best_schedule = test_schedule;
                        best_conflicts = test_conflicts;
                    }
                }
            }

            let mut new_conflicts = solution.conflicts.clone();
            new_conflicts.series_same_room = best_conflicts;
            optimized_solutions.push(Solution::new(best_schedule, new_conflicts));
        }

        optimized_solutions
    }

    fn conflict_type(&self) -> ConflictType {
        ConflictType::SeriesSameRoom
    }

    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32 {
        let mut series_rooms: HashMap<String, HashSet<usize>> = HashMap::new();

        for (_session_idx, session_slot) in schedule.iter().enumerate() {
            for (room_idx, &ms_idx) in session_slot.iter().enumerate() {
                if ms_idx < all_sessions.len() {
                    if let Some(series_key) = &all_sessions[ms_idx].series_key {
                        series_rooms.entry(series_key.clone())
                            .or_insert_with(HashSet::new)
                            .insert(room_idx);
                    }
                }
            }
        }

        // Conflict if a series appears in multiple rooms
        series_rooms.values()
            .filter(|rooms| rooms.len() > 1)
            .count() as i32
    }

    fn algorithm_name(&self) -> &'static str {
        "Room Consolidation with Targeted Swaps"
    }
}

impl SeriesRoomOptimizer {
    /// ADJACENT-ROW ROOM CONSOLIDATION STRATEGY:
    /// For each series with room conflicts, try all possible target rooms
    /// Swap with sessions in the same room column of adjacent rows (above/below)
    /// Choose the consolidation option that minimizes conflicts
    fn find_series_room_swap(&self, schedule: &Schedule, all_sessions: &[Minisymposium])
        -> Option<((usize, usize), (usize, usize))> {

        // Build map of series to their locations
        let mut series_locations: HashMap<String, Vec<(usize, usize, usize)>> = HashMap::new();

        for (time_idx, time_slot) in schedule.iter().enumerate() {
            for (room_idx, &ms_idx) in time_slot.iter().enumerate() {
                if ms_idx < all_sessions.len() {
                    if let Some(series_key) = &all_sessions[ms_idx].series_key {
                        series_locations.entry(series_key.clone())
                            .or_insert_with(Vec::new)
                            .push((time_idx, room_idx, ms_idx));
                    }
                }
            }
        }

        // Find series that span multiple rooms
        let multi_room_series: Vec<_> = series_locations.iter()
            .filter(|(_, locations)| {
                let rooms: HashSet<_> = locations.iter().map(|(_, r, _)| r).collect();
                rooms.len() > 1
            })
            .collect();

        if multi_room_series.is_empty() {
            return None;
        }

        // Try each problematic series (deterministic order)
        for (_series_key, locations) in multi_room_series {
            // Get all rooms this series occupies
            let occupied_rooms: HashSet<usize> = locations.iter().map(|(_, r, _)| *r).collect();

            // Try consolidating to each possible room
            for &target_room in &occupied_rooms {
                // Find parts NOT in the target room
                let wrong_room_parts: Vec<_> = locations.iter()
                    .filter(|(_, room, _)| *room != target_room)
                    .collect();

                if wrong_room_parts.is_empty() {
                    continue;
                }

                // For each part in wrong room, try swapping with adjacent rows in target room
                for &(time_idx, room_idx, _) in &wrong_room_parts {
                    // Try row above (time_idx - 1)
                    if *time_idx > 0 {
                        let adjacent_time = *time_idx - 1;
                        if target_room < schedule[adjacent_time].len() {
                            // Swap with session in target_room at adjacent row
                            return Some(((*time_idx, *room_idx), (adjacent_time, target_room)));
                        }
                    }

                    // Try row below (time_idx + 1)
                    if *time_idx + 1 < schedule.len() {
                        let adjacent_time = *time_idx + 1;
                        if target_room < schedule[adjacent_time].len() {
                            // Swap with session in target_room at adjacent row
                            return Some(((*time_idx, *room_idx), (adjacent_time, target_room)));
                        }
                    }
                }
            }
        }

        None
    }
}

// Series Back-to-Back Optimizer (combines parallel and timing checks)
struct SeriesBackOptimizer;

impl ParameterOptimizer for SeriesBackOptimizer {
    fn optimize_parameter(&self, solutions: &[Solution], all_sessions: &[Minisymposium],
                         config: &SchedulerConfig, protected_types: &[ConflictType]) -> Vec<Solution> {
        let mut optimized_solutions = Vec::new();

        for solution in solutions {
            let current_conflicts = self.calculate_conflicts(&solution.schedule, all_sessions);
            let mut best_schedule = solution.schedule.clone();
            let mut best_conflicts = current_conflicts;

            // SERIES PLACEMENT: Move WHOLE series to consecutive time slots
            // Much simpler and faster than pairwise swaps!
            // OPTIMIZED: Increased from 10 to 50 iterations for better quality
            let max_iterations = 50;
            for iteration in 0..max_iterations {
                if best_conflicts == 0 {
                    break;
                }

                // Find series with conflicts and try to place them consecutively
                let improved = self.try_series_placement(&mut best_schedule, all_sessions, &solution.conflicts,
                                                         protected_types, config.strict_hierarchy);

                if !improved {
                    break; // No more improvements possible
                }

                best_conflicts = self.calculate_conflicts(&best_schedule, all_sessions);
                println!("    Iteration {}: Placed series → {} conflicts remaining", iteration, best_conflicts);
            }

            let mut new_conflicts = solution.conflicts.clone();
            new_conflicts.series_back_to_back = best_conflicts;
            optimized_solutions.push(Solution::new(best_schedule, new_conflicts));
        }

        optimized_solutions
    }

    fn conflict_type(&self) -> ConflictType {
        ConflictType::SeriesBackToBack
    }

    fn calculate_conflicts(&self, schedule: &Schedule, all_sessions: &[Minisymposium]) -> i32 {
        let mut conflicts = 0;
        let period_names = [
            "TUE First Period", "TUE Second Period",
            "WED First Period", "WED Second Period",
            "THU First Period", "THU Second Period",
            "FRI First Period", "FRI Second Period",
        ];

        // Group sessions by series
        let mut series_positions: HashMap<String, Vec<(usize, &Minisymposium)>> = HashMap::new();

        for (session_idx, session_slot) in schedule.iter().enumerate() {
            for &ms_idx in session_slot {
                if ms_idx < all_sessions.len() {
                    let ms = &all_sessions[ms_idx];
                    if let Some(series_key) = &ms.series_key {
                        series_positions.entry(series_key.clone())
                            .or_insert_with(Vec::new)
                            .push((session_idx, ms));
                    }
                }
            }
        }

        // Check each series for back-to-back conflicts (parallel + non-consecutive)
        for (_, positions) in series_positions {
            if positions.len() < 2 {
                continue;
            }

            // Check all pairs
            for i in 0..positions.len() {
                for j in (i + 1)..positions.len() {
                    let (idx1, ms1) = positions[i];
                    let (idx2, ms2) = positions[j];

                    let period1 = period_names.get(idx1).unwrap_or(&"Unknown");
                    let period2 = period_names.get(idx2).unwrap_or(&"Unknown");

                    // Use shared conflict detection logic
                    if conflict_detection::has_series_back_to_back_conflict(
                        &ms1.title, period1,
                        &ms2.title, period2
                    ) {
                        conflicts += 1;
                    }
                }
            }
        }

        conflicts
    }

    fn algorithm_name(&self) -> &'static str {
        "Temporal Adjacency with Targeted Swaps (Parallel + Consecutive)"
    }
}

impl SeriesBackOptimizer {
    /// Try to swap a problematic series with another block of consecutive sessions
    /// Returns true if any improvement was made
    fn try_series_placement(&self, schedule: &mut Schedule, all_sessions: &[Minisymposium],
                            original_conflicts: &ConflictCounts, protected_types: &[ConflictType],
                            strict_hierarchy: bool) -> bool {
        use conflict_detection::extract_series_info;

        // Build map of series to their current locations
        let mut series_map: HashMap<String, Vec<(usize, usize, usize, usize)>> = HashMap::new();

        for (time_idx, time_slot) in schedule.iter().enumerate() {
            for (room_idx, &ms_idx) in time_slot.iter().enumerate() {
                if ms_idx < all_sessions.len() {
                    if let Some((base_title, part_num)) = extract_series_info(&all_sessions[ms_idx].title) {
                        series_map.entry(base_title)
                            .or_insert_with(Vec::new)
                            .push((time_idx, room_idx, part_num, ms_idx));
                    }
                }
            }
        }

        // Find FIRST series with conflicts and try to fix it
        for (series_name, mut parts) in series_map {
            if parts.len() < 2 {
                continue;
            }

            parts.sort_by_key(|(_, _, part_num, _)| *part_num);

            // Check if this series has conflicts
            let current_conflicts = self.calculate_conflicts(schedule, all_sessions);
            if current_conflicts == 0 {
                return false; // All done!
            }

            let mut has_conflict = false;
            for i in 0..parts.len() - 1 {
                let (time1, _, part1, _) = parts[i];
                let (time2, _, part2, _) = parts[i + 1];

                if part2 == part1 + 1 {
                    let time_diff = (time1 as i32 - time2 as i32).abs();
                    if time_diff != 1 || time1 == time2 {
                        has_conflict = true;
                        break;
                    }
                }
            }

            if !has_conflict {
                continue; // This series is fine
            }

            println!("    Trying to fix series '{}' with {} parts",
                     series_name.chars().take(40).collect::<String>(), parts.len());

            let num_parts = parts.len();
            let series_sessions: Vec<usize> = parts.iter().map(|(_, _, _, ms_idx)| *ms_idx).collect();

            // Try to swap this WHOLE series with consecutive blocks elsewhere
            // PERFORMANCE: Limit to first 8 rooms only (most schedules have ~16 rooms)
            // This cuts search space in half while still finding good solutions
            for start_time in 0..schedule.len().saturating_sub(num_parts - 1) {
                let max_rooms = schedule[start_time].len().min(8);
                for start_room in 0..max_rooms {
                    // Collect the target block of sessions we'd swap with
                    let mut target_block = Vec::new();
                    let mut valid_block = true;

                    for offset in 0..num_parts {
                        let target_time = start_time + offset;
                        if target_time >= schedule.len() {
                            valid_block = false;
                            break;
                        }
                        if start_room >= schedule[target_time].len() {
                            valid_block = false;
                            break;
                        }
                        target_block.push((target_time, start_room, schedule[target_time][start_room]));
                    }

                    if !valid_block {
                        continue;
                    }

                    // Don't swap with itself
                    let target_sessions: Vec<usize> = target_block.iter().map(|(_, _, s)| *s).collect();
                    if target_sessions == series_sessions {
                        continue;
                    }

                    // Try the swap: put series in target block, put target block in series positions
                    let mut test_schedule = schedule.clone();

                    // Place series in target positions (consecutive slots)
                    for (i, &session_idx) in series_sessions.iter().enumerate() {
                        test_schedule[start_time + i][start_room] = session_idx;
                    }

                    // Place target sessions in series' old positions
                    for (i, &(old_time, old_room, _, _)) in parts.iter().enumerate() {
                        if i < target_sessions.len() {
                            test_schedule[old_time][old_room] = target_sessions[i];
                        }
                    }

                    // Check if this improves series conflicts
                    let test_conflicts = self.calculate_conflicts(&test_schedule, all_sessions);
                    if test_conflicts >= current_conflicts {
                        continue; // No improvement
                    }

                    // Check protected constraints (OPTIMIZED: only calculate needed conflict types)
                    let test_all_conflicts = calculate_selective_conflicts(
                        &test_schedule, all_sessions,
                        ConflictType::SeriesBackToBack, protected_types
                    );
                    if violates_protected_constraints(&test_all_conflicts, original_conflicts,
                                                      protected_types, strict_hierarchy) {
                        continue;
                    }

                    // Accept this improvement!
                    *schedule = test_schedule;
                    println!("    ✓ Swapped series → {} conflicts remaining", test_conflicts);
                    return true;
                }
            }
        }

        false // No improvement found
    }
}

// TBD optimizers removed - not part of the 5 core conflict types

// ============================================================================
// HIERARCHICAL ENGINE
// ============================================================================

pub struct HierarchicalEngine {
    config: SchedulerConfig,
}

impl HierarchicalEngine {
    pub fn new(config: SchedulerConfig) -> Self {
        Self { config }
    }

    pub fn optimize(&self, initial_solutions: Vec<Solution>, all_sessions: &[Minisymposium]) -> Vec<Solution> {
        println!("\n=== HIERARCHICAL OPTIMIZATION ===");
        println!("Initial solutions: {}", initial_solutions.len());
        if self.config.strict_hierarchy {
            println!("Strict hierarchy mode: ENABLED (higher-priority conflicts will be preserved)");
        } else {
            println!("Strict hierarchy mode: DISABLED (allowing small violations for better overall performance)");
        }

        let parameters = self.get_sorted_parameters();
        let mut current_solutions = initial_solutions;
        let mut protected_types: Vec<ConflictType> = Vec::new();

        for (param_optimizer, rank) in parameters {
            if rank == 0 {
                println!("\nSkipping {} (rank 0 - disabled)", param_optimizer.conflict_type().name());
                continue;
            }

            let conflict_type = param_optimizer.conflict_type();
            let start = Instant::now();

            let conflicts_before = if !current_solutions.is_empty() {
                param_optimizer.calculate_conflicts(&current_solutions[0].schedule, all_sessions)
            } else {
                0
            };

            println!("\n--- Optimizing {} (Rank {}) ---", conflict_type.name(), rank);
            println!("  Algorithm: {}", param_optimizer.algorithm_name());
            println!("  Conflicts before: {}", conflicts_before);
            println!("  Input solutions: {}", current_solutions.len());
            if !protected_types.is_empty() {
                println!("  Protected constraints: {:?}", protected_types.iter().map(|t| t.name()).collect::<Vec<_>>());
            }

            current_solutions = param_optimizer.optimize_parameter(&current_solutions, all_sessions, &self.config, &protected_types);
            current_solutions = self.prune_solutions(current_solutions, conflict_type);

            let conflicts_after = if !current_solutions.is_empty() {
                param_optimizer.calculate_conflicts(&current_solutions[0].schedule, all_sessions)
            } else {
                0
            };

            // Validate that protected constraints weren't violated
            if self.config.strict_hierarchy && !current_solutions.is_empty() && !protected_types.is_empty() {
                let all_conflicts_after = calculate_all_conflicts(&current_solutions[0].schedule, all_sessions);
                for &protected_type in &protected_types {
                    let protected_count = all_conflicts_after.get(protected_type);
                    if protected_count > 0 {
                        println!("  ⚠ WARNING: Protected constraint {} increased to {} conflicts!",
                                 protected_type.name(), protected_count);
                    }
                }
            }

            let elapsed = start.elapsed();
            println!("  Conflicts after: {}", conflicts_after);
            println!("  Improvement: {}", conflicts_before - conflicts_after);
            println!("  Output solutions: {}", current_solutions.len());
            println!("  Time: {:?}", elapsed);

            // Add this parameter to the protected list for subsequent optimizations
            protected_types.push(conflict_type);
        }

        current_solutions
    }

    fn get_sorted_parameters(&self) -> Vec<(Box<dyn ParameterOptimizer>, u8)> {
        let mut params: Vec<(Box<dyn ParameterOptimizer>, u8)> = vec![
            (Box::new(ClassificationOptimizer) as Box<dyn ParameterOptimizer>, self.config.classification_rank),
            (Box::new(SpeakerOrganizerOptimizer) as Box<dyn ParameterOptimizer>, self.config.speaker_org_rank),
            (Box::new(PersonalClassificationOptimizer) as Box<dyn ParameterOptimizer>, self.config.personal_class_rank),
            (Box::new(SeriesRoomOptimizer) as Box<dyn ParameterOptimizer>, self.config.series_room_rank),
            (Box::new(SeriesBackOptimizer) as Box<dyn ParameterOptimizer>, self.config.series_back_rank),
        ];

        params.sort_by_key(|(_, rank)| *rank);
        params
    }

    fn prune_solutions(&self, mut solutions: Vec<Solution>, current_param: ConflictType) -> Vec<Solution> {
        if solutions.len() <= self.config.max_solutions_per_level {
            return solutions;
        }

        solutions.sort_by_key(|s| s.conflicts.get(current_param));
        solutions.truncate(self.config.max_solutions_per_level);
        solutions
    }
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

fn pick_random_swap(schedule: &Schedule) -> Option<((usize, usize), (usize, usize))> {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    if schedule.len() < 2 {
        return None;
    }

    let session1 = rng.gen_range(0..schedule.len());
    let mut session2 = rng.gen_range(0..schedule.len());

    while session2 == session1 {
        session2 = rng.gen_range(0..schedule.len());
    }

    if schedule[session1].is_empty() || schedule[session2].is_empty() {
        return None;
    }

    let room1 = rng.gen_range(0..schedule[session1].len());
    let room2 = rng.gen_range(0..schedule[session2].len());

    Some(((session1, room1), (session2, room2)))
}

fn apply_swap(schedule: &mut Schedule, pos1: (usize, usize), pos2: (usize, usize)) {
    let (s1, r1) = pos1;
    let (s2, r2) = pos2;

    if s1 < schedule.len() && s2 < schedule.len() &&
       r1 < schedule[s1].len() && r2 < schedule[s2].len() {
        let temp = schedule[s1][r1];
        schedule[s1][r1] = schedule[s2][r2];
        schedule[s2][r2] = temp;
    }
}

/// Fix series ordering: ensure Part I comes before Part II, Part II before Part III, etc.
fn fix_series_ordering(schedule: &mut Schedule, all_sessions: &[Minisymposium]) {
    use conflict_detection::extract_series_info;

    println!("\n=== SERIES ORDERING CLEANUP ===");

    // Build a map of series base titles to their parts and positions
    let mut series_parts: std::collections::HashMap<String, Vec<(usize, usize, usize, usize)>> = std::collections::HashMap::new();

    for (time_idx, time_slot) in schedule.iter().enumerate() {
        for (room_idx, &ms_idx) in time_slot.iter().enumerate() {
            if ms_idx < all_sessions.len() {
                if let Some((base_title, part_num)) = extract_series_info(&all_sessions[ms_idx].title) {
                    series_parts.entry(base_title)
                        .or_insert_with(Vec::new)
                        .push((time_idx, room_idx, part_num, ms_idx));
                }
            }
        }
    }

    let mut swaps_made = 0;

    // For each series, check if parts are in chronological order
    for (series_name, mut parts) in series_parts {
        if parts.len() < 2 {
            continue; // Single-part series, no ordering needed
        }

        // Sort by part number (what it should be)
        parts.sort_by_key(|(_, _, part_num, _)| *part_num);

        // Check if they're in chronological order by time slot index (0-7)
        let mut needs_fix = false;
        for i in 0..parts.len() - 1 {
            let (time1, _, _, _) = parts[i];
            let (time2, _, _, _) = parts[i + 1];

            // Simply check if later part comes before earlier part in schedule
            if time1 > time2 {
                needs_fix = true;
                break;
            }
        }

        if needs_fix {
            println!("  Fixing series: {}", series_name);

            // Simple pairwise swap approach: if Part i+1 comes before Part i, swap them
            for i in 0..parts.len() - 1 {
                let (time1, room1, part1, _) = parts[i];
                let (time2, room2, part2, _) = parts[i + 1];

                // If later part comes before earlier part, swap
                if time1 > time2 {
                    println!("    Swapping Part {} (Slot {}, Room {}) with Part {} (Slot {}, Room {})",
                             part1, time1 + 1, room1 + 1, part2, time2 + 1, room2 + 1);

                    apply_swap(schedule, (time1, room1), (time2, room2));
                    swaps_made += 1;

                    // Update parts array for next iteration
                    parts[i] = (time2, room2, part1, parts[i].3);
                    parts[i + 1] = (time1, room1, part2, parts[i + 1].3);
                }
            }
        }
    }

    if swaps_made == 0 {
        println!("  ✓ All series are already in chronological order");
    } else {
        println!("  ✓ Fixed {} series ordering issues", swaps_made);
    }
}

/// Check if a series is already consolidated (all parts in same room)
fn is_series_consolidated(series_base: &str, schedule: &Schedule, all_sessions: &[Minisymposium]) -> bool {
    use conflict_detection::extract_series_info;

    let mut rooms = std::collections::HashSet::new();

    for time_slot in schedule {
        for &ms_idx in time_slot {
            if ms_idx < all_sessions.len() {
                if let Some((base_title, _)) = extract_series_info(&all_sessions[ms_idx].title) {
                    if base_title == series_base {
                        // Find which room this part is in
                        for (time_idx, slot) in schedule.iter().enumerate() {
                            for (room_idx, &idx) in slot.iter().enumerate() {
                                if idx == ms_idx {
                                    rooms.insert(room_idx);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Series is consolidated if all parts are in same room (only 1 unique room)
    rooms.len() <= 1
}

/// Consolidate series parts into the same room when possible
/// Uses smart adjacent-row checking and respects already-consolidated series
fn consolidate_series_rooms(schedule: &mut Schedule, all_sessions: &[Minisymposium]) {
    use conflict_detection::extract_series_info;

    println!("\n=== SERIES ROOM CONSOLIDATION ===");

    let mut moves_made = 0;
    let mut skipped_consolidated = 0;

    // Collect all swaps first (to avoid borrow checker issues)
    let mut swaps_to_apply: Vec<((usize, usize), (usize, usize), String, usize, String)> = Vec::new();

    // Iterate through each time slot (row)
    for time_idx in 0..schedule.len() {
        let num_rooms = schedule[time_idx].len();

        for room_idx in 0..num_rooms {
            let ms_idx = schedule[time_idx][room_idx];

            if ms_idx >= all_sessions.len() {
                continue;
            }

            // Check if current session is part of a series
            if let Some((series_base, part_num)) = extract_series_info(&all_sessions[ms_idx].title) {

                // Check row ABOVE (previous time slot)
                if time_idx > 0 {
                    for (other_room_idx, &other_ms_idx) in schedule[time_idx - 1].iter().enumerate() {
                        if other_ms_idx < all_sessions.len() {
                            if let Some((other_series_base, _)) = extract_series_info(&all_sessions[other_ms_idx].title) {
                                // Found same series in row above
                                if series_base == other_series_base && room_idx != other_room_idx {
                                    // Check what's blocking the target room
                                    let blocking_ms_idx = schedule[time_idx][other_room_idx];

                                    if blocking_ms_idx < all_sessions.len() {
                                        // Check if blocking session is a series
                                        if let Some((blocking_series, _)) = extract_series_info(&all_sessions[blocking_ms_idx].title) {
                                            // It's a series - check if it's already consolidated
                                            if is_series_consolidated(&blocking_series, schedule, all_sessions) {
                                                // Skip - don't break correctly placed series
                                                skipped_consolidated += 1;
                                                continue;
                                            }
                                        }
                                    }

                                    // Safe to swap - either non-series or scattered series
                                    swaps_to_apply.push((
                                        (time_idx, room_idx),
                                        (time_idx, other_room_idx),
                                        series_base.clone(),
                                        part_num,
                                        "row above".to_string()
                                    ));
                                }
                            }
                        }
                    }
                }

                // Check row BELOW (next time slot)
                if time_idx + 1 < schedule.len() {
                    for (other_room_idx, &other_ms_idx) in schedule[time_idx + 1].iter().enumerate() {
                        if other_ms_idx < all_sessions.len() {
                            if let Some((other_series_base, _)) = extract_series_info(&all_sessions[other_ms_idx].title) {
                                // Found same series in row below
                                if series_base == other_series_base && room_idx != other_room_idx {
                                    // Check what's blocking the target room
                                    let blocking_ms_idx = schedule[time_idx][other_room_idx];

                                    if blocking_ms_idx < all_sessions.len() {
                                        // Check if blocking session is a series
                                        if let Some((blocking_series, _)) = extract_series_info(&all_sessions[blocking_ms_idx].title) {
                                            // It's a series - check if it's already consolidated
                                            if is_series_consolidated(&blocking_series, schedule, all_sessions) {
                                                // Skip - don't break correctly placed series
                                                skipped_consolidated += 1;
                                                continue;
                                            }
                                        }
                                    }

                                    // Safe to swap - either non-series or scattered series
                                    swaps_to_apply.push((
                                        (time_idx, room_idx),
                                        (time_idx, other_room_idx),
                                        series_base.clone(),
                                        part_num,
                                        "row below".to_string()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply all collected swaps
    for (from_pos, to_pos, series_base, part_num, direction) in swaps_to_apply {
        println!("  ✓ Consolidating {} Part {} into Room {} (from {})",
                series_base, part_num, to_pos.1 + 1, direction);
        apply_swap(schedule, from_pos, to_pos);
        moves_made += 1;
    }

    if moves_made == 0 && skipped_consolidated == 0 {
        println!("  ✓ All series parts are already adjacent in the same rooms");
    } else {
        println!("  ✓ Made {} room consolidations", moves_made);
        if skipped_consolidated > 0 {
            println!("  ℹ️  Skipped {} swaps to preserve already-consolidated series", skipped_consolidated);
        }
    }
}

/// Calculate difficulty score for a session (higher = harder to place)
/// Simple property-based scoring (works better than conflict-aware)
fn calculate_session_difficulty(session: &Minisymposium) -> usize {
    let num_classifications = session.classification.len();
    let num_people = session.organizers.len() + session.speakers.len();
    let is_series = session.series_key.is_some();

    // Difficulty formula: classifications are most important, then people, then series
    (num_classifications * 10) + (num_people * 2) + (if is_series { 5 } else { 0 })
}

fn create_initial_schedule(sessions: &[Minisymposium], config: &SchedulerConfig) -> Schedule {
    let mut schedule: Schedule = vec![vec![]; config.num_sessions];

    // OPTIMIZED: Place sessions sorted by difficulty (most constrained first)
    // Create index-difficulty pairs
    let mut session_difficulties: Vec<(usize, usize)> = sessions.iter()
        .enumerate()
        .map(|(idx, session)| (idx, calculate_session_difficulty(session)))
        .collect();

    // Sort by difficulty (descending - hardest first)
    session_difficulties.sort_by_key(|(_, diff)| std::cmp::Reverse(*diff));

    // Place sessions in difficulty order
    let mut session_idx = 0;

    for (ms_idx, _difficulty) in session_difficulties {
        if schedule[session_idx].len() >= config.num_rooms {
            session_idx += 1;
            if session_idx >= config.num_sessions {
                break; // Schedule full
            }
        }

        schedule[session_idx].push(ms_idx);
    }

    schedule
}

fn calculate_all_conflicts(schedule: &Schedule, all_sessions: &[Minisymposium]) -> ConflictCounts {
    let mut conflicts = ConflictCounts::default();

    conflicts.classification = ClassificationOptimizer.calculate_conflicts(schedule, all_sessions);
    conflicts.speaker_organizer = SpeakerOrganizerOptimizer.calculate_conflicts(schedule, all_sessions);
    conflicts.personal_classification = PersonalClassificationOptimizer.calculate_conflicts(schedule, all_sessions);
    conflicts.series_same_room = SeriesRoomOptimizer.calculate_conflicts(schedule, all_sessions);
    conflicts.series_back_to_back = SeriesBackOptimizer.calculate_conflicts(schedule, all_sessions);

    conflicts
}

/// OPTIMIZATION 1: Selective conflict calculation
/// Only calculates conflicts for the specified types (current optimization target + protected types)
/// This avoids calculating all 5 conflict types when we only need 2-3
fn calculate_selective_conflicts(
    schedule: &Schedule,
    all_sessions: &[Minisymposium],
    current_type: ConflictType,
    protected_types: &[ConflictType]
) -> ConflictCounts {
    let mut conflicts = ConflictCounts::default();

    // Calculate current optimization target
    conflicts.set(current_type, get_optimizer_for_type(current_type).calculate_conflicts(schedule, all_sessions));

    // Calculate only protected constraints
    for &protected_type in protected_types {
        if protected_type != current_type {
            conflicts.set(protected_type, get_optimizer_for_type(protected_type).calculate_conflicts(schedule, all_sessions));
        }
    }

    conflicts
}

/// Helper to get optimizer instance for a conflict type
fn get_optimizer_for_type(conflict_type: ConflictType) -> Box<dyn ParameterOptimizer> {
    match conflict_type {
        ConflictType::Classification => Box::new(ClassificationOptimizer),
        ConflictType::SpeakerOrganizer => Box::new(SpeakerOrganizerOptimizer),
        ConflictType::PersonalClassification => Box::new(PersonalClassificationOptimizer),
        ConflictType::SeriesSameRoom => Box::new(SeriesRoomOptimizer),
        ConflictType::SeriesBackToBack => Box::new(SeriesBackOptimizer),
    }
}

// ============================================================================
// EXCEL I/O FUNCTIONS
// ============================================================================

use calamine::{Reader, open_workbook, Xlsx, Data};

fn read_pp26_file(path: &Path) -> (Vec<Minisymposium>, Vec<ContributedLecture>) {
    println!("\n=== READING INPUT FILE ===");
    println!("File: {}", path.display());

    let mut workbook: Xlsx<_> = open_workbook(path).expect("Cannot open Excel file");

    // Read Minisymposia sheet
    let mut minisymposia = Vec::new();
    if let Ok(range) = workbook.worksheet_range("Minisymposia") {
        println!("Reading Minisymposia sheet...");

        for (row_idx, row) in range.rows().enumerate().skip(1) {
            if row_idx > 1000 { break; } // Safety limit

            // Correct column mapping based on actual Excel structure:
            // Col 0: Day & Time, Col 1-3: Class Codes, Col 4: Session#,
            // Col 5: Title, Col 6: Organizers, Col 7-9: Speakers
            let class_code_1 = get_cell_as_string(row, 1);
            let class_code_2 = get_cell_as_string(row, 2);
            let class_code_3 = get_cell_as_string(row, 3);
            let id = get_cell_as_i32(row, 4); // Session Number
            let title = get_cell_as_string(row, 5); // Actual title
            let organizers_str = get_cell_as_string(row, 6);
            let speaker1 = get_cell_as_string(row, 7);
            let speaker2 = get_cell_as_string(row, 8);
            let speaker3 = get_cell_as_string(row, 9);
            let speaker4 = get_cell_as_string(row, 10);

            // Look for series information and notes in later columns
            let notes = get_cell_as_string(row, 11); // Adjust if needed
            let series_key = String::new(); // Will extract from data if available
            let part_number = 0;

            // Skip empty titles or header row
            if title.is_empty() || title.contains("Proposal TITLE") || (title.contains("TITLE") && organizers_str.contains("Organizers")) {
                continue;
            }

            // Parse organizers (split by comma)
            let organizers: Vec<String> = organizers_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            // Parse speakers from multiple columns
            let mut speakers = Vec::new();
            for speaker_str in [speaker1, speaker2, speaker3, speaker4] {
                if !speaker_str.is_empty() {
                    // Extract name (before first dash or parenthesis)
                    let name = speaker_str
                        .split('-')
                        .next()
                        .unwrap_or(&speaker_str)
                        .split('(')
                        .next()
                        .unwrap_or(&speaker_str)
                        .trim()
                        .to_string();
                    if !name.is_empty() {
                        speakers.push(name);
                    }
                }
            }

            // Parse classification codes from 3 columns
            let mut classification = Vec::new();
            for code_str in [class_code_1, class_code_2, class_code_3] {
                if let Ok(code) = code_str.trim().parse::<i32>() {
                    if code > 0 { // Ignore 0 or invalid codes
                        classification.push(code);
                    }
                }
            }

            // Extract series information from title if present (e.g., "Part I of II")
            // Use the base title as the series key so all parts share the same key
            let series_info = if let Some((base_title, _part_num)) = conflict_detection::extract_series_info(&title) {
                Some(base_title)
            } else {
                None
            };

            minisymposia.push(Minisymposium {
                id,
                title,
                notes,
                organizers,
                speakers,
                classification,
                series_key: series_info,
                part_number: None,
                is_cl_minisymposium: false,
                cl_lecture_ids: Vec::new(),
            });
        }
        println!("Read {} minisymposia", minisymposia.len());
    }

    // Read Contributed Lectures sheet
    let mut cl_lectures = Vec::new();
    if let Ok(range) = workbook.worksheet_range("Contributed Lectures") {
        println!("Reading Contributed Lectures sheet...");

        for (row_idx, row) in range.rows().enumerate().skip(2) {
            if row_idx > 5000 { break; } // Safety limit

            // Column mapping from main.rs:
            // Cols 1-3: Classification codes
            // Col 4: Lecture ID
            // Col 5: Last name
            // Col 6: First name
            // Col 8: Title
            // Col 9: Notes

            let id = get_cell_as_i32(row, 4);
            let title = get_cell_as_string(row, 8);
            let first_name = get_cell_as_string(row, 6);
            let last_name = get_cell_as_string(row, 5);
            let speaker = format!("{} {}", first_name, last_name).trim().to_string();
            let notes = get_cell_as_string(row, 9);

            // Skip empty rows
            if id == 0 || title.trim().is_empty() {
                continue;
            }

            // Read classification codes from columns 1-3
            let mut classification = Vec::new();
            for col_idx in 1..=3 {
                if let Ok(code) = get_cell_as_string(row, col_idx).trim().parse::<i32>() {
                    if code > 0 {
                        classification.push(code);
                    }
                }
            }

            cl_lectures.push(ContributedLecture {
                id,
                title,
                speaker,
                classification,
                notes,
            });
        }
        println!("Read {} contributed lectures", cl_lectures.len());
    }

    (minisymposia, cl_lectures)
}

fn get_cell_as_string(row: &[Data], col: usize) -> String {
    if col >= row.len() {
        return String::new();
    }
    match &row[col] {
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => format!("{:?}", dt),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{:?}", e),
        Data::Empty => String::new(),
    }
}

fn get_cell_as_i32(row: &[Data], col: usize) -> i32 {
    if col >= row.len() {
        return 0;
    }
    match &row[col] {
        Data::Int(i) => *i as i32,
        Data::Float(f) => *f as i32,
        Data::String(s) => s.parse().unwrap_or(0),
        Data::Bool(b) => if *b { 1 } else { 0 },
        _ => 0,
    }
}

// ============================================================================
// EXCEL OUTPUT FUNCTIONS
// ============================================================================

use rust_xlsxwriter::{Workbook, Format, Color, FormatAlign, XlsxError};

fn write_schedule_to_excel(
    schedule: &Schedule,
    all_sessions: &[Minisymposium],
    conflicts: &ConflictCounts,
    output_file: &str
) -> Result<(), XlsxError> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    worksheet.set_name("Schedule")?;

    // Create formats
    let header_format = Format::new()
        .set_bold()
        .set_font_size(12)
        .set_background_color(Color::RGB(0x4472C4))
        .set_font_color(Color::White)
        .set_align(FormatAlign::Center);

    let session_header_format = Format::new()
        .set_bold()
        .set_font_size(11)
        .set_background_color(Color::RGB(0xD9E1F2))
        .set_align(FormatAlign::Left);

    let id_format = Format::new()
        .set_font_size(10)
        .set_align(FormatAlign::Center);

    let title_format = Format::new()
        .set_font_size(10)
        .set_text_wrap();

    // Set column widths
    worksheet.set_column_width(0, 18)?;  // Session
    worksheet.set_column_width(1, 12)?;  // Room
    worksheet.set_column_width(2, 12)?;  // ID
    worksheet.set_column_width(3, 60)?;  // Title
    worksheet.set_column_width(4, 30)?;  // Classification
    worksheet.set_column_width(5, 40)?;  // Organizers
    worksheet.set_column_width(6, 40)?;  // Speakers
    worksheet.set_column_width(7, 30)?;  // Notes

    // Write headers
    let mut row: u32 = 0;
    worksheet.write_with_format(row, 0, "Session", &header_format)?;
    worksheet.write_with_format(row, 1, "Room", &header_format)?;
    worksheet.write_with_format(row, 2, "ID", &header_format)?;
    worksheet.write_with_format(row, 3, "Title", &header_format)?;
    worksheet.write_with_format(row, 4, "Classification", &header_format)?;
    worksheet.write_with_format(row, 5, "Organizers", &header_format)?;
    worksheet.write_with_format(row, 6, "Speakers", &header_format)?;
    worksheet.write_with_format(row, 7, "Notes", &header_format)?;
    row += 1;

    // Session names in format compatible with web analyzer: "DAY First/Second Period"
    let session_names = [
        "TUE First Period", "TUE Second Period",
        "WED First Period", "WED Second Period",
        "THU First Period", "THU Second Period",
        "FRI First Period", "FRI Second Period",
    ];

    // Find scheduled indices
    let mut scheduled_indices: HashSet<usize> = HashSet::new();
    for session in schedule.iter() {
        for &idx in session {
            scheduled_indices.insert(idx);
        }
    }

    // Write each session
    for (session_idx, session) in schedule.iter().enumerate() {
        if session.is_empty() {
            continue;
        }

        let session_name = if session_idx < session_names.len() {
            session_names[session_idx]
        } else {
            "SESSION"
        };

        worksheet.write_with_format(row, 0, session_name, &session_header_format)?;
        worksheet.write_with_format(row, 1, format!("({} rooms)", session.len()), &session_header_format)?;
        row += 1;

        // Write each minisymposium
        for (room_idx, &ms_idx) in session.iter().enumerate() {
            if ms_idx >= all_sessions.len() {
                continue;
            }

            let ms = &all_sessions[ms_idx];

            worksheet.write(row, 0, session_name)?;
            worksheet.write_with_format(row, 1, (room_idx + 1) as u32, &id_format)?;

            if ms.is_cl_minisymposium {
                worksheet.write_with_format(row, 2, format!("CL-{}", ms.id), &id_format)?;
            } else {
                worksheet.write_with_format(row, 2, ms.id, &id_format)?;
            }

            worksheet.write_with_format(row, 3, &ms.title, &title_format)?;

            let class_str = ms.classification.iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            worksheet.write(row, 4, &class_str)?;

            // Write organizers and speakers in separate columns
            let organizers_str = if ms.is_cl_minisymposium {
                String::new()  // CL groups don't have organizers
            } else {
                ms.organizers.join(", ")
            };
            worksheet.write_with_format(row, 5, &organizers_str, &title_format)?;

            let speakers_str = ms.speakers.join(", ");
            worksheet.write_with_format(row, 6, &speakers_str, &title_format)?;

            worksheet.write_with_format(row, 7, &ms.notes, &title_format)?;

            row += 1;
        }

        row += 1; // Blank row between sessions
    }

    // Add unscheduled section
    let unscheduled: Vec<(usize, &Minisymposium)> = all_sessions.iter()
        .enumerate()
        .filter(|(idx, _)| !scheduled_indices.contains(idx))
        .collect();

    if !unscheduled.is_empty() {
        row += 1;
        let unscheduled_header_format = Format::new()
            .set_bold()
            .set_font_size(12)
            .set_background_color(Color::RGB(0xFF0000))
            .set_font_color(Color::White);

        worksheet.write_with_format(row, 0, format!("UNSCHEDULED ({} sessions)", unscheduled.len()), &unscheduled_header_format)?;
        row += 1;

        for (_, ms) in unscheduled {
            worksheet.write_with_format(row, 2, ms.id, &id_format)?;
            worksheet.write_with_format(row, 3, &ms.title, &title_format)?;
            row += 1;
        }
    }

    workbook.save(output_file)?;
    Ok(())
}

// ============================================================================
// MAIN FUNCTION
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Use default output path like main.rs, or allow override
    let input_file = if args.len() > 1 {
        Path::new(&args[1])
    } else {
        Path::new("data/uploads/current_source.xlsx")
    };

    let output_file = if args.len() > 2 {
        args[2].clone()
    } else {
        "data/generated/PP26_Schedule_Output.xlsx".to_string()
    };

    // Optional: Accept JSON configuration as 3rd argument
    // Format: {"strict_hierarchy": true, "priorities": {"personal": 1, "classification": 5, ...}}
    let mut config = SchedulerConfig::default();

    if args.len() > 3 {
        match serde_json::from_str::<serde_json::Value>(&args[3]) {
            Ok(json_config) => {
                println!("\n📋 Loading custom configuration from JSON...");

                // Parse strict_hierarchy
                if let Some(strict) = json_config.get("strict_hierarchy").and_then(|v| v.as_bool()) {
                    config.strict_hierarchy = strict;
                    println!("   Strict hierarchy: {}", strict);
                }

                // Parse priorities
                if let Some(priorities) = json_config.get("priorities").and_then(|v| v.as_object()) {
                    if let Some(personal) = priorities.get("personal").and_then(|v| v.as_u64()) {
                        config.speaker_org_rank = personal as u8;
                        println!("   Personal conflicts priority: {}", personal);
                    }
                    if let Some(classification) = priorities.get("classification").and_then(|v| v.as_u64()) {
                        config.classification_rank = classification as u8;
                        println!("   Classification conflicts priority: {}", classification);
                    }
                    if let Some(personal_class) = priorities.get("personal_classification").and_then(|v| v.as_u64()) {
                        config.personal_class_rank = personal_class as u8;
                        println!("   Personal-Classification conflicts priority: {}", personal_class);
                    }
                    if let Some(series_back) = priorities.get("series_back").and_then(|v| v.as_u64()) {
                        config.series_back_rank = series_back as u8;
                        println!("   Series Back-to-Back priority: {}", series_back);
                    }
                    if let Some(series_room) = priorities.get("series_room").and_then(|v| v.as_u64()) {
                        config.series_room_rank = series_room as u8;
                        println!("   Series Room Consistency priority: {}", series_room);
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Failed to parse JSON config: {}", e);
                eprintln!("   Using default configuration instead");
            }
        }
    }

    // Print final configuration for debugging
    println!("\n📋 FINAL CONFIGURATION:");
    println!("   Strict hierarchy: {}", config.strict_hierarchy);
    println!("   Classification rank: {} (optimize {})", config.classification_rank,
             if config.classification_rank == 1 { "FIRST" } else if config.classification_rank == 5 { "LAST" } else { "MIDDLE" });
    println!("   Speaker/Organizer rank: {} (optimize {})", config.speaker_org_rank,
             if config.speaker_org_rank == 1 { "FIRST" } else if config.speaker_org_rank == 5 { "LAST" } else { "MIDDLE" });
    println!("   Personal-Classification rank: {} (optimize {})", config.personal_class_rank,
             if config.personal_class_rank == 1 { "FIRST" } else if config.personal_class_rank == 5 { "LAST" } else { "MIDDLE" });
    println!("   Series Room rank: {} (optimize {})", config.series_room_rank,
             if config.series_room_rank == 1 { "FIRST" } else if config.series_room_rank == 5 { "LAST" } else { "MIDDLE" });
    println!("   Series Back-to-Back rank: {} (optimize {})", config.series_back_rank,
             if config.series_back_rank == 1 { "FIRST" } else if config.series_back_rank == 5 { "LAST" } else { "MIDDLE" });

    println!("═══════════════════════════════════════════════════════");
    println!("     HIERARCHICAL CONFERENCE SCHEDULER TEST");
    println!("═══════════════════════════════════════════════════════");

    use std::time::Instant;
    let overall_start = Instant::now();

    // Read input data
    let t_start = Instant::now();
    let (regular_sessions, cl_lectures) = read_pp26_file(input_file);
    println!("⏱️  Read input file: {:.3}s", t_start.elapsed().as_secs_f64());

    // Preprocess CL lectures into groups of 4
    let t_start = Instant::now();
    let cl_groups = CLPreprocessor::new().group_lectures(cl_lectures);
    println!("⏱️  Group CL lectures: {:.3}s", t_start.elapsed().as_secs_f64());

    // Combine all sessions
    let mut all_sessions = regular_sessions;
    all_sessions.extend(cl_groups);

    println!("\n=== TOTAL SESSIONS ===");
    println!("Total minisymposia to schedule: {}", all_sessions.len());

    // RUN FEASIBILITY ANALYSIS BEFORE SCHEDULING
    let t_start = Instant::now();
    let _feasibility = analyze_feasibility(&all_sessions, config.num_sessions, config.num_rooms);
    println!("⏱️  Feasibility analysis: {:.3}s", t_start.elapsed().as_secs_f64());

    // Create initial schedule
    let t_start = Instant::now();
    let initial_schedule = create_initial_schedule(&all_sessions, &config);
    println!("⏱️  Create initial schedule: {:.3}s", t_start.elapsed().as_secs_f64());

    let t_start = Instant::now();
    let initial_conflicts = calculate_all_conflicts(&initial_schedule, &all_sessions);
    println!("⏱️  Calculate initial conflicts: {:.3}s", t_start.elapsed().as_secs_f64());

    println!("\n=== INITIAL SCHEDULE ===");
    println!("Total conflicts: {}", initial_conflicts.total());
    println!("  Classification: {}", initial_conflicts.classification);
    println!("  Speaker/Organizer: {}", initial_conflicts.speaker_organizer);
    println!("  Personal-Classification: {}", initial_conflicts.personal_classification);
    println!("  Series Same-Room: {}", initial_conflicts.series_same_room);
    println!("  Series Back-to-Back: {}", initial_conflicts.series_back_to_back);

    // Create initial solution
    let initial_solution = Solution::new(initial_schedule, initial_conflicts);

    // Run hierarchical optimization
    let t_start = Instant::now();
    let engine = HierarchicalEngine::new(config);
    let mut optimized_solutions = engine.optimize(vec![initial_solution], &all_sessions);
    println!("⏱️  HIERARCHICAL OPTIMIZATION: {:.3}s", t_start.elapsed().as_secs_f64());

    // Clean up series ordering (ensure Part I comes before Part II)
    let t_start = Instant::now();
    if let Some(best_solution) = optimized_solutions.first_mut() {
        fix_series_ordering(&mut best_solution.schedule, &all_sessions);
    }
    println!("⏱️  Fix series ordering: {:.3}s", t_start.elapsed().as_secs_f64());

    // Consolidate series parts into same rooms
    let t_start = Instant::now();
    if let Some(best_solution) = optimized_solutions.first_mut() {
        consolidate_series_rooms(&mut best_solution.schedule, &all_sessions);
    }
    println!("⏱️  Consolidate series rooms: {:.3}s", t_start.elapsed().as_secs_f64());

    // Display best solution
    if let Some(best_solution) = optimized_solutions.first() {
        println!("\n=== FINAL OPTIMIZED SCHEDULE ===");
        println!("Total conflicts: {}", best_solution.conflicts.total());
        println!("  Classification: {}", best_solution.conflicts.classification);
        println!("  Speaker/Organizer: {}", best_solution.conflicts.speaker_organizer);
        println!("  Personal-Classification: {}", best_solution.conflicts.personal_classification);
        println!("  Series Same-Room: {}", best_solution.conflicts.series_same_room);
        println!("  Series Back-to-Back: {}", best_solution.conflicts.series_back_to_back);

        println!("\n=== SCHEDULE SUMMARY ===");
        for (session_idx, session_slot) in best_solution.schedule.iter().enumerate() {
            println!("Session {}: {} rooms occupied", session_idx + 1, session_slot.len());
        }

        // Write to Excel file
        println!("\n=== WRITING OUTPUT FILE ===");
        let t_start = Instant::now();
        match write_schedule_to_excel(&best_solution.schedule, &all_sessions, &best_solution.conflicts, &output_file) {
            Ok(_) => {
                println!("✅ Schedule written to: {}", output_file);
                println!("  - All sessions organized by time slot and room");
                println!("⏱️  Write Excel file: {:.3}s", t_start.elapsed().as_secs_f64());

                // Validate output format for web analyzer compatibility
                match validate_output_for_web(&output_file) {
                    Ok(_) => {},
                    Err(e) => {
                        eprintln!("\n⚠️  WARNING: Output validation failed: {}", e);
                        eprintln!("    File may not be compatible with web analyzer");
                    }
                }
            }
            Err(e) => {
                eprintln!("✗ Error writing Excel file: {}", e);
            }
        }
    } else {
        println!("\nERROR: No solutions found!");
    }

    let total_time = overall_start.elapsed();
    println!("\n═══════════════════════════════════════════════════════");
    println!("           HIERARCHICAL SCHEDULER COMPLETE");
    println!("⏱️  TOTAL TIME: {:.3}s ({:.2} minutes)", total_time.as_secs_f64(), total_time.as_secs_f64() / 60.0);
    println!("═══════════════════════════════════════════════════════");
}

// Validation function to test compatibility with web analyzer
fn validate_output_for_web(file_path: &str) -> Result<(), String> {
    use calamine::{open_workbook, Reader, Xlsx};
    
    println!("\n=== VALIDATING OUTPUT FOR WEB ANALYZER ===");
    
    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| format!("Failed to open: {}", e))?;
    
    let sheet_names = workbook.sheet_names();
    println!("Sheet count: {} (expected: 1)", sheet_names.len());
    println!("Sheet names: {:?}", sheet_names);
    
    if sheet_names.len() != 1 {
        return Err(format!("Expected 1 sheet, found {}", sheet_names.len()));
    }
    
    let range = workbook.worksheet_range(&sheet_names[0])
        .map_err(|e| format!("Failed to read sheet: {}", e))?;
    
    let row_count = range.rows().count();
    println!("Total rows: {}", row_count);
    
    if row_count < 2 {
        return Err("No data rows found".to_string());
    }
    
    if let Some(header) = range.rows().next() {
        let headers: Vec<String> = header.iter()
            .map(|c| c.to_string().trim().to_string())
            .collect();
        
        let headers_lower: Vec<String> = headers.iter()
            .map(|h| h.to_lowercase())
            .collect();
        
        println!("Headers: {:?}", headers);
        
        let required = vec!["session", "room", "id", "title", "classification", "organizers", "speakers"];
        println!("\nRequired column checks:");
        
        for req in &required {
            let found = headers_lower.iter().any(|h| h.contains(req));
            println!("  {} '{}': {}", if found { "✓" } else { "✗" }, req, if found { "FOUND" } else { "MISSING" });
        }
        
        let missing: Vec<_> = required.iter()
            .filter(|&col| !headers_lower.iter().any(|h| h.contains(col)))
            .collect();
        
        if !missing.is_empty() {
            return Err(format!("Missing columns: {:?}", missing));
        }
    }
    
    // Check session format
    println!("\nChecking session format...");
    let mut found_period_format = false;
    for (i, row) in range.rows().enumerate().skip(1).take(5) {
        if let Some(cell) = row.get(0) {
            let session_str = cell.to_string();
            if session_str.contains("First Period") || session_str.contains("Second Period") {
                found_period_format = true;
                println!("  Row {}: '{}' ✓", i + 1, session_str);
            } else if !session_str.is_empty() {
                println!("  Row {}: '{}' (no Period format)", i + 1, session_str);
            }
        }
    }
    
    if !found_period_format {
        return Err("Session names don't contain 'First Period' or 'Second Period'".to_string());
    }
    
    println!("\n✅ ALL VALIDATION CHECKS PASSED");
    Ok(())
}
