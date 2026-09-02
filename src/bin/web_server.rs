use axum::{
    extract::{Multipart, Form, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
    http::{StatusCode, header},
    body::Body,
};
use std::fs;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use serde::{Deserialize, Serialize};

// Note: Old constraint imports removed - now using priority-based configuration
// use conference_scheduler::constraints::{ConstraintConfig};
// use conference_scheduler::ConstraintType;

// ============================================================================
// PATH CONFIGURATION - Centralized path management for dev/prod environments
// ============================================================================

#[derive(Clone)]
struct PathConfig {
    uploads_dir: String,
    generated_dir: String,
    analyzer_bin: String,
    scheduler_bin: String,
    is_dev: bool,
}

impl PathConfig {
    fn new() -> Self {
        // Detect environment: check if we're in development or production
        let is_dev = std::path::Path::new("./target/release").exists();

        println!("🔧 Environment detected: {}", if is_dev { "Development" } else { "Production" });

        let config = PathConfig {
            uploads_dir: if is_dev {
                "data/uploads".to_string()
            } else {
                "uploads".to_string()
            },
            generated_dir: if is_dev {
                "data/generated".to_string()
            } else {
                "generated".to_string()
            },
            analyzer_bin: if is_dev {
                "./target/release/analyzer".to_string()
            } else {
                "./analyzer".to_string()
            },
            scheduler_bin: if is_dev {
                "./target/release/test_hierarchical".to_string()
            } else {
                "./test_hierarchical".to_string()
            },
            is_dev,
        };

        println!("📂 Uploads directory: {}", config.uploads_dir);
        println!("📂 Generated directory: {}", config.generated_dir);
        println!("🔨 Scheduler binary: {}", config.scheduler_bin);
        println!("📊 Analyzer binary: {}", config.analyzer_bin);

        config
    }

    fn ensure_directories(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.uploads_dir)?;
        fs::create_dir_all(&self.generated_dir)?;
        println!("✅ Directories created/verified");
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    println!("🚀 Starting Conference Scheduler Web Server...");

    // Initialize path configuration
    let path_config = Arc::new(PathConfig::new());
    path_config.ensure_directories().expect("Failed to create directories");

    // Build our application with routes and shared state
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/process", post(process_handler))
        .route("/regenerate", post(regenerate_handler))
        .route("/download/:filename", get(download_handler))
        .route("/view/:filename", get(view_schedule_handler))
        .route("/api/schedule/:filename", get(get_schedule_json))
        .route("/api/save-modified", post(save_modified_schedule))
        .route("/api/analyze", post(analyze_schedule))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(path_config);

    let addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");

    println!("✅ Server running at http://{}", addr);
    println!("📂 Conference Scheduler Web Interface");

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}

async fn index_handler() -> Html<&'static str> {
    let html = include_str!("../../templates/index.html");
    Html(html)
}

async fn process_handler(
    State(config): State<Arc<PathConfig>>,
    mut multipart: Multipart
) -> Response {
    println!("\n📨 ========== NEW REQUEST RECEIVED ==========");

    let mut mode = String::from("analyze");
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name = String::from("upload.xlsx");

    // Parse multipart data
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "mode" => {
                mode = field.text().await.unwrap_or(String::from("analyze"));
            }
            "file" => {
                file_name = field.file_name().unwrap_or("upload.xlsx").to_string();
                file_data = Some(field.bytes().await.unwrap_or_default().to_vec());
            }
            _ => {}
        }
    }

    let Some(data) = file_data else {
        println!("❌ No file data received");
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": "No file uploaded"
            }).to_string()
        ).into_response();
    };

    println!("📁 Mode: {}", mode);
    println!("📁 File: {} ({} bytes)", file_name, data.len());

    // Validate file format
    if !file_name.ends_with(".xlsx") && !file_name.ends_with(".xls") {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": "Invalid file format. Please upload an Excel file (.xlsx or .xls)"
            }).to_string()
        ).into_response();
    }

    // Save file with unique timestamp to preserve uploads
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let unique_file_name = format!("{}__{}", timestamp, file_name);
    let file_path = format!("{}/{}", config.uploads_dir, unique_file_name);
    if let Err(e) = fs::write(&file_path, &data) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": format!("Failed to save file: {}", e)
            }).to_string()
        ).into_response();
    }

    // Process based on mode
    match mode.as_str() {
        "analyze" => {
            // Validate schedule format
            match validate_schedule_format(&file_path) {
                Ok(_) => {},
                Err(e) => {
                    let _ = fs::remove_file(&file_path);
                    return (
                        StatusCode::BAD_REQUEST,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "error": format!("Invalid schedule format: {}", e)
                        }).to_string()
                    ).into_response();
                }
            }

            // Analyze the schedule
            match analyze_uploaded_schedule(&file_path, &config) {
                Ok(analysis) => {
                    // File is preserved in uploads directory for user access
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "analysis": analysis,
                            "file_saved": unique_file_name
                        }).to_string()
                    ).into_response()
                }
                Err(e) => {
                    // Keep file even if analysis fails for debugging purposes
                    (
                        StatusCode::BAD_REQUEST,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "error": format!("Analysis failed: {}", e)
                        }).to_string()
                    ).into_response()
                }
            }
        }
        "generate" => {
            println!("🔄 Starting schedule generation...");

            // Validate input data format
            println!("📋 Validating input file format...");
            match validate_input_format(&file_path) {
                Ok(_) => {
                    println!("✅ Input file format validated successfully");
                },
                Err(e) => {
                    println!("❌ Input validation failed: {}", e);
                    let _ = fs::remove_file(&file_path);
                    return (
                        StatusCode::BAD_REQUEST,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "error": format!("Invalid input format: {}", e)
                        }).to_string()
                    ).into_response();
                }
            }

            // Generate schedule and save to file
            println!("🚀 Calling scheduler to generate schedule...");
            match generate_schedule_direct(&file_path, &config) {
                Ok(file_data) => {
                    println!("✅ Scheduler completed successfully, file data size: {} bytes", file_data.len());
                    let _ = fs::remove_file(&file_path);
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let filename = format!("schedule_{}.xlsx", timestamp);
                    let output_path = format!("{}/{}", config.generated_dir, filename);

                    // Save the generated file
                    println!("💾 Saving generated schedule to: {}", output_path);
                    if let Err(e) = fs::write(&output_path, &file_data) {
                        println!("❌ Failed to save file: {}", e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            [(header::CONTENT_TYPE, "application/json")],
                            serde_json::json!({
                                "error": format!("Failed to save generated schedule: {}", e)
                            }).to_string()
                        ).into_response();
                    }

                    println!("🎉 Schedule generation complete! File: {}", filename);
                    println!("========================================\n");

                    // Return JSON with filename for viewing
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "success": true,
                            "filename": filename,
                            "message": "Schedule generated successfully! Redirecting to view..."
                        }).to_string()
                    ).into_response()
                }
                Err(e) => {
                    println!("❌ Schedule generation failed: {}", e);
                    println!("========================================\n");
                    let _ = fs::remove_file(&file_path);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [(header::CONTENT_TYPE, "application/json")],
                        serde_json::json!({
                            "error": format!("Schedule generation failed: {}", e)
                        }).to_string()
                    ).into_response()
                }
            }
        }
        _ => {
            let _ = fs::remove_file(&file_path);
            (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": "Invalid mode"
                }).to_string()
            ).into_response()
        }
    }
}

// ============================================================================
// ANALYSIS ENHANCEMENT STRUCTURES (for enhanced downloads)
// ============================================================================

#[derive(Deserialize)]
struct AnalysisResult {
    metadata: AnalysisMetadata,
    summary: ConflictSummaryStats,
}

#[derive(Deserialize)]
struct AnalysisMetadata {
    total_sessions: usize,
    total_conflicts: usize,
}

#[derive(Deserialize)]
struct ConflictSummaryStats {
    classification: ConflictSummary,
    personal: ConflictSummary,
    personal_classification: ConflictSummary,
    series_room: ConflictSummary,
    series_back_to_back: ConflictSummary,
}

#[derive(Deserialize)]
struct ConflictSummary {
    count: usize,
}

/// Run analyzer and get analysis JSON
fn run_analyzer(file_path: &str, config: &PathConfig) -> Result<String, String> {
    let output = std::process::Command::new(&config.analyzer_bin)
        .arg(file_path)
        .output()
        .map_err(|e| format!("Failed to run analyzer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Analyzer failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Extract JSON part (after "✅ Analysis complete!" marker)
    let json_start = if let Some(pos) = stdout.find("✅ Analysis complete!") {
        if let Some(newline_pos) = stdout[pos..].find('\n') {
            pos + newline_pos + 1
        } else {
            pos
        }
    } else {
        0
    };

    let json_output = stdout[json_start..].trim();
    Ok(json_output.to_string())
}

/// Write session data from JSON to Excel with formatting matching original scheduler
fn write_sessions_to_excel(file_path: &str, sessions: &[serde_json::Value]) -> Result<(), String> {
    use rust_xlsxwriter::{Workbook, Format, FormatAlign, Color};

    if sessions.is_empty() {
        return Err("No sessions provided".to_string());
    }

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    worksheet.set_name("Schedule")
        .map_err(|e| format!("Failed to set sheet name: {}", e))?;

    // Create formats matching original
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
    worksheet.set_column_width(0, 18).map_err(|e| format!("Failed to set column width: {}", e))?;  // Session
    worksheet.set_column_width(1, 12).map_err(|e| format!("Failed to set column width: {}", e))?;  // Room
    worksheet.set_column_width(2, 12).map_err(|e| format!("Failed to set column width: {}", e))?;  // ID
    worksheet.set_column_width(3, 60).map_err(|e| format!("Failed to set column width: {}", e))?;  // Title
    worksheet.set_column_width(4, 30).map_err(|e| format!("Failed to set column width: {}", e))?;  // Classification
    worksheet.set_column_width(5, 40).map_err(|e| format!("Failed to set column width: {}", e))?;  // Organizers
    worksheet.set_column_width(6, 40).map_err(|e| format!("Failed to set column width: {}", e))?;  // Speakers
    worksheet.set_column_width(7, 30).map_err(|e| format!("Failed to set column width: {}", e))?;  // Notes

    // Write headers
    worksheet.write_with_format(0, 0, "Session", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 1, "Room", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 2, "ID", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 3, "Title", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 4, "Classification", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 5, "Organizers", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 6, "Speakers", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;
    worksheet.write_with_format(0, 7, "Notes", &header_format)
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Group sessions by their Session field (TUE First Period, etc.)
    let mut grouped_sessions: std::collections::HashMap<String, Vec<&serde_json::Value>> = std::collections::HashMap::new();
    for session in sessions {
        if let Some(session_obj) = session.as_object() {
            let session_name = session_obj.get("Session")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            grouped_sessions.entry(session_name).or_insert_with(Vec::new).push(session);
        }
    }

    // Define session order (chronological, not alphabetical)
    let session_order = [
        "TUE First Period", "TUE Second Period",
        "WED First Period", "WED Second Period",
        "THU First Period", "THU Second Period",
        "FRI First Period", "FRI Second Period",
    ];

    let mut row: u32 = 1;

    // Write each session group in chronological order
    for session_name in session_order.iter() {
        let session_group = match grouped_sessions.get(*session_name) {
            Some(group) => group,
            None => continue,
        };
        if session_group.is_empty() {
            continue;
        }

        // Sort sessions by room number (numeric sort)
        let mut sorted_group = session_group.clone();
        sorted_group.sort_by(|a, b| {
            let room_a = a.as_object()
                .and_then(|obj| obj.get("Room"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(9999);
            let room_b = b.as_object()
                .and_then(|obj| obj.get("Room"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(9999);
            room_a.cmp(&room_b)
        });

        // Write session header row (light blue background)
        worksheet.write_with_format(row, 0, *session_name, &session_header_format)
            .map_err(|e| format!("Failed to write session header: {}", e))?;
        worksheet.write_with_format(row, 1, format!("({} rooms)", sorted_group.len()), &session_header_format)
            .map_err(|e| format!("Failed to write room count: {}", e))?;
        row += 1;

        // Write each session in this group
        for session in sorted_group {
            let session_obj = session.as_object()
                .ok_or("Session is not a valid object")?;

            let room = session_obj.get("Room")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id = session_obj.get("ID")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let title = session_obj.get("Title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let classification = session_obj.get("Classification")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let organizers = session_obj.get("Organizers")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let speakers = session_obj.get("Speakers")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let notes = session_obj.get("Notes")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            worksheet.write(row, 0, *session_name)
                .map_err(|e| format!("Failed to write session: {}", e))?;
            worksheet.write_with_format(row, 1, room, &id_format)
                .map_err(|e| format!("Failed to write room: {}", e))?;
            worksheet.write_with_format(row, 2, id, &id_format)
                .map_err(|e| format!("Failed to write ID: {}", e))?;
            worksheet.write_with_format(row, 3, title, &title_format)
                .map_err(|e| format!("Failed to write title: {}", e))?;
            worksheet.write(row, 4, classification)
                .map_err(|e| format!("Failed to write classification: {}", e))?;
            worksheet.write_with_format(row, 5, organizers, &title_format)
                .map_err(|e| format!("Failed to write organizers: {}", e))?;
            worksheet.write_with_format(row, 6, speakers, &title_format)
                .map_err(|e| format!("Failed to write speakers: {}", e))?;
            worksheet.write_with_format(row, 7, notes, &title_format)
                .map_err(|e| format!("Failed to write notes: {}", e))?;

            row += 1;
        }

        // Add blank row between session groups
        row += 1;
    }

    // Save workbook
    workbook.save(file_path)
        .map_err(|e| format!("Failed to save workbook: {}", e))?;

    Ok(())
}

/// Create enhanced Excel with analysis appended after schedule data
async fn create_enhanced_excel(file_path: &str, config: &PathConfig) -> Result<Vec<u8>, String> {
    use umya_spreadsheet::*;

    // Step 1: Run analyzer to get analysis data
    let json_output = run_analyzer(file_path, config)?;

    // Step 2: Parse analysis JSON
    let analysis: AnalysisResult = serde_json::from_str(&json_output)
        .map_err(|e| format!("Failed to parse analysis JSON: {}", e))?;

    // Step 3: Load original Excel file
    let mut book = reader::xlsx::read(file_path)
        .map_err(|e| format!("Failed to read Excel file: {}", e))?;

    // Step 4: Get the Schedule sheet
    let sheet = book.get_sheet_by_name_mut("Schedule")
        .ok_or("Schedule sheet not found")?;

    // Step 5: Find the last row with data
    // Note: We need to scan all rows, not break on blank rows, because we have blank rows between session groups
    let mut last_row = 0;
    for row_idx in 1..1000 {
        if let Some(cell) = sheet.get_cell((1, row_idx)) {
            if !cell.get_value().is_empty() {
                last_row = row_idx;
            }
        }
        // Don't break on None - keep scanning for more data rows
    }

    // Step 6: Append analysis summary after schedule (with blank row separator)
    let start_row = last_row + 3;

    // Add header
    sheet.get_cell_mut((1, start_row))
        .set_value("CONFLICT ANALYSIS SUMMARY");

    // Add separator
    sheet.get_cell_mut((1, start_row + 1))
        .set_value("─────────────────────────────────");

    // Add total sessions and conflicts
    sheet.get_cell_mut((1, start_row + 2))
        .set_value(format!("Total Sessions: {}", analysis.metadata.total_sessions));

    sheet.get_cell_mut((1, start_row + 3))
        .set_value(format!("Total Conflicts: {}", analysis.metadata.total_conflicts));

    // Add blank row
    sheet.get_cell_mut((1, start_row + 4))
        .set_value("");

    // Add conflict breakdown
    sheet.get_cell_mut((1, start_row + 5))
        .set_value("Conflict Breakdown:");

    sheet.get_cell_mut((1, start_row + 6))
        .set_value(format!("  • Classification Conflicts: {}", analysis.summary.classification.count));

    sheet.get_cell_mut((1, start_row + 7))
        .set_value(format!("  • Personal Conflicts: {}", analysis.summary.personal.count));

    sheet.get_cell_mut((1, start_row + 8))
        .set_value(format!("  • Personal-Classification Conflicts: {}", analysis.summary.personal_classification.count));

    sheet.get_cell_mut((1, start_row + 9))
        .set_value(format!("  • Series Room Conflicts: {}", analysis.summary.series_room.count));

    sheet.get_cell_mut((1, start_row + 10))
        .set_value(format!("  • Series Back-to-Back Conflicts: {}", analysis.summary.series_back_to_back.count));

    // Step 7: Write to temporary file and read back as bytes
    let temp_path = format!("{}.enhanced.tmp", file_path);
    writer::xlsx::write(&book, &temp_path)
        .map_err(|e| format!("Failed to write Excel: {}", e))?;

    let buf = fs::read(&temp_path)
        .map_err(|e| format!("Failed to read temp file: {}", e))?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    Ok(buf)
}

async fn download_handler(
    State(config): State<Arc<PathConfig>>,
    axum::extract::Path(filename): axum::extract::Path<String>
) -> Response {
    let file_path = format!("{}/{}", config.generated_dir, filename);

    // Check if file exists
    if !std::path::Path::new(&file_path).exists() {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }

    // Generate enhanced Excel with analysis appended
    match create_enhanced_excel(&file_path, &config).await {
        Ok(enhanced_data) => {
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
                    (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", filename)),
                ],
                Body::from(enhanced_data)
            ).into_response()
        }
        Err(e) => {
            eprintln!("❌ Failed to create enhanced Excel: {}", e);
            // Fallback to original file if enhancement fails
            match File::open(&file_path).await {
                Ok(file) => {
                    let stream = ReaderStream::new(file);
                    let body = Body::from_stream(stream);
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
                            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", filename)),
                        ],
                        body
                    ).into_response()
                }
                Err(_) => {
                    (StatusCode::NOT_FOUND, "File not found").into_response()
                }
            }
        }
    }
}

fn validate_schedule_format(file_path: &str) -> Result<(), String> {
    use calamine::{open_workbook, Reader, Xlsx};

    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| format!("Failed to open Excel file: {}", e))?;

    // Check if file has exactly one sheet
    let sheet_names = workbook.sheet_names();
    if sheet_names.len() != 1 {
        return Err("File must contain exactly one sheet (no multiple sheets)".to_string());
    }

    let range = workbook.worksheet_range(&sheet_names[0])
        .map_err(|_| "Failed to read the sheet")?;

    // Check if sheet has data
    if range.rows().count() < 2 {
        return Err("Sheet is empty or has no data rows".to_string());
    }

    // Validate header row (first row should have required columns)
    if let Some(header) = range.rows().next() {
        let header_str: Vec<String> = header.iter()
            .map(|c| c.to_string().trim().to_lowercase())
            .collect();

        let required_columns = vec!["session", "room", "id", "title", "classification", "organizers", "speakers"];
        let missing_columns: Vec<String> = required_columns.iter()
            .filter(|&col| !header_str.iter().any(|h| h.contains(col)))
            .map(|s| s.to_string())
            .collect();

        if !missing_columns.is_empty() {
            return Err(format!("Missing required columns: {}. Found: {}",
                missing_columns.join(", "), header_str.join(", ")));
        }
    }

    Ok(())
}

fn validate_input_format(file_path: &str) -> Result<(), String> {
    use calamine::{open_workbook, Reader, Xlsx};

    let workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| format!("Failed to open Excel file: {}", e))?;

    let sheet_names = workbook.sheet_names();

    // Check if required sheets exist
    if !sheet_names.contains(&"Minisymposia".to_string()) {
        return Err("File must contain a 'Minisymposia' sheet".to_string());
    }

    if !sheet_names.contains(&"Contributed Lectures".to_string()) {
        return Err("File must contain a 'Contributed Lectures' sheet".to_string());
    }

    Ok(())
}

fn analyze_uploaded_schedule(file_path: &str, config: &PathConfig) -> Result<String, Box<dyn std::error::Error>> {
    println!("📊 ANALYZING SCHEDULE FROM: {}", file_path);

    // Validate that this is a proper schedule file (not a random Excel file)
    if let Err(e) = validate_schedule_format(file_path) {
        println!("❌ Schedule format validation failed: {}", e);
        return Err(format!("Invalid schedule format: {}", e).into());
    }

    println!("✅ Schedule file validation passed, calling analyzer binary...");

    // Call the analyzer binary and capture JSON output
    let output = std::process::Command::new(&config.analyzer_bin)
        .arg(file_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("❌ Analyzer failed: {}", stderr);
        return Err(format!("Analysis failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The analyzer outputs info to stdout before JSON, so we need to extract just the JSON part
    // Find the line that starts with "✅ Analysis complete!" and take everything after it
    let json_start = if let Some(pos) = stdout.find("✅ Analysis complete!") {
        // Skip past the "Analysis complete!" line and any blank lines
        if let Some(newline_pos) = stdout[pos..].find('\n') {
            pos + newline_pos + 1
        } else {
            pos
        }
    } else {
        0
    };

    let json_output = stdout[json_start..].trim();

    // Validate it's valid JSON
    serde_json::from_str::<serde_json::Value>(json_output)?;

    println!("✅ Comprehensive analysis completed successfully");
    Ok(json_output.to_string())
}

// ============================================================================
// OLD ANALYSIS CODE REMOVED - NOW USING ANALYZER BINARY
// ============================================================================
// The old text-based analysis functions (perform_comprehensive_analysis,
// analyze_all_conflicts, etc.) have been removed. The web server now calls
// the standalone analyzer binary which returns structured JSON.
// ============================================================================

fn generate_schedule_direct(input_file: &str, config: &PathConfig) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Generate unique timestamp for this session
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Use unique filenames for each upload to avoid conflicts
    let unique_source_file = format!("{}/source_{}.xlsx", config.uploads_dir, timestamp);

    // Copy input file to unique location
    println!("📋 Preparing source file: {}", unique_source_file);
    fs::copy(input_file, &unique_source_file)?;

    // Also save a persistent copy for future rebuilds
    let persistent_source = format!("{}/current_source.xlsx", config.uploads_dir);
    fs::copy(input_file, &persistent_source)?;
    println!("📁 Saved source file to {}/current_source.xlsx for future rebuilds", config.uploads_dir);

    // Define output path for generated schedule
    let output_path = format!("{}/schedule_{}.xlsx", config.generated_dir, timestamp);
    println!("🎯 Target output file: {}", output_path);

    // Run the new hierarchical scheduler with input and output arguments
    let scheduler_path = &config.scheduler_bin;
    println!("🚀 Running scheduler: {} {} {}", scheduler_path, unique_source_file, output_path);

    let start_time = std::time::Instant::now();
    let output = std::process::Command::new(scheduler_path)
        .arg(&unique_source_file)
        .arg(&output_path)
        .current_dir(std::env::current_dir()?)
        .output()?;
    let elapsed = start_time.elapsed();

    println!("⏱️  Scheduler execution time: {:.2}s", elapsed.as_secs_f64());

    // Clean up temporary unique source file
    let _ = fs::remove_file(&unique_source_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("❌ Scheduler process failed with stderr: {}", stderr);
        return Err(format!("Schedule generation failed:\n{}", stderr).into());
    }

    println!("✅ Scheduler process completed successfully");

    // Read the generated file and return its contents
    let file_data = fs::read(&output_path)?;
    println!("📊 Generated file size: {} bytes", file_data.len());

    // Clean up generated schedule file
    let _ = fs::remove_file(&output_path);

    Ok(file_data)
}

async fn view_schedule_handler(axum::extract::Path(filename): axum::extract::Path<String>) -> Html<String> {
    let html = include_str!("../../templates/viewer.html");
    let html_with_filename = html.replace("{{FILENAME}}", &filename);
    Html(html_with_filename)
}

async fn get_schedule_json(
    State(config): State<Arc<PathConfig>>,
    axum::extract::Path(filename): axum::extract::Path<String>
) -> Response {
    use calamine::{open_workbook, Reader, Xlsx};
    use serde_json::json;

    let file_path = format!("{}/{}", config.generated_dir, filename);

    let mut workbook: Xlsx<_> = match open_workbook(&file_path) {
        Ok(wb) => wb,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": format!("Failed to open file: {}", e)}).to_string()
            ).into_response();
        }
    };

    let range = match workbook.worksheet_range("Schedule") {
        Ok(range) => range,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": "Schedule sheet not found"}).to_string()
            ).into_response();
        }
    };

    let mut sessions: Vec<serde_json::Value> = Vec::new();
    let mut headers: Vec<String> = Vec::new();

    for (idx, row) in range.rows().enumerate() {
        if idx == 0 {
            headers = row.iter().map(|cell| cell.to_string()).collect();
            continue;
        }

        let mut session = serde_json::Map::new();
        for (col_idx, cell) in row.iter().enumerate() {
            if col_idx < headers.len() {
                let value = json!(cell.to_string());
                session.insert(headers[col_idx].clone(), value);
            }
        }
        sessions.push(json!(session));
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({"sessions": sessions}).to_string()
    ).into_response()
}

#[derive(serde::Deserialize)]
struct ModifiedScheduleRequest {
    sessions: Vec<serde_json::Value>,
    original_filename: String,
}

#[derive(serde::Deserialize, Debug)]
struct RegenerateRequest {
    constraints: serde_json::Value,  // Accept any JSON - could be old or new format
}

// Note: Old constraint mapping function removed - now using priority-based configuration directly

async fn regenerate_handler(
    State(config): State<Arc<PathConfig>>,
    axum::extract::Json(payload): axum::extract::Json<RegenerateRequest>
) -> Response {
    use serde_json::json;

    println!("\n🔄 ========== REGENERATE REQUEST RECEIVED ==========");
    println!("🔄 Regenerate schedule request received with priority-based configuration");

    // Check if we have a source file to regenerate from
    let source_file_path = format!("{}/current_source.xlsx", config.uploads_dir);
    println!("📁 Looking for source file: {}", source_file_path);

    if !std::path::Path::new(&source_file_path).exists() {
        println!("❌ No source file found!");
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "No source file available for regeneration. Please upload a file first."}).to_string()
        ).into_response();
    }

    println!("✅ Source file found");

    // Generate unique timestamp for this regeneration
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Convert constraints to JSON string for passing to test_hierarchical
    let constraints_json = serde_json::to_string(&payload.constraints)
        .unwrap_or_else(|e| {
            eprintln!("❌ Failed to serialize constraints: {}", e);
            "{}".to_string()
        });

    println!("📋 User configuration: {:?}", payload.constraints);
    println!("🎯 Using hierarchical scheduler with priority configuration");

    // Define output path
    let output_filename = format!("regenerated_schedule_{}.xlsx", timestamp);
    let output_path = format!("{}/{}", config.generated_dir, output_filename);
    println!("🎯 Target output file: {}", output_path);

    // Run the test_hierarchical scheduler with JSON configuration as 3rd argument
    // Args: [input_file, output_file, json_config]
    // Use tokio::time::timeout to prevent long-running jobs (60 second limit)
    use tokio::time::{timeout, Duration};

    println!("🚀 Running scheduler: {} {} {} [config]", config.scheduler_bin, source_file_path, output_path);
    println!("⏱️  Starting scheduler execution (60s timeout)...");

    let start_time = tokio::time::Instant::now();
    let scheduler_task = async {
        tokio::process::Command::new(&config.scheduler_bin)
            .args(&[&source_file_path, &output_path, &constraints_json])
            .current_dir(std::env::current_dir().unwrap())
            .output()
            .await
    };

    let output_result = timeout(Duration::from_secs(60), scheduler_task).await;
    let elapsed = start_time.elapsed();

    match output_result {
        Err(_) => {
            println!("❌ Scheduler timed out after {:.2}s (limit: 60s)", elapsed.as_secs_f64());
            println!("========================================\n");
            (
                StatusCode::REQUEST_TIMEOUT,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": "Schedule regeneration timed out after 60 seconds. This priority configuration may be too complex. Try using recommended default priorities (Personal conflicts first, Classification last)."
                }).to_string()
            ).into_response()
        }
        Ok(output) => match output {
        Ok(result) => {
            println!("⏱️  Scheduler execution time: {:.2}s", elapsed.as_secs_f64());

            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);

            if !stdout.is_empty() {
                println!("📋 Scheduler stdout: {}", stdout);
            }
            if !stderr.is_empty() {
                eprintln!("⚠️  Scheduler stderr: {}", stderr);
            }

            if result.status.success() {
                println!("✅ Scheduler process completed successfully");

                // Check if output file was created
                if std::path::Path::new(&output_path).exists() {
                    println!("✅ Output file created: {}", output_filename);
                    println!("🎉 Schedule regeneration complete!");
                    println!("========================================\n");
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        json!({
                            "success": true,
                            "filename": output_filename,
                            "message": "Schedule regenerated successfully with priority-based optimization!",
                            "view_url": format!("/view/{}", output_filename)
                        }).to_string()
                    ).into_response()
                } else {
                    println!("❌ Output file was not created: {}", output_path);
                    println!("========================================\n");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [(header::CONTENT_TYPE, "application/json")],
                        json!({"error": "Scheduler completed but output file was not created"}).to_string()
                    ).into_response()
                }
            } else {
                println!("❌ Scheduler failed with status: {:?}", result.status);
                println!("========================================\n");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({"error": format!("Schedule regeneration failed: {}\nstderr: {}", result.status, stderr)}).to_string()
                ).into_response()
            }
        }
        Err(e) => {
            println!("❌ Failed to execute scheduler: {}", e);
            println!("========================================\n");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": format!("Failed to execute scheduler: {}", e)}).to_string()
            ).into_response()
        }
        }
    }
}

#[derive(serde::Deserialize)]
struct AnalyzeRequest {
    sessions: Vec<serde_json::Value>,
}

async fn analyze_schedule(
    axum::extract::Json(payload): axum::extract::Json<AnalyzeRequest>
) -> Response {
    use serde_json::json;
    use conference_scheduler::conflict_detection;
    use std::collections::HashMap;

    // Count conflicts using the same logic as the analyzer
    let mut classification_conflicts = 0;
    let mut personal_conflicts = 0;
    let mut personal_classification_conflicts = 0;
    let mut series_room_conflicts = 0;
    let mut series_back_to_back_conflicts = 0;

    // Group sessions by time period
    let mut by_period: HashMap<String, Vec<&serde_json::Value>> = HashMap::new();

    for session in &payload.sessions {
        if let Some(period) = session.get("Session").and_then(|v| v.as_str()) {
            by_period.entry(period.to_string()).or_default().push(session);
        }
    }

    // Check conflicts within each time period (concurrent conflicts)
    for (_period, sessions) in &by_period {
        for i in 0..sessions.len() {
            for j in (i + 1)..sessions.len() {
                let s1 = sessions[i];
                let s2 = sessions[j];

                // Classification conflicts
                let class1_str = s1.get("Classification").and_then(|v| v.as_str()).unwrap_or("");
                let class2_str = s2.get("Classification").and_then(|v| v.as_str()).unwrap_or("");
                let class1: Vec<String> = class1_str.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                let class2: Vec<String> = class2_str.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

                if conflict_detection::has_classification_conflict(&class1, &class2) {
                    classification_conflicts += 1;
                }

                // Personal conflicts (speakers/organizers)
                let speakers1 = s1.get("Speakers").and_then(|v| v.as_str()).unwrap_or("");
                let organizers1 = s1.get("Organizers").and_then(|v| v.as_str()).unwrap_or("");
                let speakers2 = s2.get("Speakers").and_then(|v| v.as_str()).unwrap_or("");
                let organizers2 = s2.get("Organizers").and_then(|v| v.as_str()).unwrap_or("");

                let spk1: Vec<String> = speakers1.split(&[';', ',', '|'][..]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                let org1: Vec<String> = organizers1.split(&[';', ',', '|'][..]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                let spk2: Vec<String> = speakers2.split(&[';', ',', '|'][..]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                let org2: Vec<String> = organizers2.split(&[';', ',', '|'][..]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

                if conflict_detection::has_speaker_organizer_conflict(&spk1, &org1, &spk2, &org2) {
                    personal_conflicts += 1;
                }

                // Personal-classification conflicts (Notes field)
                let notes1 = s1.get("Notes").and_then(|v| v.as_str()).unwrap_or("");
                let notes2 = s2.get("Notes").and_then(|v| v.as_str()).unwrap_or("");

                if conflict_detection::has_personal_classification_conflict(notes1, notes2) {
                    personal_classification_conflicts += 1;
                }
            }
        }
    }

    // Check series conflicts across ALL sessions (not just within periods)
    for i in 0..payload.sessions.len() {
        for j in (i + 1)..payload.sessions.len() {
            let s1 = &payload.sessions[i];
            let s2 = &payload.sessions[j];

            let title1 = s1.get("Title").and_then(|v| v.as_str()).unwrap_or("");
            let title2 = s2.get("Title").and_then(|v| v.as_str()).unwrap_or("");
            let room1 = s1.get("Room").and_then(|v| v.as_str()).unwrap_or("");
            let room2 = s2.get("Room").and_then(|v| v.as_str()).unwrap_or("");
            let period1 = s1.get("Session").and_then(|v| v.as_str()).unwrap_or("");
            let period2 = s2.get("Session").and_then(|v| v.as_str()).unwrap_or("");

            // Series room consistency conflict
            if conflict_detection::has_series_room_conflict(title1, room1, title2, room2) {
                series_room_conflicts += 1;
            }

            // Series back-to-back conflict
            if conflict_detection::has_series_back_to_back_conflict(title1, period1, title2, period2) {
                series_back_to_back_conflicts += 1;
            }
        }
    }

    let total_conflicts = classification_conflicts + personal_conflicts + personal_classification_conflicts
                         + series_room_conflicts + series_back_to_back_conflicts;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "classificationConflicts": classification_conflicts,
            "personalConflicts": personal_conflicts,
            "personalClassificationConflicts": personal_classification_conflicts,
            "seriesRoomConflicts": series_room_conflicts,
            "seriesBackToBackConflicts": series_back_to_back_conflicts,
            "totalConflicts": total_conflicts
        }).to_string()
    ).into_response()
}

async fn save_modified_schedule(
    State(config): State<Arc<PathConfig>>,
    axum::extract::Json(payload): axum::extract::Json<ModifiedScheduleRequest>
) -> Response {
    use serde_json::json;

    println!("📝 Save modified schedule request received");

    // Generate unique filename
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let output_filename = format!("modified_schedule_{}.xlsx", timestamp);
    let output_path = format!("{}/{}", config.generated_dir, output_filename);

    println!("  Output file: {}", output_path);

    // Step 1: Write sessions to Excel (creates base file with Schedule sheet)
    println!("  Writing sessions to Excel...");
    if let Err(e) = write_sessions_to_excel(&output_path, &payload.sessions) {
        eprintln!("❌ Failed to write Excel file: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": format!("Failed to create Excel file: {}", e)}).to_string()
        ).into_response();
    }

    // Step 2: Enhance with conflict analysis (appends to same Schedule sheet)
    println!("  Adding conflict analysis...");
    match create_enhanced_excel(&output_path, &config).await {
        Ok(enhanced_data) => {
            // Write the enhanced data back to the file
            if let Err(e) = fs::write(&output_path, &enhanced_data) {
                eprintln!("❌ Failed to write enhanced Excel: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({"error": format!("Failed to save enhanced Excel: {}", e)}).to_string()
                ).into_response();
            }
            println!("✅ Modified schedule saved successfully with conflict analysis: {}", output_filename);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json!({
                    "success": true,
                    "filename": output_filename,
                    "download_url": format!("/download/{}", output_filename)
                }).to_string()
            ).into_response()
        }
        Err(e) => {
            eprintln!("⚠️  Excel created but analysis failed: {}", e);
            // Still return success since base file was created
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json!({
                    "success": true,
                    "filename": output_filename,
                    "download_url": format!("/download/{}", output_filename),
                    "warning": format!("Schedule saved but conflict analysis unavailable: {}", e)
                }).to_string()
            ).into_response()
        }
    }
}
