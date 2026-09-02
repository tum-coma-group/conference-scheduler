//! Shared Conflict Detection Module
//!
//! Provides unified conflict detection logic used by both:
//! - analyzer binary (for post-generation analysis)
//! - test_hierarchical (for real-time optimization during scheduling)
//!
//! This ensures both systems count conflicts consistently using the same algorithms.

use std::collections::{HashMap, HashSet};

// ============================================================================
// NORMALIZATION HELPERS
// ============================================================================

/// Normalize text for exact matching: remove ALL spaces and convert to lowercase
/// Used for both person names and notes keywords
pub fn normalize_exact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

// ============================================================================
// CLASSIFICATION CONFLICTS
// ============================================================================

/// Check if two sessions have classification conflicts
/// Returns true if they share ANY classification code
/// Note: CL groups are already combined into single sessions, so internal
/// CL lecture conflicts are never checked (as intended)
pub fn has_classification_conflict(
    classification1: &[String],
    classification2: &[String],
) -> bool {
    for code1 in classification1 {
        for code2 in classification2 {
            if code1 == code2 {
                return true;
            }
        }
    }
    false
}

// ============================================================================
// SPEAKER/ORGANIZER CONFLICTS
// ============================================================================

/// Check if two sessions have speaker/organizer conflicts
/// Returns true if ANY person appears in both sessions (exact match only)
/// Normalization: remove ALL spaces, lowercase
pub fn has_speaker_organizer_conflict(
    speakers1: &[String],
    organizers1: &[String],
    speakers2: &[String],
    organizers2: &[String],
) -> bool {
    // Combine speakers and organizers for both sessions
    let mut people1 = speakers1.to_vec();
    people1.extend(organizers1.iter().cloned());

    let mut people2 = speakers2.to_vec();
    people2.extend(organizers2.iter().cloned());

    // Normalize and check for exact matches
    let normalized1: Vec<_> = people1.iter()
        .map(|p| normalize_exact(p))
        .filter(|p| !p.is_empty())
        .collect();

    let normalized2: Vec<_> = people2.iter()
        .map(|p| normalize_exact(p))
        .filter(|p| !p.is_empty())
        .collect();

    // Check for any exact match
    for n1 in &normalized1 {
        for n2 in &normalized2 {
            if n1 == n2 {
                return true;
            }
        }
    }

    false
}

// ============================================================================
// PERSONAL-CLASSIFICATION CONFLICTS
// ============================================================================

/// Check if two sessions have personal-classification conflicts
/// Returns true if notes keywords match exactly (after normalization)
/// Normalization: remove ALL spaces, lowercase
/// NO substring matching
pub fn has_personal_classification_conflict(notes1: &str, notes2: &str) -> bool {
    let norm1 = normalize_exact(notes1);
    let norm2 = normalize_exact(notes2);

    // Skip if either notes is empty
    if norm1.is_empty() || norm2.is_empty() {
        return false;
    }

    // Skip header keywords
    if norm1.contains("notes") || norm2.contains("notes") {
        return false;
    }

    // Exact match only (no substring)
    norm1 == norm2
}

// ============================================================================
// TOPIC SIMILARITY (for CL grouping)
// ============================================================================

/// Calculate topic similarity between two sessions (for TBD grouping and CL clustering)
pub fn calculate_topic_similarity(notes1: &str, notes2: &str) -> f64 {
    let words1: HashSet<String> = notes1
        .split_whitespace()
        .map(|w| w.to_lowercase().trim_matches(&['.', ',', ';', ':', '!', '?'][..]).to_string())
        .filter(|w| w.len() > 3)
        .collect();

    let words2: HashSet<String> = notes2
        .split_whitespace()
        .map(|w| w.to_lowercase().trim_matches(&['.', ',', ';', ':', '!', '?'][..]).to_string())
        .filter(|w| w.len() > 3)
        .collect();

    if words1.is_empty() || words2.is_empty() {
        return 0.0;
    }

    let intersection = words1.intersection(&words2).count();
    let union = words1.union(&words2).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// ============================================================================
// SERIES DETECTION HELPERS
// ============================================================================

/// Extract series information from title (base title and part number)
pub fn extract_series_info(title: &str) -> Option<(String, usize)> {
    // Look for patterns like "Part I", "Part II", "Part 1", "Part 2", etc.
    let patterns = vec![
        (r" - Part ([IVXLCDM]+) of ([IVXLCDM]+)", true),
        (r" - Part (\d+) of (\d+)", false),
        (r" - Part ([IVXLCDM]+)$", true),
        (r" - Part (\d+)$", false),
        (r"Part ([IVXLCDM]+) of ([IVXLCDM]+)", true),
        (r"Part (\d+) of (\d+)", false),
        (r"Part ([IVXLCDM]+)$", true),
        (r"Part (\d+)$", false),
    ];

    for (pattern, is_roman) in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(title) {
                if caps.len() >= 2 {
                    let part_str = caps.get(1).unwrap().as_str();
                    let part_num = if is_roman {
                        roman_to_int(part_str)?
                    } else {
                        part_str.parse().ok()?
                    };

                    let base_title = title.replace(caps.get(0).unwrap().as_str(), "")
                        .trim()
                        .to_string();

                    if part_num > 0 && !base_title.is_empty() {
                        // Normalize the base title: remove spaces, lowercase
                        // This ensures consistent series grouping regardless of formatting
                        let normalized_title = normalize_exact(&base_title);
                        return Some((normalized_title, part_num));
                    }
                }
            }
        }
    }

    None
}

/// Convert Roman numeral to integer
fn roman_to_int(roman: &str) -> Option<usize> {
    let roman = roman.to_uppercase();
    let mut total = 0;
    let mut prev_value = 0;

    let roman_values = HashMap::from([
        ('I', 1), ('V', 5), ('X', 10), ('L', 50),
        ('C', 100), ('D', 500), ('M', 1000)
    ]);

    for ch in roman.chars().rev() {
        if let Some(&value) = roman_values.get(&ch) {
            if value < prev_value {
                total -= value;
            } else {
                total += value;
            }
            prev_value = value;
        } else {
            return None;
        }
    }

    Some(total)
}

/// Get the chronological order of a time period (0-7)
pub fn get_period_order(period: &str) -> usize {
    if period.contains("TUE First") { return 0; }
    if period.contains("TUE Second") { return 1; }
    if period.contains("WED First") { return 2; }
    if period.contains("WED Second") { return 3; }
    if period.contains("THU First") { return 4; }
    if period.contains("THU Second") { return 5; }
    if period.contains("FRI First") { return 6; }
    if period.contains("FRI Second") { return 7; }
    999 // Unknown
}

// ============================================================================
// SERIES CONFLICTS
// ============================================================================

/// Check if two sessions are part of the same series
pub fn is_series_sibling(title1: &str, title2: &str) -> bool {
    let info1 = extract_series_info(title1);
    let info2 = extract_series_info(title2);

    match (info1, info2) {
        (Some((base1, _)), Some((base2, _))) => base1 == base2,
        _ => false,
    }
}

/// Check if series parts are in different rooms (series room conflict)
pub fn has_series_room_conflict(
    title1: &str,
    room1: &str,
    title2: &str,
    room2: &str,
) -> bool {
    if !is_series_sibling(title1, title2) {
        return false;
    }

    room1 != room2
}

/// Check if series parts have back-to-back conflicts:
/// - Parallel: scheduled in same period (bad)
/// - Non-consecutive: consecutive parts (e.g., Part I and Part II) are not in adjacent periods (bad)
///
/// NOTE: This should only check CONSECUTIVE parts (Part I→II, II→III),
/// NOT all combinations (e.g., Part I vs Part III is fine if Part II is between them)
pub fn has_series_back_to_back_conflict(
    title1: &str,
    period1: &str,
    title2: &str,
    period2: &str,
) -> bool {
    // First check if they're part of the same series
    let info1 = extract_series_info(title1);
    let info2 = extract_series_info(title2);

    match (info1, info2) {
        (Some((base1, part1)), Some((base2, part2))) => {
            // Must be same series
            if base1 != base2 {
                return false;
            }

            // Only check CONSECUTIVE parts
            let part_diff = (part1 as i32 - part2 as i32).abs();
            if part_diff != 1 {
                // Not consecutive parts, so no conflict to check
                return false;
            }

            let order1 = get_period_order(period1);
            let order2 = get_period_order(period2);

            // Parallel conflict: same period
            if order1 == order2 {
                return true;
            }

            // Timing conflict: consecutive parts should be in adjacent periods
            let period_diff = (order1 as i32 - order2 as i32).abs();
            period_diff != 1
        }
        _ => false,
    }
}

// ============================================================================
// SERIES CHRONOLOGICAL ORDER VALIDATION (Hard Constraint)
// ============================================================================

/// Check if ANY part of a series appears BEFORE an earlier part in the schedule.
/// This is a HARD CONSTRAINT - the final schedule must never violate this.
///
/// Example violations:
/// - Part II scheduled before Part I
/// - Part III scheduled before Part II
///
/// Returns true if chronological order is violated (bad!)
pub fn has_series_chronological_violation(
    title1: &str,
    period1: &str,
    title2: &str,
    period2: &str,
) -> bool {
    let info1 = extract_series_info(title1);
    let info2 = extract_series_info(title2);

    match (info1, info2) {
        (Some((base1, part1)), Some((base2, part2))) => {
            // Must be same series
            if base1 != base2 {
                return false;
            }

            let order1 = get_period_order(period1);
            let order2 = get_period_order(period2);

            // Violation: Later part (higher number) appears earlier in time (lower order)
            // E.g., Part II (part2=2) appears in TUE First (order=0),
            //       but Part I (part1=1) appears in TUE Second (order=1)
            //       This is WRONG - Part I should come before Part II!
            if part1 < part2 && order1 > order2 {
                return true; // Part1 is earlier in series but later in time - VIOLATION!
            }
            if part2 < part1 && order2 > order1 {
                return true; // Part2 is earlier in series but later in time - VIOLATION!
            }

            false // Chronological order is preserved
        }
        _ => false,
    }
}
