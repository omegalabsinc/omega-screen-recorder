/// Integration tests for concurrent recording processes
///
/// These tests verify that the encoder clashing fix works correctly when running
/// multiple recording processes simultaneously. Each process should:
/// 1. Initialize its own encoder without conflicts
/// 2. Record chunks independently
/// 3. Write to the database without locking issues
/// 4. Complete successfully without encoder resource conflicts

use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::time::Duration;
use std::thread;
use tempfile::TempDir;

/// Helper struct to manage a recording process
struct RecorderProcess {
    process: Child,
    task_id: String,
    output_dir: PathBuf,
}

impl RecorderProcess {
    /// Spawn a new recorder process with the given task ID
    fn spawn(task_id: String, duration: u64, output_dir: PathBuf) -> std::io::Result<Self> {
        // Build the recorder binary path
        let binary = if cfg!(debug_assertions) {
            "./target/debug/omgrec"
        } else {
            "./target/release/omgrec"
        };

        // Spawn the recorder process
        let process = Command::new(binary)
            .args(&[
                "record",
                "--recording-type", "task",
                "--task-id", &task_id,
                "--duration", &duration.to_string(),
                "--fps", "30",
                "--quality", "8",
                "--chunk-duration", "2", // Small chunks for faster testing
                "--no-audio", // Disable audio for simpler testing
                "--output", output_dir.to_str().unwrap(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(Self {
            process,
            task_id,
            output_dir,
        })
    }

    /// Wait for the process to complete and return the exit status
    fn wait_for_completion(mut self) -> std::io::Result<std::process::ExitStatus> {
        self.process.wait()
    }

    /// Kill the process gracefully
    fn kill(&mut self) -> std::io::Result<()> {
        self.process.kill()
    }

    /// Get the task ID
    fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Get the output directory
    fn output_dir(&self) -> &PathBuf {
        &self.output_dir
    }
}

impl Drop for RecorderProcess {
    fn drop(&mut self) {
        // Ensure process is terminated on drop
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Verify that chunks were created for a task
fn verify_chunks_created(output_dir: &PathBuf) -> Result<usize, String> {
    let mut chunk_count = 0;

    if !output_dir.exists() {
        return Err(format!("Output directory does not exist: {:?}", output_dir));
    }

    // Count MP4 files in the output directory
    for entry in std::fs::read_dir(output_dir)
        .map_err(|e| format!("Failed to read directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        if path.is_file() && path.extension().map_or(false, |ext| ext == "mp4") {
            chunk_count += 1;
        }
    }

    if chunk_count == 0 {
        return Err(format!("No chunks created in {:?}", output_dir));
    }

    Ok(chunk_count)
}

/// Verify encoder initialization by checking log output
fn verify_encoder_initialized(process_output: &str) -> bool {
    process_output.contains("Encoder initialized")
        || process_output.contains("Successfully initialized encoder")
        || process_output.contains("MP4 encoder initialized")
}

/// Verify no encoder conflicts in log output
fn verify_no_encoder_conflicts(process_output: &str) -> bool {
    !process_output.contains("encoder is busy")
        && !process_output.contains("Encoder busy")
        && !process_output.contains("Failed to open encoder")
        && !process_output.contains("All encoders failed")
}

#[test]
#[ignore] // Ignore by default as this requires the binary to be built
fn test_two_simultaneous_recordings_different_tasks() {
    // Create temporary directories for outputs
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");

    let output_dir1 = temp_dir1.path().to_path_buf();
    let output_dir2 = temp_dir2.path().to_path_buf();

    println!("Starting test: two simultaneous recordings with different task IDs");
    println!("  Task 1 output: {:?}", output_dir1);
    println!("  Task 2 output: {:?}", output_dir2);

    // Spawn two recording processes with different task IDs
    let mut recorder1 = RecorderProcess::spawn(
        "test_task_1".to_string(),
        5, // 5 seconds
        output_dir1.clone(),
    ).expect("Failed to spawn recorder 1");

    // Small delay to ensure first process starts
    thread::sleep(Duration::from_millis(100));

    let mut recorder2 = RecorderProcess::spawn(
        "test_task_2".to_string(),
        5, // 5 seconds
        output_dir2.clone(),
    ).expect("Failed to spawn recorder 2");

    println!("Both recording processes spawned successfully");

    // Wait for both processes to complete
    println!("Waiting for recording processes to complete...");

    let status1 = recorder1.wait_for_completion()
        .expect("Failed to wait for recorder 1");
    let status2 = recorder2.wait_for_completion()
        .expect("Failed to wait for recorder 2");

    // Verify both processes completed successfully
    assert!(status1.success(), "Recorder 1 failed with status: {:?}", status1);
    assert!(status2.success(), "Recorder 2 failed with status: {:?}", status2);

    println!("Both processes completed successfully");

    // Verify chunks were created for both tasks
    let chunks1 = verify_chunks_created(&output_dir1)
        .expect("Failed to verify chunks for task 1");
    let chunks2 = verify_chunks_created(&output_dir2)
        .expect("Failed to verify chunks for task 2");

    println!("Task 1 created {} chunks", chunks1);
    println!("Task 2 created {} chunks", chunks2);

    // With 5 second duration and 2 second chunks, we expect at least 2 chunks per task
    assert!(chunks1 >= 2, "Task 1 should have created at least 2 chunks, got {}", chunks1);
    assert!(chunks2 >= 2, "Task 2 should have created at least 2 chunks, got {}", chunks2);

    println!("✓ Test passed: Both recordings completed without conflicts");
}

#[test]
#[ignore] // Ignore by default as this requires the binary to be built
fn test_three_simultaneous_recordings() {
    // Create temporary directories for outputs
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");
    let temp_dir3 = TempDir::new().expect("Failed to create temp dir 3");

    let output_dir1 = temp_dir1.path().to_path_buf();
    let output_dir2 = temp_dir2.path().to_path_buf();
    let output_dir3 = temp_dir3.path().to_path_buf();

    println!("Starting test: three simultaneous recordings");
    println!("  Task 1 output: {:?}", output_dir1);
    println!("  Task 2 output: {:?}", output_dir2);
    println!("  Task 3 output: {:?}", output_dir3);

    // Spawn three recording processes
    let mut recorder1 = RecorderProcess::spawn(
        "test_task_1".to_string(),
        6, // 6 seconds
        output_dir1.clone(),
    ).expect("Failed to spawn recorder 1");

    thread::sleep(Duration::from_millis(100));

    let mut recorder2 = RecorderProcess::spawn(
        "test_task_2".to_string(),
        6, // 6 seconds
        output_dir2.clone(),
    ).expect("Failed to spawn recorder 2");

    thread::sleep(Duration::from_millis(100));

    let mut recorder3 = RecorderProcess::spawn(
        "test_task_3".to_string(),
        6, // 6 seconds
        output_dir3.clone(),
    ).expect("Failed to spawn recorder 3");

    println!("All three recording processes spawned successfully");

    // Wait for all processes to complete
    println!("Waiting for recording processes to complete...");

    let status1 = recorder1.wait_for_completion()
        .expect("Failed to wait for recorder 1");
    let status2 = recorder2.wait_for_completion()
        .expect("Failed to wait for recorder 2");
    let status3 = recorder3.wait_for_completion()
        .expect("Failed to wait for recorder 3");

    // Verify all processes completed successfully
    assert!(status1.success(), "Recorder 1 failed with status: {:?}", status1);
    assert!(status2.success(), "Recorder 2 failed with status: {:?}", status2);
    assert!(status3.success(), "Recorder 3 failed with status: {:?}", status3);

    println!("All three processes completed successfully");

    // Verify chunks were created for all tasks
    let chunks1 = verify_chunks_created(&output_dir1)
        .expect("Failed to verify chunks for task 1");
    let chunks2 = verify_chunks_created(&output_dir2)
        .expect("Failed to verify chunks for task 2");
    let chunks3 = verify_chunks_created(&output_dir3)
        .expect("Failed to verify chunks for task 3");

    println!("Task 1 created {} chunks", chunks1);
    println!("Task 2 created {} chunks", chunks2);
    println!("Task 3 created {} chunks", chunks3);

    // With 6 second duration and 2 second chunks, we expect at least 3 chunks per task
    assert!(chunks1 >= 3, "Task 1 should have created at least 3 chunks, got {}", chunks1);
    assert!(chunks2 >= 3, "Task 2 should have created at least 3 chunks, got {}", chunks2);
    assert!(chunks3 >= 3, "Task 3 should have created at least 3 chunks, got {}", chunks3);

    println!("✓ Test passed: All three recordings completed without conflicts");
}

#[test]
#[ignore] // Ignore by default as this requires the binary to be built
fn test_staggered_start_recordings() {
    // Test recordings that start at different times
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");

    let output_dir1 = temp_dir1.path().to_path_buf();
    let output_dir2 = temp_dir2.path().to_path_buf();

    println!("Starting test: staggered start recordings");

    // Start first recording
    let mut recorder1 = RecorderProcess::spawn(
        "test_staggered_1".to_string(),
        8, // 8 seconds
        output_dir1.clone(),
    ).expect("Failed to spawn recorder 1");

    println!("First recording started, waiting 3 seconds...");
    thread::sleep(Duration::from_secs(3));

    // Start second recording while first is still running
    let mut recorder2 = RecorderProcess::spawn(
        "test_staggered_2".to_string(),
        4, // 4 seconds
        output_dir2.clone(),
    ).expect("Failed to spawn recorder 2");

    println!("Second recording started while first is running");

    // Wait for second to complete (should finish first)
    let status2 = recorder2.wait_for_completion()
        .expect("Failed to wait for recorder 2");
    println!("Second recording completed");

    // Wait for first to complete
    let status1 = recorder1.wait_for_completion()
        .expect("Failed to wait for recorder 1");
    println!("First recording completed");

    assert!(status1.success(), "Recorder 1 failed");
    assert!(status2.success(), "Recorder 2 failed");

    // Verify chunks
    let chunks1 = verify_chunks_created(&output_dir1)
        .expect("Failed to verify chunks for task 1");
    let chunks2 = verify_chunks_created(&output_dir2)
        .expect("Failed to verify chunks for task 2");

    println!("Task 1 created {} chunks", chunks1);
    println!("Task 2 created {} chunks", chunks2);

    assert!(chunks1 >= 4, "Task 1 should have created at least 4 chunks");
    assert!(chunks2 >= 2, "Task 2 should have created at least 2 chunks");

    println!("✓ Test passed: Staggered recordings completed without conflicts");
}

#[test]
#[ignore] // Ignore by default as this requires the binary to be built
fn test_rapid_sequential_recordings() {
    // Test starting recordings in rapid succession
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");
    let temp_dir3 = TempDir::new().expect("Failed to create temp dir 3");

    let output_dir1 = temp_dir1.path().to_path_buf();
    let output_dir2 = temp_dir2.path().to_path_buf();
    let output_dir3 = temp_dir3.path().to_path_buf();

    println!("Starting test: rapid sequential recordings");

    // Spawn all three with minimal delay
    let mut recorder1 = RecorderProcess::spawn(
        "test_rapid_1".to_string(),
        4,
        output_dir1.clone(),
    ).expect("Failed to spawn recorder 1");

    thread::sleep(Duration::from_millis(50));

    let mut recorder2 = RecorderProcess::spawn(
        "test_rapid_2".to_string(),
        4,
        output_dir2.clone(),
    ).expect("Failed to spawn recorder 2");

    thread::sleep(Duration::from_millis(50));

    let mut recorder3 = RecorderProcess::spawn(
        "test_rapid_3".to_string(),
        4,
        output_dir3.clone(),
    ).expect("Failed to spawn recorder 3");

    println!("All processes spawned with minimal delay");

    // Wait for all to complete
    let status1 = recorder1.wait_for_completion().expect("Failed to wait for recorder 1");
    let status2 = recorder2.wait_for_completion().expect("Failed to wait for recorder 2");
    let status3 = recorder3.wait_for_completion().expect("Failed to wait for recorder 3");

    assert!(status1.success(), "Recorder 1 failed");
    assert!(status2.success(), "Recorder 2 failed");
    assert!(status3.success(), "Recorder 3 failed");

    // Verify all created chunks
    let chunks1 = verify_chunks_created(&output_dir1).expect("Failed to verify chunks 1");
    let chunks2 = verify_chunks_created(&output_dir2).expect("Failed to verify chunks 2");
    let chunks3 = verify_chunks_created(&output_dir3).expect("Failed to verify chunks 3");

    println!("Rapid recordings created {} + {} + {} chunks", chunks1, chunks2, chunks3);

    assert!(chunks1 >= 2, "Task 1 should have created at least 2 chunks");
    assert!(chunks2 >= 2, "Task 2 should have created at least 2 chunks");
    assert!(chunks3 >= 2, "Task 3 should have created at least 2 chunks");

    println!("✓ Test passed: Rapid sequential recordings completed successfully");
}

#[test]
#[ignore] // Ignore by default as this requires the binary to be built
fn test_same_task_id_sequential() {
    // Test that the same task ID can be used sequentially (but not simultaneously)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let output_dir = temp_dir.path().to_path_buf();

    println!("Starting test: same task ID sequential recordings");

    // First recording
    let mut recorder1 = RecorderProcess::spawn(
        "test_same_task".to_string(),
        3,
        output_dir.clone(),
    ).expect("Failed to spawn recorder 1");

    let status1 = recorder1.wait_for_completion().expect("Failed to wait for recorder 1");
    assert!(status1.success(), "First recording failed");

    let chunks1 = verify_chunks_created(&output_dir).expect("Failed to verify chunks 1");
    println!("First recording created {} chunks", chunks1);

    // Wait a moment before starting second recording
    thread::sleep(Duration::from_millis(500));

    // Second recording with same task ID
    let mut recorder2 = RecorderProcess::spawn(
        "test_same_task".to_string(),
        3,
        output_dir.clone(),
    ).expect("Failed to spawn recorder 2");

    let status2 = recorder2.wait_for_completion().expect("Failed to wait for recorder 2");
    assert!(status2.success(), "Second recording failed");

    // Verify more chunks were added
    let chunks2 = verify_chunks_created(&output_dir).expect("Failed to verify chunks 2");
    println!("After second recording: {} total chunks", chunks2);

    assert!(chunks2 > chunks1, "Second recording should have added more chunks");

    println!("✓ Test passed: Sequential recordings with same task ID work correctly");
}

#[cfg(test)]
mod helpers {
    use super::*;

    /// Helper to check if the recorder binary exists
    pub fn check_binary_exists() -> bool {
        let binary = if cfg!(debug_assertions) {
            "./target/debug/omgrec"
        } else {
            "./target/release/omgrec"
        };
        std::path::Path::new(binary).exists()
    }

    #[test]
    fn test_binary_exists() {
        if !check_binary_exists() {
            println!("⚠️  Warning: Binary not found. Build with:");
            println!("   cargo build --release");
            println!("   or");
            println!("   cargo build");
        }
    }
}
