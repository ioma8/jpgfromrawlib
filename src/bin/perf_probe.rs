use anyhow::{Result, ensure};
use jpgfromraw::{FindJpegType, process_file_bytes};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

struct Args {
    input_dir: PathBuf,
    output_dir: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args_os().skip(1);
    let input_dir = args
        .next()
        .map_or_else(|| PathBuf::from("test_images"), PathBuf::from);
    let output_dir = args.next().map_or_else(
        || PathBuf::from("perf_artifacts/latest_outputs"),
        PathBuf::from,
    );

    ensure!(
        args.next().is_none(),
        "usage: perf_probe [input_dir] [output_dir]"
    );

    Ok(Args {
        input_dir,
        output_dir,
    })
}

fn collect_raw_files(input_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(input_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(OsStr::to_str)
                .is_some_and(|ext| ext.eq_ignore_ascii_case("arw"))
        })
        .collect::<Vec<_>>();

    files.sort();
    ensure!(
        !files.is_empty(),
        "no .arw files found in {}",
        input_dir.display()
    );
    Ok(files)
}

async fn run() -> Result<()> {
    let args = parse_args()?;
    let files = collect_raw_files(&args.input_dir)?;
    let semaphore = Arc::new(Semaphore::new(2));
    let mut tasks = JoinSet::new();

    std::fs::create_dir_all(&args.output_dir)?;

    for file in files {
        let output_dir = args.output_dir.clone();
        let permit = semaphore.clone().acquire_owned().await?;
        tasks.spawn(async move {
            let _permit = permit;
            let jpeg_data = process_file_bytes(&file, FindJpegType::Smallest).await?;
            let output_path = output_path(&output_dir, &file)?;
            std::fs::write(output_path, jpeg_data)?;
            Ok::<(), anyhow::Error>(())
        });

        if tasks.len() >= 2
            && let Some(task) = tasks.join_next().await
        {
            task??;
        }
    }

    while let Some(task) = tasks.join_next().await {
        task??;
    }

    Ok(())
}

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run())
}

fn output_path(output_dir: &Path, input_path: &Path) -> Result<PathBuf> {
    let stem = input_path
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("missing file stem for {}", input_path.display()))?;
    let mut output_path = output_dir.join(stem);
    output_path.set_extension("jpg");
    Ok(output_path)
}
