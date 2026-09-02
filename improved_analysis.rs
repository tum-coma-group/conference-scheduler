// This file contains the improved analysis functions
// We'll replace the existing functions with these improved versions

use std::collections::HashMap;

fn truncate_title(title: &str, max_len: usize) -> String {
    if title.len() <= max_len {
        title.to_string()
    } else {
        format!("{}...", &title[..max_len.saturating_sub(3)])
    }
}

fn analyze_all_conflicts_improved(sessions: &[SessionInfo]) -> String {
    let mut result = String::new();

    // Header
    result.push_str("╔══════════════════════════════════════════════════════════════╗\n");
    result.push_str("║                    CONFERENCE SCHEDULE ANALYSIS                 ║\n");
    result.push_str("╚══════════════════════════════════════════════════════════════╝\n\n");

    result.push_str(&format!("📊 Schedule Overview: {} sessions total\n\n", sessions.len()));

    // Group sessions by time period
    let mut sessions_by_time: HashMap<String, Vec<&SessionInfo>> = HashMap::new();
    for session in sessions {
        sessions_by_time.entry(session.session_period.clone()).or_insert_with(Vec::new).push(session);
    }

    let mut total_conflicts = 0;
    let mut classification_conflicts = 0;
    let mut personal_conflicts = 0;
    let mut room_conflicts = 0;

    // Count conflicts first for summary
    for (_period, period_sessions) in &sessions_by_time {
        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let session1 = &period_sessions[i];
                let session2 = &period_sessions[j];

                if has_classification_conflict(session1, session2) {
                    classification_conflicts += 1;
                    total_conflicts += 1;
                }

                if has_personal_conflict(session1, session2) {
                    personal_conflicts += 1;
                    total_conflicts += 1;
                }

                if session1.room == session2.room {
                    room_conflicts += 1;
                    total_conflicts += 1;
                }
            }
        }
    }

    // Executive Summary
    result.push_str("┌─────────────────────────────────────────────────────────────┐\n");
    result.push_str("│                        EXECUTIVE SUMMARY                      │\n");
    result.push_str("└─────────────────────────────────────────────────────────────┘\n\n");

    result.push_str(&format!("  🎯 Total Conflicts Found: {}\n", total_conflicts));
    result.push_str(&format!("  📚 Classification Conflicts: {}\n", classification_conflicts));
    result.push_str(&format!("  👥 Personal Conflicts: {}\n", personal_conflicts));
    result.push_str(&format!("  🏠 Room Conflicts: {}\n\n", room_conflicts));

    // Conflict Rate
    let total_possible_pairs = sessions.iter().enumerate().map(|(i, _)| {
        sessions.iter().enumerate().skip(i + 1).filter(|(j, _)| {
            sessions[i].session_period == sessions[*j].session_period
        }).count()
    }).sum::<usize>();

    if total_possible_pairs > 0 {
        let conflict_rate = (total_conflicts as f64 / total_possible_pairs as f64) * 100.0;
        result.push_str(&format!("  📈 Overall Conflict Rate: {:.1}%\n", conflict_rate));
    }

    // Time Slot Analysis
    result.push_str("\n┌─────────────────────────────────────────────────────────────┐\n");
    result.push_str("│                       TIME SLOT ANALYSIS                       │\n");
    result.push_str("└─────────────────────────────────────────────────────────────┘\n\n");

    for (period, period_sessions) in &sessions_by_time {
        let period_name = period.replace(" Period", " ").trim();
        result.push_str(&format!("🕐 {} — {} sessions\n", period_name, period_sessions.len()));

        let mut conflicts_in_period = 0;
        let mut period_conflicts = Vec::new();

        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let session1 = &period_sessions[i];
                let session2 = &period_sessions[j];

                if has_classification_conflict(session1, session2) {
                    conflicts_in_period += 1;
                    period_conflicts.push(format!("  ⚠️  {} vs {} (same classification)",
                        truncate_title(&session1.title, 40),
                        truncate_title(&session2.title, 40)));
                }

                if has_personal_conflict(session1, session2) {
                    conflicts_in_period += 1;
                    let conflict_people = get_conflicting_people(session1, session2);
                    period_conflicts.push(format!("  👥 {} vs {} ({})",
                        truncate_title(&session1.title, 30),
                        truncate_title(&session2.title, 30),
                        conflict_people));
                }

                if session1.room == session2.room {
                    conflicts_in_period += 1;
                    period_conflicts.push(format!("  🏠 {} vs {} (both in room {})",
                        truncate_title(&session1.title, 30),
                        truncate_title(&session2.title, 30),
                        session1.room));
                }
            }
        }

        if conflicts_in_period > 0 {
            result.push_str(&format!("  ❌ {} conflicts found:\n", conflicts_in_period));
            for conflict in period_conflicts.iter().take(3) { // Show max 3 conflicts per period
                result.push_str(&format!("{}\n", conflict));
            }
            if period_conflicts.len() > 3 {
                result.push_str(&format!("  ... and {} more conflicts\n", period_conflicts.len() - 3));
            }
        } else {
            result.push_str("  ✅ No conflicts in this time slot\n");
        }
        result.push_str("\n");
    }

    // Room Usage Analysis
    result.push_str("┌─────────────────────────────────────────────────────────────┐\n");
    result.push_str("│                       ROOM USAGE ANALYSIS                        │\n");
    result.push_str("└─────────────────────────────────────────────────────────────┘\n\n");

    let mut room_counts: HashMap<String, usize> = HashMap::new();
    let mut room_time_slots: HashMap<String, Vec<String>> = HashMap::new();

    for session in sessions {
        *room_counts.entry(session.room.clone()).or_insert(0) += 1;
        room_time_slots.entry(session.room.clone()).or_insert_with(Vec::new).push(session.session_period.clone());
    }

    let mut sorted_rooms: Vec<_> = room_counts.iter().collect();
    sorted_rooms.sort_by(|a, b| a.0.cmp(b.0));

    for (room, count) in sorted_rooms {
        let time_slots: std::collections::HashSet<_> = room_time_slots[room].iter().collect();
        result.push_str(&format!("  🏠 Room {}: {} sessions ({:.0}% utilization)\n",
            room, count, (*count as f64 / sessions.len() as f64) * 100.0));
    }

    // Series Analysis
    result.push_str("\n┌─────────────────────────────────────────────────────────────┐\n");
    result.push_str("│                       SERIES ANALYSIS                           │\n");
    result.push_str("└─────────────────────────────────────────────────────────────┘\n\n");

    result.push_str(&analyze_all_series_formatted(sessions));

    // Footer
    result.push_str("\n╔══════════════════════════════════════════════════════════════╗\n");
    result.push_str("║                         ANALYSIS COMPLETE                        ║\n");
    result.push_str("╚══════════════════════════════════════════════════════════════╝\n");

    result
}

fn analyze_all_series_formatted(sessions: &[SessionInfo]) -> String {
    let series_list = detect_series(sessions);

    if series_list.is_empty() {
        return "  ℹ️  No multi-part series found in this schedule\n".to_string();
    }

    let mut result = String::new();
    let mut optimal_count = 0;
    let mut suboptimal_count = 0;

    result.push_str(&format!("  🔍 Found {} multi-part series\n\n", series_list.len()));

    // Show suboptimal series first (most important)
    let mut suboptimal_series = Vec::new();
    let mut optimal_series = Vec::new();

    for series in &series_list {
        if series.is_optimal {
            optimal_series.push(series);
        } else {
            suboptimal_series.push(series);
        }
    }

    if !suboptimal_series.is_empty() {
        result.push_str("  ⚠️  SERIES REQUIRING ATTENTION:\n");
        for series in suboptimal_series {
            suboptimal_count += 1;
            result.push_str(&format!("    • {} ({} parts)\n", truncate_title(&series.base_title, 50), series.total_parts));
            for issue in &series.issues {
                result.push_str(&format!("      - {}\n", issue));
            }
        }
        result.push_str("\n");
    }

    if !optimal_series.is_empty() {
        result.push_str(&format!("  ✅ OPTIMAL SERIES ({}) - well scheduled\n", optimal_series.len()));
        for series in optimal_series.iter().take(3) { // Show max 3 optimal series
            optimal_count += 1;
            result.push_str(&format!("    • {} ({} parts)\n", truncate_title(&series.base_title, 50), series.total_parts));
        }
        if optimal_series.len() > 3 {
            result.push_str(&format!("    ... and {} more optimal series\n", optimal_series.len() - 3));
        }
        result.push_str("\n");
    }

    result.push_str(&format!("  📊 Series Summary: {:.1}% optimal, {:.1}% need improvement\n",
        (optimal_count as f64 / series_list.len() as f64) * 100.0,
        (suboptimal_count as f64 / series_list.len() as f64) * 100.0));

    result
}