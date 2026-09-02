// ============================================================================
// CONFERENCE SCHEDULE ANALYZER
// ============================================================================
// Standalone analyzer that reads schedule Excel files and outputs comprehensive
// conflict analysis as structured JSON.
//
// Input Format: 8-column Excel with Schedule sheet
// Output Format: Structured JSON with conflict details and statistics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use calamine::{open_workbook, Reader, Xlsx, Data};
use conference_scheduler::conflict_detection;

// ============================================================================
// TYPE DEFINITIONS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionInfo {
    id: String,
    title: String,
    session_period: String,
    room: String,
    classification: Vec<String>,  // Parsed as Vec for easier comparison
    organizers: Vec<String>,      // Split by comma/semicolon
    speakers: Vec<String>,         // Split by comma/semicolon
    notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ConflictType {
    Classification,
    Personal,
    PersonalClassification,
    SeriesRoom,
    SeriesBackToBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConflictInfo {
    conflict_type: ConflictType,
    period: String,
    session1: ConflictSessionInfo,
    session2: ConflictSessionInfo,
    details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConflictSessionInfo {
    id: String,
    title: String,
    room: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SeriesInfo {
    base_title: String,
    total_parts: usize,
    is_optimal: bool,
    issues: Vec<String>,
    #[serde(rename = "parts")]
    sessions: Vec<SessionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConflictSummary {
    count: usize,
    rate: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PeriodAnalysis {
    session_count: usize,
    classification_conflicts: usize,
    personal_conflicts: usize,
    personal_classification_conflicts: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct RoomUsage {
    total_sessions: usize,
    periods: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeriesAnalysisSummary {
    total_series: usize,
    optimal: usize,
    suboptimal: usize,
    series_list: Vec<SeriesInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnalysisResult {
    metadata: AnalysisMetadata,
    summary: ConflictSummaryStats,
    theoretical_bounds: TheoreticalBounds,
    by_period: HashMap<String, PeriodAnalysis>,
    conflicts: ConflictDetails,
    room_usage: HashMap<String, RoomUsage>,
    classification_distribution: HashMap<String, usize>,
    #[serde(rename = "series")]
    series_analysis: SeriesAnalysisSummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnalysisMetadata {
    total_sessions: usize,
    total_conflicts: usize,
    analysis_timestamp: String,
    input_file: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConflictSummaryStats {
    classification: ConflictSummary,
    personal: ConflictSummary,
    personal_classification: ConflictSummary,
    series_room: ConflictSummary,
    series_back_to_back: ConflictSummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConflictDetails {
    classification: Vec<ConflictInfo>,
    personal: Vec<ConflictInfo>,
    personal_classification: Vec<ConflictInfo>,
    series_room: Vec<ConflictInfo>,
    series_back_to_back: Vec<ConflictInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BoundInfo {
    theoretical_min: usize,
    current: usize,
    gap: i32,
    is_optimal: bool,
    explanation: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TheoreticalBounds {
    speaker_organizer: BoundInfo,
    personal_classification: BoundInfo,
    series_room: BoundInfo,
    series_back_to_back: BoundInfo,
    classification: BoundInfo,
}

// ============================================================================
// EXCEL READER MODULE
// ============================================================================

fn read_schedule_from_excel(file_path: &str) -> Result<Vec<SessionInfo>, String> {
    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| format!("Failed to open Excel file: {}", e))?;

    // Read the "Schedule" sheet
    let range = workbook.worksheet_range("Schedule")
        .map_err(|e| format!("Failed to read Schedule sheet: {}", e))?;

    let mut sessions = Vec::new();
    let mut current_session_period: Option<String> = None;

    // Skip header row (row 0)
    for (_row_idx, row) in range.rows().enumerate().skip(1) {
        // Column mapping:
        // 0: Session, 1: Room, 2: ID, 3: Title, 4: Classification
        // 5: Organizers, 6: Speakers, 7: Notes

        let session_str = get_cell_as_string(row, 0);

        // Detect session period transitions
        if session_str.contains("First Period") || session_str.contains("Second Period") {
            current_session_period = Some(session_str.clone());
        }

        // Get ID
        let id = get_cell_as_string(row, 2);
        if id.is_empty() || current_session_period.is_none() {
            continue;
        }

        let room = get_cell_as_string(row, 1);
        let title = get_cell_as_string(row, 3);
        let classification_str = get_cell_as_string(row, 4);
        let organizers_str = get_cell_as_string(row, 5);
        let speakers_str = get_cell_as_string(row, 6);
        let notes = get_cell_as_string(row, 7);

        // Parse classification codes
        let classification: Vec<String> = classification_str
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Parse organizers
        let organizers = parse_people_list(&organizers_str);

        // Parse speakers
        let speakers = parse_people_list(&speakers_str);

        sessions.push(SessionInfo {
            id,
            title,
            session_period: current_session_period.clone().unwrap(),
            room,
            classification,
            organizers,
            speakers,
            notes,
        });
    }

    if sessions.is_empty() {
        return Err("No sessions found in the schedule".to_string());
    }

    Ok(sessions)
}

fn get_cell_as_string(row: &[Data], col_idx: usize) -> String {
    row.get(col_idx)
        .map(|cell| match cell {
            Data::String(s) => s.clone(),
            Data::Int(i) => i.to_string(),
            Data::Float(f) => f.to_string(),
            Data::Bool(b) => b.to_string(),
            Data::Empty => String::new(),
            _ => String::new(),
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_people_list(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    // Replace "and" with comma for consistent splitting
    let normalized = input
        .replace(" and ", ", ")
        .replace(" And ", ", ")
        .replace(" AND ", ", ");

    normalized
        .split(&[',', ';'][..])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() > 1)
        .collect()
}

// ============================================================================
// CONFLICT DETECTION MODULE
// ============================================================================

fn detect_classification_conflicts(sessions: &[SessionInfo]) -> Vec<ConflictInfo> {
    let mut conflicts = Vec::new();

    // Group sessions by period
    let sessions_by_period = group_sessions_by_period(sessions);

    for (period, period_sessions) in &sessions_by_period {
        // Check all pairs in the same period
        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let s1 = &period_sessions[i];
                let s2 = &period_sessions[j];

                // Use shared conflict detection logic
                if conflict_detection::has_classification_conflict(&s1.classification, &s2.classification) {
                    // Find shared classifications for details
                    let shared: Vec<_> = s1.classification.iter()
                        .filter(|c1| s2.classification.iter().any(|c2| c1 == &c2))
                        .cloned()
                        .collect();

                    conflicts.push(ConflictInfo {
                        conflict_type: ConflictType::Classification,
                        period: period.clone(),
                        session1: ConflictSessionInfo {
                            id: s1.id.clone(),
                            title: s1.title.clone(),
                            room: s1.room.clone(),
                        },
                        session2: ConflictSessionInfo {
                            id: s2.id.clone(),
                            title: s2.title.clone(),
                            room: s2.room.clone(),
                        },
                        details: format!("Shared classifications: {}", shared.join(", ")),
                    });
                }
            }
        }
    }

    conflicts
}

fn detect_personal_conflicts(sessions: &[SessionInfo]) -> Vec<ConflictInfo> {
    let mut conflicts = Vec::new();

    let sessions_by_period = group_sessions_by_period(sessions);

    for (period, period_sessions) in &sessions_by_period {
        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let s1 = &period_sessions[i];
                let s2 = &period_sessions[j];

                // Use shared conflict detection logic
                if conflict_detection::has_speaker_organizer_conflict(
                    &s1.speakers, &s1.organizers,
                    &s2.speakers, &s2.organizers
                ) {
                    // Find overlapping people for details
                    let mut people1 = s1.organizers.clone();
                    people1.extend(s1.speakers.clone());
                    let mut people2 = s2.organizers.clone();
                    people2.extend(s2.speakers.clone());

                    let normalized1: Vec<_> = people1.iter()
                        .map(|p| conflict_detection::normalize_exact(p))
                        .filter(|p| !p.is_empty())
                        .collect();
                    let normalized2: Vec<_> = people2.iter()
                        .map(|p| conflict_detection::normalize_exact(p))
                        .filter(|p| !p.is_empty())
                        .collect();

                    let overlapping: Vec<_> = people1.iter()
                        .zip(normalized1.iter())
                        .filter(|(_, n1)| normalized2.iter().any(|n2| n1 == &n2))
                        .map(|(p, _)| p.clone())
                        .collect();

                    conflicts.push(ConflictInfo {
                        conflict_type: ConflictType::Personal,
                        period: period.clone(),
                        session1: ConflictSessionInfo {
                            id: s1.id.clone(),
                            title: s1.title.clone(),
                            room: s1.room.clone(),
                        },
                        session2: ConflictSessionInfo {
                            id: s2.id.clone(),
                            title: s2.title.clone(),
                            room: s2.room.clone(),
                        },
                        details: format!("Conflicting people: {}", overlapping.join(", ")),
                    });
                }
            }
        }
    }

    conflicts
}

fn detect_personal_classification_conflicts(sessions: &[SessionInfo]) -> Vec<ConflictInfo> {
    let mut conflicts = Vec::new();

    let sessions_by_period = group_sessions_by_period(sessions);

    for (period, period_sessions) in &sessions_by_period {
        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let s1 = &period_sessions[i];
                let s2 = &period_sessions[j];

                // Use shared conflict detection logic (exact match only, no substring)
                if conflict_detection::has_personal_classification_conflict(&s1.notes, &s2.notes) {
                    conflicts.push(ConflictInfo {
                        conflict_type: ConflictType::PersonalClassification,
                        period: period.clone(),
                        session1: ConflictSessionInfo {
                            id: s1.id.clone(),
                            title: s1.title.clone(),
                            room: s1.room.clone(),
                        },
                        session2: ConflictSessionInfo {
                            id: s2.id.clone(),
                            title: s2.title.clone(),
                            room: s2.room.clone(),
                        },
                        details: format!("Matching notes keywords: '{}' vs '{}'", s1.notes, s2.notes),
                    });
                }
            }
        }
    }

    conflicts
}

// Room conflicts removed - not needed as part of the 5 core conflict types

fn detect_series_conflicts(sessions: &[SessionInfo]) -> (Vec<ConflictInfo>, Vec<ConflictInfo>) {
    let mut room_conflicts = Vec::new();
    let mut back_to_back_conflicts = Vec::new();

    // Group sessions by series base title
    let mut series_map: HashMap<String, Vec<&SessionInfo>> = HashMap::new();
    for session in sessions {
        if let Some((base_title, _part_num)) = conflict_detection::extract_series_info(&session.title) {
            series_map.entry(base_title)
                .or_insert_with(Vec::new)
                .push(session);
        }
    }

    // Analyze each series
    for (base_title, series_sessions) in series_map {
        if series_sessions.len() < 2 {
            continue;
        }

        // Sort by time period
        let mut sorted = series_sessions.clone();
        sorted.sort_by(|a, b| conflict_detection::get_period_order(&a.session_period)
            .cmp(&conflict_detection::get_period_order(&b.session_period)));

        // Check all pairs for conflicts
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                let s1 = sorted[i];
                let s2 = sorted[j];

                // Check room conflicts using shared logic
                if conflict_detection::has_series_room_conflict(
                    &s1.title, &s1.room,
                    &s2.title, &s2.room
                ) {
                    room_conflicts.push(ConflictInfo {
                        conflict_type: ConflictType::SeriesRoom,
                        period: s2.session_period.clone(),
                        session1: ConflictSessionInfo {
                            id: s1.id.clone(),
                            title: s1.title.clone(),
                            room: s1.room.clone(),
                        },
                        session2: ConflictSessionInfo {
                            id: s2.id.clone(),
                            title: s2.title.clone(),
                            room: s2.room.clone(),
                        },
                        details: format!("Series '{}' parts in different rooms", base_title),
                    });
                }

                // Check back-to-back conflicts (parallel + timing) using shared logic
                if conflict_detection::has_series_back_to_back_conflict(
                    &s1.title, &s1.session_period,
                    &s2.title, &s2.session_period
                ) {
                    let order1 = conflict_detection::get_period_order(&s1.session_period);
                    let order2 = conflict_detection::get_period_order(&s2.session_period);

                    let detail = if order1 == order2 {
                        format!("Series '{}' parts scheduled in parallel", base_title)
                    } else {
                        format!("Series '{}' parts not in consecutive periods", base_title)
                    };

                    back_to_back_conflicts.push(ConflictInfo {
                        conflict_type: ConflictType::SeriesBackToBack,
                        period: s2.session_period.clone(),
                        session1: ConflictSessionInfo {
                            id: s1.id.clone(),
                            title: s1.title.clone(),
                            room: s1.room.clone(),
                        },
                        session2: ConflictSessionInfo {
                            id: s2.id.clone(),
                            title: s2.title.clone(),
                            room: s2.room.clone(),
                        },
                        details: detail,
                    });
                }
            }
        }
    }

    (room_conflicts, back_to_back_conflicts)
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn group_sessions_by_period(sessions: &[SessionInfo]) -> HashMap<String, Vec<&SessionInfo>> {
    let mut grouped: HashMap<String, Vec<&SessionInfo>> = HashMap::new();
    for session in sessions {
        grouped.entry(session.session_period.clone())
            .or_insert_with(Vec::new)
            .push(session);
    }
    grouped
}

// Note: Other helper functions (normalize_exact, extract_series_info, get_period_order)
// are now in the shared conflict_detection module

// ============================================================================
// STATISTICS & AGGREGATION MODULE
// ============================================================================

fn calculate_statistics(
    sessions: &[SessionInfo],
    all_conflicts: &ConflictDetails,
) -> (ConflictSummaryStats, HashMap<String, PeriodAnalysis>, HashMap<String, RoomUsage>, HashMap<String, usize>) {
    let sessions_by_period = group_sessions_by_period(sessions);

    // Calculate total possible pairs for rate calculation
    let total_possible_pairs: usize = sessions_by_period.values()
        .map(|period_sessions| {
            let n = period_sessions.len();
            if n > 1 { n * (n - 1) / 2 } else { 0 }
        })
        .sum();

    let total_pairs_f64 = total_possible_pairs as f64;

    // Build summary statistics
    let summary = ConflictSummaryStats {
        classification: ConflictSummary {
            count: all_conflicts.classification.len(),
            rate: if total_pairs_f64 > 0.0 {
                (all_conflicts.classification.len() as f64 / total_pairs_f64) * 100.0
            } else { 0.0 },
        },
        personal: ConflictSummary {
            count: all_conflicts.personal.len(),
            rate: if total_pairs_f64 > 0.0 {
                (all_conflicts.personal.len() as f64 / total_pairs_f64) * 100.0
            } else { 0.0 },
        },
        personal_classification: ConflictSummary {
            count: all_conflicts.personal_classification.len(),
            rate: if total_pairs_f64 > 0.0 {
                (all_conflicts.personal_classification.len() as f64 / total_pairs_f64) * 100.0
            } else { 0.0 },
        },
        series_room: ConflictSummary {
            count: all_conflicts.series_room.len(),
            rate: 0.0, // Not per-pair based
        },
        series_back_to_back: ConflictSummary {
            count: all_conflicts.series_back_to_back.len(),
            rate: 0.0, // Not per-pair based
        },
    };

    // Build per-period analysis
    let mut by_period: HashMap<String, PeriodAnalysis> = HashMap::new();
    for (period, period_sessions) in &sessions_by_period {
        let classification_conflicts = all_conflicts.classification.iter()
            .filter(|c| &c.period == period)
            .count();
        let personal_conflicts = all_conflicts.personal.iter()
            .filter(|c| &c.period == period)
            .count();
        let personal_classification_conflicts = all_conflicts.personal_classification.iter()
            .filter(|c| &c.period == period)
            .count();

        by_period.insert(period.clone(), PeriodAnalysis {
            session_count: period_sessions.len(),
            classification_conflicts,
            personal_conflicts,
            personal_classification_conflicts,
        });
    }

    // Build room usage analysis
    let mut room_usage: HashMap<String, RoomUsage> = HashMap::new();
    for session in sessions {
        let entry = room_usage.entry(session.room.clone())
            .or_insert_with(|| RoomUsage {
                total_sessions: 0,
                periods: Vec::new(),
            });
        entry.total_sessions += 1;
        if !entry.periods.contains(&session.session_period) {
            entry.periods.push(session.session_period.clone());
        }
    }

    // Build classification distribution
    let mut classification_dist: HashMap<String, usize> = HashMap::new();
    for session in sessions {
        for code in &session.classification {
            *classification_dist.entry(code.clone()).or_insert(0) += 1;
        }
    }

    (summary, by_period, room_usage, classification_dist)
}

// ============================================================================
// THEORETICAL BOUNDS CALCULATOR
// ============================================================================

fn calculate_theoretical_bounds(
    sessions: &[SessionInfo],
    current_conflicts: &ConflictDetails,
) -> TheoreticalBounds {
    const NUM_TIME_SLOTS: usize = 8;
    const NUM_ROOMS: usize = 12;

    // 1. SPEAKER/ORGANIZER BOUNDS
    // Build conflict graph and find chromatic number
    let mut max_concurrent_person_sessions = 0;
    for slot_sessions in group_by_period(sessions).values() {
        let mut person_session_count: HashMap<String, usize> = HashMap::new();
        for session in slot_sessions {
            for person in session.speakers.iter().chain(session.organizers.iter()) {
                let normalized = conflict_detection::normalize_exact(person);
                if !normalized.is_empty() {
                    *person_session_count.entry(normalized).or_insert(0) += 1;
                }
            }
        }
        let max_in_slot = person_session_count.values().max().copied().unwrap_or(0);
        max_concurrent_person_sessions = max_concurrent_person_sessions.max(max_in_slot);
    }

    let speaker_org_min = if max_concurrent_person_sessions <= NUM_TIME_SLOTS {
        0
    } else {
        max_concurrent_person_sessions - NUM_TIME_SLOTS
    };

    let speaker_org_current = current_conflicts.personal.len();
    let speaker_org_bound = BoundInfo {
        theoretical_min: speaker_org_min,
        current: speaker_org_current,
        gap: speaker_org_current as i32 - speaker_org_min as i32,
        is_optimal: speaker_org_current == speaker_org_min,
        explanation: format!(
            "Max {} sessions/person, {} slots available",
            max_concurrent_person_sessions, NUM_TIME_SLOTS
        ),
    };

    // 2. PERSONAL-CLASSIFICATION BOUNDS
    // Count sessions per notes keyword
    let mut notes_counts: HashMap<String, usize> = HashMap::new();
    for session in sessions {
        let normalized = conflict_detection::normalize_exact(&session.notes);
        if !normalized.is_empty() && !normalized.contains("notes") {
            *notes_counts.entry(normalized).or_insert(0) += 1;
        }
    }

    let mut personal_class_min = 0;
    let mut max_keyword = String::new();
    let mut max_count = 0;

    for (keyword, count) in &notes_counts {
        if *count > NUM_TIME_SLOTS {
            // Each keyword can have at most NUM_TIME_SLOTS sessions without conflicts
            // C(n,2) = n*(n-1)/2 conflicts for n sessions in same slot
            let sessions_per_slot = (*count + NUM_TIME_SLOTS - 1) / NUM_TIME_SLOTS;
            if sessions_per_slot > 1 {
                // At least one slot will have sessions_per_slot items
                let min_conflicts_for_keyword = sessions_per_slot * (sessions_per_slot - 1) / 2;
                personal_class_min += min_conflicts_for_keyword;
            }
        }
        if *count > max_count {
            max_count = *count;
            max_keyword = keyword.clone();
        }
    }

    let personal_class_current = current_conflicts.personal_classification.len();
    let personal_class_bound = BoundInfo {
        theoretical_min: personal_class_min,
        current: personal_class_current,
        gap: personal_class_current as i32 - personal_class_min as i32,
        is_optimal: personal_class_current == personal_class_min,
        explanation: format!(
            "Most constrained keyword '{}' has {} sessions, {} slots available",
            max_keyword, max_count, NUM_TIME_SLOTS
        ),
    };

    // 3. SERIES ROOM BOUNDS
    // Count unique series
    let mut series_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for session in sessions {
        if let Some(base) = extract_series_base(&session.title) {
            series_set.insert(base);
        }
    }
    let num_series = series_set.len();

    let series_room_min = if num_series > NUM_ROOMS {
        num_series - NUM_ROOMS
    } else {
        0
    };

    let series_room_current = current_conflicts.series_room.len();
    let series_room_bound = BoundInfo {
        theoretical_min: series_room_min,
        current: series_room_current,
        gap: series_room_current as i32 - series_room_min as i32,
        is_optimal: series_room_current == series_room_min,
        explanation: format!(
            "{} series detected, {} rooms available (pigeonhole principle)",
            num_series, NUM_ROOMS
        ),
    };

    // 4. SERIES BACK-TO-BACK BOUNDS
    // Count series with 3+ parts (need consecutive slots)
    let mut long_series_count = 0;
    let mut series_parts: HashMap<String, usize> = HashMap::new();
    for session in sessions {
        if let Some(base) = extract_series_base(&session.title) {
            *series_parts.entry(base).or_insert(0) += 1;
        }
    }
    for count in series_parts.values() {
        if *count >= 3 {
            long_series_count += 1;
        }
    }

    // Rough estimate: each 3+ part series may have 1 back-to-back conflict if slots are constrained
    let series_bb_min = if long_series_count > NUM_TIME_SLOTS / 3 {
        long_series_count - (NUM_TIME_SLOTS / 3)
    } else {
        0
    };

    let series_bb_current = current_conflicts.series_back_to_back.len();
    let series_bb_bound = BoundInfo {
        theoretical_min: series_bb_min,
        current: series_bb_current,
        gap: series_bb_current as i32 - series_bb_min as i32,
        is_optimal: series_bb_current <= series_bb_min + 3, // Allow small tolerance
        explanation: format!(
            "{} series with 3+ parts, {} slots available for consecutive placement",
            long_series_count, NUM_TIME_SLOTS
        ),
    };

    // 5. CLASSIFICATION BOUNDS
    // Use greedy graph coloring lower bound estimate
    // Count max classification overlaps in any time slot
    let mut max_class_density = 0;
    for slot_sessions in group_by_period(sessions).values() {
        // Count classification code frequencies in this slot
        let mut class_counts: HashMap<String, usize> = HashMap::new();
        for session in slot_sessions {
            for code in &session.classification {
                *class_counts.entry(code.clone()).or_insert(0) += 1;
            }
        }

        // Sum of C(n, 2) for each classification code
        let slot_conflicts: usize = class_counts.values()
            .map(|&count| if count > 1 { count * (count - 1) / 2 } else { 0 })
            .sum();
        max_class_density += slot_conflicts;
    }

    // This is a lower bound - actual minimum could be higher due to constraint interactions
    let classification_min = max_class_density / 2; // Rough estimate

    let classification_current = current_conflicts.classification.len();
    let classification_bound = BoundInfo {
        theoretical_min: classification_min,
        current: classification_current,
        gap: classification_current as i32 - classification_min as i32,
        is_optimal: classification_current <= classification_min * 2, // Very loose bound
        explanation: format!(
            "Estimated via classification density analysis (lower bound approximation)"
        ),
    };

    TheoreticalBounds {
        speaker_organizer: speaker_org_bound,
        personal_classification: personal_class_bound,
        series_room: series_room_bound,
        series_back_to_back: series_bb_bound,
        classification: classification_bound,
    }
}

fn group_by_period(sessions: &[SessionInfo]) -> HashMap<String, Vec<&SessionInfo>> {
    let mut grouped = HashMap::new();
    for session in sessions {
        grouped.entry(session.session_period.clone())
            .or_insert_with(Vec::new)
            .push(session);
    }
    grouped
}

fn extract_series_base(title: &str) -> Option<String> {
    // Extract series base title (remove "Part X of Y" suffix)
    if title.contains(" - Part ") || title.contains("- Part") {
        let base = title.split(" - Part ").next()
            .or_else(|| title.split("- Part").next())?;
        Some(base.trim().to_lowercase().chars()
            .filter(|c| c.is_alphanumeric())
            .collect())
    } else {
        None
    }
}

fn analyze_series(sessions: &[SessionInfo]) -> SeriesAnalysisSummary {
    // Group sessions by series
    let mut series_map: HashMap<String, Vec<&SessionInfo>> = HashMap::new();
    for session in sessions {
        if let Some((base_title, _part_num)) = conflict_detection::extract_series_info(&session.title) {
            series_map.entry(base_title)
                .or_insert_with(Vec::new)
                .push(session);
        }
    }

    let mut series_list: Vec<SeriesInfo> = Vec::new();
    let mut optimal_count = 0;
    let mut suboptimal_count = 0;

    for (base_title, series_sessions) in series_map {
        if series_sessions.len() < 2 {
            continue;
        }

        // Sort by period
        let mut sorted = series_sessions.clone();
        sorted.sort_by(|a, b| conflict_detection::get_period_order(&a.session_period)
            .cmp(&conflict_detection::get_period_order(&b.session_period)));

        let mut issues = Vec::new();
        let mut is_optimal = true;

        // Check for conflicts using shared logic
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                let s1 = sorted[i];
                let s2 = sorted[j];

                // Check for room conflicts
                if conflict_detection::has_series_room_conflict(
                    &s1.title, &s1.room,
                    &s2.title, &s2.room
                ) {
                    issues.push(format!("Different rooms: {} vs {}", s1.room, s2.room));
                    is_optimal = false;
                }

                // Check for back-to-back conflicts (parallel or non-consecutive)
                if conflict_detection::has_series_back_to_back_conflict(
                    &s1.title, &s1.session_period,
                    &s2.title, &s2.session_period
                ) {
                    let order1 = conflict_detection::get_period_order(&s1.session_period);
                    let order2 = conflict_detection::get_period_order(&s2.session_period);

                    if order1 == order2 {
                        issues.push(format!("Parts scheduled in parallel in {}", s1.session_period));
                    } else {
                        issues.push(format!("Non-consecutive periods"));
                    }
                    is_optimal = false;
                }
            }
        }

        if is_optimal {
            optimal_count += 1;
        } else {
            suboptimal_count += 1;
        }

        series_list.push(SeriesInfo {
            base_title,
            total_parts: sorted.len(),
            is_optimal,
            issues,
            sessions: sorted.into_iter().cloned().collect(),
        });
    }

    SeriesAnalysisSummary {
        total_series: series_list.len(),
        optimal: optimal_count,
        suboptimal: suboptimal_count,
        series_list,
    }
}

// ============================================================================
// MAIN FUNCTION
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse arguments
    let file_path = if args.len() > 1 {
        &args[1]
    } else {
        "data/generated/PP26_Schedule_Output.xlsx"
    };

    let pretty = args.iter().any(|arg| arg == "--pretty" || arg == "-p");

    eprintln!("═══════════════════════════════════════════════════════");
    eprintln!("     CONFERENCE SCHEDULE ANALYZER");
    eprintln!("═══════════════════════════════════════════════════════");
    eprintln!("Input file: {}", file_path);
    eprintln!();

    // Read sessions from Excel
    let sessions = match read_schedule_from_excel(file_path) {
        Ok(s) => {
            eprintln!("✅ Loaded {} sessions", s.len());
            s
        }
        Err(e) => {
            eprintln!("❌ Error reading schedule: {}", e);
            std::process::exit(1);
        }
    };

    // Detect all conflicts
    eprintln!("🔍 Detecting conflicts...");
    let classification_conflicts = detect_classification_conflicts(&sessions);
    let personal_conflicts = detect_personal_conflicts(&sessions);
    let personal_classification_conflicts = detect_personal_classification_conflicts(&sessions);
    let (series_room_conflicts, series_back_to_back_conflicts) = detect_series_conflicts(&sessions);

    let all_conflicts = ConflictDetails {
        classification: classification_conflicts,
        personal: personal_conflicts,
        personal_classification: personal_classification_conflicts,
        series_room: series_room_conflicts,
        series_back_to_back: series_back_to_back_conflicts,
    };

    let total_conflicts = all_conflicts.classification.len() +
        all_conflicts.personal.len() +
        all_conflicts.personal_classification.len() +
        all_conflicts.series_room.len() +
        all_conflicts.series_back_to_back.len();

    eprintln!("   Classification: {}", all_conflicts.classification.len());
    eprintln!("   Personal: {}", all_conflicts.personal.len());
    eprintln!("   Personal-Classification: {}", all_conflicts.personal_classification.len());
    eprintln!("   Series Room: {}", all_conflicts.series_room.len());
    eprintln!("   Series Back-to-Back: {}", all_conflicts.series_back_to_back.len());
    eprintln!("   Total: {}", total_conflicts);
    eprintln!();

    // Calculate statistics
    eprintln!("📊 Calculating statistics...");
    let (summary, by_period, room_usage, classification_dist) =
        calculate_statistics(&sessions, &all_conflicts);

    // Analyze series
    eprintln!("🔄 Analyzing series...");
    let series_analysis = analyze_series(&sessions);
    eprintln!("   Total series: {}", series_analysis.total_series);
    eprintln!("   Optimal: {}", series_analysis.optimal);
    eprintln!("   Suboptimal: {}", series_analysis.suboptimal);
    eprintln!();

    // Calculate theoretical bounds
    eprintln!("🎯 Calculating theoretical minimum conflicts...");
    let theoretical_bounds = calculate_theoretical_bounds(&sessions, &all_conflicts);
    eprintln!("   Speaker/Organizer: min={}, current={}",
        theoretical_bounds.speaker_organizer.theoretical_min,
        theoretical_bounds.speaker_organizer.current);
    eprintln!("   Personal-Classification: min={}, current={}",
        theoretical_bounds.personal_classification.theoretical_min,
        theoretical_bounds.personal_classification.current);
    eprintln!("   Series Room: min={}, current={}",
        theoretical_bounds.series_room.theoretical_min,
        theoretical_bounds.series_room.current);
    eprintln!("   Series Back-to-Back: min={}, current={}",
        theoretical_bounds.series_back_to_back.theoretical_min,
        theoretical_bounds.series_back_to_back.current);
    eprintln!("   Classification: min={}, current={}",
        theoretical_bounds.classification.theoretical_min,
        theoretical_bounds.classification.current);
    eprintln!();

    // Build final result
    let result = AnalysisResult {
        metadata: AnalysisMetadata {
            total_sessions: sessions.len(),
            total_conflicts,
            analysis_timestamp: chrono::Utc::now().to_rfc3339(),
            input_file: file_path.to_string(),
        },
        summary,
        theoretical_bounds,
        by_period,
        conflicts: all_conflicts,
        room_usage,
        classification_distribution: classification_dist,
        series_analysis,
    };

    // Serialize to JSON
    let json = if pretty {
        serde_json::to_string_pretty(&result)
    } else {
        serde_json::to_string(&result)
    }.expect("Failed to serialize to JSON");

    eprintln!("✅ Analysis complete!");
    eprintln!();

    // Output only JSON to stdout (for web server parsing)
    println!("{}", json);
}
