use anyhow::{Result, bail};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use jpgfromraw::SUPPORTED_EXTENSIONS;
use jpgfromraw::process_file;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::task::JoinSet;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    /// Input directory containing RAW files
    input_dir: PathBuf,

    /// Output directory to store extracted JPEGs
    #[arg(default_value = ".")]
    output_dir: PathBuf,

    /// How many files to process at once
    #[arg(short, long, default_value_t = 8, value_parser = parse_transfers)]
    transfers: usize,

    /// Look for this extension in addition to the default list.
    ///
    /// Default list covers common RAW and cinema extensions including 3fr, ari, arw, bay, braw,
    /// cap, cr2, cr3, cri, crw, dcr, dcs, dng, drf, eip, erf, fff, gpr, iiq, jxs, k25, kdc, mdc,
    /// mef, mos, mrw, nef, nrw, orf, pef, ptx, pxn, r3d, raf, raw, rw2, rwl, rwz, sr2, srf, srw,
    /// tco, x3f.
    #[arg(short, long)]
    extension: Option<OsString>,
}

struct ProcessingResult {
    result: Result<()>,
    path: PathBuf,
}

fn parse_transfers(value: &str) -> std::result::Result<usize, String> {
    let transfers = value
        .parse::<usize>()
        .map_err(|err| format!("invalid transfers value: {err}"))?;
    if transfers == 0 {
        return Err("transfers must be at least 1".to_string());
    }
    Ok(transfers)
}

/// Recursively process a directory of RAW files, extracting embedded JPEGs and writing them to the
/// output directory.
///
/// This function recursively searches the input directory for RAW files with valid extensions,
/// processes each file to extract the embedded JPEG, and writes the JPEGs to the corresponding
/// location in the output directory. The directory structure relative to the input directory is
/// maintained.
async fn process_directory(
    in_dir: &Path,
    out_dir: Arc<PathBuf>,
    ext: Option<OsString>,
    transfers: usize,
) -> Result<()> {
    let is_valid_extension = |candidate: &OsStr| {
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|supported| candidate.eq_ignore_ascii_case(supported))
            || ext
                .as_deref()
                .is_some_and(|extra| candidate.eq_ignore_ascii_case(extra))
    };

    let mut dir_queue = vec![in_dir.to_path_buf()];
    let progress_bar = ProgressBar::new_spinner();
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{pos} processed [{spinner}]")?
            .progress_chars("##-"),
    );

    let mut tasks = JoinSet::new();
    let mut nr_failed = 0;

    while let Some(current_dir) = dir_queue.pop() {
        let mut read_dir = fs::read_dir(&current_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                dir_queue.push(path);
            } else if path.extension().is_some_and(is_valid_extension) {
                if tasks.len() >= transfers {
                    collect_processing_result(
                        tasks.join_next().await,
                        &progress_bar,
                        &mut nr_failed,
                    )?;
                }
                let relative_path = path.strip_prefix(in_dir)?.to_path_buf();
                let task_progress_bar = progress_bar.clone();
                let out_dir = Arc::clone(&out_dir);
                tasks.spawn(async move {
                    let result = async {
                        process_file(
                            &path,
                            out_dir.as_path(),
                            &relative_path,
                            jpgfromraw::FindJpegType::Largest,
                        )
                        .await?;
                        Ok(())
                    }
                    .await;
                    task_progress_bar.inc(1);
                    Ok(ProcessingResult { result, path })
                });
            }
        }
    }

    while !tasks.is_empty() {
        collect_processing_result(tasks.join_next().await, &progress_bar, &mut nr_failed)?;
    }

    progress_bar.abandon();

    if nr_failed != 0 {
        bail!("Failed to process {} files", nr_failed);
    }

    Ok(())
}

fn collect_processing_result(
    task_result: Option<std::result::Result<Result<ProcessingResult>, tokio::task::JoinError>>,
    progress_bar: &ProgressBar,
    nr_failed: &mut usize,
) -> Result<()> {
    let Some(task_result) = task_result else {
        return Ok(());
    };
    let pr_res = task_result??;
    if let Err(e) = pr_res.result {
        *nr_failed += 1;
        let msg = format!("Error processing file {}: {:?}", pr_res.path.display(), e);
        progress_bar.println(msg);
    }
    Ok(())
}

async fn run(args: Args) -> Result<()> {
    let output_dir = Arc::new(args.output_dir);

    fs::create_dir_all(output_dir.as_path()).await?;
    process_directory(&args.input_dir, output_dir, args.extension, args.transfers).await?;

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let worker_threads = args.transfers.max(1);

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()?
        .block_on(run(args))
}

#[cfg(test)]
mod tests {
    use super::parse_transfers;

    #[test]
    fn parse_transfers_rejects_zero() {
        assert!(parse_transfers("0").is_err());
        assert_eq!(parse_transfers("1").unwrap(), 1);
    }
}
