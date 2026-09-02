use std::fs;
use calamine::{open_workbook, Reader, Xlsx};

#[derive(Debug, Clone)]
struct SessionInfo {
    id: String,
    session_period: String,
    room: String,
    title: String,
    classification: String,
    organizers: String,
    speakers: String,
}

fn has_classification_conflict(session1: &SessionInfo, session2: &SessionInfo) -> bool {
    if session1.classification.is_empty() || session2.classification.is_empty() {
        return false;
    }

    // Split classification numbers by semicolon (;) and parse as strings
    let classifications1: Vec<String> = session1.classification
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .collect();

    let classifications2: Vec<String> = session2.classification
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .collect();

    // Check for identical classification numbers (competing sessions)
    for class1 in &classifications1 {
        for class2 in &classifications2 {
            if class1 == class2 {
                println!("  🎯 Found matching classification: {}", class1);
                return true; // Same classification number = conflict (competing for same audience)
            }
        }
    }

    false
}

fn analyze_test_schedule(file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut workbook: Xlsx<_> = open_workbook(file_path)?;
    let sheet = workbook.worksheet_range("Schedule")?;

    // Parse the schedule data
    let mut sessions: Vec<SessionInfo> = Vec::new();
    let mut current_session: Option<String> = None;

    for (row_idx, row) in sheet.rows().enumerate() {
        if row_idx == 0 {
            continue; // Skip header row
        }

        // Get session name from column 0
        let session_str = row.get(0)
            .map(|c| c.to_string())
            .unwrap_or_default();

        // Detect session transitions
        if session_str.contains("First Period") || session_str.contains("Second Period") {
            current_session = Some(session_str.clone());
        }

        // Get ID from column 2
        let id_cell = row.get(2).map(|c| c.to_string()).unwrap_or_default();
        if id_cell.is_empty() || current_session.is_none() {
            continue;
        }

        // Extract session information
        let room = row.get(1).map(|c| c.to_string()).unwrap_or_default();
        let title = row.get(3).map(|c| c.to_string()).unwrap_or_default();
        let classification = row.get(4).map(|c| c.to_string()).unwrap_or_default();
        let organizers = row.get(5).map(|c| c.to_string()).unwrap_or_default();
        let speakers = row.get(6).map(|c| c.to_string()).unwrap_or_default();

        sessions.push(SessionInfo {
            id: id_cell,
            session_period: current_session.clone().unwrap(),
            room,
            title,
            classification,
            organizers,
            speakers,
        });
    }

    if sessions.is_empty() {
        return Err("No valid sessions found in the schedule file".into());
    }

    println!("📊 Total sessions parsed: {}", sessions.len());

    // Show sample classifications
    println!("\n📋 Sample classification data:");
    for (i, session) in sessions.iter().take(5).enumerate() {
        println!("  {}. {} → {}", i+1, session.title, session.classification);
    }

    // Group sessions by time period
    let mut sessions_by_time: std::collections::HashMap<String, Vec<&SessionInfo>> = std::collections::HashMap::new();
    for session in &sessions {
        sessions_by_time.entry(session.session_period.clone()).or_insert_with(Vec::new).push(session);
    }

    let mut total_classification_conflicts = 0;

    println!("\n==================================================");
    println!("              CLASSIFICATION CONFLICT ANALYSIS");
    println!("==================================================");

    // Analyze each time period
    for (period, period_sessions) in &sessions_by_time {
        println!("\n🕐 {} - {} sessions", period, period_sessions.len());

        let mut conflicts_in_period = 0;

        // Check all pairs of sessions in this time period
        for i in 0..period_sessions.len() {
            for j in (i + 1)..period_sessions.len() {
                let session1 = &period_sessions[i];
                let session2 = &period_sessions[j];

                // Check for classification conflicts
                if has_classification_conflict(session1, session2) {
                    conflicts_in_period += 1;
                    total_classification_conflicts += 1;

                    println!("  ⚠️  Classification Conflict:");
                    println!("      Session 1: {} ({})", session1.title, session1.classification);
                    println!("      Session 2: {} ({})", session2.title, session2.classification);
                }
            }
        }

        if conflicts_in_period == 0 {
            println!("  ✅ No classification conflicts in this period");
        } else {
            println!("  📊 Period conflicts: {}", conflicts_in_period);
        }
    }

    let result = format!(
        "\n==================================================\n✅ Analysis Complete!\nTotal classification conflicts: {}\n==================================================",
        total_classification_conflicts
    );

    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Classification Conflict Logic");
    println!("=====================================");

    let file_path = "data/uploads/modified_schedule_1760831662.xlsx";
    let analysis = analyze_test_schedule(file_path)?;
    println!("{}", analysis);

    Ok(())
}