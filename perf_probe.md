# perf_probe.sh

`perf_probe.sh` benchmarks thumbnail extraction from RAW files in `test_images` and verifies that the generated JPEG outputs still match the saved baseline.

## Usage

Run from the repo root:

```bash
./perf_probe.sh
```

This will:

- build `target/release/perf_probe`
- run `hyperfine` against the probe binary
- write fresh outputs to `perf_artifacts/latest/outputs`
- compare their SHA-256 manifest with `perf_artifacts/baseline/sha256.txt`

## Baseline

The saved baseline lives in `perf_artifacts/baseline/`.

To intentionally refresh it:

```bash
UPDATE_BASELINE=1 ./perf_probe.sh
```

## Optional env vars

- `INPUT_DIR`: override the input RAW directory
- `RUNS`: number of `hyperfine` runs, default `10`
- `WARMUPS`: number of warmup runs, default `3`
- `ARTIFACT_ROOT`: override the `perf_artifacts` directory
