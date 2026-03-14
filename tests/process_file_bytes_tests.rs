use anyhow::Result;
use jpgfromraw::parser::{process_file_bytes, FindJpegType};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs;

fn file_name_display(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Path to a directory containing test RAW files
const TEST_RAW_DIR: &str = "/Users/jakubkolcar/Pictures/2024/2024-12-24";

#[tokio::test]
async fn test_process_file_bytes_on_directory() -> Result<()> {
    // Ensure the test directory exists
    if !Path::new(TEST_RAW_DIR).exists() {
        eprintln!("Test directory not found: {TEST_RAW_DIR}");
        eprintln!("Please create this directory with some RAW files for testing");
        return Ok(());
    }

    let mut entries = fs::read_dir(TEST_RAW_DIR).await?;
    let mut success_count = 0;
    let mut failure_count = 0;
    let mut total_size = 0;
    let overall_start = Instant::now();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Skip non-files and files without extensions
        if !path.is_file() || path.extension().is_none() {
            continue;
        }

        println!("Processing: {}", path.display());
        let file_start = Instant::now();

        match process_file_bytes(&path, FindJpegType::Largest).await {
            Ok(jpeg_data) => {
                success_count += 1;
                total_size += jpeg_data.len();
                println!(
                    "✅ Success: {} bytes in {:?}",
                    jpeg_data.len(),
                    file_start.elapsed()
                );
            }
            Err(e) => {
                failure_count += 1;
                let file_name = file_name_display(&path);
                println!("❌ Failed: {file_name} - {e}");
            }
        }

        println!("---");
    }

    let total_time = overall_start.elapsed();
    println!("Test complete:");
    println!("  Processed: {} files", success_count + failure_count);
    println!("  Successful: {success_count} files");
    println!("  Failed: {failure_count} files");
    println!("  Total JPEG data: {total_size} bytes");
    println!("  Total time: {total_time:?}");

    if failure_count > 0 {
        println!("⚠️  Warning: Some files failed to process");
    }

    Ok(())
}

#[tokio::test]
async fn test_process_file_bytes_with_different_find_types() -> Result<()> {
    // Ensure the test directory exists
    if !Path::new(TEST_RAW_DIR).exists() {
        eprintln!("Test directory not found: {TEST_RAW_DIR}");
        eprintln!("Please create this directory with some RAW files for testing");
        return Ok(());
    }

    let mut entries = fs::read_dir(TEST_RAW_DIR).await?;
    let mut files_tested = 0;

    // Test only first 5 files to keep the test run time reasonable
    'outer: while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if !path.is_file() || path.extension().is_none() {
            continue;
        }

        println!(
            "\nTesting both FindJpegType variants on: {}",
            path.display()
        );

        // Test with Largest
        match process_file_bytes(&path, FindJpegType::Largest).await {
            Ok(largest_jpeg) => {
                println!("Largest JPEG size: {} bytes", largest_jpeg.len());

                // Test with Smallest
                match process_file_bytes(&path, FindJpegType::Smallest).await {
                    Ok(smallest_jpeg) => {
                        println!("Smallest JPEG size: {} bytes", smallest_jpeg.len());

                        // Verify the types work as expected
                        if largest_jpeg.len() >= smallest_jpeg.len() {
                            println!("✅ Verified: Largest >= Smallest");
                        } else {
                            println!("❌ Error: Largest < Smallest");
                        }
                    }
                    Err(e) => println!("❌ Failed to get smallest JPEG: {e}"),
                }
            }
            Err(e) => println!("❌ Failed to get largest JPEG: {e}"),
        }

        files_tested += 1;
        if files_tested >= 5 {
            break 'outer;
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_process_file_bytes_performance_benchmark() -> Result<()> {
    // Ensure the test directory exists
    if !Path::new(TEST_RAW_DIR).exists() {
        eprintln!("Test directory not found: {TEST_RAW_DIR}");
        eprintln!("Please create this directory with some RAW files for testing");
        return Ok(());
    }

    let mut entries = fs::read_dir(TEST_RAW_DIR).await?;
    let mut times = Vec::new();
    let mut sizes = Vec::new();
    let mut count = 0;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if !path.is_file() || path.extension().is_none() {
            continue;
        }

        // Limit to 20 files for the benchmark
        if count >= 20 {
            break;
        }

        let start = Instant::now();
        match process_file_bytes(&path, FindJpegType::Largest).await {
            Ok(jpeg_data) => {
                let elapsed = start.elapsed();
                times.push(elapsed);
                sizes.push(jpeg_data.len());
                count += 1;
                let file_name = file_name_display(&path);
                println!(
                    "Processed file {file_name}: {} bytes in {elapsed:?}",
                    jpeg_data.len()
                );
            }
            Err(e) => {
                // Skip failed files in benchmark
                let file_name = file_name_display(&path);
                println!("❌ Failed to process {file_name}: {e}");
            }
        }
    }

    if times.is_empty() {
        println!("No files were successfully processed for benchmark");
    } else {
        let total_seconds = times.iter().map(Duration::as_secs_f64).sum::<f64>();
        let files_processed = f64::from(u32::try_from(times.len())?);
        let total_size = sizes.iter().sum::<usize>();
        let total_size_f64 = f64::from(u32::try_from(total_size)?);
        let avg_time_ms = (total_seconds / files_processed) * 1000.0;
        let avg_size_kib = (total_size_f64 / files_processed) / 1024.0;
        let throughput_mib_s = (total_size_f64 / total_seconds) / (1024.0 * 1024.0);

        println!("Performance benchmark results:");
        println!("  Files processed: {}", times.len());
        println!("  Average processing time: {avg_time_ms:.2} ms");
        println!("  Average JPEG size: {avg_size_kib:.2} KB");
        println!("  Throughput: {throughput_mib_s:.2} MB/s");
    }

    Ok(())
}
